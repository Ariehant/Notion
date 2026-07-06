//! SSRF guard — audit §2.7.
//!
//! The blueprint scrapes user-supplied URLs but adds no SSRF protection, so a
//! malicious note could make the app fetch `http://169.254.169.254/…` (cloud
//! metadata), `http://127.0.0.1`, private-range hosts, or `file://`. `robots.txt`
//! is **not** an SSRF control.
//!
//! This module enforces, before any fetch:
//!   * scheme allowlist — only `http` / `https`;
//!   * a host must be present;
//!   * any IP literal, and every DNS-resolved address, must not be loopback,
//!     link-local (incl. `169.254.169.254`), private, unique-local, CGNAT,
//!     unspecified, multicast, or otherwise non-public;
//!   * redirects are capped at [`MAX_REDIRECTS`] and each hop is re-checked by
//!     the caller.
//!
//! The IP classification ([`is_blocked_ip`]) is pure and fully unit-tested. DNS
//! resolution is the caller's responsibility (it is I/O); [`guard_url`] blocks
//! IP-literal hosts synchronously and returns the host to resolve otherwise.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use thiserror::Error;
use url::{Host, Url};

/// Maximum number of redirects to follow when fetching (§2.7).
pub const MAX_REDIRECTS: u8 = 5;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum UrlGuardError {
    #[error("could not parse URL")]
    Parse,
    #[error("scheme not allowed (only http/https)")]
    SchemeNotAllowed,
    #[error("URL has no host")]
    MissingHost,
    #[error("host resolves to a blocked (non-public) address")]
    BlockedAddress,
}

/// What a guarded URL still needs before fetching.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuardedTarget {
    /// Host was an IP literal and is already verified public — safe to fetch.
    Ip(IpAddr),
    /// Host is a domain name; the caller MUST resolve it and pass every
    /// resulting [`IpAddr`] through [`is_blocked_ip`] before connecting.
    NeedsDnsCheck(String),
}

/// Validate scheme + host of a URL. Rejects non-http(s) schemes and IP-literal
/// hosts that are non-public. For domain hosts, returns the name to resolve.
pub fn guard_url(raw: &str) -> Result<GuardedTarget, UrlGuardError> {
    let url = Url::parse(raw).map_err(|_| UrlGuardError::Parse)?;

    match url.scheme() {
        "http" | "https" => {}
        _ => return Err(UrlGuardError::SchemeNotAllowed),
    }

    match url.host() {
        Some(Host::Ipv4(ip)) => {
            let ip = IpAddr::V4(ip);
            if is_blocked_ip(&ip) {
                Err(UrlGuardError::BlockedAddress)
            } else {
                Ok(GuardedTarget::Ip(ip))
            }
        }
        Some(Host::Ipv6(ip)) => {
            let ip = IpAddr::V6(ip);
            if is_blocked_ip(&ip) {
                Err(UrlGuardError::BlockedAddress)
            } else {
                Ok(GuardedTarget::Ip(ip))
            }
        }
        Some(Host::Domain(d)) => {
            let d = d.to_ascii_lowercase();
            // Belt-and-suspenders: block obvious local names even before DNS.
            if d == "localhost" || d.ends_with(".localhost") {
                return Err(UrlGuardError::BlockedAddress);
            }
            Ok(GuardedTarget::NeedsDnsCheck(d))
        }
        None => Err(UrlGuardError::MissingHost),
    }
}

/// True if `ip` must never be fetched (SSRF-dangerous). Pure & exhaustive.
pub fn is_blocked_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_blocked_v4(v4),
        IpAddr::V6(v6) => {
            // IPv4-*mapped* addresses (::ffff:a.b.c.d) must be judged as their v4
            // form, else ::ffff:127.0.0.1 would sneak past the v6 checks. We use
            // `to_ipv4_mapped()` (NOT `to_ipv4()`): the latter also matches the
            // deprecated IPv4-*compatible* range, which would wrongly remap ::1
            // to 0.0.0.1 and treat loopback as public — an SSRF bypass.
            if let Some(mapped) = v6.to_ipv4_mapped() {
                return is_blocked_v4(&mapped);
            }
            is_blocked_v6(v6)
        }
    }
}

