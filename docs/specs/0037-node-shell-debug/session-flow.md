# 0037 · Session flow: guarded create, wait, attach, cleanup

[Back to index](README.md) · Steps 1–3 · Modules: `crates/cluster/src/debug_shell.rs` (new) + `debug_shell_tests.rs`, `pod_shell.rs` (0036), `access_review.rs`; `crates/app/src/write_flow.rs`, `shell_tab.rs` (0036), `dock.rs`, `app_shell.rs`, `node_shell_sweep.rs`. Decisions 3, 11–13, 18–24, 26.

## Cluster API (`debug_shell.rs`)

```rust
/// Proof that the session SSAR allowed both attach verbs. Not `Clone`.
pub struct AttachPermit(());
impl AccessReport { pub fn attach_permit(&self) -> Option<AttachPermit>; }   // access_review.rs
pub enum AttachWait { NodeShellPod, EphemeralContainer }
pub struct AttachRequest { pub namespace: String, pub pod: String, pub container: String,
    pub wait: AttachWait, pub size: GridSize }
impl ClusterConnection {
    /// action: "attaching a debug shell". Polls until the container runs (≤ 120 s), attaches with a TTY,
    /// then yields 0036 `ShellUpdate`s through `pod_shell::drive`. Dropping the stream ends the attach.
    pub fn attach_shell(&self, permit: AttachPermit, request: AttachRequest,
        input: impl Stream<Item = ShellInput> + Send + Unpin + 'static) -> impl Stream<Item = ShellUpdate> + Send + 'static;
}
```

- **Kill switch first**: policy `Blocked` → one `Failed` with the 0030 `WritesBlocked` text, zero requests (as 0036 `pod_shell`).
- **Wait** = a 1 s GET poll of the pod (`connection.run`, read-only) under a 120 s cap, behind the same private seam as the attach (tests script the GET answers through `FakeApi`; no watcher). Pure `fn readiness(pod: Option<&Pod>, container: &str, wait: AttachWait) -> Readiness` with `Readiness::{Waiting(Option<String>), Running, Failed(String)}`:
  - a running state → `Running`;
  - waiting reason `ErrImagePull` or `ImagePullBackOff` → `ImagePullFailed`: the poll reads the pod's events once (`list events`, field selector `involvedObject.name`), keeps the cause of the newest `Failed to pull image` event as one masked line of at most 200 characters, and sends `ShellUpdate::ImagePullFailed { detail }` (a failed read gives `detail: None`);
  - waiting reason `InvalidImageName`, `CreateContainerConfigError`, `CreateContainerError`, `RunContainerError` → `Failed(reason)`;
  - node shell terminated with exit code 126 or 127 → `Failed("the node has no shell; node shell needs sh on the host")`; any other termination or pod phase `Failed`/`Succeeded` → `Failed`; pod gone → `Failed("the pod no longer exists")`;
  - cap reached → `Failed("container did not start within 120 s ({last waiting reason})")`.
  Waiting and termination **messages** are dropped (registry errors can quote credentials); reasons are fixed CamelCase words. The one exception is the pull event cause above (URL userinfo masked).
- **Attach**: `Api::<Pod>::attach(pod, &AttachParams::interactive_tty().container(c))` inside 0030 `run_raw` (so `UpgradeConnection` reaches `upgrade_error` before `classify_error`), then `Started`, the initial `Resize`, and 0036 `drive` (made `pub(crate)`). 0036 `upgrade_error` gains the verb name (`exec`, `attach`).
- Allow-list row: `pods/attach` (pod-specs.md); the documentation grep gains `debug_shell.rs`.

## Guarded create-then-attach (0030 `run_guarded`)

The variant `GuardedKind::CreateThenAttach { request, open }` is defined in the full enum of 0030 write-flow.md (`open: Box<dyn FnOnce(AttachPermit, WriteOutcome, &mut Window, &mut App)>`); this spec owns its branch:

