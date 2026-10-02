# 0030 · Write flow, confirm dialog, first consumer

[Back to index](README.md) · Step 4 (unlock dialog: step 2b) · Modules: `write_flow.rs` (new) + `write_flow_tests.rs`, `confirm_dialog.rs` (new in step 2b for unlock), `resource_actions.rs`, `app_shell.rs`, `launch_options.rs`, `screenshot.rs`. Decisions 9–12, 15–18, 23–29. Wireframe: W10 modal, W6 typed name.

## Verified APIs

| API | Where |
|---|---|
| `kube::Client::new(service, ns)`; `client::Body: From<Vec<u8>>` + `http_body::Body` | `kube-client-4.2.0/src/client/mod.rs:153`, `client/body.rs:80, 86` |
| `PatchParams { dry_run, force, field_manager }`; `DeleteParams.preconditions`, `Preconditions { resource_version, uid }` | `kube-core-4.2.0/src/params.rs:664, 763, 951` |
| `Status { code, message, reason, details.causes[].field }` | `kube-core-4.2.0/src/response.rs:26, 205` |
| `tower::service_fn` (feature `util`, enabled by kube) | `tower-0.5.3/src/util/service_fn.rs:46` |
| `Styled::border_dashed`; `IconName::LockOpen` (from `assets/icons/lock-open.svg`) | `gpui-pre-0.3.7/src/styled.rs:500`; `gpui-kit-assets-0.7.0/build.rs` |
| `Dialog::{on_ok, on_cancel, on_close}`, `WindowExt::{open_dialog, push_notification}` | `gpui-component-0.7.0/src/dialog/dialog.rs:405–435`, `window_ext.rs` |

## Sequence (the only caller of `ClusterConnection::write`)

```rust
pub(crate) struct WriteIntent { pub(crate) cluster: ClusterRef /* the row's cluster (guardrails.md) */, pub(crate) action: ResourceAction, pub(crate) label: SharedString,
    pub(crate) request: WriteRequest, pub(crate) risk: ActionRisk, pub(crate) expected_name: Option<String> }
/// One guarded action. `Connect` is for streaming verbs (0036 exec, 0035 port-forward), which have no dry-run.
pub(crate) struct GuardedIntent { pub(crate) cluster: ClusterRef, pub(crate) action: ResourceAction,
    pub(crate) label: SharedString, pub(crate) risk: ActionRisk, pub(crate) expected_name: Option<String>,
    pub(crate) warnings: Vec<SharedString> /* warn lines in the dialog */, pub(crate) kind: GuardedKind }
    // 0031 step 4 adds `on_commit: Option<CommitCallback>`: when set, the dialog closes after the commit (0031 editor-view.md).
/// The full list; each variant's spec owns its branch of `run_guarded`.
pub(crate) enum GuardedKind {
    Write(WriteRequest),                                                    // 0030
    Connect(ConnectIntent),                                                 // 0036 exec, 0035 port-forward
    Batch(BatchPlan),                                                       // 0032 bulk-write.md
    CreateThenAttach { request: WriteRequest,
        open: Box<dyn FnOnce(AttachPermit, WriteOutcome, &mut Window, &mut App)> }, // 0037 session-flow.md
    Helm { cli: HelmCli, request: HelmRequest },                            // 0038 release-actions.md
}
/// App-side mode: a commit carries the proof that the confirm step was satisfied.
pub(crate) enum CommitMode { DryRun, Commit { confirmed: Confirmed } }   // maps to `cluster::WriteMode` (unchanged)
#[derive(Clone, Copy)] pub(crate) struct Confirmed { dry_run_generation: u64 }   // private field: only `confirmed()` builds it; Copy so a batch or drain reuses it per commit
/// `Some` only when the dry-run passed and the typed name matches or is not needed (`run_guarded`, 0034 drain dialog).
pub(crate) fn confirmed(dry_run: &DryRunState, typed: TypedMatch, dry_run_generation: u64) -> Option<Confirmed>;
pub(crate) struct WriteStep { pub(crate) intent: GuardedIntent /* kind: Write */, pub(crate) generation: u64,
    pub(crate) mode: CommitMode, pub(crate) note: Option<String> }
pub(crate) enum CheckedWriteError { Blocked(SharedString) /* commit_block text; never audited */, Write(WriteError) }
/// Steps 5–6 of the `Write` branch and the **only** caller of `ClusterConnection::write`. Commit: re-resolve
/// `guard_for`, `commit_block`, send, append the audit line. Dry-run: re-resolve the guard, send, no audit
/// line (0031 `preview_write` relies on this).
pub(crate) async fn checked_write(shell: &WeakEntity<AppShell>, step: WriteStep, cx: &mut AsyncApp)
    -> Result<WriteOutcome, CheckedWriteError>;
impl AppShell {
    /// The one guarded core: gate → confirm step → dry-run (`Write` only) → (dialog) → lock re-check →
    /// commit or connect → audit → notice. The dry-run is the only branch on `kind`.
    pub(crate) fn run_guarded(&mut self, intent: GuardedIntent, trigger: Trigger, window: &mut Window, cx: &mut Context<Self>);
    /// Thin wrapper: `run_guarded` with `GuardedKind::Write(intent.request)`.
    pub(crate) fn start_write(&mut self, intent: WriteIntent, trigger: Trigger, window: &mut Window, cx: &mut Context<Self>);
}
/// Pure: may the commit go now? Runs right before **every** commit, on the Dialog and the Run path.
pub(crate) fn commit_block(guard: Option<&ClusterGuard>, dry_run_generation: u64, dry_run: &DryRunState,
    typed: TypedMatch) -> Option<SharedString>;
pub(crate) enum DryRunState { Running, Passed { elapsed: Duration }, Failed(SharedString), Rejected(SharedString), NotSupported }
pub(crate) enum TypedMatch { NotNeeded, Matches, Differs }
```

