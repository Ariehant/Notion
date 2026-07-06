//! Networking guards for the web-capture / scraper path.

pub mod ssrf;

pub use ssrf::{
    guard_resolved_ips, guard_url, is_blocked_ip, GuardedTarget, UrlGuardError, MAX_REDIRECTS,
};
