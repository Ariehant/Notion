//! Local-AI "Ask" mode: turn "Design review Friday at 3pm for an hour" into a
//! structured [`CompanionEvent`].
//!
//! The pipeline is: build a strict JSON-only system prompt → send it (with the
//! user's text) to a local LLM via the [`LlmClient`] trait → extract and parse
//! the JSON → validate + normalize the fields → detect calendar conflicts. Every
//! step except the network call is pure and unit-tested; the real Ollama HTTP
//! client is behind the `ollama-http` feature.
//!
//! Nothing here trusts the model blindly: we extract only the first JSON object
//! (LLMs love to wrap output in prose or ``` fences), validate the title and
//! times, and clamp obviously-broken durations rather than writing garbage into
//! the shared database.

use serde::Deserialize;
use thiserror::Error;

use crate::event::CompanionEvent;
use crate::time::{self, TimeError};

/// Default Ollama endpoint + model (overridable by the caller).
pub const DEFAULT_OLLAMA_URL: &str = "http://localhost:11434";
pub const DEFAULT_MODEL: &str = "llama3.2";

/// Fallback event duration when the model omits or inverts the end time.
const DEFAULT_DURATION_SECS: i64 = 3600;

#[derive(Debug, Error)]
pub enum AiError {
    #[error("the AI response contained no JSON object")]
    NoJson,
    #[error("the AI response was not valid JSON: {0}")]
    BadJson(String),
    #[error("the AI omitted a required field: {0}")]
    MissingField(&'static str),
    #[error("could not interpret the AI's date/time: {0}")]
    Time(#[from] TimeError),
    #[error("the local AI service is unavailable: {0}")]
    Transport(String),
}

/// A minimal chat-completion abstraction so the parsing/validation logic can be
/// tested without a running model.
pub trait LlmClient {
    /// Return the assistant's raw text for `system_prompt` + `user_prompt`.
    fn complete(&self, system_prompt: &str, user_prompt: &str) -> Result<String, AiError>;
}

/// The exact JSON contract the model is told to emit.
#[derive(Debug, Deserialize)]
struct AiEventFields {
    title: String,
    start_time: String,
    #[serde(default)]
    end_time: Option<String>,
    #[serde(default)]
    all_day: bool,
    #[serde(default)]
    location: Option<String>,
}

/// The result of interpreting a request: the proposed event plus any existing
/// events it collides with (the UI confirms before writing).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Interpretation {
    pub event: CompanionEvent,
    pub conflicts: Vec<CompanionEvent>,
}

/// Build the JSON-only system prompt, stamping in the current local time so the
/// model can resolve relative phrases ("this Friday", "tomorrow").
pub fn build_system_prompt(now_local: &str) -> String {
    format!(
        "You are a calendar parsing assistant. Extract the following fields from \
the user's request and respond ONLY in valid JSON with no prose and no code \
fences:\n\
{{ \"title\": string, \"start_time\": \"YYYY-MM-DD HH:MM\", \"end_time\": \
\"YYYY-MM-DD HH:MM\", \"all_day\": boolean, \"location\": string | null }}.\n\
Use 24-hour time. If a duration is implied but no end is given, choose a \
reasonable end. If the date/time is ambiguous, interpret it relative to the \
current timestamp: {now_local}."
    )
}

/// Extract the first balanced JSON object from arbitrary model output, ignoring
/// braces that appear inside string literals. Returns the `{...}` slice.
pub fn extract_json_object(raw: &str) -> Option<&str> {
    let bytes = raw.as_bytes();
    let start = bytes.iter().position(|&b| b == b'{')?;
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escaped = false;
    for (i, &b) in bytes.iter().enumerate().skip(start) {
        if in_string {
            match b {
                _ if escaped => escaped = false,
                b'\\' => escaped = true,
                b'"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match b {
            b'"' => in_string = true,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&raw[start..=i]);
                }
            }
            _ => {}
        }
    }
    None
}

