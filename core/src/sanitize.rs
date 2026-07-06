//! HTML sanitization — audit §2.8.
//!
//! The blueprint sanitizes *scraped* HTML but leaves **pasted** rich text as an
//! equal, unguarded XSS vector, and offers "arbitrary iframe" embeds. This
//! module funnels **all** untrusted HTML — scraped and pasted — through one
//! sanitizer ([`sanitize_html`]) before it can become a block, and renders
//! embeds only inside **sandboxed** iframes ([`sandboxed_embed`]) that never
//! combine `allow-scripts` with `allow-same-origin`.

use ammonia::Builder;
use std::collections::HashSet;

use crate::net::ssrf::{guard_url, GuardedTarget, UrlGuardError};

/// Sanitize untrusted HTML from any source (scraped **or** pasted, §2.8).
///
/// Strips `<script>`, event-handler attributes, `javascript:`/`data:` URLs, and
/// any tag/attribute not on the allowlist. `iframe`/`embed`/`object` are removed
/// here — embeds must go through [`sandboxed_embed`] instead.
pub fn sanitize_html(input: &str) -> String {
    builder().clean(input).to_string()
}

fn builder() -> Builder<'static> {
    let mut b = Builder::default();
    // Only http/https/mailto links; ammonia drops javascript:/data: by default,
    // but we set the allowlist explicitly to be unambiguous.
    let mut schemes = HashSet::new();
    schemes.insert("http");
    schemes.insert("https");
    schemes.insert("mailto");
    b.url_schemes(schemes);
    // Force safe rel on links (defense against tab-nabbing).
    b.link_rel(Some("noopener noreferrer nofollow"));
    // Never allow embedding vectors through the generic sanitizer.
    let mut forbidden = HashSet::new();
    for t in [
        "script", "style", "iframe", "object", "embed", "form", "meta", "link", "base",
    ] {
        forbidden.insert(t);
    }
    b.rm_tags(forbidden);
    b
}

/// The sandbox policy we allow embeds to run under. Critically it does **not**
/// combine `allow-scripts` with `allow-same-origin` (§2.8) — together they let
/// framed content escape the sandbox.
pub const EMBED_SANDBOX: &str = "allow-scripts allow-popups allow-forms allow-presentation";

/// Provider allowlist for embeds. Embeds are loaded by the WebView itself, so we
/// cannot DNS-pin them (see the SSRF module's rebinding note); instead we only
/// allow known embed providers. Matches the host exactly or as a subdomain.
pub const EMBED_HOST_ALLOWLIST: &[&str] = &[
    "youtube.com",
    "youtube-nocookie.com",
    "youtu.be",
    "vimeo.com",
    "player.vimeo.com",
    "loom.com",
    "figma.com",
    "codepen.io",
    "codesandbox.io",
    "replit.com",
    "github.com",
    "gist.github.com",
    "google.com",
    "docs.google.com",
    "drive.google.com",
    "maps.google.com",
    "twitter.com",
    "x.com",
    "spotify.com",
    "open.spotify.com",
    "soundcloud.com",
    "miro.com",
    "canva.com",
    "airtable.com",
    "typeform.com",
];

