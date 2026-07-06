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

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum EmbedError {
    #[error("embed source rejected: {0}")]
    Blocked(#[from] UrlGuardError),
    #[error("embed source must use https")]
    NotHttps,
}

/// Build a locked-down `<iframe>` for an allowed embed URL.
///
/// The `src` is SSRF-guarded (§2.7) and required to be https; the iframe is
/// sandboxed ([`EMBED_SANDBOX`]) and referrer-stripped. Tauri IPC is never
/// exposed to embedded content (enforced at the WebView layer).
pub fn sandboxed_embed(src: &str) -> Result<String, EmbedError> {
    if !src.starts_with("https://") {
        return Err(EmbedError::NotHttps);
    }
    // Reject SSRF targets (IP literals resolved here; domains flagged for the
    // caller's resolver, but still scheme/host validated).
    match guard_url(src)? {
        GuardedTarget::Ip(_) | GuardedTarget::NeedsDnsCheck(_) => {}
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
    fn embed_rejects_non_https_and_ssrf() {
        assert_eq!(
            sandboxed_embed("http://example.com/x"),
            Err(EmbedError::NotHttps)
        );
        assert!(matches!(
            sandboxed_embed("https://127.0.0.1/x"),
            Err(EmbedError::Blocked(_))
        ));
    }

    #[test]
    fn embed_src_is_attribute_escaped() {
        let out = sandboxed_embed("https://ok.example/x?a=1&b=2\"><script>").unwrap();
        assert!(!out.contains("\"><script>"));
        assert!(out.contains("&amp;"));
    }
}