/// Parse + validate a model response into a [`CompanionEvent`].
///
/// `now` is the current Unix second, `utc_offset_secs` the local zone offset,
/// and `id` a caller-supplied identity for the new row (a fresh UUID in the
/// app). `block_id` optionally links the event back to a CRDT block.
pub fn parse_event_response(
    raw: &str,
    now: i64,
    utc_offset_secs: i64,
    id: impl Into<String>,
    block_id: Option<String>,
) -> Result<CompanionEvent, AiError> {
    let json = extract_json_object(raw).ok_or(AiError::NoJson)?;
    let fields: AiEventFields =
        serde_json::from_str(json).map_err(|e| AiError::BadJson(e.to_string()))?;

    let title = fields.title.trim().to_string();
    if title.is_empty() {
        return Err(AiError::MissingField("title"));
    }

    let start = time::parse_naive_local(&fields.start_time, utc_offset_secs)?;

    if fields.all_day {
        // Normalize an all-day event to its full local day.
        let (day_start, day_end) = time::day_bounds(start, utc_offset_secs);
        return Ok(CompanionEvent {
            id: id.into(),
            title,
            start_time: day_start,
            end_time: day_end,
            all_day: true,
            location: normalize_opt(fields.location),
            description: None,
            block_id,
            last_modified: now,
        });
    }

    // Timed event: parse the end, else default; clamp inverted/zero durations.
    let end = match fields.end_time.as_deref() {
        Some(s) if !s.trim().is_empty() => time::parse_naive_local(s, utc_offset_secs)?,
        _ => start + DEFAULT_DURATION_SECS,
    };
    let end = if end <= start {
        start + DEFAULT_DURATION_SECS
    } else {
        end
    };

    Ok(CompanionEvent {
        id: id.into(),
        title,
        start_time: start,
        end_time: end,
        all_day: false,
        location: normalize_opt(fields.location),
        description: None,
        block_id,
        last_modified: now,
    })
}

/// Existing events that collide with `candidate` (same overlap rule the DB
/// range query uses), excluding the candidate itself and all-day rows.
pub fn find_conflicts(
    candidate: &CompanionEvent,
    existing: &[CompanionEvent],
) -> Vec<CompanionEvent> {
    existing
        .iter()
        .filter(|e| e.id != candidate.id)
        .filter(|e| !e.all_day && !candidate.all_day)
        .filter(|e| e.overlaps(candidate.start_time, candidate.end_time))
        .cloned()
        .collect()
}

/// End-to-end: prompt the model, parse the reply, and report conflicts against
/// `existing`. The caller decides whether to write despite conflicts.
pub fn interpret(
    client: &dyn LlmClient,
    text: &str,
    now: i64,
    utc_offset_secs: i64,
    id: impl Into<String>,
    block_id: Option<String>,
    existing: &[CompanionEvent],
) -> Result<Interpretation, AiError> {
    let system = build_system_prompt(&time::format_local(now, utc_offset_secs));
    let raw = client.complete(&system, text)?;
    let event = parse_event_response(&raw, now, utc_offset_secs, id, block_id)?;
    let conflicts = find_conflicts(&event, existing);
    Ok(Interpretation { event, conflicts })
}

