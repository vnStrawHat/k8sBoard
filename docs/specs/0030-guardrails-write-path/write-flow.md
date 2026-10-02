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
impl AppShell {
    /// Gate → confirm step → dry-run → (dialog) → lock re-check → commit → audit → notice.
    pub(crate) fn start_write(&mut self, intent: WriteIntent, trigger: Trigger, window: &mut Window, cx: &mut Context<Self>);
}
/// Pure: may the commit go now? Runs right before **every** commit, on the Dialog and the Run path.
pub(crate) fn commit_block(guard: Option<&ClusterGuard>, dry_run_generation: u64, dry_run: &DryRunState,
    typed: TypedMatch) -> Option<SharedString>;
pub(crate) enum DryRunState { Running, Passed { elapsed: Duration }, Failed(SharedString), Rejected(SharedString), NotSupported }
pub(crate) enum TypedMatch { NotNeeded, Matches, Differs }
```

1. `guard_for(&intent.cluster)` (the row's cluster, never the primary; guardrails.md); `action_availability` must be `Enabled` (a stale menu cannot bypass it). Remember `guard.generation` as the dry-run generation.
2. `confirm_step(guard.confirm, risk, trigger, expected)`.
3. `Run`: dry-run; when it passed, step 5 at once; otherwise an error notice and no commit.
4. `Dialog`: open the confirm dialog and start the dry-run at once; Apply is enabled only while `commit_block` is `None`.
5. Commit (both paths): `guard_for(&intent.cluster)` again, then `commit_block`; only `None` lets `connection.write(&request, Commit)` run.
6. Audit line (audit-log.md), then a notification: success `{label}: done` (theme success); failure per the table below.

`commit_block` reasons, first match wins: guard `None` or generation changed → `{cluster} is no longer open; nothing was changed`; `Locked` → `{cluster} was locked; nothing was changed`; dry-run `Running` → `Waiting for the dry-run…`; `Failed(text)` → text; `Rejected(reason)` → `An admission webhook does not support dry-run, so this change cannot be checked: {reason}. Nothing was changed.`; `Differs` → `Type {expected} to confirm`.

**Dry-run rejected by a webhook blocks the commit** (`WriteError::DryRunRejected` → `DryRunState::Rejected`). There is no "apply without dry-run" in 0030; a later spec may add an explicit, audited escape. `WritesBlocked` (debug build) shows as `Failed` with its text.

## Confirm dialog (`confirm_dialog.rs`)

W10 small modal, kit `Dialog`, width 480:

| Part | Content |
|---|---|
| Title | `environment_badge` + `{label} on {cluster}?` (e.g. `Cordon node wk-04 on prod-eu-1?`) |
| Object row | kind icon text, `{namespace}/{name}` mono, `{n} field(s) changed` |
| Changes | one line per `changed_fields()`: `spec.unschedulable → true` (value omitted when `None`) |
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
