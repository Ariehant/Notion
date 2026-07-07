//! Action-executing agents: turn "add a calendar event for tomorrow at 3pm" or
//! "make a page for the Q3 planning notes" into a *validated* action, execute it
//! against the injected [`NotebookStorage`], and log it to `agent_logs`.
//!
//! The model is never trusted. It is asked for strict JSON; we extract the first
//! balanced object (through any prose/fences), deserialize into a permissive raw
//! struct, then **validate and clamp** before touching the database: titles must
//! be non-empty, event durations are defaulted/clamped, and an unknown action is
//! rejected. Timestamps are exchanged as Unix **seconds** (the prompt states the
//! current time) so there is no timezone parsing to get wrong.

use serde::Deserialize;

use crate::gateway::{GatewayError, LlmClient};
use crate::jsonutil::extract_json_object;
use crate::storage::{AgentLog, NotebookCalendarEvent, NotebookStorage, StorageError};

const DEFAULT_DURATION_SECS: i64 = 3600;
const ALL_DAY_SECS: i64 = 86_400;
/// Anything longer than this is treated as a model error and clamped.
const MAX_DURATION_SECS: i64 = 30 * ALL_DAY_SECS;

#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    #[error("the AI response contained no JSON object")]
    NoJson,
    #[error("the AI response was not valid JSON: {0}")]
    BadJson(String),
    #[error("the AI omitted a required field: {0}")]
    MissingField(&'static str),
    #[error("the AI requested an unknown action: {0}")]
    UnknownAction(String),
    #[error(transparent)]
    Gateway(#[from] GatewayError),
    #[error(transparent)]
    Storage(#[from] StorageError),
}

/// A validated action the agent will perform.
#[derive(Debug, Clone, PartialEq)]
pub enum AgentAction {
    /// Add an event to the shared `calendar_events` table (Unix-second times).
    AddEvent {
        title: String,
        start_time: i64,
        end_time: i64,
        all_day: bool,
        location: Option<String>,
    },
    /// Create a new page in the sidebar.
    CreatePage { title: String },
    /// No side effect — a direct textual answer to the user.
    Answer { text: String },
}

impl AgentAction {
    /// A short, stable label for the `agent_logs.action_taken` column.
    pub fn kind(&self) -> &'static str {
        match self {
            AgentAction::AddEvent { .. } => "add_event",
            AgentAction::CreatePage { .. } => "create_page",
            AgentAction::Answer { .. } => "answer",
        }
    }
}

/// What `run` did.
#[derive(Debug, Clone, PartialEq)]
pub struct AgentOutcome {
    pub action: AgentAction,
    /// A human-facing message describing the result.
    pub message: String,
    /// The id of a row this action created (event/page), if any.
    pub created_id: Option<String>,
}

/// The permissive shape we ask the model to emit; validated into [`AgentAction`].
#[derive(Debug, Deserialize)]
struct RawAction {
    action: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    start_time: Option<i64>,
    #[serde(default)]
    end_time: Option<i64>,
    #[serde(default)]
    all_day: bool,
    #[serde(default)]
    location: Option<String>,
    #[serde(default)]
    text: Option<String>,
}

pub struct AgentRunner<L: LlmClient> {
    llm: L,
}

impl<L: LlmClient> AgentRunner<L> {
    pub fn new(llm: L) -> Self {
        Self { llm }
    }

    /// Build the strict system prompt, stamped with the current time so the
    /// model can resolve "tomorrow at 3pm" into a Unix second.
    pub fn build_system_prompt(now_unix: i64) -> String {
        format!(
            "You are an action planner for a notes+calendar app. The current time is \
             {now_unix} (Unix seconds, UTC). Reply with ONE JSON object and nothing else.\n\
             Allowed actions:\n\
             - Add a calendar event: {{\"action\":\"add_event\",\"title\":str,\
             \"start_time\":int,\"end_time\":int,\"all_day\":bool,\"location\":str|null}}\n\
             - Create a page: {{\"action\":\"create_page\",\"title\":str}}\n\
             - Answer directly: {{\"action\":\"answer\",\"text\":str}}\n\
             All timestamps are Unix seconds (UTC). Resolve relative times against the \
             current time above. Do not include comments or trailing text."
        )
    }

    /// Plan (do not execute) the action for `prompt`. Pure aside from the LLM call.
    pub fn plan(&self, prompt: &str, now_unix: i64) -> Result<AgentAction, AgentError> {
        let system = Self::build_system_prompt(now_unix);
        let raw_text = self.llm.complete(&system, prompt)?;
        let json = extract_json_object(&raw_text).ok_or(AgentError::NoJson)?;
        let raw: RawAction =
            serde_json::from_str(json).map_err(|e| AgentError::BadJson(e.to_string()))?;
        validate(raw, now_unix)
    }

    /// Plan and execute against `store`, logging the action. `new_id` is used for
    /// whichever row gets created (event or page); `context_block_id` is recorded
    /// as the affected block (e.g. the editor block the user invoked `/ai` from).
    pub fn run(
        &self,
        store: &dyn NotebookStorage,
        prompt: &str,
        context_block_id: Option<&str>,
        new_id: &str,
        now_unix: i64,
    ) -> Result<AgentOutcome, AgentError> {
        let action = self.plan(prompt, now_unix)?;
        let (message, created_id) = match &action {
            AgentAction::AddEvent {
                title,
                start_time,
                end_time,
                all_day,
                location,
            } => {
                store.add_calendar_event(&NotebookCalendarEvent {
                    id: new_id.to_string(),
                    title: title.clone(),
                    start_time: *start_time,
                    end_time: *end_time,
                    all_day: *all_day,
                    location: location.clone(),
                    description: None,
                    last_modified: now_unix,
                })?;
                (format!("Added event “{title}”."), Some(new_id.to_string()))
            }
            AgentAction::CreatePage { title } => {
                // pages.created_at/updated_at are milliseconds in the main schema.
                store.create_page(new_id, title, now_unix * 1000)?;
                (format!("Created page “{title}”."), Some(new_id.to_string()))
            }
            AgentAction::Answer { text } => (text.clone(), None),
        };

        store.log_agent(&AgentLog {
            id: format!("log:{new_id}"),
            agent_type: action.kind().to_string(),
            prompt: prompt.to_string(),
            action_taken: message.clone(),
            block_affected: context_block_id.map(String::from),
            timestamp: now_unix,
        })?;

        Ok(AgentOutcome {
            action,
            message,
            created_id,
        })
    }
}

/// Validate + normalize a raw model action.
fn validate(raw: RawAction, now_unix: i64) -> Result<AgentAction, AgentError> {
    match raw.action.as_str() {
        "add_event" => {
            let title = non_empty(raw.title, "title")?;
            let all_day = raw.all_day;
            // Missing start defaults to "now"; better to schedule something the
            // user can move than to fail the whole request.
            let start_time = raw.start_time.unwrap_or(now_unix);
            let default_len = if all_day {
                ALL_DAY_SECS
            } else {
                DEFAULT_DURATION_SECS
            };
            let end_time = match raw.end_time {
                // Reject inverted or absurd durations; fall back to a sane block.
                // Saturating math: the timestamps come straight from the model,
                // so extreme values (e.g. i64::MIN/MAX) must not overflow — that
                // would panic in debug and, worse, wrap past the max-duration
                // guard in release.
                Some(end)
                    if end > start_time && end.saturating_sub(start_time) <= MAX_DURATION_SECS =>
                {
                    end
                }
                _ => start_time.saturating_add(default_len),
            };
            Ok(AgentAction::AddEvent {
                title,
                start_time,
                end_time,
                all_day,
                location: raw.location.filter(|s| !s.trim().is_empty()),
            })
        }
        "create_page" => Ok(AgentAction::CreatePage {
            title: non_empty(raw.title, "title")?,
        }),
        "answer" => Ok(AgentAction::Answer {
            text: non_empty(raw.text, "text")?,
        }),
        other => Err(AgentError::UnknownAction(other.to_string())),
    }
}

fn non_empty(field: Option<String>, name: &'static str) -> Result<String, AgentError> {
    match field.map(|s| s.trim().to_string()) {
        Some(s) if !s.is_empty() => Ok(s),
        _ => Err(AgentError::MissingField(name)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::testutil::FakeLlm;
    use crate::storage::MemStorage;

    const NOW: i64 = 1_000_000;

    fn runner(reply: &'static str) -> AgentRunner<FakeLlm> {
        AgentRunner::new(FakeLlm::new([reply]))
    }

    #[test]
    fn plans_add_event_with_explicit_times() {
        let r = runner(
            r#"{"action":"add_event","title":"Standup","start_time":1000000,"end_time":1003600}"#,
        );
        assert_eq!(
            r.plan("standup at 9", NOW).unwrap(),
            AgentAction::AddEvent {
                title: "Standup".into(),
                start_time: 1_000_000,
                end_time: 1_003_600,
                all_day: false,
                location: None,
            }
        );
    }

    #[test]
    fn defaults_missing_end_to_one_hour() {
        let r = runner(r#"{"action":"add_event","title":"Call","start_time":1000000}"#);
        match r.plan("call", NOW).unwrap() {
            AgentAction::AddEvent { end_time, .. } => {
                assert_eq!(end_time, 1_000_000 + DEFAULT_DURATION_SECS)
            }
            _ => panic!("expected add_event"),
        }
    }

    #[test]
    fn clamps_inverted_and_absurd_durations() {
        // end before start ⇒ clamp to +1h
        let r =
            runner(r#"{"action":"add_event","title":"X","start_time":1000000,"end_time":999000}"#);
        match r.plan("x", NOW).unwrap() {
            AgentAction::AddEvent { end_time, .. } => {
                assert_eq!(end_time, 1_000_000 + DEFAULT_DURATION_SECS)
            }
            _ => panic!(),
        }
        // absurdly long ⇒ clamp
        let r = runner(
            r#"{"action":"add_event","title":"X","start_time":1000000,"end_time":9999999999}"#,
        );
        match r.plan("x", NOW).unwrap() {
            AgentAction::AddEvent { end_time, .. } => {
                assert_eq!(end_time, 1_000_000 + DEFAULT_DURATION_SECS)
            }
            _ => panic!(),
        }
    }

    #[test]
    fn extreme_timestamps_do_not_overflow() {
        // The model output is untrusted: i64::MIN start with i64::MAX end must
        // neither panic (debug) nor wrap past the max-duration guard (release).
        let r = runner(
            r#"{"action":"add_event","title":"X","start_time":-9223372036854775808,"end_time":9223372036854775807}"#,
        );
        match r.plan("x", NOW).unwrap() {
            AgentAction::AddEvent {
                start_time,
                end_time,
                ..
            } => {
                assert_eq!(start_time, i64::MIN);
                // The absurd span is rejected and clamped to the default block,
                // computed with saturating math (no overflow at i64::MIN).
                assert_eq!(end_time, i64::MIN.saturating_add(DEFAULT_DURATION_SECS));
            }
            _ => panic!("expected add_event"),
        }
    }

    #[test]
    fn all_day_uses_a_full_day_default() {
        let r = runner(
            r#"{"action":"add_event","title":"Holiday","start_time":1000000,"all_day":true}"#,
        );
        match r.plan("holiday", NOW).unwrap() {
            AgentAction::AddEvent {
                all_day, end_time, ..
            } => {
                assert!(all_day);
                assert_eq!(end_time, 1_000_000 + ALL_DAY_SECS);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn missing_title_is_rejected() {
        let r = runner(r#"{"action":"add_event","start_time":1000000}"#);
        assert!(matches!(
            r.plan("x", NOW),
            Err(AgentError::MissingField("title"))
        ));
    }

    #[test]
    fn unknown_action_is_rejected() {
        let r = runner(r#"{"action":"delete_everything"}"#);
        assert!(matches!(
            r.plan("x", NOW),
            Err(AgentError::UnknownAction(_))
        ));
    }

    #[test]
    fn tolerates_prose_and_fences_around_json() {
        let r = runner("Sure!\n```json\n{\"action\":\"answer\",\"text\":\"Hello\"}\n```");
        assert_eq!(
            r.plan("hi", NOW).unwrap(),
            AgentAction::Answer {
                text: "Hello".into()
            }
        );
    }

    #[test]
    fn no_json_errors() {
        let r = runner("I cannot help with that.");
        assert!(matches!(r.plan("x", NOW), Err(AgentError::NoJson)));
    }

    #[test]
    fn run_add_event_writes_to_calendar_and_logs() {
        let store = MemStorage::new();
        let r = runner(
            r#"{"action":"add_event","title":"Review","start_time":1000000,"end_time":1003600,"location":"Zoom"}"#,
        );
        let outcome = r
            .run(&store, "schedule review", Some("blk-7"), "ev-1", NOW)
            .unwrap();
        assert_eq!(outcome.created_id.as_deref(), Some("ev-1"));
        assert_eq!(store.event_count(), 1);
        let ev = &store.events()[0];
        assert_eq!(ev.title, "Review");
        assert_eq!(ev.location.as_deref(), Some("Zoom"));
        // Logged with the affected block for transparency.
        let logs = store.list_agent_logs(10).unwrap();
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].agent_type, "add_event");
        assert_eq!(logs[0].block_affected.as_deref(), Some("blk-7"));
    }

    #[test]
    fn run_create_page_inserts_page() {
        let store = MemStorage::new();
        let r = runner(r#"{"action":"create_page","title":"Q3 Planning"}"#);
        let outcome = r.run(&store, "make a page", None, "pg-1", NOW).unwrap();
        assert_eq!(outcome.action.kind(), "create_page");
        assert_eq!(store.page_count(), 1);
    }

    #[test]
    fn run_answer_has_no_side_effects() {
        let store = MemStorage::new();
        let r = runner(r#"{"action":"answer","text":"42"}"#);
        let outcome = r.run(&store, "meaning of life", None, "x", NOW).unwrap();
        assert_eq!(outcome.message, "42");
        assert_eq!(outcome.created_id, None);
        assert_eq!(store.event_count(), 0);
        assert_eq!(store.page_count(), 0);
        // An answer is still logged.
        assert_eq!(store.list_agent_logs(10).unwrap().len(), 1);
    }
}