fn normalize_opt(s: Option<String>) -> Option<String> {
    s.map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

// ---------------------------------------------------------------------------
// Real Ollama client (feature `ollama-http`)
// ---------------------------------------------------------------------------

/// A blocking Ollama chat client. Talks to `/api/chat` with `format: "json"` so
/// the model is constrained to structured output. Only compiled with the
/// `ollama-http` feature; the request/response shapes stay private.
#[cfg(feature = "ollama-http")]
pub struct OllamaClient {
    base_url: String,
    model: String,
    http: reqwest::blocking::Client,
}

#[cfg(feature = "ollama-http")]
impl OllamaClient {
    pub fn new(base_url: impl Into<String>, model: impl Into<String>) -> Self {
        OllamaClient {
            base_url: base_url.into(),
            model: model.into(),
            http: reqwest::blocking::Client::new(),
        }
    }
}

#[cfg(feature = "ollama-http")]
impl Default for OllamaClient {
    fn default() -> Self {
        Self::new(DEFAULT_OLLAMA_URL, DEFAULT_MODEL)
    }
}

#[cfg(feature = "ollama-http")]
impl LlmClient for OllamaClient {
    fn complete(&self, system_prompt: &str, user_prompt: &str) -> Result<String, AiError> {
        use serde_json::json;
        let url = format!("{}/api/chat", self.base_url.trim_end_matches('/'));
        let body = json!({
            "model": self.model,
            "stream": false,
            "format": "json",
            "messages": [
                { "role": "system", "content": system_prompt },
                { "role": "user", "content": user_prompt },
            ],
        });
        let resp = self
            .http
            .post(url)
            .json(&body)
            .send()
            .map_err(|e| AiError::Transport(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(AiError::Transport(format!("HTTP {}", resp.status())));
        }
        let value: serde_json::Value =
            resp.json().map_err(|e| AiError::Transport(e.to_string()))?;
        value
            .get("message")
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| AiError::Transport("missing message.content in Ollama reply".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Canned(String);
    impl LlmClient for Canned {
        fn complete(&self, _sys: &str, _user: &str) -> Result<String, AiError> {
            Ok(self.0.clone())
        }
    }

    // 2023-11-15 12:00:00 UTC, treated as local (offset 0) in these tests.
    const NOW: i64 = 1_700_049_600;

    #[test]
    fn extracts_json_from_fenced_and_prose_output() {
        let raw = "Sure! Here you go:\n```json\n{\"title\": \"Hi\"}\n```\nHope that helps.";
        assert_eq!(extract_json_object(raw), Some("{\"title\": \"Hi\"}"));
    }

    #[test]
    fn extracts_json_ignoring_braces_inside_strings() {
        let raw = r#"{"title": "a } b { c", "location": null}"#;
        assert_eq!(extract_json_object(raw), Some(raw));
    }

    #[test]
    fn no_json_object_reports_error() {
        assert!(extract_json_object("no braces here").is_none());
        let err = parse_event_response("nothing", NOW, 0, "id1", None).unwrap_err();
        assert!(matches!(err, AiError::NoJson));
    }

    #[test]
    fn parses_timed_event_with_explicit_end() {
        let raw = r#"{"title":"Design review","start_time":"2023-11-17 15:00",
                      "end_time":"2023-11-17 16:00","all_day":false,"location":"Zoom"}"#;
        let ev = parse_event_response(raw, NOW, 0, "id1", Some("blk".into())).unwrap();
        assert_eq!(ev.title, "Design review");
        assert_eq!(ev.location.as_deref(), Some("Zoom"));
        assert_eq!(ev.block_id.as_deref(), Some("blk"));
        assert!(!ev.all_day);
        assert_eq!(ev.duration_secs(), 3600);
        assert_eq!(ev.last_modified, NOW);
    }

    #[test]
    fn missing_end_defaults_to_one_hour() {
        let raw = r#"{"title":"Standup","start_time":"2023-11-17 09:00"}"#;
        let ev = parse_event_response(raw, NOW, 0, "id1", None).unwrap();
        assert_eq!(ev.duration_secs(), DEFAULT_DURATION_SECS);
    }

    #[test]
    fn inverted_end_is_clamped_not_negative() {
        let raw =
            r#"{"title":"Oops","start_time":"2023-11-17 15:00","end_time":"2023-11-17 14:00"}"#;
        let ev = parse_event_response(raw, NOW, 0, "id1", None).unwrap();
        assert!(ev.end_time > ev.start_time);
        assert_eq!(ev.duration_secs(), DEFAULT_DURATION_SECS);
    }

    #[test]
    fn all_day_is_normalized_to_full_local_day() {
        let raw = r#"{"title":"Holiday","start_time":"2023-11-17 00:00","all_day":true}"#;
        let ev = parse_event_response(raw, NOW, 0, "id1", None).unwrap();
        assert!(ev.all_day);
        assert_eq!(ev.duration_secs(), time::SECS_PER_DAY);
    }

    #[test]
    fn blank_title_is_rejected() {
        let raw = r#"{"title":"   ","start_time":"2023-11-17 09:00"}"#;
        assert!(matches!(
            parse_event_response(raw, NOW, 0, "id1", None),
            Err(AiError::MissingField("title"))
        ));
    }

    #[test]
    fn unparseable_time_is_reported() {
        let raw = r#"{"title":"X","start_time":"whenever"}"#;
        assert!(matches!(
            parse_event_response(raw, NOW, 0, "id1", None),
            Err(AiError::Time(_))
        ));
    }

    #[test]
    fn interpret_detects_conflicts() {
        let existing = vec![CompanionEvent {
            id: "existing".into(),
            title: "Booked".into(),
            start_time: 1_700_235_000, // 2023-11-17 15:30 UTC
            end_time: 1_700_237_400,   // 16:10 UTC
            all_day: false,
            location: None,
            description: None,
            block_id: None,
            last_modified: 0,
        }];
        let client = Canned(
            r#"{"title":"Design review","start_time":"2023-11-17 15:00","end_time":"2023-11-17 16:00"}"#
                .into(),
        );
        let out = interpret(
            &client,
            "design review friday 3pm",
            NOW,
            0,
            "new",
            None,
            &existing,
        )
        .unwrap();
        assert_eq!(out.event.title, "Design review");
        assert_eq!(out.conflicts.len(), 1);
        assert_eq!(out.conflicts[0].id, "existing");
    }

    #[test]
    fn all_day_does_not_conflict_with_timed() {
        let all_day = CompanionEvent {
            id: "hol".into(),
            title: "Holiday".into(),
            start_time: 1_700_179_200,
            end_time: 1_700_265_600,
            all_day: true,
            location: None,
            description: None,
            block_id: None,
            last_modified: 0,
        };
        let timed = CompanionEvent {
            id: "mtg".into(),
            all_day: false,
            ..all_day.clone()
        };
        assert!(find_conflicts(&timed, &[all_day]).is_empty());
    }
}
