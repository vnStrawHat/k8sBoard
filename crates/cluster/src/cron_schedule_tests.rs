use super::*;

fn ts(text: &str) -> Timestamp {
    text.parse().expect("valid timestamp")
}

fn parse(schedule: &str) -> CronSchedule {
    CronSchedule::parse(schedule, None).expect("valid schedule")
}

fn parse_in(schedule: &str, zone: &str) -> CronSchedule {
    CronSchedule::parse(schedule, Some(zone)).expect("valid schedule")
}

fn error_of(schedule: &str) -> ScheduleError {
    CronSchedule::parse(schedule, None).expect_err("invalid schedule")
}

fn show(run: &Zoned) -> String {
    run.strftime("%Y-%m-%d %H:%M %Z").to_string()
}

fn runs(schedule: &CronSchedule, after: &str, count: usize) -> Vec<String> {
    schedule
        .next_runs(ts(after), count)
        .iter()
        .map(show)
        .collect()
}

fn mask(values: &[u32]) -> u64 {
    values.iter().fold(0, |bits, value| bits | 1 << value)
}

fn field(text: &str, range: &FieldRange) -> (u64, bool) {
    parse_field(text, range).expect("valid field")
}

#[test]
fn parses_every_field_form() {
    assert_eq!(field("*", &MINUTES), ((1 << 60) - 1, true));
    assert_eq!(field("7", &MINUTES), (mask(&[7]), false));
    assert_eq!(field("10-12", &HOURS), (mask(&[10, 11, 12]), false));
    assert_eq!(field("*/15", &MINUTES), (mask(&[0, 15, 30, 45]), false));
    assert_eq!(field("5/20", &MINUTES), (mask(&[5, 25, 45]), false));
    assert_eq!(
        field("1,15,30", &DAYS_OF_MONTH),
        (mask(&[1, 15, 30]), false)
    );
    assert_eq!(
        field("1-10/3,20", &DAYS_OF_MONTH),
        (mask(&[1, 4, 7, 10, 20]), false)
    );
}

#[test]
fn empty_list_parts_are_skipped() {
    // robfig splits the list with `FieldsFunc`, which drops empty parts.
    assert_eq!(field("0,,5", &MINUTES), (mask(&[0, 5]), false));
    assert_eq!(field("5,", &MINUTES), (mask(&[5]), false));
    assert_eq!(field("*,", &MINUTES), ((1 << 60) - 1, true));
    assert_eq!(parse("0,,5 * * * *"), parse("0,5 * * * *"));
}

#[test]
fn question_mark_is_star() {
    let schedule = parse("0 0 ? * 1");
    assert_eq!(
        schedule.timetable,
        Timetable::Calendar(Calendar {
            minutes: mask(&[0]),
            hours: mask(&[0]),
            days_of_month: field("*", &DAYS_OF_MONTH).0,
            months: field("*", &MONTHS).0,
            days_of_week: mask(&[1]),
            is_day_of_month_star: true,
            is_day_of_week_star: false,
        })
    );
    // 2024-05-01 is a Wednesday and the 1st: only Mondays run.
    assert_eq!(
        runs(&schedule, "2024-05-01T00:00:00Z", 2),
        ["2024-05-06 00:00 UTC", "2024-05-13 00:00 UTC"]
    );
}

#[test]
fn names_in_ranges_with_step() {
    let schedule = parse("0 0 * jan-jun/2 mon-fri");
    let Timetable::Calendar(Calendar {
        months,
        days_of_week,
        ..
    }) = schedule.timetable
    else {
        panic!("calendar schedule");
    };
    assert_eq!(months, mask(&[1, 3, 5]));
    assert_eq!(days_of_week, mask(&[1, 2, 3, 4, 5]));
    assert_eq!(parse("0 0 * JAN-JUN/2 MON-FRI"), schedule);
    assert_eq!(parse("0 0 * Dec sUn"), parse("0 0 * 12 0"));
}

#[test]
fn descriptors_expand() {
    for (descriptor, fields) in [
        ("@yearly", "0 0 1 1 *"),
        ("@annually", "0 0 1 1 *"),
        ("@monthly", "0 0 1 * *"),
        ("@weekly", "0 0 * * 0"),
        ("@daily", "0 0 * * *"),
        ("@midnight", "0 0 * * *"),
        ("@hourly", "0 * * * *"),
    ] {
        assert_eq!(parse(descriptor), parse(fields), "{descriptor}");
    }
}

