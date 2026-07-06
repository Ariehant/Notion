//! Defensive JSON extraction shared by [`crate::agents`] and [`crate::mcp`].
//!
//! Local LLMs love to wrap a JSON object in prose, markdown ``` fences, or a
//! leading "Sure! Here you go:". We never trust the raw string: we scan for the
//! first *balanced* top-level `{...}` object, correctly skipping braces that
//! appear inside string literals (and their `\"` escapes). This is the same
//! hardening `notion-companion` applies to calendar parsing, reimplemented here
//! so the crate stays self-contained.

/// Return the first balanced JSON object substring in `raw`, or `None`.
///
/// "Balanced" means brace depth returns to zero; braces inside `"..."` string
/// literals are ignored, and `\"`/`\\` escapes inside a string are respected so
/// a quote-containing value cannot end the scan early.
pub fn extract_json_object(raw: &str) -> Option<&str> {
    let bytes = raw.as_bytes();
    let start = bytes.iter().position(|&b| b == b'{')?;

    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;

    for (i, &b) in bytes.iter().enumerate().skip(start) {
        if in_string {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_string = false;
            }
            continue;
        }
        match b {
            b'"' => in_string = true,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    // `i` is at a `}` (ASCII), so `start..=i` is a char boundary.
                    return Some(&raw[start..=i]);
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_object() {
        assert_eq!(extract_json_object(r#"{"a":1}"#), Some(r#"{"a":1}"#));
    }

    #[test]
    fn strips_prose_and_fences() {
        let raw = "Sure! Here you go:\n```json\n{\"title\":\"Hi\"}\n```\nHope that helps.";
        assert_eq!(extract_json_object(raw), Some(r#"{"title":"Hi"}"#));
    }

    #[test]
    fn brace_inside_string_does_not_end_early() {
        let raw = r#"{"note":"a } here","n":2}"#;
        assert_eq!(extract_json_object(raw), Some(raw));
    }

    #[test]
    fn escaped_quote_inside_string() {
        let raw = r#"{"q":"she said \"hi\" }","ok":true}"#;
        assert_eq!(extract_json_object(raw), Some(raw));
    }

    #[test]
    fn nested_objects() {
        let raw = r#"prefix {"a":{"b":{"c":1}},"d":2} suffix"#;
        assert_eq!(
            extract_json_object(raw),
            Some(r#"{"a":{"b":{"c":1}},"d":2}"#)
        );
    }

    #[test]
    fn none_when_absent_or_unbalanced() {
        assert!(extract_json_object("no json here").is_none());
        assert!(extract_json_object(r#"{"a":1"#).is_none());
    }
}
