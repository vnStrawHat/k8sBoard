# 0003 · Pods and Nodes tables

[Back to index](README.md) · Modules: `pod_table.rs`, `node_table.rs`, `status_tone.rs`, `age.rs`

## Common

- `DataTable::new(&table_state)` over `TableState<…Delegate>`. Rows are virtualized by the kit.
- Settings: `row_selectable(true)`, `col_resizable(true)`, `sortable(false)`, and `cell_selectable(false)`, `col_movable(false)`.
- The delegate holds `session: Option<Entity<ClusterSession>>`:
  - `rows_count` = the length of the `Ready` items (0 otherwise);
  - `loading()` = the list is `Loading`.
- Text uses the UI font. Name, ready, numbers, IP, and version cells use the mono font (`theme.mono_font_family`, Lilex, built into the app: `bundled_fonts.rs`, spec 0052) with right alignment for numbers (`Column::text_right`).
- Muted text: `theme.muted_foreground`. "—" marks an absent value.

## Pods columns (wireframe kind definition, W4b)

| Column | Width | Cell |
|---|---|---|
| Name | 300, min 160 | `{namespace}/` muted + `{name}` (in one line, ellipsis) |
| Status | 170 | label + tone color (below) |
| Ready | 70 | `ReadyCount` Display (`3/4`, counts sidecars) |
| Restarts | 80, right | `restarts` |
| Node | 180 | `node_name` or "—" |
| Image | 200, hidden by default | the first main container's `name:tag` (registry and path cut) plus a muted ` +N`; tooltip lists every full reference; "—" when none |
| Age | 70, right | `format_age(created_at, now)` |

The quick filter searches every image reference of a pod (init containers too), and `image:nginx` keeps only the pods with a matching image. The workload kinds have the same opt-in Image column (0005 `kind-columns.md`).

## Nodes columns (W5 without CPU/Memory)

Widths are the base widths of `NODE_COLUMNS`; the sum fits a 1100 px window (Memory and Age stay inside it). Taints takes its share of the spare width first (weight 6, up to 150 px: `work…:NoSched` and `main…:NoSched` read apart at 1320 px, the cell cuts the end of the key, never the middle, so the effect stays whole), and Name takes the rest (weight 2, up to 300 px; 26 mono characters at 1320 px with the default columns, so `k8sboard-lab-control-plane` stays whole next to Taints). Below the base widths Taints gives way first. Name is cut with the sibling-aware rule of `cell_truncation.rs` (0009), as is the Pods Node column. Roles (84, cut early), Status (84, so `Cordoned` shows whole), Internal IP, Version (90), CPU and Memory (80, with a 28 px bar), and Age do not grow. The table above lists the original W5 widths.

| Column | Width | Cell |
|---|---|---|
| Name | 200 | `name` |
| Status | 200 | label + tone (below) |
| Roles | 140 | `roles.join(", ")`, or "—" when empty (no `worker` inference) |
| Taints | 300 | the first taint as `key:effect` (key domain and value dropped, effect cut to `NoSched`, `PreferNoSched`, `NoExec`; `workload:NoSched +1`), plus a muted ` +N` when more; tooltip lists every taint whole; "—" when none |
| Version | 100 | `kubelet_version` |
| Internal IP | 140, fixed | `internal_ip` or "—"; never truncates (fits `255.255.255.255`) |
| Age | 70, right | `format_age` |
| Labels | 200, hidden by default | the first two labels without a system key (`kubernetes.io/`, `node.kubernetes.io/`, `beta.kubernetes.io/`, `node-role.kubernetes.io/`) plus a muted ` +N`; tooltip lists every label; "—" when none |

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

- On a light theme every tone is pulled toward the foreground only as far as 4.5:1 on the background needs (cap: 40%), so amber Warn text (Severity, Blocks drain, Restarts) reads on the Default light panel too (walk J14); a dark theme keeps the fill.
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

## As built (2026-10-07): narrow windows shed columns

A column with a `shed_order` (`KindColumn::sheds(n)`) is dropped when the table is too narrow for the base widths of the columns shown, the lowest number first, so a 1024 px window needs no horizontal scroll. The Name column and columns without an order always stay. Pods: Age, then Memory, then CPU (Name base 280 px). Nodes: Age, then Version, then Roles (Name base 190 px). Every kind table sheds Age. A shed column still shows as ticked in the Columns menu; it returns when the window is wider. The sidebar stays 250 px: the kit's icon mode hides the group items and the count badges, so a rail would cut navigation.

## As built (2026-10-07): the last column ends at the right edge

When every growing column is capped by `up_to` and width is still left over (Jobs, Nodes, Namespaces at 1920 px), `distribute_spare_width` gives the rest to the flexible (Name) column beyond its cap. This is what StatefulSets and Events already did, through an uncapped growing column (Service, Message), so a right-aligned Age ends at the edge on every screen. The widths of a wide table always sum to the available width; the 1024 px shedding is unchanged.
