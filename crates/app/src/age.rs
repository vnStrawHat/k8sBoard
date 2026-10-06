const SECONDS_PER_MINUTE: i64 = 60;
const SECONDS_PER_HOUR: i64 = 60 * SECONDS_PER_MINUTE;
const SECONDS_PER_DAY: i64 = 24 * SECONDS_PER_HOUR;

/// Formats the time since `created_at` as `Ns`, `Nm`, `Nh`, or `Nd`. A missing creation
/// time is a dash, and a timestamp in the future (clock skew) is clamped to `0s`.
pub(crate) fn format_age(created_at: Option<jiff::Timestamp>, now: jiff::Timestamp) -> String {
    let Some(created_at) = created_at else {
        return "—".to_owned();
    };
    let seconds = now
        .as_second()
        .saturating_sub(created_at.as_second())
        .max(0);
    match seconds {
        s if s < SECONDS_PER_MINUTE => format!("{s}s"),
        s if s < SECONDS_PER_HOUR => format!("{}m", s / SECONDS_PER_MINUTE),
        s if s < SECONDS_PER_DAY => format!("{}h", s / SECONDS_PER_HOUR),
        s => format!("{}d", s / SECONDS_PER_DAY),
    }
}

/// `2026-10-06 17:26 +07` for a drawer field: the date and clock of `zone`, which the caller
/// takes from the system so a drawer reads like the log tab and the tables.
pub(crate) fn format_local_time(time: jiff::Timestamp, zone: &jiff::tz::TimeZone) -> String {
    time.to_zoned(zone.clone())
        .strftime("%Y-%m-%d %H:%M %Z")
        .to_string()
}

/// A time left in Go duration style: `40s`, `3m20s`, `1h0m5s`. Negative values read as `0s`.
pub(crate) fn format_countdown(seconds: i64) -> String {
    let seconds = seconds.max(0);
    let (hours, minutes, seconds) = (
        seconds / SECONDS_PER_HOUR,
        seconds % SECONDS_PER_HOUR / SECONDS_PER_MINUTE,
        seconds % SECONDS_PER_MINUTE,
    );
    if hours > 0 {
        format!("{hours}h{minutes}m{seconds}s")
    } else if minutes > 0 {
        format!("{minutes}m{seconds}s")
    } else {
        format!("{seconds}s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(seconds: i64) -> jiff::Timestamp {
        jiff::Timestamp::from_second(seconds).expect("valid timestamp")
    }

    #[test]
    fn a_drawer_time_reads_in_the_given_zone_with_its_label() {
        let time: jiff::Timestamp = "2026-10-06T10:26:00Z".parse().expect("a timestamp");
        let utc = jiff::tz::TimeZone::UTC;
        assert_eq!(format_local_time(time, &utc), "2026-10-06 10:26 UTC");
        let plus_seven = jiff::tz::TimeZone::fixed(jiff::tz::offset(7));
        assert_eq!(format_local_time(time, &plus_seven), "2026-10-06 17:26 +07");
    }

    #[test]
    fn age_none_is_dash() {
        assert_eq!(format_age(None, at(1_000)), "—");
    }

    #[test]
    fn age_boundaries_seconds_minutes_hours_days() {
        let age = |elapsed: i64| format_age(Some(at(0)), at(elapsed));
        assert_eq!(age(59), "59s");
        assert_eq!(age(60), "1m");
        assert_eq!(age(59 * 60), "59m");
        assert_eq!(age(60 * 60), "1h");
        assert_eq!(age(23 * 3600), "23h");
        assert_eq!(age(24 * 3600), "1d");
    }

    #[test]
    fn countdown_text_forms() {
        assert_eq!(format_countdown(40), "40s");
        assert_eq!(format_countdown(200), "3m20s");
        assert_eq!(format_countdown(180), "3m0s");
        assert_eq!(format_countdown(3_605), "1h0m5s");
        assert_eq!(format_countdown(0), "0s");
        assert_eq!(format_countdown(-5), "0s");
    }

    #[test]
    fn age_future_timestamp_clamps_to_zero() {
        assert_eq!(format_age(Some(at(500)), at(100)), "0s");
    }
}
