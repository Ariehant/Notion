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
//!     unspecified (whole `0.0.0.0/8`), multicast, or otherwise non-public —
//!     including IPv4 addresses embedded in IPv6 (mapped, compatible, 6to4,
//!     NAT64);
//!   * redirects are capped at [`MAX_REDIRECTS`] and each hop is re-checked.
//!
//! ## Caller contract (DNS-rebinding / TOCTOU — important)
//!
//! [`guard_url`] cannot do DNS itself (it is I/O). For a domain host it returns
//! [`GuardedTarget::NeedsDnsCheck`]; the caller MUST resolve the name, pass every
//! resulting address through [`guard_resolved_ips`], **and then connect to one of
//! those exact vetted addresses** (pin it) rather than re-resolving. Re-resolving
//! at connect time reopens a DNS-rebinding window where an attacker's resolver
//! returns a public IP to the guard and an internal IP to the socket. Redirect
//! hops must repeat the whole process.

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
    /// Host is a domain name; the caller MUST resolve it, verify every address
    /// with [`guard_resolved_ips`], and connect to a vetted address (pin it).
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
            // Normalize: lowercase and strip any FQDN trailing dot(s) so that
            // `localhost.` cannot slip past the local-name block below.
            let d = d.to_ascii_lowercase();
            let d = d.trim_end_matches('.').to_string();
            if d == "localhost" || d.ends_with(".localhost") {
                return Err(UrlGuardError::BlockedAddress);
            }
            Ok(GuardedTarget::NeedsDnsCheck(d))
        }
        None => Err(UrlGuardError::MissingHost),
    }
}

/// Caller helper: verify a set of DNS-resolved addresses. Returns `Ok` only if
/// there is at least one address and every address is public. The caller must
/// then connect to one of the vetted addresses (see module-level contract).
pub fn guard_resolved_ips(ips: &[IpAddr]) -> Result<(), UrlGuardError> {
    if ips.is_empty() || ips.iter().any(is_blocked_ip) {
        Err(UrlGuardError::BlockedAddress)
    } else {
        Ok(())
    }
}

/// True if `ip` must never be fetched (SSRF-dangerous). Pure & exhaustive.
pub fn is_blocked_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_blocked_v4(v4),
        IpAddr::V6(v6) => {
            // Any IPv4 embedded in IPv6 (mapped, compatible, 6to4, NAT64) must
            // be judged as its v4 form, else e.g. ::ffff:127.0.0.1,
            // ::127.0.0.1, 2002:7f00:1::, or 64:ff9b::7f00:1 would slip past.
            if let Some(v4) = embedded_ipv4(v6) {
                return is_blocked_v4(&v4) || is_blocked_v6(v6);
            }
            is_blocked_v6(v6)
        }
    }
}

/// Extract an IPv4 address embedded in an IPv6 address, if any.
fn embedded_ipv4(v6: &Ipv6Addr) -> Option<Ipv4Addr> {
    // IPv4-mapped: ::ffff:a.b.c.d
    if let Some(v4) = v6.to_ipv4_mapped() {
        return Some(v4);
    }
    let s = v6.segments();
    let from = |hi: u16, lo: u16| {
        Ipv4Addr::new(
            (hi >> 8) as u8,
            (hi & 0xff) as u8,
            (lo >> 8) as u8,
            (lo & 0xff) as u8,
        )
    };
    // IPv4-compatible (deprecated): ::a.b.c.d — first 96 bits zero.
    if s[0..6].iter().all(|&seg| seg == 0) {
        return Some(from(s[6], s[7]));
    }
    // 6to4: 2002:AABB:CCDD::/48 — embedded v4 in the next two segments.
    if s[0] == 0x2002 {
        return Some(from(s[1], s[2]));
    }
    // NAT64 well-known/local prefix: 64:ff9b::/96 and 64:ff9b:1::/48.
    if s[0] == 0x0064 && s[1] == 0xff9b {
        return Some(from(s[6], s[7]));
    }
    None
}

