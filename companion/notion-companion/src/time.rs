//! Local-day boundary math and `YYYY-MM-DD HH:MM` parsing.
//!
//! "Today's events" is a *local* notion, but the DB stores UTC seconds. The pure
//! functions here take an explicit `utc_offset_secs` so they are fully
//! deterministic and unit-tested; only [`local_offset_secs`] touches the real
//! system clock (thin glue for the daemon/app).

use chrono::{FixedOffset, NaiveDateTime, TimeZone};
use thiserror::Error;

pub const SECS_PER_DAY: i64 = 86_400;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum TimeError {
    #[error("could not parse date-time: {0:?}")]
    Unparseable(String),
    #[error("utc offset out of range")]
    BadOffset,
}

/// The `[start, end)` Unix-second bounds of the local day containing `now`.
///
/// `utc_offset_secs` is the local zone's offset from UTC (e.g. `+7*3600` for
/// UTC+7). Uses Euclidean division so it is correct for negative offsets and
/// pre-epoch instants.
pub fn day_bounds(now: i64, utc_offset_secs: i64) -> (i64, i64) {
    let local = now + utc_offset_secs;
    let local_midnight = local.div_euclid(SECS_PER_DAY) * SECS_PER_DAY;
    let start = local_midnight - utc_offset_secs;
    (start, start + SECS_PER_DAY)
}

/// Bounds spanning `days` local days starting at the local midnight of `now`
/// (e.g. `days = 2` covers today **and** tomorrow — what the top bar shows).
pub fn multiday_bounds(now: i64, utc_offset_secs: i64, days: i64) -> (i64, i64) {
    let (start, _) = day_bounds(now, utc_offset_secs);
    (start, start + days.max(1) * SECS_PER_DAY)
}

/// Parse a naive local date-time string to Unix seconds, given the local
/// offset. Accepts `YYYY-MM-DD HH:MM[:SS]` and the ISO `T` separator, which are
/// the shapes the AI is instructed to emit.
pub fn parse_naive_local(s: &str, utc_offset_secs: i64) -> Result<i64, TimeError> {
    let offset =
        FixedOffset::east_opt(i32::try_from(utc_offset_secs).map_err(|_| TimeError::BadOffset)?)
            .ok_or(TimeError::BadOffset)?;

    const FORMATS: &[&str] = &[
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%d %H:%M",
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%dT%H:%M",
    ];
    let trimmed = s.trim();
    for fmt in FORMATS {
        if let Ok(naive) = NaiveDateTime::parse_from_str(trimmed, fmt) {
            // A wall-clock time can be invalid/ambiguous across DST folds; a
            // fixed offset never folds, so `.single()` always resolves.
            if let Some(dt) = offset.from_local_datetime(&naive).single() {
                return Ok(dt.timestamp());
            }
        }
    }
    Err(TimeError::Unparseable(trimmed.to_string()))
}

/// The current local UTC offset in seconds, from the system time zone.
pub fn local_offset_secs() -> i64 {
    use chrono::{Local, Offset};
    i64::from(Local::now().offset().fix().local_minus_utc())
}

/// Format a Unix-second instant as `YYYY-MM-DD HH:MM` in the given local offset
/// (used to stamp "current time" into the AI system prompt).
pub fn format_local(ts: i64, utc_offset_secs: i64) -> String {
    let offset = FixedOffset::east_opt(utc_offset_secs.clamp(-86_399, 86_399) as i32)
        .unwrap_or_else(|| FixedOffset::east_opt(0).unwrap());
    offset
        .timestamp_opt(ts, 0)
        .single()
        .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    // 2023-11-15 12:00:00 UTC.
    const NOON_UTC: i64 = 1_700_049_600;

    #[test]
    fn day_bounds_utc() {
        let (start, end) = day_bounds(NOON_UTC, 0);
        assert_eq!(start, 1_700_006_400); // 2023-11-15 00:00 UTC
        assert_eq!(end, start + SECS_PER_DAY);
        assert!(start <= NOON_UTC && NOON_UTC < end);
    }

    #[test]
    fn day_bounds_positive_offset_shifts_local_midnight() {
        // At UTC+7, local time is 19:00 on the 15th; the local day is still the
        // 15th, whose local midnight (00:00+07) is 17:00 UTC on the 14th.
        let (start, end) = day_bounds(NOON_UTC, 7 * 3600);
        assert_eq!(end - start, SECS_PER_DAY);
        assert!(start <= NOON_UTC && NOON_UTC < end);
        // local midnight = 00:00 local = start; start + offset is a UTC midnight boundary.
        assert_eq!((start + 7 * 3600).rem_euclid(SECS_PER_DAY), 0);
    }

    #[test]
    fn day_bounds_negative_offset() {
        // UTC-8: local time is 04:00 on the 15th; still the 15th locally.
        let (start, end) = day_bounds(NOON_UTC, -8 * 3600);
        assert_eq!(end - start, SECS_PER_DAY);
        assert!(start <= NOON_UTC && NOON_UTC < end);
        assert_eq!((start - 8 * 3600).rem_euclid(SECS_PER_DAY), 0);
    }

    #[test]
    fn multiday_covers_today_and_tomorrow() {
        let (start, end) = multiday_bounds(NOON_UTC, 0, 2);
        assert_eq!(end - start, 2 * SECS_PER_DAY);
    }

    #[test]
    fn parse_naive_local_utc_and_offset() {
        // 2023-11-15 09:30 at UTC == 1_700_040_600.
        assert_eq!(
            parse_naive_local("2023-11-15 09:30", 0).unwrap(),
            1_700_040_600
        );
        // Same wall clock at UTC+2 is two hours earlier in UTC.
        assert_eq!(
            parse_naive_local("2023-11-15 09:30", 2 * 3600).unwrap(),
            1_700_040_600 - 2 * 3600
        );
        // ISO 'T' separator + seconds also parse.
        assert_eq!(
            parse_naive_local("2023-11-15T09:30:00", 0).unwrap(),
            1_700_040_600
        );
    }

    #[test]
    fn parse_rejects_garbage() {
        assert!(matches!(
            parse_naive_local("next tuesday-ish", 0),
            Err(TimeError::Unparseable(_))
        ));
    }

    #[test]
    fn format_local_round_trips_with_parse() {
        let s = format_local(NOON_UTC, 0);
        assert_eq!(s, "2023-11-15 12:00");
        assert_eq!(parse_naive_local(&s, 0).unwrap(), NOON_UTC);
    }
}
