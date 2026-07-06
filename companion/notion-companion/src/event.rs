//! The calendar event model shared across the companion, plus its JSON wire
//! format.
//!
//! [`CompanionEvent`] is the single event type used by the AI parser, the DBus
//! payload, and the GTK views. It serializes to camelCase JSON so the GJS
//! extension can read fields directly (`ev.startTime`, `ev.allDay`, …). All
//! timestamps are **Unix seconds (UTC)**, matching the `calendar_events` table.

use serde::{Deserialize, Serialize};

/// A calendar event as it travels between the companion processes.
///
/// This is intentionally a mirror of `notion_core::db::CalendarEvent`; the
/// [`From`] conversions (compiled with the `sqlcipher` feature) bridge the two
/// so the DB layer and the wire layer never drift.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompanionEvent {
    pub id: String,
    pub title: String,
    #[serde(rename = "startTime")]
    pub start_time: i64,
    #[serde(rename = "endTime")]
    pub end_time: i64,
    #[serde(rename = "allDay", default)]
    pub all_day: bool,
    #[serde(default)]
    pub location: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(rename = "blockId", default)]
    pub block_id: Option<String>,
    #[serde(rename = "lastModified", default)]
    pub last_modified: i64,
}

impl CompanionEvent {
    /// Whether this event overlaps the half-open interval `[start, end)`.
    pub fn overlaps(&self, start: i64, end: i64) -> bool {
        self.start_time < end && self.end_time > start
    }

    /// Duration in seconds (clamped to zero for degenerate rows).
    pub fn duration_secs(&self) -> i64 {
        (self.end_time - self.start_time).max(0)
    }
}

/// Serialize a list of events to the compact JSON array the DBus interface
/// carries as a string (`type="s"`).
pub fn events_to_json(events: &[CompanionEvent]) -> String {
    // Serializing a Vec of plain structs cannot fail; fall back to an empty
    // array rather than panicking in a daemon.
    serde_json::to_string(events).unwrap_or_else(|_| "[]".to_string())
}

/// Parse the JSON array carried over DBus back into events.
pub fn events_from_json(json: &str) -> Result<Vec<CompanionEvent>, serde_json::Error> {
    serde_json::from_str(json)
}

#[cfg(feature = "sqlcipher")]
impl From<notion_core::db::CalendarEvent> for CompanionEvent {
    fn from(e: notion_core::db::CalendarEvent) -> Self {
        CompanionEvent {
            id: e.id,
            title: e.title,
            start_time: e.start_time,
            end_time: e.end_time,
            all_day: e.all_day,
            location: e.location,
            description: e.description,
            block_id: e.block_id,
            last_modified: e.last_modified,
        }
    }
}

#[cfg(feature = "sqlcipher")]
impl From<CompanionEvent> for notion_core::db::CalendarEvent {
    fn from(e: CompanionEvent) -> Self {
        notion_core::db::CalendarEvent {
            id: e.id,
            title: e.title,
            start_time: e.start_time,
            end_time: e.end_time,
            all_day: e.all_day,
            location: e.location,
            description: e.description,
            block_id: e.block_id,
            last_modified: e.last_modified,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(id: &str, start: i64, end: i64) -> CompanionEvent {
        CompanionEvent {
            id: id.into(),
            title: "T".into(),
            start_time: start,
            end_time: end,
            all_day: false,
            location: None,
            description: None,
            block_id: None,
            last_modified: 0,
        }
    }

    #[test]
    fn overlap_is_half_open() {
        let e = ev("a", 100, 200);
        assert!(e.overlaps(150, 160));
        assert!(e.overlaps(50, 150));
        assert!(!e.overlaps(200, 300)); // touching end does not overlap
        assert!(!e.overlaps(0, 100)); // touching start does not overlap
    }

    #[test]
    fn json_round_trips_and_uses_camel_case() {
        let events = vec![ev("a", 100, 200)];
        let json = events_to_json(&events);
        assert!(json.contains("\"startTime\":100"));
        assert!(json.contains("\"allDay\":false"));
        let back = events_from_json(&json).unwrap();
        assert_eq!(back, events);
    }

    #[test]
    fn json_tolerates_missing_optional_fields() {
        // The GJS side may send only the required fields; defaults fill the rest.
        let json = r#"[{"id":"x","title":"Y","startTime":10,"endTime":20}]"#;
        let back = events_from_json(json).unwrap();
        assert!(!back[0].all_day);
        assert_eq!(back[0].location, None);
        assert_eq!(back[0].last_modified, 0);
    }
}
