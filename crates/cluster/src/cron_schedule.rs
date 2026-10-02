//! CronJob schedules: a port of robfig/cron v3 `ParseStandard` and `Next`, the library the
//! Kubernetes CronJob controller uses, so the next run shown here equals the controller's.

use jiff::civil::{Date, DateTime, Time};
use jiff::tz::{AmbiguousOffset, TimeZone};
use jiff::{SignedDuration, Span, Timestamp, Zoned};

/// The controller gives up after this many years without a match.
const YEAR_LIMIT: i16 = 5;
const NANOS_PER_SECOND: u128 = 1_000_000_000;

/// A parsed CronJob `schedule` in its `timeZone`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CronSchedule {
    timetable: Timetable,
    zone: TimeZone,
    /// `None`: `timeZone` unset, UTC assumed.
    zone_name: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Timetable {
    Calendar(Calendar),
    /// `@every`: runs `delay` after the anchor (lastScheduleTime, else creationTimestamp).
    Every {
        delay: SignedDuration,
        anchor: Option<Timestamp>,
    },
}

/// The five `ParseStandard` fields; bit n of a mask is set when the value n matches.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Calendar {
    minutes: u64,
    hours: u64,
    days_of_month: u64,
    months: u64,
    days_of_week: u64,
    is_day_of_month_star: bool,
    is_day_of_week_star: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ScheduleError {
    #[error("expected 5 fields, found {0}")]
    FieldCount(usize),
    #[error("invalid {field} value '{text}'")]
    Value { field: &'static str, text: String },
    /// `TZ=`, `CRON_TZ=`, and descriptors other than the supported ones.
    #[error("'{0}' schedules are not supported")]
    Unsupported(String),
    #[error("unknown time zone '{0}'")]
    UnknownZone(String),
}

/// The accepted values of one field; `names[i]` stands for `min + i`.
struct FieldRange {
    name: &'static str,
    min: u32,
    max: u32,
    names: &'static [&'static str],
}

const MINUTES: FieldRange = FieldRange {
    name: "minute",
    min: 0,
    max: 59,
    names: &[],
};
const HOURS: FieldRange = FieldRange {
    name: "hour",
    min: 0,
    max: 23,
    names: &[],
};
const DAYS_OF_MONTH: FieldRange = FieldRange {
    name: "day of month",
    min: 1,
    max: 31,
    names: &[],
};
const MONTHS: FieldRange = FieldRange {
    name: "month",
    min: 1,
    max: 12,
    names: &[
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ],
};
const DAYS_OF_WEEK: FieldRange = FieldRange {
    name: "day of week",
    min: 0,
    max: 6,
    names: &["sun", "mon", "tue", "wed", "thu", "fri", "sat"],
};

impl CronSchedule {
    /// Parses `schedule` (five fields or a descriptor) in `time_zone` (UTC when `None`).
    pub fn parse(schedule: &str, time_zone: Option<&str>) -> Result<Self, ScheduleError> {
        let timetable = parse_timetable(schedule)?;
        let zone = match time_zone {
            Some(name) => jiff::tz::db()
                .get(name)
                .map_err(|_| ScheduleError::UnknownZone(name.to_owned()))?,
            None => TimeZone::UTC,
        };
        Ok(Self {
            timetable,
            zone,
            zone_name: time_zone.map(str::to_owned),
        })
    }

    /// Sets the `@every` anchor; no effect on calendar schedules. The summarizer passes
    /// `lastScheduleTime.or(creationTimestamp)`.
    #[must_use]
    pub fn anchored_at(mut self, anchor: Option<Timestamp>) -> Self {
        if let Timetable::Every { anchor: slot, .. } = &mut self.timetable {
            *slot = anchor;
        }
        self
    }

    /// The first run strictly after `after`; `None` past the 5-year limit or without an
    /// `@every` anchor.
    pub fn next_after(&self, after: Timestamp) -> Option<Zoned> {
        match &self.timetable {
            Timetable::Calendar(calendar) => calendar.next(&self.zone, after),
            Timetable::Every { delay, anchor } => {
                let next = next_every_run(*delay, (*anchor)?, after)?;
                Some(next.to_zoned(self.zone.clone()))
            }
        }
    }

    /// Up to `count` consecutive runs after `after`.
    pub fn next_runs(&self, after: Timestamp, count: usize) -> Vec<Zoned> {
        let mut runs = Vec::with_capacity(count);
        let mut cursor = after;
        while runs.len() < count {
            let Some(run) = self.next_after(cursor) else {
                break;
            };
            cursor = run.timestamp();
            runs.push(run);
        }
        runs
    }

