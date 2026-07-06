//! Networking guards for the web-capture / scraper path.

pub mod ssrf;

pub use ssrf::{guard_url, is_blocked_ip, UrlGuardError, MAX_REDIRECTS};
