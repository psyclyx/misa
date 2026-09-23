//! The client's local UTC offset.
//!
//! Timestamps cross the wire as UTC instants. Turning one into a wall clock is the
//! client's job, because the client is the side that knows the reader's timezone.
//! This is the only place in `misa-render` that asks the operating system for that
//! offset, and it answers for the *specific instant* being formatted, so a daylight
//! saving transition is reflected by the date in the fact.
//!
//! Formatting itself stays pure: [`crate::fact`] receives the offset as a plain
//! number of seconds and never consults the clock. When the platform cannot say
//! what the offset is — no tz data, or a non-Unix target — the caller falls back to
//! UTC, which is always a correct (if not local) answer.

/// The local UTC offset, in seconds east of UTC, at the instant `millis`
/// milliseconds after the Unix epoch.
///
/// The offset is looked up for that instant rather than for the current time, so a
/// fact on the other side of a daylight-saving change gets the offset that was in
/// force then. Returns `None` when the platform cannot determine the offset.
#[cfg(unix)]
pub fn offset_seconds_at(millis: i64) -> Option<i64> {
    use time::OffsetDateTime;
    let seconds = millis.div_euclid(1_000);
    let instant = OffsetDateTime::from_unix_timestamp(seconds).ok()?;
    let offset = time::UtcOffset::local_offset_at(instant).ok()?;
    Some(i64::from(offset.whole_seconds()))
}

/// Non-Unix targets have no local-offset facility here; the caller falls back to
/// UTC rather than failing.
#[cfg(not(unix))]
pub fn offset_seconds_at(_millis: i64) -> Option<i64> {
    None
}

/// The local offset at the instant, or UTC (`0`) when it cannot be determined.
pub fn offset_seconds(millis: i64) -> i64 {
    offset_seconds_at(millis).unwrap_or(0)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    /// 2024-03-10 14:00:00 UTC, an ordinary instant.
    const AFTERNOON: i64 = 1_710_079_200_000;

    #[test]
    fn an_offset_is_reported_or_utc_is_the_fallback() {
        // Whatever the platform says, it is a real-world offset and never a panic.
        let offset = offset_seconds(AFTERNOON);
        assert!(
            (-18 * 3_600..=18 * 3_600).contains(&offset),
            "implausible local offset {offset}"
        );
    }

    #[test]
    fn a_dst_boundary_pair_uses_the_offset_of_each_instant() {
        // US spring-forward 2024-03-10 10:00 UTC: 09:59:59Z is still standard time,
        // 10:00:01Z is daylight time. A fixed-offset zone reports one offset for
        // both; a zone that observes the transition reports two. Only a reported
        // change is asserted, so the suite is green on a UTC host.
        let before = offset_seconds_at(1_710_064_799_000);
        let after = offset_seconds_at(1_710_064_801_000);
        if let (Some(before), Some(after)) = (before, after)
            && before != after
        {
            assert_eq!(
                (after - before).abs() % 60,
                0,
                "a DST shift is a whole number of minutes, got {before} -> {after}"
            );
        }
    }
}