    pub fn zone_name(&self) -> Option<&str> {
        self.zone_name.as_deref()
    }

    /// Whether this is an `@every` schedule, which counts from an anchor instead of the calendar
    /// and has no next run until it has one.
    pub fn is_every(&self) -> bool {
        matches!(self.timetable, Timetable::Every { .. })
    }
}

impl Calendar {
    /// A direct port of robfig `SpecSchedule.Next`. Civil steps (month, day) go through the
    /// zone with Go `time.Date` disambiguation; hour, minute, and second steps are absolute,
    /// so a spring-forward skips the missing hour and a fall-back repeats one.
    fn next(&self, zone: &TimeZone, after: Timestamp) -> Option<Zoned> {
        let first_second = Timestamp::from_second(after.as_second().checked_add(1)?).ok()?;
        let mut t = first_second.to_zoned(zone.clone());
        let year_limit = t.year().saturating_add(YEAR_LIMIT);
        // Once the time moved, the finer parts of the old time no longer matter.
        let mut is_moved = false;
        'wrap: loop {
            if t.year() > year_limit {
                return None;
            }
            while !has(self.months, t.month()) {
                let first_of_month = t.date().first_of_month();
                if !is_moved {
                    is_moved = true;
                    t = civil(zone, first_of_month.to_datetime(Time::midnight()))?;
                }
                let next_month = first_of_month.checked_add(Span::new().months(1)).ok()?;
                t = civil(zone, next_month.to_datetime(t.time()))?;
                if t.month() == 1 {
                    continue 'wrap;
                }
            }
            while !self.day_matches(&t) {
                if !is_moved {
                    is_moved = true;
                    t = civil(zone, t.date().to_datetime(Time::midnight()))?;
                }
                let tomorrow: Date = t.date().tomorrow().ok()?;
                t = civil(zone, tomorrow.to_datetime(t.time()))?;
                // A DST change moved midnight: bring the time back to the start of the day.
                if t.hour() != 0 {
                    let hour = i64::from(t.hour());
                    let shift = if hour > 12 { 24 - hour } else { -hour };
                    t = t.checked_add(SignedDuration::from_hours(shift)).ok()?;
                }
                if t.day() == 1 {
                    continue 'wrap;
                }
            }
            while !has(self.hours, t.hour()) {
                if !is_moved {
                    is_moved = true;
                    t = civil(zone, t.date().at(t.hour(), 0, 0, 0))?;
                }
                t = t.checked_add(SignedDuration::from_hours(1)).ok()?;
                if t.hour() == 0 {
                    continue 'wrap;
                }
            }
            while !has(self.minutes, t.minute()) {
                if !is_moved {
                    is_moved = true;
                    t = t
                        .checked_sub(SignedDuration::from_secs(i64::from(t.second())))
                        .ok()?;
                }
                t = t.checked_add(SignedDuration::from_mins(1)).ok()?;
                if t.minute() == 0 {
                    continue 'wrap;
                }
            }
            // Only second 0 matches (`ParseStandard` fixes the seconds field), and the next
            // whole minute starts the search over, as robfig's one-second loop does.
            if t.second() != 0 {
                is_moved = true;
                let to_next_minute = SignedDuration::from_secs(i64::from(60 - t.second()));
                t = t.checked_add(to_next_minute).ok()?;
                continue 'wrap;
            }
            return Some(t);
        }
    }

    fn day_matches(&self, t: &Zoned) -> bool {
        let is_month_day = has(self.days_of_month, t.day());
        let is_week_day = has(self.days_of_week, t.weekday().to_sunday_zero_offset());
        if self.is_day_of_month_star || self.is_day_of_week_star {
            is_month_day && is_week_day
        } else {
            is_month_day || is_week_day
        }
    }
}

fn has(mask: u64, value: i8) -> bool {
    u32::try_from(value).is_ok_and(|bit| mask.checked_shr(bit).is_some_and(|rest| rest & 1 == 1))
}