1. Gate (row's cluster) → `confirm_step` (`Privileged` for node shell, `Change` for debug) → dry-run of `request` (0030 dialog line).
2. Before the commit: lock re-check and `commit_block` (0030), **then take `attach_permit()`**; `None` → the gate reason, no commit, no audit line.
3. Commit through `write`; audit line (outcome, `fields` from `changed_fields`).
4. Success → `open(permit, outcome, ..)`; failure → the 0030 error surfaces, nothing opens.

## App side (`AppShell::open_debug_session`)

```rust
pub(crate) enum ShellSource { Exec(ShellRequest) /* 0036 */, Attach { request: AttachRequest, cleanup: Option<NodeShellCleanup> } }
pub(crate) struct NodeShellCleanup { connection: ClusterConnection, request: WriteRequest /* DeleteNodeShellPod */,
    audit: AuditContext /* cluster label, context, user: copied at start */ }
```

The tab subscribes `connection.attach_shell(permit, request, input)` like 0036's exec tabs (terminal, keys, copy, Find, dock rules are 0036's). After `Started` it adds the note `If you don't see a prompt, press Enter.`

## Cleanup (node shell; decisions 12–13)

`write_flow::run_cleanup(cleanup, cx)` (the second named caller of `ClusterConnection::write` beside `checked_write`; README open item 9): **commit only** (no dry-run; 0030 decision 5 lists it) of `DeleteNodeShellPod` on the held connection; **no** gate, lock, tier, or dialog; one audit line (`Delete node shell pod`). `NodeShellCleanup` is built only from this run's created pod or from a sweep row (a listed pod carrying the k8sBoard node-shell labels, with its uid), so the bypass covers nothing else. At most once per tab (`Option::take`).

| Event | Cleanup |
|---|---|
| Shell exits (`Exited`), attach fails after the create, wait fails | at once |
| Tab closed (×, Ctrl W), dock `close_all` (0026 switch, `release_all`) or `close_tabs_of` (0027 `release_slot`), both after the `leaving_work` confirm | at once (the tab's release handler, on its held connection) |
| **Main window close** (`Window::on_window_should_close`) | returns `false` while deletes are pending, shows `Removing node shell pods…`, closes the window when they finish (each bounded by `REQUEST_TIMEOUT`, 30 s); no GPUI timeout applies here |
| App quit by another path (`on_app_quit`) | best effort within GPUI's 200 ms `SHUTDOWN_TIMEOUT` (gpui `app.rs:78`, shared by all quit futures); `stdinOnce` exit and the sweep cover the rest |

The ephemeral container needs no delete: dropping the attach closes stdin and `stdinOnce` ends `sh` (decision 3).

## Orphans (defence in depth)

| Failure | What ends the process | What removes the object |
|---|---|---|
| App crash after attach | stdin closes with the connection (`stdinOnce`) | the sweep at the next session start |
| App crash between create and attach | `activeDeadlineSeconds` 4 h | the sweep |
| Delete fails (network, quit window) | as above | notice `Could not delete node shell pod {ns}/{name}: {error}` + the sweep |
| Debug container: crash between patch and attach | **nothing**: `sh` waits on an unopened stdin until the pod is deleted (the API cannot remove it) | the pod's own lifecycle; documented ceiling |

## Leftover sweep (decision 18)

- Every app run has a random `instance` id (`random_suffix` × 2); node shell pods carry `k8sboard.io/instance: {id}`.
- When a slot first goes Live (0027 `on_first_live(cluster)`) and `list pods` is allowed there: one read-only list over the session scope with selector `app.kubernetes.io/managed-by=k8sboard,k8sboard.io/purpose=node-shell,k8sboard.io/instance!={id}`, **any phase**.
- `n > 0` → a notice `{n} leftover node shell pods` with `Review…`: a dialog lists `{ns}/{name}`, node, phase, age, with the warning `Running pods may belong to another k8sBoard window or user.`; checkboxes default on for finished pods and for pods of an earlier run of this settings folder (`<config>/node-shell-runs.log`: `<run id> started|quit <time>`, written when a run creates its first node shell pod and when it quits), whose row says `left by your session, quit at 14:41`; other running pods stay off; `Delete selected` is the click confirmation.
- Deletes go through `run_cleanup` (one audit line each), only when the cluster is unlocked and `delete pods` is allowed; otherwise the button shows the gate reason. Never automatic.
