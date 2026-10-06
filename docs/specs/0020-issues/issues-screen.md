# 0020 · Issues screen, sidebar counts, title-bar button

[Back to index](README.md) · Step 1a (sidebar, title-bar button) and step 1b (screen) · Modules: `issue_table.rs` (new, + tests in module), `app_shell.rs`, `workspace.rs`, `navigation.rs`, `title_bar.rs`, `launch_options.rs`, `screenshot.rs`

## Sidebar (`navigation.rs`, step 1a)

- `NavigationCounts` gains `issue_total: Option<(usize, IssueSeverity)>` and `issue_counts: Vec<(Screen, usize, IssueSeverity)>` from `board.summary()` / `count_for`.
- Pods, Nodes, and kind items: when `count_for(screen)` > 0, the suffix is `h_flex().gap_1p5()` of the issue count as a pill (tinted `tone_color` background, `px_1`) in `tone_color(worst.tone())`, then the muted total (existing). Tooltip `{k} issues`. Denied items keep the lock.
- The Issues top item shows the toned total in step 1a while still disabled (tooltip `Issues screen comes in the next step`); step 1b enables it (`screen_of("Issues")` → `Screen::Issues`). Overview and Topology stay disabled.

## Title bar (`title_bar.rs`, step 1a)

`issues_button(shell, cx)` between the Read-only badge and Settings: `Button::new("issues").ghost().small().icon(IconName::Flag)` (decision 29), label = total, text color `tone_color(worst)`. 0 → muted icon, no label; summary `None` → muted icon, tooltip `Checking for issues…`. Tooltip `{n} issues ({c} critical)` + ` · partial coverage` when partial. Without a live session: disabled, tooltip `Not connected`. Click: step 1a none (tooltip as above plus ` · Issues screen comes next`); step 1b → `show_screen(Screen::Issues)`.

## Screen (step 1b)

- `Screen::Issues` (new variant); `kind()` → `None`, so `show_screen` drops the explorer like Pods and Nodes, and calls `session.set_issues_visible(screen == Issues)`. `default_filter(Issues)` = no filter.
- Header (0009 layout): title `Issues`, count `{n}` / `{shown} of {n} match`, right side muted `auto-detected · live` (W3 header). When `coverage.note()` is `Some`: `Partial coverage` in Warn with the note as tooltip; a Limited-only note shows muted, untoned.
- Filter bar: quick filter `/` and the Namespace chip (0009). No severity chips, no row checkboxes (decision 27).
- State views (existing helpers): no live session → the session state view; `summary()` `None` → spinner `Checking the cluster…`; zero issues, full coverage → `No issues found.`; zero issues, partial → `No issues found in what k8sBoard watches.` plus the note.

## Table (`issue_table.rs`, step 1b)

```rust
pub(crate) struct IssueTable { view: TableView, layout: ColumnPlan, /* delegate state like PodTable */ }
impl TableDelegate for IssueTable { /* rows read `session.issues().issues()` by index at paint */ }
impl FilteredTable for IssueTable { .. }
impl TableRow for Issue { .. }  // namespace = shown.namespace, name = shown.name, no labels, tone = severity
```

| # | Column | Width | Cell | `value` (sort/filter) |
|---|---|---|---|---|
| 0 | Severity | 90 | `Critical` / `Warning`, toned | `Status` (Critical sorts first) |
| 1 | Reason | 160 | pill text, toned | `Text` |
| 2 | Kind | 110 | `shown.kind` | `Text` |
| 3 | Object | 178, grows | mono `shown.name`, plus a muted ` · {c}` when the name keeps 16 characters beside it; otherwise the container joins the Cause tooltip | `Text` |
| 4 | Namespace | 92, grows to 150 | `—` for cluster objects | `Text` |
| 5 | Cause | 146, grows most | one line (`message_line`), full text (and `Container: {c}`) as tooltip | `Text` |
| 6 | Count | 64, right | `count` (`—` when 1) | `Number` |
| 7 | Age | 70 | `format_age(onset, now)`; `—` when no rule knows the onset (never the first-seen time) | `Age` |

Base widths sum to what a 1100 px window leaves, so Count and Age stay inside it; Kind (up to 130) and Namespace (up to 150) shrink to their content.

Default order = the board order (severity, oldest, object); a column sort replaces it (0009). The quick filter matches Reason, Kind, Object, Namespace, and Cause text, which covers "by kind" and "by reason".

## Interaction (step 1b)

- Click or Enter on a row → `AppShell::reveal(target)`; a row without a target does nothing (tooltip `No screen for {kind}`).
- Context menu (and ⋯): `Open {kind lowercase}` (disabled without a target) · `View logs` (only when `action` is ViewLogs and the pod is in `live.pods`; `resource_actions::view_logs_item`, so access gating and container choice stay one path) · separator · `Copy object name`.
- The selected row is kept by `IssueKey`; a key that disappears clears the selection.

## Launch and screenshots (step 1b)

- `--screen issues` → `LaunchScreen::Issues`; `is_content_pending` waits until `summary()` is `Some` and no feed is `Loading`.
- Screenshots: step 1b `issues` (UAT) and `issues-empty` (a namespace with no problems via `--namespace`); step 2 `issues` again with condition feeds, and `issues-three-namespaces` (`--namespace a,b,c`: condition feeds as All-scope watches, rows filtered).

## Overview contract (0021)

0021 reads `session.issues().issues()` (sorted), shows the first rows with `reason` as the pill, `{namespace} / {shown.name}` + ` · container {c}`, `cause`, and the action button (`View logs` / `Open`); `summary()` gives the header pill.