The steps below are `run_guarded`'s. **Dry-run in steps 3–4:** `Write`, `Batch` (one item at a time; all must pass), `CreateThenAttach` (its `request`), and `Helm` (the CLI's own dry-run, 0038). **No dry-run:** `Connect` (`DryRunState::NotSupported`, dialog line `Dry-run not supported for this action`); its step 5 calls the connect callback instead of `write` (0036 `start_connect` is the wrapper). Every other step, reason, and check is shared.

1. `guard_for(&intent.cluster)` (the row's cluster, never the primary; guardrails.md); `action_availability` must be `Enabled` (a stale menu cannot bypass it). Remember `guard.generation` as the dry-run generation.
2. `confirm_step(guard.confirm, risk, trigger, expected)`.
3. `Run`: dry-run; when it passed, step 5 at once; otherwise an error notice and no commit.
4. `Dialog`: open the confirm dialog and start the dry-run at once; Apply is enabled only while `commit_block` is `None`.
5. Commit (both paths): `Write` → `checked_write(WriteStep { intent, generation, mode: Commit { confirmed }, note })`, which re-resolves `guard_for(&intent.cluster)`, runs `commit_block`, and only on `None` sends `connection.write(&request, Commit)`. `Connect` → the lock re-check, then its callback. `Batch` → one `checked_write` per item (0032 bulk-write.md).
6. Audit line (inside `checked_write` for writes; audit-log.md), then a notification: success `{label}: done`, or `{label}: created {created_name}` when the outcome names one (theme success); failure per the table below.

`commit_block` reasons, first match wins: guard `None` or generation changed → `{cluster} is no longer open; nothing was changed`; `Locked` → `{cluster} was locked; nothing was changed`; dry-run `Running` → `Waiting for the dry-run…`; `Failed(text)` → text; `Rejected(reason)` → `An admission webhook does not support dry-run, so this change cannot be checked: {reason}. Nothing was changed.`; `Differs` → `Type {expected} to confirm`.

**Dry-run rejected by a webhook blocks the commit** (`WriteError::DryRunRejected` → `DryRunState::Rejected`). There is no "apply without dry-run" in 0030; a later spec may add an explicit, audited escape. `WritesBlocked` (debug build) shows as `Failed` with its text.

## Confirm dialog (`confirm_dialog.rs`)

W10 small modal, kit `Dialog`, width 480:

| Part | Content |
|---|---|
| Title | `environment_badge` + `{label} on {cluster}?` (e.g. `Cordon node wk-04 on prod-eu-1?`) |
| Object row | kind icon text, `{namespace}/{name}` mono, `{n} field(s) changed` |
| Changes | one line per `changed_fields()`: `spec.unschedulable → true` (value omitted when `None`) |
| Warnings | one warn-toned line per `GuardedIntent.warnings` entry (theme warning token), under Changes |
| Dry-run line | `Server dry-run…` · `Server dry-run passed · {ms} ms` (success) · `Dry-run failed: {reason}` (danger) · `Dry-run not supported for this action` (muted) |
| Typed name | TypeName only: muted `Type the cluster name to confirm` (or `…node name…`), an `Input`, then `matches` (success) when `TypedMatch::Matches` |
| Note | checkbox `Add a note to the audit log` + an `Input` (≤ 500 chars) shown when checked |
| Buttons | `Back` (cancel), primary = the label (`Cordon`); danger variant for `Destructive` |

- **Enter handling (held Enter never confirms)**: the dialog content has `key_context("WriteConfirm")`; `keymap.rs` binds `enter` → `gpui::NoAction` in `WriteConfirm` and `WriteConfirm > Input` (after kit init), which suppresses the kit `Dialog`/`Input` Enter bindings there. The content's `on_key_down` confirms on `enter` only when `!event.is_held` and the tier is `EnterOrClick` or `TypeName` (and `commit_block` is `None`). gpui dispatches bindings before key-down listeners (`window.rs` `dispatch_key_event`), so a binding could not see `is_held`; this is why Enter is handled as a key event. Test `held_enter_does_not_confirm`.
- `ClickOnly` (Click tier, pointer): Enter does nothing; only a click on the primary button confirms. `EnterOrClick`: the primary button is focused. `TypeName`: focus starts in the input.
- Closing the dialog drops the dry-run task. A started commit is not cancelled (its task is detached; decision 16).
- Unlock uses the same dialog with no object row, no dry-run, and primary `Unlock`.

## Error surfaces

| Where | Failure | Text |
|---|---|---|
| dialog dry-run line | `Denied` / `Invalid` / `NotFound` / `WritesBlocked` / `Cluster(_)` | the (redacted) `WriteError` `Display`, top level only; `Invalid` lists `fields` |
| dialog dry-run line | `DryRunRejected` | the webhook text above; Apply stays disabled |
| dialog after commit | `Conflict` | `The object changed since the check: {message}` + `Retry` (re-runs the dry-run) |
| dialog after commit or dry-run | `TooManyRequests` | `The server refused for now: {message}` + `Retry` (re-runs the dry-run), like Conflict |
| notification (`Run` path or after a closed dialog) | any | `{label} failed: {error}`; `OutcomeUnknown` (any commit error after the request may have left: timeout, transport, service, unreadable response) → `{label}: the outcome is unknown; the change may have been applied. Refresh to check.` |
| menus, keys | gate | guardrails.md reasons |

Credentials never appear (0001 errors carry none); request bodies never appear in any text.

## Async contract

| Work | Runs on | Reaches UI by |
|---|---|---|
| dry-run, commit | `ClusterRuntime::spawn(connection.write(..))` (tokio) | `RuntimeTask` awaited in `cx.spawn`; dialog state `this.update` |
| commit lifetime | `cx.spawn(..).detach()` on `AppShell` | ends with the audit append and the notification |
| audit append | `background_executor().spawn` | none (failure → a title-bar notice) |

## First consumer: Cordon / Uncordon

- Node menu item (0003): label `Uncordon` when `NodeScheduling::Disabled`, else `Cordon`; 0028 key C runs the same `start_write` with `Trigger::Key`. `ResourceAction::Cordon` becomes shipped; `Drain` stays `Comes in a later version` (0034).
- Request: `WriteRequest::new(ObjectRef(Node, None, name), SetNodeSchedulable { schedulable })`; risk `Change`; expected name = cluster display name.
- The node row updates from the existing nodes watch; no optimistic UI.
- Rationale: one field, reversible, cluster-scoped (no namespace SSAR question), dry-run supported, already a menu item and key. 0034 keeps bulk cordon, drain, taints, labels.

## Screenshot (`--screen cordon-confirm`, screenshot feature only)

Opens the dialog for the first node with a fixture state (`DryRunState::Passed { 412 ms }`, TypeName tier, seeded PROD entry) and **no** connection call: the screen bypasses steps 1–3 and never reaches step 5. Screenshot builds are debug builds, so `write` would return `WritesBlocked` anyway. Listed in `USAGE`.
