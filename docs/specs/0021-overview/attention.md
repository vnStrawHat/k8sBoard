# 0021 · Needs attention (step 3)

[Back to index](README.md) · Modules: `overview.rs`, `resource_actions.rs` · Needs 0020 step 1b merged. Implements the 0020 "Overview contract" ([../0020-issues/issues-screen.md](../0020-issues/issues-screen.md)).

## Reads (no new state)

| Read | Use |
|---|---|
| `session.issues().issues()` | board order (severity, oldest, object); first `ATTENTION_ROWS` |
| `session.issues().summary()` | header pill: `total`, tone Bad when `critical > 0` else Warn; `None` → spinner |
| `session.issues().coverage()` | header right: `Partial coverage` + `note()` tooltip (same helper as the Issues header) |
| `live.pods` | resolve the pod for View logs |
| `live.access` | gate View logs (`action_availability(ViewLogs)`) |

```rust
const ATTENTION_ROWS: usize = 6;
```

Overview never calls `set_issues_visible`; it shows no ages, so the board's on-change notify is enough.

## Row anatomy (W3 `.issue`: 118 px | 1fr | auto)

```
h_flex().gap_2().px_3().py_2().border_b_1() (last row no border)
  pill   w(px(118)) flex_none: kit Tag/badge, text = issue.reason, color = tone_color(severity.tone())
  what   v_flex().flex_1().min_w_0():
           mono text_xs: object_line(issue)                     — truncate
           muted text_xs: message_line(&issue.cause)             — truncate, full text in tooltip
  action flex_none: ghost small Button (below), or nothing
```

```rust
/// `payments / api-7d9f8c-x2k4q · container api` (+ ` · {count} pods` when count > 1);
/// a cluster object has no `ns / ` part.
fn object_line(issue: &Issue) -> String;
```

If 0020 already renders a pill helper for the Issues table, reuse it; else a local `fn severity_pill`.

## Action button

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
enum AttentionAction { ViewLogs, Open { label: String } }
/// ViewLogs → ViewLogs; target Pod → Open "See why"; other target → Open "Open {Kind}" (API kind casing: "Open Secret"); no target → None.
fn attention_action(issue: &Issue) -> Option<AttentionAction>;
```

| Action | Click | Disabled when |
|---|---|---|
| ViewLogs | open the log dock on the issue's pod and container | access denies logs; the pod is not in `live.pods` (`The pod is gone`); no containers |
| Open | `shell.reveal(target)` | never |

- **One log path.** The pod is resolved and gated by the same function the 0020 Issues menu uses. If that function only builds a `PopupMenuItem`, split it in `resource_actions.rs`:

```rust
/// Availability and target of View logs for one pod; the menu item and the Overview button both call it.
pub(crate) fn logs_launch(pod: &PodSummary, live: &LiveCluster) -> Result<(ClusterConnection, LogTarget), SharedString>;
```

  `view_logs_item` then wraps it; the button calls `dock.update(cx, |dock, cx| dock.open(connection, target, window, cx))`. The container hint (`IssueAction::ViewLogs { container }`) is passed the same way the 0020 menu passes it.
- The button's click stops propagation so the row's reveal does not also fire.

## Footer

When `summary.total > ATTENTION_ROWS`: a last line `View all {total} issues →` (muted link, `text_xs`, `px_3 py_2`) → `show_screen(Screen::Issues)`.
