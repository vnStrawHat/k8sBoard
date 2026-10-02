# 0017 · App: `HelmReleaseView` (Overview diff, Values, Manifest, Notes)

[Back to index](README.md) · Step 3 · Modules: `helm_release_view.rs` (new) + `helm_release_view_tests.rs`, `drawer.rs`, `app_shell.rs`, `kind_drawer.rs`, `live_sections.rs`, `resource_actions.rs`, `launch_options.rs`, `screenshot.rs`. Pattern: 0007 `YamlView` and 0016 `SecretValuesView` (one entity per shown subject, created only in the render sync, dropped to cancel and wipe).

## Types (`helm_release_view.rs`)

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub(crate) enum HelmTab { Overview, Values, Manifest, Notes }
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub(crate) enum ValuesSource { User, Computed }
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub(crate) enum ValuesLayout { Document, Diff }
/// Helm content of one revision. Dropping it aborts requests and wipes every text.
pub(crate) struct HelmReleaseView {
    connection: ClusterConnection, revision: HelmRevisionRef, latest_revision: u32, access: ValueAccess /* 0016 */,
    tab: HelmTab, source: ValuesSource /* User */, layout: ValuesLayout /* Document */, env: EnvValues /* Hidden */,
    earlier_revision: Option<u32>,            // highest History revision below `revision`
    editor: Entity<EditorState>,              // yaml, read-only, like YamlView
    shown: Option<ShownText>,                 // which text the editor holds; set_value only on change
    detail: Fetched<HelmReleaseDetail>,       // masked; no notes text
    diff: Fetched<HelmValuesDiff>,            // masked, against `earlier_revision`
    revealed: Option<Revealed>,
    copy_error: Option<SharedString>,         // inline, cleared on the next copy or hide
    _ticker: Option<Task<()>>,                // 1 s, runs while `revealed` is Some
}
struct Revealed { hides_at: Option<Instant> /* first revealed arrival + 30 s */, payload: Fetched<HelmRevealed>, diff: Fetched<HelmValuesDiff> }
enum Fetched<T> { Absent, Running { _task: Task<()> }, Ready(T), Failed { message: SharedString } }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ShownText { Values(ValuesSource, ValueVisibility), Manifest(EnvValues) }
pub(crate) struct ShowLatest; // event: the header's Latest button; AppShell sets `helm_revision = None`
```

## Pure core (unit-tested, no GPUI)

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)] enum Need { Detail, Revealed, Diff(ValueVisibility) }
/// The fetch the shown state is missing. Only `Absent` slots are fetched; `Failed` waits for Refresh.
fn next_need(tab: HelmTab, layout: ValuesLayout, slots: SlotStates) -> Option<Need>;
fn earlier_revision(history: &[u32], revision: u32) -> Option<u32>;  // max r < revision
fn shown_text(tab: HelmTab, layout: ValuesLayout, source: ValuesSource, env: EnvValues, is_revealed: bool) -> Option<ShownText>; // editor only
fn is_expired(hides_at: Option<Instant>, now: Instant) -> bool;
/// Revealed copy: empty selection → None; else the writer's result. Never another clipboard path.
fn private_copy<E>(selection: Zeroizing<String>, write: impl FnOnce(&str) -> Result<ClipboardMark, E>) -> Option<Result<ClipboardMark, E>>;
```

| Tab / layout | Not revealed | Revealed |
|---|---|---|
| Overview (user values, latest vs earlier; needs `earlier_revision`) | `Diff(Masked)` | `Diff(Revealed)` |
| Values · Document | `Detail` | `Revealed` |
| Values · Diff (needs `earlier_revision`) | `Diff(Masked)` | `Diff(Revealed)` |
| Manifest | `Detail` | `Detail` (never revealed) |
| Notes (M1) | `Detail` (for `notes_lines`) | `Revealed` (`HelmRevealed.notes`) |

## Async contract

| Step | Rule |
|---|---|
| `new(..)` | the first fetch waits `DRAWER_SUBJECT_DELAY` (arrowing through rows sends no GET); never notifies synchronously |
| fetch | `cx.spawn_in(window, ..)` → `runtime.spawn(connection.helm_release_detail / helm_revealed / helm_values_diff)` → `update_in`; the slot holds the task; dropping it aborts |
| success | slot `Ready`; editor text updated when `shown_text` changed; a revealed result sets `hides_at` if unset; notify |
| failure | slot `Failed { error_text(e) }` (403 "Not permitted: get secrets"; 404 when the revision was pruned) |
| `set_tab` | called by the sync every render; a **different tab drops `revealed`** (hide at once) and editor text returns to masked; starts the next need; never notifies synchronously |
| `set_history` | every render; updates `earlier_revision`; a change resets `diff` and starts the need |
| Reveal | no-op when `Blocked`; `revealed = Some(Absent slots)`, start the need and the ticker |
| ticker | every 1 s: `is_expired` → `revealed = None`, editor back to masked, notify; ends with `revealed` |
| Hide | `revealed = None`, `copy_error = None`, notify |
| Env values | flip `env`, `detail = Absent`, fetch at once (no timer, 0007 rule) |
| Refresh | the current need's slot (and `revealed` slots) → `Absent`, fetch at once |

Invariants (0007): `sync_helm_view` only assigns and is the only creator; `new`, `set_tab`, `set_history` never notify synchronously.