#[test]
fn rejects_bad_fields() {
    assert_eq!(error_of("* * * *"), ScheduleError::FieldCount(4));
    assert_eq!(error_of("* * * * * *"), ScheduleError::FieldCount(6));
    assert_eq!(error_of(""), ScheduleError::FieldCount(0));
    for (schedule, field) in [
        ("61 * * * *", "minute"),
        ("60 * * * *", "minute"),
        ("* 24 * * *", "hour"),
        ("* * 0 * *", "day of month"),
        ("* * 32 * *", "day of month"),
        ("* * * 0 *", "month"),
        ("* * * 13 *", "month"),
        ("* * * foo *", "month"),
        ("* * * * 7", "day of week"),
        ("*/0 * * * *", "minute"),
        ("5/0 * * * *", "minute"),
        ("5-1 * * * *", "minute"),
        ("1-2-3 * * * *", "minute"),
        ("1//2 * * * *", "minute"),
        ("*/2/2 * * * *", "minute"),
        (", * * * *", "minute"),
        ("*-5 * * * *", "minute"),
        ("*/ * * * *", "minute"),
    ] {
        assert!(
            matches!(error_of(schedule), ScheduleError::Value { field: name, .. } if name == field),
            "{schedule}"
        );
    }
}

#[test]
fn numbers_are_ascii_digits_only() {
    for schedule in [
        "+5 * * * *",
        "*/+5 * * * *",
        "\u{663} * * * *",
        "-1 * * * *",
    ] {
        assert!(
            matches!(error_of(schedule), ScheduleError::Value { .. }),
            "{schedule}"
        );
    }
}

#[test]
fn tz_prefix_checked_before_trim() {
    assert_eq!(
        error_of("TZ=UTC 0 0 * * *"),
        ScheduleError::Unsupported("TZ=".to_owned())
    );
    assert_eq!(
        error_of("CRON_TZ=Europe/Berlin 0 0 * * *"),
        ScheduleError::Unsupported("CRON_TZ=".to_owned())
    );
    // A leading space hides the prefix, so it reads as a sixth field.
    assert_eq!(
        error_of(" CRON_TZ=UTC 0 0 * * *"),
        ScheduleError::FieldCount(6)
    );
    assert_eq!(parse("  0 0 * * *\n"), parse("0 0 * * *"));
}

#[test]
fn unknown_descriptor_is_unsupported() {
    for descriptor in ["@reboot", "@Daily", "@daily extra", "@every"] {
        assert_eq!(
            error_of(descriptor),
            ScheduleError::Unsupported(descriptor.to_owned())
        );
    }
}

#[test]
fn star_flag_ignores_stepped_star() {
    let stars = |schedule: &str| match parse(schedule).timetable {
        Timetable::Calendar(Calendar {
            is_day_of_month_star,
            is_day_of_week_star,
            ..
        }) => (is_day_of_month_star, is_day_of_week_star),
        Timetable::Every { .. } => panic!("calendar schedule"),
    };
    assert_eq!(stars("0 0 * * *"), (true, true));
    assert_eq!(stars("0 0 */2 * */2"), (false, false));
    assert_eq!(stars("0 0 ?/2 * *"), (false, true));
    assert_eq!(stars("0 0 */1 * *"), (true, true));
    assert_eq!(stars("0 0 1-31 * 0-6"), (false, false));
    assert_eq!(stars("0 0 1,* * *"), (true, true));
}

#[test]
fn day_rule_ors_restricted_fields() {
    // The 1st and Mondays; 2024-06-01 is a Saturday.
    assert_eq!(
        runs(&parse("0 0 1 * 1"), "2024-05-27T00:00:00Z", 3),
        [
            "2024-06-01 00:00 UTC",
            "2024-06-03 00:00 UTC",
            "2024-06-10 00:00 UTC"
        ]
    );
}

#[test]
fn day_rule_ands_when_one_is_star() {
    assert_eq!(
        runs(&parse("0 0 * * 1"), "2024-05-01T00:00:00Z", 2),
        ["2024-05-06 00:00 UTC", "2024-05-13 00:00 UTC"]
    );
    assert_eq!(
        runs(&parse("0 0 15 * *"), "2024-05-01T00:00:00Z", 2),
        ["2024-05-15 00:00 UTC", "2024-06-15 00:00 UTC"]
    );
}

#[test]
fn next_after_is_strictly_later() {
    let schedule = parse("*/5 * * * *");
    let next = |after: &str| show(&schedule.next_after(ts(after)).expect("next run"));
    assert_eq!(next("2024-05-01T10:05:00Z"), "2024-05-01 10:10 UTC");
    assert_eq!(next("2024-05-01T10:04:59Z"), "2024-05-01 10:05 UTC");
    assert_eq!(next("2024-05-01T10:04:59.5Z"), "2024-05-01 10:05 UTC");
    assert_eq!(next("2024-05-01T10:05:00.5Z"), "2024-05-01 10:10 UTC");
}

