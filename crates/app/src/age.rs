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

#[cfg(test)]
mod tests {
    use super::*;

    fn at(seconds: i64) -> jiff::Timestamp {
        jiff::Timestamp::from_second(seconds).expect("valid timestamp")
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
    fn age_future_timestamp_clamps_to_zero() {
        assert_eq!(format_age(Some(at(500)), at(100)), "0s");
    }
}