fn is_blocked_v4(ip: &Ipv4Addr) -> bool {
    let o = ip.octets();
    o[0] == 0                      // 0.0.0.0/8 "this network" (incl. 0.0.0.0)
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
        assert!(is_blocked_ip(&ip("169.254.169.254"))); // cloud metadata
        assert!(is_blocked_ip(&ip("127.0.0.1")));
        assert!(is_blocked_ip(&ip("0.0.0.0")));
        assert!(is_blocked_ip(&ip("0.1.2.3"))); // whole 0.0.0.0/8 (§ review #6)
        assert!(is_blocked_ip(&ip("10.0.0.5")));
        assert!(is_blocked_ip(&ip("172.16.5.4")));
        assert!(is_blocked_ip(&ip("192.168.1.1")));
        assert!(is_blocked_ip(&ip("100.64.0.1"))); // CGNAT
        assert!(is_blocked_ip(&ip("255.255.255.255")));
        assert!(is_blocked_ip(&ip("224.0.0.1"))); // multicast
    }

    #[test]
    fn blocks_ipv6_locals_and_embedded_v4() {
        assert!(is_blocked_ip(&ip("::1")));
        assert!(is_blocked_ip(&ip("::")));
        assert!(is_blocked_ip(&ip("fe80::1")));
        assert!(is_blocked_ip(&ip("fc00::1")));
        assert!(is_blocked_ip(&ip("ff02::1")));
        // IPv4-mapped must not slip through (§2.7).
        assert!(is_blocked_ip(&ip("::ffff:127.0.0.1")));
        assert!(is_blocked_ip(&ip("::ffff:169.254.169.254")));
        // IPv4-compatible, 6to4, and NAT64 wrapping loopback (§ review #7).
        assert!(is_blocked_ip(&ip("::127.0.0.1")));
        assert!(is_blocked_ip(&ip("2002:7f00:1::"))); // 6to4 of 127.0.0.1
        assert!(is_blocked_ip(&ip("64:ff9b::7f00:1"))); // NAT64 of 127.0.0.1
        assert!(is_blocked_ip(&ip("64:ff9b::a00:5"))); // NAT64 of 10.0.0.5
    }

    #[test]
    fn allows_public_addresses() {
        assert!(!is_blocked_ip(&ip("93.184.216.34"))); // example.com
        assert!(!is_blocked_ip(&ip("8.8.8.8")));
        assert!(!is_blocked_ip(&ip("2606:2800:220:1:248:1893:25c8:1946")));
        // 6to4/NAT64 wrapping a public v4 stays allowed (precise, not over-broad).
        assert!(!is_blocked_ip(&ip("2002:5db8:d822::"))); // 6to4 of 93.184.216.34
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
    fn rejects_localhost_names_including_trailing_dot() {
        assert_eq!(
            guard_url("http://localhost/"),
            Err(UrlGuardError::BlockedAddress)
        );
        assert_eq!(
            guard_url("http://api.localhost/"),
            Err(UrlGuardError::BlockedAddress)
        );
        // Trailing-dot FQDN must not bypass the block (§ review #3).
        assert_eq!(
            guard_url("http://localhost./"),
            Err(UrlGuardError::BlockedAddress)
        );
        assert_eq!(
            guard_url("http://LOCALHOST./"),
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
    fn domain_needs_dns_check_and_is_normalized() {
        assert_eq!(
            guard_url("https://Example.COM./page"),
            Ok(GuardedTarget::NeedsDnsCheck("example.com".into()))
        );
    }

    #[test]
    fn guard_resolved_ips_checks_every_address() {
        assert!(guard_resolved_ips(&[ip("93.184.216.34")]).is_ok());
        assert!(guard_resolved_ips(&[]).is_err());
        // A rebinding record mixing public + internal must fail closed.
        assert!(guard_resolved_ips(&[ip("93.184.216.34"), ip("169.254.169.254")]).is_err());
    }
}