/// Go `time.Date`. Go reads the wall time as if it were UTC to pick the zone period, so the
/// answer depends on which side of UTC the zone lies. West of UTC (negative offset before the
/// change) the wall time lands before the change: a gap resolves with the offset after it and
/// moves back (Santiago's skipped midnight becomes 23:00 of the day before), and a repeated
/// time takes the first occurrence. East of UTC (positive offset before the change) it lands
/// after the change: a repeated time takes the second occurrence, and a gap moves forward.
fn civil(zone: &TimeZone, datetime: DateTime) -> Option<Zoned> {
    let ambiguous = zone.to_ambiguous_zoned(datetime);
    match ambiguous.offset() {
        AmbiguousOffset::Gap { before, .. } if before.is_negative() => ambiguous.earlier().ok(),
        AmbiguousOffset::Fold { before, .. } if before.is_positive() => ambiguous.later().ok(),
        _ => ambiguous.compatible().ok(),
    }
}

/// The smallest `anchor + k * delay` (k >= 1) after `after`.
fn next_every_run(delay: SignedDuration, anchor: Timestamp, after: Timestamp) -> Option<Timestamp> {
    let elapsed = after.duration_since(anchor).as_nanos();
    let periods = if elapsed < 0 {
        1
    } else {
        elapsed / delay.as_nanos() + 1
    };
    let seconds = delay.as_secs().checked_mul(i64::try_from(periods).ok()?)?;
    anchor.checked_add(SignedDuration::from_secs(seconds)).ok()
}

fn parse_timetable(schedule: &str) -> Result<Timetable, ScheduleError> {
    // robfig reads the zone prefix, then the `@`, before it splits on whitespace, so a leading
    // space hides both and leaves a descriptor as an ordinary (invalid) field.
    for prefix in ["TZ=", "CRON_TZ="] {
        if schedule.starts_with(prefix) {
            return Err(ScheduleError::Unsupported(prefix.to_owned()));
        }
    }
    if let Some(delay) = schedule.strip_prefix("@every ") {
        let delay = parse_delay(delay).ok_or_else(|| ScheduleError::Value {
            field: "every",
            text: delay.to_owned(),
        })?;
        return Ok(Timetable::Every {
            delay,
            anchor: None,
        });
    }
    if schedule.starts_with('@') {
        let fields = match schedule {
            "@yearly" | "@annually" => "0 0 1 1 *",
            "@monthly" => "0 0 1 * *",
            "@weekly" => "0 0 * * 0",
            "@daily" | "@midnight" => "0 0 * * *",
            "@hourly" => "0 * * * *",
            _ => return Err(ScheduleError::Unsupported(schedule.to_owned())),
        };
        return parse_calendar(fields);
    }
    parse_calendar(schedule)
}

fn parse_calendar(fields: &str) -> Result<Timetable, ScheduleError> {
    let fields: Vec<&str> = fields.split_ascii_whitespace().collect();
    let &[minute, hour, day_of_month, month, day_of_week] = fields.as_slice() else {
        return Err(ScheduleError::FieldCount(fields.len()));
    };
    let (minutes, _) = parse_field(minute, &MINUTES)?;
    let (hours, _) = parse_field(hour, &HOURS)?;
    let (days_of_month, is_day_of_month_star) = parse_field(day_of_month, &DAYS_OF_MONTH)?;
    let (months, _) = parse_field(month, &MONTHS)?;
    let (days_of_week, is_day_of_week_star) = parse_field(day_of_week, &DAYS_OF_WEEK)?;
    Ok(Timetable::Calendar(Calendar {
        minutes,
        hours,
        days_of_month,
        months,
        days_of_week,
        is_day_of_month_star,
        is_day_of_week_star,
    }))
}

/// A comma list of parts: the value bits, and whether any part is an unstepped star.
/// Empty parts are skipped (robfig splits with `FieldsFunc`); a field with no value at all,
/// such as `,`, is an error.
fn parse_field(field: &str, range: &FieldRange) -> Result<(u64, bool), ScheduleError> {
    let invalid = |text: &str| ScheduleError::Value {
        field: range.name,
        text: text.to_owned(),
    };
    let mut bits = 0;
    let mut is_star = false;
    for part in field.split(',').filter(|part| !part.is_empty()) {
        let (part_bits, is_part_star) = parse_part(part, range).ok_or_else(|| invalid(part))?;
        bits |= part_bits;
        is_star |= is_part_star;
    }
    if bits == 0 {
        return Err(invalid(field));
    }
    Ok((bits, is_star))
}