fn embed_host_allowed(host: &str) -> bool {
    let h = host.trim_end_matches('.').to_ascii_lowercase();
    EMBED_HOST_ALLOWLIST
        .iter()
        .any(|a| h == *a || h.ends_with(&format!(".{a}")))
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum EmbedError {
    #[error("embed source rejected: {0}")]
    Blocked(#[from] UrlGuardError),
    #[error("embed source must use https")]
    NotHttps,
    #[error("embed host is not an allowed provider")]
    HostNotAllowed,
}

/// Build a locked-down `<iframe>` for an allowed embed URL.
///
/// The `src` must be https, must be an allowlisted provider host (raw IP hosts
/// are rejected — an embed is never a bare IP), and passes the SSRF scheme/host
/// checks (§2.7). The iframe is sandboxed ([`EMBED_SANDBOX`]) and
/// referrer-stripped. Tauri IPC is never exposed to embedded content (enforced
/// at the WebView layer).
pub fn sandboxed_embed(src: &str) -> Result<String, EmbedError> {
    if !src.starts_with("https://") {
        return Err(EmbedError::NotHttps);
    }
    // Rejects bad schemes and IP-literal SSRF targets (loopback/private/etc.).
    match guard_url(src)? {
        // A bare IP is never a legitimate provider embed — require a known host.
        GuardedTarget::Ip(_) => return Err(EmbedError::HostNotAllowed),
        GuardedTarget::NeedsDnsCheck(domain) => {
            if !embed_host_allowed(&domain) {
                return Err(EmbedError::HostNotAllowed);
            }
        }
    }
    let escaped = html_escape_attr(src);
    Ok(format!(
        "<iframe src=\"{escaped}\" sandbox=\"{EMBED_SANDBOX}\" \
         referrerpolicy=\"no-referrer\" loading=\"lazy\"></iframe>"
    ))
}

/// Minimal attribute-context HTML escaping.
fn html_escape_attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_script_tags() {
        let dirty = r#"<p>hi</p><script>alert('xss')</script>"#;
        let clean = sanitize_html(dirty);
        assert!(!clean.contains("script"));
        assert!(clean.contains("hi"));
    }

    #[test]
    fn strips_event_handlers_and_js_urls() {
        let dirty = r#"<a href="javascript:alert(1)" onclick="steal()">click</a>"#;
        let clean = sanitize_html(dirty);
        assert!(!clean.contains("onclick"));
        assert!(!clean.contains("javascript:"));
    }

    #[test]
    fn strips_iframes_from_generic_html() {
        // Pasted/scraped iframes must not survive the generic path (§2.8).
        let dirty = r#"<iframe src="https://evil.example/x"></iframe><b>ok</b>"#;
        let clean = sanitize_html(dirty);
        assert!(!clean.contains("iframe"));
        assert!(clean.contains("ok"));
    }

    #[test]
    fn keeps_safe_formatting_and_adds_rel() {
        let dirty = r#"<p><strong>bold</strong> <a href="https://ok.example">link</a></p>"#;
        let clean = sanitize_html(dirty);
        assert!(clean.contains("<strong>"));
        assert!(clean.contains("href=\"https://ok.example\""));
        assert!(clean.contains("noopener"));
    }

    #[test]
    fn pasted_html_is_sanitized_same_as_scraped() {
        // Same function, same guarantees for both sources (§2.8).
        let pasted = r#"<img src=x onerror="alert(1)">text"#;
        let clean = sanitize_html(pasted);
        assert!(!clean.contains("onerror"));
    }

    #[test]
    fn sandboxed_embed_is_locked_down() {
        let out = sandboxed_embed("https://www.youtube.com/embed/abc").unwrap();
        assert!(out.contains("sandbox="));
        // Must never combine allow-scripts + allow-same-origin (§2.8).
        assert!(!out.contains("allow-same-origin"));
        assert!(out.contains("referrerpolicy=\"no-referrer\""));
    }

    #[test]
    fn embed_rejects_non_https_ssrf_and_unknown_hosts() {
        assert_eq!(
            sandboxed_embed("http://www.youtube.com/x"),
            Err(EmbedError::NotHttps)
        );
        // IP-literal SSRF target blocked by the guard.
        assert!(matches!(
            sandboxed_embed("https://127.0.0.1/x"),
            Err(EmbedError::Blocked(_))
        ));
        // Bare public IP is not an allowed provider.
        assert_eq!(
            sandboxed_embed("https://93.184.216.34/x"),
            Err(EmbedError::HostNotAllowed)
        );
        // Unknown domain (incl. a rebinding-style host) is rejected (§ review #5).
        assert_eq!(
            sandboxed_embed("https://intranet.attacker.example/admin"),
            Err(EmbedError::HostNotAllowed)
        );
        // Trailing-dot localhost cannot reach the allowlist either.
        assert!(sandboxed_embed("https://localhost./x").is_err());
    }

    #[test]
    fn embed_allows_known_provider_subdomains() {
        assert!(sandboxed_embed("https://player.vimeo.com/video/123").is_ok());
        assert!(sandboxed_embed("https://gist.github.com/u/abc").is_ok());
    }

    #[test]
    fn embed_src_is_attribute_escaped() {
        // Allowlisted host with an injection attempt in the path/query.
        let out = sandboxed_embed("https://www.youtube.com/embed/x?a=1&b=2\"><script>").unwrap();
        assert!(!out.contains("\"><script>"));
        assert!(out.contains("&amp;"));
    }
}
