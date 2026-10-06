# 0012 · Cron schedules

[Back to index](README.md) · Step 1a · Module: `crates/cluster/src/cron_schedule.rs` + `cron_schedule_tests.rs`. Display rules (app) are at the end. Source of truth: robfig/cron v3 `ParseStandard` and `SpecSchedule.Next` / `ConstantDelaySchedule.Next`, which the CronJob controller uses.

## API

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CronSchedule { timetable: Timetable, zone: jiff::tz::TimeZone, // Clone + Eq + Debug in jiff 0.2
    zone_name: Option<String> }                                          // None: `timeZone` unset, UTC assumed
#[derive(Clone, Debug, PartialEq, Eq)]
enum Timetable {
    Calendar(Calendar),
    /// `@every`: runs `delay` after the anchor (lastScheduleTime, else creationTimestamp).
    Every { delay: jiff::SignedDuration, anchor: Option<jiff::Timestamp> },
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Calendar { minutes: u64, hours: u64, days_of_month: u64, months: u64, days_of_week: u64, // bit n = value n
                  is_day_of_month_star: bool, is_day_of_week_star: bool }  // `next(&self, zone, after)` ports robfig `Next`
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ScheduleError {
    #[error("expected 5 fields, found {0}")] FieldCount(usize),
    #[error("invalid {field} value '{text}'")] Value { field: &'static str, text: String },
    #[error("'{0}' schedules are not supported")] Unsupported(String), // TZ=, CRON_TZ=, other @…
    #[error("unknown time zone '{0}'")] UnknownZone(String),
}
impl CronSchedule {
    pub fn parse(schedule: &str, time_zone: Option<&str>) -> Result<Self, ScheduleError>;
    /// Sets the `@every` anchor; no effect on calendar schedules. The summarizer passes
    /// `lastScheduleTime.or(creationTimestamp)`.
    pub fn anchored_at(self, anchor: Option<jiff::Timestamp>) -> Self;
    /// The first run strictly after `after`; `None` past the 5-year limit or without an `@every` anchor.
    pub fn next_after(&self, after: jiff::Timestamp) -> Option<jiff::Zoned>;
    pub fn next_runs(&self, after: jiff::Timestamp, count: usize) -> Vec<jiff::Zoned>;
    pub fn zone_name(&self) -> Option<&str>;
}
```

Error texts carry only schedule text. The module does not log.

## Grammar (`ParseStandard`), in this order

| Rule | Detail |
|---|---|
| 1. Prefix | `TZ=` or `CRON_TZ=` at the start → `Unsupported` (the API rejects them for new objects since 1.27) |
| 2. Descriptor check | `@every ` and any other `@` are read from the untrimmed text, as robfig does: a leading space hides the `@` (the text becomes one invalid field), and a trailing space spoils a descriptor. Fields split on ASCII whitespace, so they need no trim |
| 3. Descriptors (exact, lowercase) | `@yearly`/`@annually` = `0 0 1 1 *`; `@monthly` = `0 0 1 * *`; `@weekly` = `0 0 * * 0`; `@daily`/`@midnight` = `0 0 * * *`; `@hourly` = `0 * * * *`; `@every <d>` (below); any other `@…` → `Unsupported` |
| 4. Fields | split on ASCII whitespace; exactly 5: minute 0–59, hour 0–23, day of month 1–31, month 1–12, day of week 0–6 |
| List, range, step | `,`-separated parts; empty parts are skipped (robfig `FieldsFunc`), and a field with no value at all (`,`) is an error; `*` or `?` (whole range), `N`, `N-M` (`N ≤ M`); optional `/step`, `step ≥ 1`; `N/step` means `N-max/step`. Deviation: robfig `getRange` reads any part whose first hyphen piece is `*` or `?` (`*-5`) as `*`; here only an exact `*` or `?` is accepted |
| Numbers | ASCII digits only (`bytes().all(is_ascii_digit)` before `parse`), so `+5` and `٣` fail |
| Names | months `jan`–`dec`, days `sun`–`sat`, ASCII case-insensitive; allowed as a value or a range end, with or without a step |
| Star flag | a part that is `*` or `?` with no step > 1 marks the field star (`*/2` and `?/2` do not) |
| Errors | more than one `/` or `-`, non-digits, out of range, start > end, step 0, a field with no value → `Value` with the field name |

`@every <d>`: a Go `time.ParseDuration` value: optional sign, one or more `<decimal><unit>` groups (`1h30m`, `1.5h`, `90s`, `250ms`) with units `ns`, `us`, `µs`, `ms`, `s`, `m`, `h`, or a bare `0`; anything else → `Value { field: "every" }`. Nanoseconds accumulate, then the delay is cut to whole seconds; under 1 s (zero and negative included) becomes 1 s (robfig `Every`).

## Day rule

If the day-of-month **or** day-of-week field is star: `dom ∧ dow`; else `dom ∨ dow`. So `0 0 1 * 1` runs on the 1st and on Mondays; `0 0 * * 1` only on Mondays.

## Next run: a direct port of robfig `SpecSchedule.Next` (calendar)

`t` is a `jiff::Zoned` in the schedule's zone; civil construction follows Go `time.Date`, which reads the wall time as UTC to pick the zone period: `zone.to_ambiguous_zoned(dt)` with `compatible()` (a gap moves forward, a repeated time takes the first), except in a zone east of UTC a fold (`Fold { before }` with a positive `before`) takes `later()`, and in a zone west of UTC a gap (`Gap { before }` with a negative `before`) uses `earlier()`, so Santiago's skipped midnight becomes 23:00 of the day before. `added = false`; `year_limit = t.year() + 5`.

1. Start: `t = after + 1 s`, truncated to the second.
2. **WRAP**: if `t.year() > year_limit` → `None`.
3. Month: while the month is not in `months`: on the first change (`!added`) set `t` to the 1st of this month 00:00 (civil); then add 1 civil month; if the month became January → WRAP.
4. Day: while the day fails the day rule: on the first change set `t` to this day 00:00 (civil); add 1 civil day; **hour fix-up**: if `t.hour() != 0`, add `24 − hour` hours when hour > 12, else subtract `hour` hours (absolute); if the day became 1 → WRAP.
5. Hour: while the hour is not in `hours`: on the first change set `t` to this hour :00 (civil); add **1 h absolute**; if the hour became 0 → WRAP.
6. Minute: while the minute is not in `minutes`: on the first change truncate to the minute; add **1 min absolute**; if the minute became 0 → WRAP.
7. Second: while the second is not 0: on the first change truncate to the second; add 1 s absolute; if it became 0 → WRAP.
8. Return `t`.

Consequences, as in the controller: a spring-forward skips the missing hour (`30 2 * * *` does not run that day); a fall-back repeats the hour (`30 1 * * *` runs twice).

`@every`: `next_after(after)` = `anchor + k·delay` for the smallest `k ≥ 1` with a result `> after` (one division, no loop), in the zone; no anchor → `None`.

## Time zones

- `Some(name)` → `jiff::tz::db().get(name)`, else `UnknownZone(name)`; `None` → `TimeZone::UTC`, `zone_name = None`.
- Root `Cargo.toml`: `jiff` features `["std", "tzdb-bundle-always"]`. Trade-off: about 100 KiB in the binary, and the bundled rules can lag an OS update until the next k8sBoard release.

## Display (app, `batch_rows.rs`, `live_sections.rs`)

| Place | Rule |
|---|---|
| Next run cell | `KindCell::NextRun(schedule)`; paint `in {format_age(Some(now), next)}` (`in 11m`, `in 15h`, `in 3d`); no next → "—"; suspended or `Err` → `Absent` |
| Next run sort | `CellValue::Number(next.timestamp().as_second())`: ascending = soonest; no next → `Absent` |
| Next runs section | 3 rows of `next_runs(now, 3)`: label `%H:%M %Z` in the system zone (`17:45 +07`; the schedule still runs in its own zone) on today's date in the zone, else `%b %-d %H:%M %Z` (`Oct 6 02:30 UTC`; jiff supports the `-` flag); value `in 11m` |
| Notes | suspended: "Suspended: no runs are scheduled"; `Err(e)`: "Cannot compute next runs: {e}"; no run and `CronSchedule::is_every()`: "Next run is known after the first run"; no run otherwise (past the 5-year limit): "No run within the next 5 years" |
| Time zone field | the name, or "Cluster default (UTC assumed)" |