/// One list part: `*`, `?`, `N`, or `N-M`, each with an optional `/step`.
fn parse_part(part: &str, range: &FieldRange) -> Option<(u64, bool)> {
    let (span, step) = match part.split_once('/') {
        Some((span, step)) => (span, Some(parse_number(step)?)),
        None => (part, None),
    };
    if step == Some(0) {
        return None;
    }
    // Deviation: robfig `getRange` takes any part whose first hyphen-split piece is `*` or
    // `?` as the whole range, so `*-5` reads as `*`. Only an exact `*` or `?` is accepted here.
    let is_all = span == "*" || span == "?";
    let (start, end) = if is_all {
        (range.min, range.max)
    } else {
        let mut ends = span.split('-');
        let start = parse_value(ends.next()?, range)?;
        let end = match (ends.next(), step) {
            (Some(end), _) => parse_value(end, range)?,
            // `N/step` means `N-max/step`.
            (None, Some(_)) => range.max,
            (None, None) => start,
        };
        if ends.next().is_some() {
            return None;
        }
        (start, end)
    };
    if start < range.min || end > range.max || start > end {
        return None;
    }
    let step = step.unwrap_or(1);
    let bits = (start..=end)
        .step_by(step as usize)
        .fold(0_u64, |bits, value| bits | 1 << value);
    Some((bits, is_all && step <= 1))
}

fn parse_value(text: &str, range: &FieldRange) -> Option<u32> {
    let named = (range.min..)
        .zip(range.names)
        .find(|(_, name)| name.eq_ignore_ascii_case(text));
    match named {
        Some((value, _)) => Some(value),
        None => parse_number(text),
    }
}

/// ASCII digits only: `parse` alone would accept a leading `+`.
fn parse_number(text: &str) -> Option<u32> {
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

/// A Go `time.ParseDuration` value: an optional sign, then `<decimal><unit>` groups with the
/// units `ns`, `us`, `µs`, `ms`, `s`, `m`, `h`; a bare `0` is valid. The total is cut to
/// whole seconds, and anything under one second (a negative value too) becomes one second
/// (robfig `Every`).
fn parse_delay(text: &str) -> Option<SignedDuration> {
    let (is_negative, mut rest) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text.strip_prefix('+').unwrap_or(text)),
    };
    let mut nanos: u128 = 0;
    if rest != "0" {
        if rest.is_empty() {
            return None;
        }
        while !rest.is_empty() {
            let (group_nanos, remaining) = parse_delay_group(rest)?;
            nanos = nanos.checked_add(group_nanos)?;
            rest = remaining;
        }
    }
    // Go rejects a total beyond `i64::MAX` nanoseconds.
    i64::try_from(nanos).ok()?;
    let seconds = if is_negative {
        0
    } else {
        i64::try_from(nanos / NANOS_PER_SECOND).ok()?
    };
    Some(SignedDuration::from_secs(seconds.max(1)))
}

/// One `<decimal><unit>` group: its nanoseconds and the text after it.
fn parse_delay_group(text: &str) -> Option<(u128, &str)> {
    let integer_length = text.bytes().take_while(u8::is_ascii_digit).count();
    let (integer, after_integer) = text.split_at(integer_length);
    let (fraction, after_number) = match after_integer.strip_prefix('.') {
        Some(after_dot) => {
            let fraction_length = after_dot.bytes().take_while(u8::is_ascii_digit).count();
            after_dot.split_at(fraction_length)
        }
        None => ("", after_integer),
    };
    if integer.is_empty() && fraction.is_empty() {
        return None;
    }
    let unit_length = after_number
        .find(|character: char| character == '.' || character.is_ascii_digit())
        .unwrap_or(after_number.len());
    let (unit, remaining) = after_number.split_at(unit_length);
    let unit_nanos: u128 = match unit {
        "ns" => 1,
        "us" | "\u{b5}s" | "\u{3bc}s" => 1_000,
        "ms" => 1_000_000,
        "s" => NANOS_PER_SECOND,
        "m" => 60 * NANOS_PER_SECOND,
        "h" => 3_600 * NANOS_PER_SECOND,
        _ => return None,
    };
    let whole = if integer.is_empty() {
        0
    } else {
        integer.parse::<u128>().ok()?
    };
    // Eighteen fraction digits are far below one nanosecond for every unit.
    let fraction = fraction.get(..fraction.len().min(18)).unwrap_or_default();
    let (fraction_value, fraction_scale) = if fraction.is_empty() {
        (0, 1)
    } else {
        let scale = fraction.bytes().fold(1_u128, |scale, _| scale * 10);
        (fraction.parse::<u128>().ok()?, scale)
    };
    let nanos = whole
        .checked_mul(unit_nanos)?
        .checked_add(fraction_value * unit_nanos / fraction_scale)?;
    Some((nanos, remaining))
}

#[cfg(test)]
#[path = "cron_schedule_tests.rs"]
mod cron_schedule_tests;