#[test]
fn next_after_rolls_over_month_and_year() {
    assert_eq!(
        runs(&parse("0 0 1 1 *"), "2024-12-31T23:59:59Z", 1),
        ["2025-01-01 00:00 UTC"]
    );
    assert_eq!(
        runs(&parse("0 0 * * *"), "2024-02-28T12:00:00Z", 2),
        ["2024-02-29 00:00 UTC", "2024-03-01 00:00 UTC"]
    );
    assert_eq!(
        runs(&parse("0 0 31 * *"), "2024-04-01T00:00:00Z", 1),
        ["2024-05-31 00:00 UTC"]
    );
}

#[test]
fn next_after_finds_leap_day() {
    assert_eq!(
        runs(&parse("0 0 29 2 *"), "2024-03-01T00:00:00Z", 1),
        ["2028-02-29 00:00 UTC"]
    );
}

#[test]
fn impossible_schedule_has_no_next() {
    assert_eq!(
        parse("0 0 30 2 *").next_after(ts("2024-01-01T00:00:00Z")),
        None
    );
}

#[test]
fn next_runs_use_the_zone() {
    let schedule = parse_in("0 9 * * *", "Europe/Berlin");
    assert_eq!(
        runs(&schedule, "2024-03-29T12:00:00Z", 3),
        [
            "2024-03-30 09:00 CET",
            "2024-03-31 09:00 CEST",
            "2024-04-01 09:00 CEST"
        ]
    );
    let first = schedule
        .next_after(ts("2024-03-29T12:00:00Z"))
        .expect("next run");
    assert_eq!(first.timestamp(), ts("2024-03-30T08:00:00Z"));
}

#[test]
fn spring_forward_skips_the_missing_hour() {
    // 02:30 does not exist on 2025-03-09 in New York, so that day has no run.
    let schedule = parse_in("30 2 * * *", "America/New_York");
    assert_eq!(
        runs(&schedule, "2025-03-07T12:00:00Z", 3),
        [
            "2025-03-08 02:30 EST",
            "2025-03-10 02:30 EDT",
            "2025-03-11 02:30 EDT"
        ]
    );
}

#[test]
fn fall_back_repeats_the_hour() {
    // 01:30 happens twice on 2025-11-02 in New York.
    let schedule = parse_in("30 1 * * *", "America/New_York");
    let found = schedule.next_runs(ts("2025-11-01T12:00:00Z"), 3);
    let shown: Vec<String> = found.iter().map(show).collect();
    assert_eq!(
        shown,
        [
            "2025-11-02 01:30 EDT",
            "2025-11-02 01:30 EST",
            "2025-11-03 01:30 EST"
        ]
    );
    let gap = found[1].timestamp().duration_since(found[0].timestamp());
    assert_eq!(gap, SignedDuration::from_hours(1));
}

#[test]
fn every_parses_go_durations() {
    let delay = |schedule: &str| match parse(schedule).timetable {
        Timetable::Every { delay, .. } => delay,
        Timetable::Calendar(_) => panic!("every schedule"),
    };
    let seconds = |schedule: &str| delay(schedule).as_secs();
    assert_eq!(delay("@every 1h30m"), SignedDuration::from_secs(5400));
    assert_eq!(delay("@every 90s"), SignedDuration::from_secs(90));
    assert_eq!(delay("@every 2h"), SignedDuration::from_hours(2));
    // Decimals, sub-second units, a sign, and a bare zero follow Go `time.ParseDuration`.
    assert_eq!(seconds("@every 1.5h"), 5400);
    assert_eq!(seconds("@every 0.5h"), 1800);
    assert_eq!(seconds("@every 1h30m15.5s"), 5415);
    assert_eq!(seconds("@every 2500ms"), 2);
    assert_eq!(seconds("@every 90000000us"), 90);
    assert_eq!(seconds("@every 90000000\u{b5}s"), 90);
    assert_eq!(seconds("@every 5000000000ns"), 5);
    assert_eq!(seconds("@every +5s"), 5);
    // Under one second, zero, and negative values read as one second (robfig `Every`).
    for schedule in [
        "@every 0s",
        "@every 0",
        "@every -5s",
        "@every .5s",
        "@every 10ms",
        "@every 100ns",
    ] {
        assert_eq!(seconds(schedule), 1, "{schedule}");
    }
    for schedule in [
        "@every 5",
        "@every 1d",
        "@every 1h 30m",
        "@every h",
        "@every .",
        "@every 1.2.3s",
        "@every 5x",
        "@every 9999999999999999999h",
        "@every --5s",
    ] {
        assert!(
            matches!(
                error_of(schedule),
                ScheduleError::Value { field: "every", .. }
            ),
            "{schedule}"
        );
    }
}

