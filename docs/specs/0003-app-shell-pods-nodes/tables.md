# 0003 · Pods and Nodes tables

[Back to index](README.md) · Modules: `pod_table.rs`, `node_table.rs`, `status_tone.rs`, `age.rs`

## Common

- `DataTable::new(&table_state)` over `TableState<…Delegate>`. Rows are virtualized by the kit.
- Settings: `row_selectable(true)`, `col_resizable(true)`, `sortable(false)`, and `cell_selectable(false)`, `col_movable(false)`.
- The delegate holds `session: Option<Entity<ClusterSession>>`:
  - `rows_count` = the length of the `Ready` items (0 otherwise);
  - `loading()` = the list is `Loading`.
- Text uses the UI font. Name, ready, numbers, IP, and version cells use the mono font (`theme.mono_font_family`, Lilex, built into the app: `mono_font.rs`, spec 0052) with right alignment for numbers (`Column::text_right`).
- Muted text: `theme.muted_foreground`. "—" marks an absent value.

## Pods columns (wireframe kind definition, W4b)

| Column | Width | Cell |
|---|---|---|
| Name | 300, min 160 | `{namespace}/` muted + `{name}` (in one line, ellipsis) |
| Status | 170 | label + tone color (below) |
| Ready | 70 | `ReadyCount` Display (`3/4`, counts sidecars) |
| Restarts | 80, right | `restarts` |
| Node | 180 | `node_name` or "—" |
| Age | 70, right | `format_age(created_at, now)` |

## Nodes columns (W5 without CPU/Memory)

Widths are the base widths of `NODE_COLUMNS`; the sum fits a 1100 px window (Memory and Age stay inside it). Taints gets the most spare width (weight 4, up to 420 px) and gives way first; Status, Name, and Roles grow a little (Internal IP, Version, CPU, Memory, Age do not). The table above lists the original W5 widths.

| Column | Width | Cell |
|---|---|---|
| Name | 200 | `name` |
| Status | 200 | label + tone (below) |
| Roles | 140 | `roles.join(", ")`, or "—" when empty (no `worker` inference) |
| Taints | 300 | the first taint Display, plus a muted ` +N` when more; "—" when none |
| Version | 100 | `kubelet_version` |
| Internal IP | 140, fixed | `internal_ip` or "—"; never truncates (fits `255.255.255.255`) |
| Age | 70, right | `format_age` |

## Status tones (`status_tone.rs`, pure)

```rust
pub(crate) enum StatusTone { Ok, Warn, Bad, Info, Done }
pub(crate) struct StatusLabel { pub(crate) text: SharedString, pub(crate) tone: StatusTone }
pub(crate) fn pod_status_label(pod: &PodSummary) -> StatusLabel;
pub(crate) fn node_status_label(status: NodeStatus, conditions: &[NodeCondition]) -> StatusLabel;
pub(crate) fn container_state_label(container: &ContainerSummary) -> StatusLabel;
pub(crate) fn tone_color(tone: StatusTone, cx: &App) -> Hsla; // the only theme lookup
```

| Tone | Theme token | Pod status | Node / container |
|---|---|---|---|
| Ok | `success` | `Running` | `Ready`; container `Running` and ready |
| Warn | `warning` | **Readiness failed**, `Pending`, `SchedulingGated`, `NotReady`, `Unknown`, `Other(_)` | `Cordoned`, `Unknown`; container `Running` not ready |
| Bad | `danger` | `CrashLoopBackOff`, `ImagePullBackOff`, `ErrImagePull`, `CreateContainerConfigError`, `OOMKilled`, `Error`, `ContainerCannotRun`, `Failed`, `Evicted`, `Signal:n`, `ExitCode:n` | `NotReady` (+ ` · Cordoned`); container waiting with a Bad reason, or terminated with a non-zero exit |
| Info | `info` | `ContainerCreating`, `PodInitializing`, `Init:n/m`, `Terminating` | container waiting with no or other reason |
| Done | `muted_foreground` | `Completed`, `Succeeded` | container terminated with exit 0; not reported |

- `Init:<reason>` takes the tone of `<reason>`. Otherwise the text is the cluster `PodStatus` Display.
- **Readiness failed** (UI rule, decision 2 of 0001) applies when:
  - `status == Reason(Running)`, and
  - `ready.ready < ready.total`, and
  - every `Main` and `Sidecar` container is `ContainerState::Running`.

  It shows immediately, including during a probe's initial delay. A grace period is a possible future refinement ([README](README.md)).
- Node text: `Ready` / `NotReady` / `Unknown`, when `scheduling == Disabled` a Ready node reads `Cordoned` (Warn) and another readiness gets ` · Cordoned` after it; the node drawer adds a Scheduling row (`Schedulable` / `Cordoned`). Active pressure conditions (`MemoryPressure`, `DiskPressure`, `PIDPressure`, in that order, `True` only) are appended after it, `Ready · DiskPressure`, and turn a green label to Warn (`issue_rules` reads the same list); the Status cell truncates with the full label as its tooltip.

## Age (`age.rs`, pure)

```rust
pub(crate) fn format_age(created_at: Option<jiff::Timestamp>, now: jiff::Timestamp) -> String;
```

| Elapsed | Output |
|---|---|
| None | "—" |
| future or < 60 s | `Ns` (negative clamps to `0s`) |
| < 60 min | `Nm` |
| < 24 h | `Nh` |
| ≥ 24 h | `Nd` |

`now` is read once per render. There is no ticking timer: renders happen on every snapshot. Add a timer only if users notice stale ages.

## Selection (by key, not index)

- `AppShell` keeps `selected: Option<ResourceKey>` (`Pod { namespace, name }` / `Node { name }`).
- `TableEvent::SelectRow(ix)`: take the key of row `ix`. Treat it as a **subject change** only if it differs from `selected`. Then store it and open or retarget the drawer ([drawer.md](drawer.md)). An equal key is a no-op, which covers the `SelectRow` re-emitted by programmatic re-selection.
- `TableEvent::ClearSelection` (the kit binds Esc to `Cancel`): set `selected = None`, which closes the drawer.
- After each session notify, re-sync with the pure `fn row_index<T>(items: &[T], key: …) -> Option<usize>`:
  - `Some(ix)` **and** `ix != table.selected_row()`: call `set_selected_row(ix)`. It scrolls and re-emits `SelectRow`, so never call it when the index is unchanged.
  - `Some(ix)` equal to the current row: do nothing.
  - `None` (the subject was deleted): set `selected = None` and call `clear_selection`, which closes the drawer. A drawer for a deleted object is not kept.
- The three cases come from a pure `fn selection_sync(table_row: Option<usize>, found: Option<usize>) -> SelectionSync { Keep, Move(usize), Clear }`, which is unit-tested.
- `TableEvent::RightClickedRow(Some(ix))`: the delegate's `context_menu(ix, …)` builds the menu ([actions.md](actions.md)). Right-click does not change the drawer subject.
