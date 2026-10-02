# 0016 · App: `SecretValuesView` (Reveal, Copy)

[Back to index](README.md) · Step 3 · Modules: `secret_values.rs` (new) + `secret_values_tests.rs`, `drawer.rs`, `app_shell.rs`, `kind_drawer.rs`, `resource_actions.rs`, `secret_clipboard.rs` ([secret-clipboard.md](secret-clipboard.md)). Pattern: 0007 `YamlView` (one entity per shown subject, created only in `render`, dropped to cancel).

## Types (`secret_values.rs`)

```rust
pub(crate) const REVEAL_DURATION: Duration = Duration::from_secs(30);
const COPIED_FEEDBACK: Duration = Duration::from_secs(2);
const TICK: Duration = Duration::from_secs(1);
pub(crate) const REVEAL_DISPLAY_LIMIT: usize = 4096; // bytes, cut back to a char boundary

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SecretAction { RevealAll, Reveal(String), Copy(String) }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ValueAccess { Enabled, Blocked }   // Blocked under `--screenshot` (decision 22)
/// The only place that decides; `AppShell.secret_value_access` stores it once at startup.
pub(crate) fn value_access(options: &LaunchOptions) -> ValueAccess; // Blocked when a screenshot output is set
pub(crate) struct SecretCopied(pub(crate) ClipboardMark);          // event; AppShell arms the 30 s clear

/// The Data section of the open Secret drawer. Dropping it drops (and wipes) every revealed value.
pub(crate) struct SecretValuesView {
    connection: ClusterConnection,
    secret: ResourceKey,                 // Secrets kind, (ns, name)
    keys: Vec<SecretKey>,                // from the summary; never values
    revealed: Vec<RevealedValue>,        // sorted by key; at most one per key
    request: ValuesRequest,
    copied: Option<(String, Instant)>,   // key name only
    access: ValueAccess,
    _ticker: Option<Task<()>>,           // runs while `revealed` or `copied` is non-empty
}
struct RevealedValue { value: SecretValue, hides_at: Instant }
enum ValuesRequest { Idle, Running { _task: Task<()> }, Failed { message: SharedString } }
```

## Pure core (unit-tested, no GPUI)

```rust
/// Values the action keeps; the rest are dropped (wiped) by the caller. `Err` names a missing key.
fn kept_values(values: Vec<SecretValue>, action: &SecretAction) -> Result<Vec<SecretValue>, MissingKey>;
/// Inserts or replaces by key with `hides_at = now + REVEAL_DURATION`.
fn reveal(revealed: &mut Vec<RevealedValue>, values: Vec<SecretValue>, now: Instant);
/// Drops values whose `hides_at <= now`; true when anything changed.
fn expire(revealed: &mut Vec<RevealedValue>, now: Instant) -> bool;
enum ValueDisplay { Text { shown: &str, is_cut: bool }, Binary { size_bytes: usize } }
fn value_display(value: &SecretValue) -> ValueDisplay;  // REVEAL_DISPLAY_LIMIT
fn seconds_left(hides_at: Instant, now: Instant) -> u64; // rounded up, for "Hides in 23s"
```

`MissingKey` text: "Key {key} no longer exists in this secret."

## Async contract

| Step | Rule |
|---|---|
| `run(action, cx)` | no-op when `Blocked` or a request is `Running`; never notifies synchronously (it may run inside `AppShell::render`) |
| fetch | `cx.spawn` → `runtime.spawn(connection.secret_values(ns, name))` on the cluster runtime → await the join handle; dropping `_task` aborts |
| `RevealAll` / `Reveal(k)` | `kept_values` → `reveal(.., now)` → start the ticker if idle → `request = Idle` → notify |
| `Copy(k)` | `kept_values` → `as_text()` → `write_private_text(text, cx)` → `Ok(mark)`: `copied = Some((k, now))`, `cx.emit(SecretCopied(mark))`; `Err` → `Failed` with its fixed text. Values dropped in the same update; nothing revealed; notify. Binary values never reach here (button disabled) |
| error | `Failed { message: error_text(e) }` (403 reads "Not permitted: get secrets"); `MissingKey` the same way |
| ticker | every `TICK`: `expire`; clear `copied` after `COPIED_FEEDBACK`; notify; ends when both are empty |
| `set_keys(keys)` | called by the sync each render: replaces `keys`; drops revealed values whose key is gone |
| `hide_all(cx)`, `hide(key, cx)` | the header and row "Hide" buttons: drop all or one revealed value, notify |

## AppShell wiring (`drawer.rs`, `app_shell.rs`)

```rust
pub(crate) struct DrawerState { /* … */ pub(crate) secret_values: Option<Entity<SecretValuesView>>,
    pub(crate) pending_secret_action: Option<(ResourceKey, SecretAction)> }
pub(crate) struct AppShell { /* … */ secret_value_access: ValueAccess /* value_access(&options), set once */,
    clipboard_clear: Option<(ClipboardMark, Task<()>)> /* secret-clipboard.md */ }
impl AppShell {
    fn sync_secret_values(&mut self, window: &mut Window, cx: &mut Context<Self>); // in render, beside sync_yaml_view
    pub(crate) fn run_secret_action(&mut self, key: ResourceKey, action: SecretAction, cx: &mut Context<Self>);
}
```

- **Subject** = selected key of kind `Secrets` and `drawer.tab == Overview`. No subject → `secret_values = None` (drop = wipe). Same subject → keep and call `set_keys` with the row's `KindObject::Secret` keys. New subject → create with `self.secret_value_access` and subscribe to `SecretCopied` (→ `arm_clipboard_clear`). The menus read the same field (decision 22: one source).
- After the sync, a `pending_secret_action` for the subject is taken and passed to `run`.
- `run_secret_action`: selects `key` (opens the drawer), sets `tab = Overview`, stores the pending action, notifies. Menu items call it; drawer buttons call the view directly.
- `show_screen`, scope change, and context switch already close the drawer, so the view drops.

## Render (`Render for SecretValuesView`; `kind_drawer.rs` places it for `Live(SecretData)`)

| Part | Content |
|---|---|
| header row | right: `Reveal all (30s)` ghost button; while anything is revealed: `Hide` instead; while `Running`: small spinner |
| masked row | key (Mono) · `MASK` muted · size muted · `Reveal` · `Copy` (disabled for binary, tooltip "Binary value") |
| revealed row | key · value Mono, wrapped (`Text`), or "Binary value, {size}"; cut → muted line "… {size} in total; Copy for the full value" · muted `Hides in {n}s` · `Hide` (this key) · `Copy` |
| copied | that row's Copy button reads `Copied` for 2 s, tooltip "Clears from the clipboard in 30 s" |
| failed | `Alert::error` with the message under the header; buttons stay enabled to retry |
| blocked | every Reveal/Copy disabled, tooltip "Disabled in screenshot runs" |

- Buttons are kit `Button` small ghost; no hardcoded colors.
- When `drawer.secret_values` is `None` or for another key (one frame before the sync), `Live(SecretData)` renders the step 2 masked rows without buttons.
- Screenshot settle needs nothing new: the view never fetches on its own.