#[test]
fn every_runs_from_the_anchor() {
    let schedule = parse("@every 1h").anchored_at(Some(ts("2024-05-01T00:15:00Z")));
    let next = |after: &str| show(&schedule.next_after(ts(after)).expect("next run"));
    assert_eq!(next("2024-05-01T10:30:00Z"), "2024-05-01 11:15 UTC");
    assert_eq!(next("2024-05-01T11:15:00Z"), "2024-05-01 12:15 UTC");
    assert_eq!(next("2024-04-30T00:00:00Z"), "2024-05-01 01:15 UTC");
    assert_eq!(
        runs(&schedule, "2024-05-01T10:30:00Z", 2),
        ["2024-05-01 11:15 UTC", "2024-05-01 12:15 UTC"]
    );
}

#[test]
fn anchor_does_not_change_a_calendar_schedule() {
    let calendar = parse("0 0 * * *");
    assert_eq!(
        calendar
            .clone()
            .anchored_at(Some(ts("2024-05-01T00:15:00Z"))),
        calendar
    );
}

#[test]
fn every_without_anchor_has_no_next() {
    assert_eq!(
        parse("@every 1h").next_after(ts("2024-05-01T00:00:00Z")),
        None
    );
}

#[test]
fn unknown_zone_is_an_error() {
    assert_eq!(
        CronSchedule::parse("* * * * *", Some("Mars/Base")),
        Err(ScheduleError::UnknownZone("Mars/Base".to_owned()))
    );
}

#[test]
fn unset_zone_has_no_name() {
    assert_eq!(parse("* * * * *").zone_name(), None);
    assert_eq!(parse_in("* * * * *", "UTC").zone_name(), Some("UTC"));
}

#[test]
fn day_step_undoes_a_dst_shift_of_midnight() {
    // Santiago skips 00:00 on 2025-09-07 (clocks go from 24:00 to 01:00). Stepping from
    // Saturday to the missing Sunday midnight lands on 01:00; the fix-up must pull it back
    // to the previous day, or Monday 00:00 would be missed and the run would slip a week.
    let schedule = parse_in("0 0 * * 1", "America/Santiago");
    assert_eq!(
        runs(&schedule, "2025-09-04T12:00:00Z", 2),
        ["2025-09-08 00:00 -03", "2025-09-15 00:00 -03"]
    );
}

#[test]
fn gap_day_keeps_a_later_hour_in_a_west_zone() {
    // Go resolves Santiago's skipped midnight to 23:00 of the day before (the day step then
    // adds an hour), so Sunday's 01:00 run still happens.
    let schedule = parse_in("0 1 * * 0", "America/Santiago");
    assert_eq!(
        runs(&schedule, "2025-09-06T12:00:00Z", 2),
        ["2025-09-07 01:00 -03", "2025-09-14 01:00 -03"]
    );
}

#[test]
fn descriptor_check_comes_before_any_trimming() {
    // Like robfig: a leading space hides the `@`, and a trailing one spoils the descriptor.
    assert_eq!(error_of(" @daily"), ScheduleError::FieldCount(1));
    assert_eq!(
        error_of("@daily "),
        ScheduleError::Unsupported("@daily ".to_owned())
    );
    assert!(matches!(
        error_of("@every 1h "),
        ScheduleError::Value { field: "every", .. }
    ));
}

#[test]
fn fold_east_of_utc_takes_the_later_occurrence() {
    let wall = |text: &str| -> DateTime { text.parse().expect("valid civil time") };
    let zone = |name: &str| jiff::tz::db().get(name).expect("bundled zone");
    // Berlin repeats 02:30 on 2025-10-26: CEST at 00:30Z, then CET at 01:30Z. Go takes the second.
    let berlin = civil(&zone("Europe/Berlin"), wall("2025-10-26T02:30:00")).expect("resolved");
    assert_eq!(berlin.timestamp(), ts("2025-10-26T01:30:00Z"));
    // New York repeats 01:30 on 2025-11-02: EDT at 05:30Z, then EST. Go takes the first.
    let new_york = civil(&zone("America/New_York"), wall("2025-11-02T01:30:00")).expect("resolved");
    assert_eq!(new_york.timestamp(), ts("2025-11-02T05:30:00Z"));
    // A gap east of UTC still moves forward: Berlin skips 02:30 on 2025-03-30.
    let skipped = civil(&zone("Europe/Berlin"), wall("2025-03-30T02:30:00")).expect("resolved");
    assert_eq!(skipped.timestamp(), ts("2025-03-30T01:30:00Z"));
}

#[test]
fn is_every_tells_anchored_schedules_apart() {
    assert!(parse("@every 1h").is_every());
    assert!(!parse("@hourly").is_every());
    assert!(!parse("0 * * * *").is_every());
}