fn is_blocked_v4(ip: &Ipv4Addr) -> bool {
    let o = ip.octets();
    ip.is_unspecified()            // 0.0.0.0
        || ip.is_loopback()        // 127.0.0.0/8
        || ip.is_private()         // 10/8, 172.16/12, 192.168/16
        || ip.is_link_local()      // 169.254.0.0/16  (incl. 169.254.169.254 metadata)
        || ip.is_broadcast()       // 255.255.255.255
        || ip.is_multicast()       // 224.0.0.0/4
        || ip.is_documentation()   // 192.0.2/24, 198.51.100/24, 203.0.113/24
        || o[0] == 100 && (o[1] & 0b1100_0000) == 0b0100_0000 // 100.64/10 CGNAT
        || o[0] == 192 && o[1] == 0 && o[2] == 0              // 192.0.0/24 IETF
        || o[0] == 198 && (o[1] == 18 || o[1] == 19)          // 198.18/15 benchmarking
        || o[0] >= 240 // 240.0.0.0/4 reserved (and 255.x)
}

fn is_blocked_v6(ip: &Ipv6Addr) -> bool {
    let seg = ip.segments();
    ip.is_unspecified()                    // ::
        || ip.is_loopback()                // ::1
        || ip.is_multicast()               // ff00::/8
        || (seg[0] & 0xfe00) == 0xfc00      // fc00::/7 unique local
        || (seg[0] & 0xffc0) == 0xfe80      // fe80::/10 link-local
        || (seg[0] == 0x0100 && seg[1] == 0) // 100::/64 discard-only
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn ip(s: &str) -> IpAddr {
        IpAddr::from_str(s).unwrap()
    }

    #[test]
    fn blocks_cloud_metadata_and_locals() {
        // The classic SSRF targets (§2.7).
        assert!(is_blocked_ip(&ip("169.254.169.254"))); // cloud metadata
        assert!(is_blocked_ip(&ip("127.0.0.1")));
        assert!(is_blocked_ip(&ip("0.0.0.0")));
        assert!(is_blocked_ip(&ip("10.0.0.5")));
        assert!(is_blocked_ip(&ip("172.16.5.4")));
        assert!(is_blocked_ip(&ip("192.168.1.1")));
        assert!(is_blocked_ip(&ip("100.64.0.1"))); // CGNAT
        assert!(is_blocked_ip(&ip("255.255.255.255")));
        assert!(is_blocked_ip(&ip("224.0.0.1"))); // multicast
    }

    #[test]
    fn blocks_ipv6_locals_and_mapped_v4() {
        assert!(is_blocked_ip(&ip("::1")));
        assert!(is_blocked_ip(&ip("::")));
        assert!(is_blocked_ip(&ip("fe80::1")));
        assert!(is_blocked_ip(&ip("fc00::1")));
        assert!(is_blocked_ip(&ip("ff02::1")));
        // IPv4-mapped loopback must not slip through (§2.7).
        assert!(is_blocked_ip(&ip("::ffff:127.0.0.1")));
        assert!(is_blocked_ip(&ip("::ffff:169.254.169.254")));
    }

    #[test]
    fn allows_public_addresses() {
        assert!(!is_blocked_ip(&ip("93.184.216.34"))); // example.com
        assert!(!is_blocked_ip(&ip("8.8.8.8")));
        assert!(!is_blocked_ip(&ip("2606:2800:220:1:248:1893:25c8:1946")));
    }

    #[test]
    fn rejects_dangerous_schemes() {
        assert_eq!(
            guard_url("file:///etc/passwd"),
            Err(UrlGuardError::SchemeNotAllowed)
        );
        assert_eq!(
            guard_url("ftp://example.com"),
            Err(UrlGuardError::SchemeNotAllowed)
        );
        assert_eq!(
            guard_url("gopher://127.0.0.1:6379"),
            Err(UrlGuardError::SchemeNotAllowed)
        );
    }

    #[test]
    fn rejects_ip_literal_ssrf_targets() {
        assert_eq!(
            guard_url("http://169.254.169.254/latest/meta-data/"),
            Err(UrlGuardError::BlockedAddress)
        );
        assert_eq!(
            guard_url("http://127.0.0.1:8080/admin"),
            Err(UrlGuardError::BlockedAddress)
        );
        assert_eq!(
            guard_url("http://[::1]/"),
            Err(UrlGuardError::BlockedAddress)
        );
    }

    #[test]
    fn rejects_localhost_names() {
        assert_eq!(
            guard_url("http://localhost/"),
            Err(UrlGuardError::BlockedAddress)
        );
        assert_eq!(
            guard_url("http://api.localhost/"),
            Err(UrlGuardError::BlockedAddress)
        );
    }

    #[test]
    fn public_ip_literal_is_allowed() {
        assert_eq!(
            guard_url("https://93.184.216.34/"),
            Ok(GuardedTarget::Ip(ip("93.184.216.34")))
        );
    }

    #[test]
    fn domain_needs_dns_check() {
        assert_eq!(
            guard_url("https://example.com/page"),
            Ok(GuardedTarget::NeedsDnsCheck("example.com".into()))
        );
    }
}
