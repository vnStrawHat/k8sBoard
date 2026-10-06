# 0021 · Screen, layout, interactions

[Back to index](README.md) · Steps 1–3 · Modules: `overview.rs` (new; the view plus small pure helpers, tests in module), `app_shell.rs`, `workspace.rs`, `navigation.rs`, `launch_options.rs`, `main.rs`, `screenshot.rs`

## Screen wiring

- **Step 1.** Add the variant `Screen::Overview`. Its `kind()` is `None`, so `show_screen` drops the explorer as it does for Pods. Every `match self.screen` arm that serves a table (rebuild, check, toolkit, quick filter, `is_namespaced`, interruption, failure) treats Overview as "no table": no filter bar, no selection bar, no drawer, and `/` does nothing.
- **Step 1.** `navigation.rs`: `screen_of("Overview")` → `Screen::Overview`; the item has no count suffix. Topology stays disabled; Issues follows 0020.
- **Step 1.** `launch_options.rs`: `--screen overview` → `LaunchScreen::Overview` (`screen()` = Overview, no drawer).
- **Step 2.** `show_screen` also calls `session.set_overview_visible(screen == Screen::Overview)` ([recent-changes.md](recent-changes.md)).
- **Step 3.** `overview` becomes the default screen and `USAGE` says `default: overview`. Add `--window-width <px>` (800–3840, else a usage error) → `LaunchOptions.window_width: Option<u16>` (a `u16` keeps `LaunchOptions` under clippy's large-variant limit and is `Eq`); `main.rs` uses it instead of `WINDOW_WIDTH`.
- **`screenshot.rs` settle rule.**
  - Step 1: pods, nodes, and namespaces are not Loading; node metrics settled (`shows_node_usage`, 1 tick); kubelet settled (`shows_kubelet_stats`).
  - Step 2 adds: the change feed is not Loading.
  - Step 3 adds: `issues().summary()` is `Some`.

## Header (`workspace.rs` → `overview.rs` helpers)

| Part | Content |
|---|---|
| Title | `Overview` |
| Count text | `headline_text(...)`: `readonly@Monitor · Kubernetes v1.29.5 · ap-southeast-1` ([capacity-and-nodes.md](capacity-and-nodes.md)) |
| Right | step 2: `Last 15 min ▾` (ghost small button + `dropdown_menu` with items `Last 15 min` and `Last 1 h`, a check on the current one, tooltip `Time window of Recent changes`); step 4: `Export report` |
| Stats line (user-requested, not in W3) | below the header, muted `text_xs`: `41 / 42 nodes ready · 1,284 / 1,310 pods running · 37 namespaces`. The not-ready part is toned Bad when ready < total. When the scope is not All the line reads `cluster: 4 / 4 nodes ready · 20 namespaces — {scope}: 13 / 13 pods running`. A part whose list is not Ready shows `—` |

## Body

```
v_flex().id("overview").overflow_y_scroll().p_4().gap_3()
  row 1: h_flex().flex_wrap().gap_3() [ Needs attention (basis 560, grow) | Capacity (basis 360, grow) ]
  row 2: h_flex().flex_wrap().gap_3() [ Nodes (basis 560, grow)           | Recent changes (basis 360, grow) ]
```

- **Panel helper.** A local `fn panel(id, header: impl IntoElement, body: impl IntoElement, cx) -> Div`: `v_flex`, `border_1`, `border_color(theme.border)`, `rounded(theme.radius)`, `bg(theme.background)`. Its header is `h_flex().px_3().py_2().gap_2().border_b_1()` with the title (`text_sm`, semibold) and a right group (`ml_auto`). Rows use `items_start`, so panel heights do not stretch.
- **Panels per step.** Step 1 renders Capacity (row 1) and Nodes (row 2). Step 2 adds Recent changes. Step 3 adds Needs attention. No placeholders.

## Panel headers

| Panel | Left | Right |
|---|---|---|
| Needs attention | title, count pill (total, `tone_color(worst)`) | muted `auto-detected · live`, or Warn `Partial coverage` with the 0020 `coverage.note()` as tooltip (same helper as the Issues header) |
| Capacity | title | legend: three 9 px swatches `used`, `requested`, `allocatable` (muted mono `text_xs`); `requested` is hidden when the scope is not All |
| Nodes | title, muted `{n} · colored by CPU` | `{k} NotReady`, toned Bad, when k > 0 |
| Recent changes | title | muted link `View all →` (decision 27: `set_event_filter(EventFilter::All)`, then `show_screen(Screen::Kind(ResourceKind::Events))`) |

Recent changes ends with a muted `text_xs` footnote: `Deployment rollouts, HPA rescales, nodes, namespaces · events kept ~1 h by the API server`.

## State views

| Condition | Where | Shows |
|---|---|---|
| No live session | whole body | the existing session state view (connecting, failed, retry) |
| Board `summary()` is `None` | Needs attention | spinner `Checking the cluster…` |
| Zero issues | Needs attention | `No issues found.`; with partial coverage: `No issues found in what k8sBoard watches.` (0020 wording) |
| Nodes list Loading / Failed | Capacity, Nodes | spinner `Loading nodes…` / `Nodes unavailable · {message}` |
| Node metrics feed not Live/Interrupted | Capacity, Nodes | Capacity used = `—` with `{FeedStatus::reason}` as tooltip; heatmap cells plain, and the header adds `· metrics unavailable` |
| Change feed Loading / Failed / denied | Recent changes | `Loading changes…` / `Changes unavailable · {message}` / `Not permitted: list events` |
| No change in the window | Recent changes | `No tracked changes seen in the last 15 min.` (window label), followed by the footnote, which names what is tracked |

## Interactions

| Element | Click | Tooltip |
|---|---|---|
| Needs attention row | `reveal(issue.target)`; nothing without a target | `{Kind} {ns}/{name}` (+ ` · No screen for {Kind}`) |
| Row action button | see [attention.md](attention.md) | the disabled reason |
| `View all {n} issues →` | `show_screen(Screen::Issues)` | — |
| Heatmap cell | `reveal(ResourceKey::Node { name })` | `HeatCell::tooltip` |
| Change row | `reveal(entry.target)` when `Some` | the full message |
| Capacity row | — | name of the row's ceiling (decision 13) |

Click handlers use `cx.listener` on `AppShell`, as in `navigation.rs`. Element ids are stable: `issue-{index}`, `node-{name}`, `change-{index}`.