## Private copy (M2, decision 26)

- While `revealed` is `Some` and the editor shows revealed text, the editor's wrapper element adds `.capture_action::<Copy>(..)` and `.capture_action::<Cut>(..)` for the kit input actions (`gpui_base::input::{Copy, Cut}` through the gpui-kit re-export; the coder confirms the path).
- Handler: `cx.stop_propagation()`; `Zeroizing::new(editor.read(cx).selected_text().to_string())`; `private_copy(selection, |text| write_private_text(text, cx))` (0016); `Some(Ok(mark))` → `cx.emit(SecretCopied(mark))` (AppShell arms the 30 s clear); `Some(Err(_))` → `copy_error = "Copy failed: the clipboard is unavailable."`, nothing written (fail closed).
- Revealed Diff rows and Notes text are plain `div`s with no selectable-text element, so they offer no other copy path. Masked editor text copies through the editor's normal Copy (no value inside).

## AppShell wiring (`drawer.rs`, `app_shell.rs`, `kind_drawer.rs`)

```rust
pub(crate) enum DrawerTab { /* … */ Values, Manifest, Notes }   // labels "Values", "Manifest", "Notes"
pub(crate) struct DrawerState { /* … */ pub(crate) helm: Option<Entity<HelmReleaseView>>,
    pub(crate) helm_revision: Option<u32>,                        // None = latest; reset on subject change
    pub(crate) pending_helm_layout: Option<(ResourceKey, ValuesLayout)> }
/// Pure: the revision and tab to show for a release drawer. Overview always uses the latest revision.
pub(crate) fn helm_subject(selected: Option<&ResourceKey>, tab: DrawerTab, summary: Option<&HelmReleaseSummary>, revision: Option<u32>) -> Option<(HelmRevisionRef, HelmTab)>;
impl AppShell {
    fn sync_helm_view(&mut self, window: &mut Window, cx: &mut Context<Self>); // in render, beside sync_yaml_view
    pub(crate) fn open_helm_values(&mut self, key: ResourceKey, revision: u32, layout: ValuesLayout, cx: &mut Context<Self>);
}
```

- Sync: no subject → `helm = None` (drop = wipe). Same `HelmRevisionRef` → keep, `set_tab`, `set_history` (from the related `HelmHistory` list). Otherwise create with `self.secret_value_access` (0016 one source) and subscribe to `ShowLatest` (→ `helm_revision = None`, notify) and `SecretCopied` (→ 0016 `arm_clipboard_clear`). Then take a `pending_helm_layout` for the selected key and call `set_layout`.
- A new latest revision changes the subject: the view is recreated (fresh data).
- `open_helm_values`: select `key` if needed (`open_drawer_tab` rule), `helm_revision = Some(revision)` (None when it equals the latest), `tab = Values`, store the pending layout, notify.
- `kind_drawer.rs` places the view for `Live(HelmValuesChange)` when `drawer.helm` is for the Overview subject; one frame before the sync it renders `Note("Loading…")`.
- History rows (`live_sections.rs`): ghost small buttons **Values** → `open_helm_values(key, n, Document)`; **Diff** → `open_helm_values(key, n, Diff)`, absent on the oldest listed revision. The shown revision's row is marked `shown` (muted).

## Render (`Render for HelmReleaseView`)

| Part | Content |
|---|---|
| Overview part | title row `Values changed in rev {r}` · right `Reveal (30s)` or `Hide` + `Hides in {n}s`; the diff list (below); no editor |
| header (other tabs) | `Revision {r}` · `{chart}-{version}` muted · `Latest` ghost button when `r != latest_revision` · Refresh (right) |
| Values toolbar | `User-supplied` / `Computed` (kit `ButtonGroup`), `Diff` toggle (disabled "No earlier revision"), right: `Reveal values (30s)` or `Hide` + muted `Hides in {n}s` |
| Values · Document | the editor; masked text is a structure view whose header says values are hidden (decision 11) |
| diff list | muted `Changes from rev {p} to rev {r}`; per change: path (Mono) and `+ after` (Ok), `- before` (Bad), or `before → after` (Warn) via `tone_color`; none → `No value changed.`; omitted → `{n} more changes not shown.` |
| Manifest | toolbar **Env values** (0007 `shows_env_toggle`), **Copy** (masked text, like YamlView); the editor |
| Notes | masked: `Notes may contain rendered passwords.` + muted `{n} lines` + `Reveal (30s)`; `notes_lines == 0` → `This release has no notes.`; revealed: scrolling Mono `text_xs`, wrapped, `Hide` + `Hides in {n}s` |
| Blocked | every Reveal disabled, tooltip "Disabled in screenshot runs" |
| copy error, loading, failed | `copy_error` muted Bad line; `Spinner` + `Reading the release…` while nothing is shown; `Alert::error(..).title("Cannot read the release")` |

No hardcoded colors; no `tracing::`.

## Screenshots (`launch_options.rs`, `screenshot.rs`)

Slugs `<plural>-values`, `<plural>-manifest` → `KindDrawer(kind, Values | Manifest)`, accepted only when `drawer_tabs` of the kind has the tab. `is_content_pending` also waits for `HelmReleaseView::is_loading()` (a needed slot `Running` and nothing shown), including the Overview diff.
