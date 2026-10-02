# 0036 · Exec transport, gating, RBAC

[Back to index](README.md) · Steps 1 and 4 · Modules: `crates/cluster/src/pod_shell.rs` (new; tests in `pod_shell_tests.rs`), `access_review.rs`, `crates/app/src/resource_actions.rs`, `write_flow.rs` (0030). Decisions 6–13, 33, 34.

## Cluster API (`pod_shell.rs`, `pub use` in `lib.rs`)

```rust
pub struct ShellRequest { pub namespace: String, pub pod: String, pub container: String,
    pub shell: ShellCommand, pub size: GridSize }
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub struct GridSize { pub cols: u16, pub rows: u16 }
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ShellCommand { #[default] Auto, Bash, Sh }
pub enum ShellInput { Bytes(Vec<u8>), Resize(GridSize) }          // manual Debug: byte count only
pub enum ShellUpdate { Started, Output(Vec<u8>), Exited(ShellExit), Failed(ClusterError) } // same
pub struct ShellExit { pub code: Option<i32>, pub message: Option<String> }
/// Proof that the session SSAR allowed both exec verbs. Not `Clone`.
pub struct ExecPermit(());
impl ClusterConnection {
    /// action: "opening a shell". Nothing happens until polled on the app's tokio runtime.
    /// Ends after `Exited` or `Failed`; dropping the stream ends the session.
    pub fn pod_shell(&self, permit: ExecPermit, request: ShellRequest,
        input: impl Stream<Item = ShellInput> + Send + Unpin + 'static) -> impl Stream<Item = ShellUpdate> + Send + 'static;
}
```

- `ShellInput` and `ShellUpdate` have **no derived `Debug`**: the manual impl prints `Bytes(12 bytes)`, `Output(4096 bytes)` (C1; AC 7).
- `ExecPermit`: the only non-test constructor is `AccessReport::exec_permit(&self) -> Option<ExecPermit>` in `access_review.rs`, `Some` only when both `GetPodExec` and `CreatePodExec` are allowed. `#[cfg(test)] ExecPermit::for_tests()` for transport tests. The app cannot open a shell without a report that allows it.
- No kube or k8s-openapi type in a public signature (0009 AC 3).
- **Kill switch first**: before anything else `pod_shell` reads the connection's 0030 `WritePolicy`; `Blocked` (debug build without `K8SBOARD_ALLOW_WRITES=1`) yields one `ShellUpdate::Failed(ClusterError::Rendered { message })` with the 0030 `WritesBlocked` text and ends, with zero requests. Exec can change anything in a container, so it is gated like a write (C3: approved by the user on 2026-10-02, one approval for all mutating specs).
- Open: `Api::<Pod>::namespaced(..).exec(pod, argv(shell), &AttachParams::interactive_tty().container(c))` inside `connection.run(SHELL_ACTION, ..)`: one HTTP `GET` with a WebSocket upgrade (no POST). Then `Started`, then the initial `Resize(request.size)`.
- **Upgrade errors (mandatory mapping)**: `kube::Error::UpgradeConnection(UpgradeConnectionError::ProtocolSwitch(code))` carries no `Status` body, so `fn upgrade_error(code: StatusCode, context) -> ClusterError` uses fixed text:

| Code | `ClusterError` | Message |
|---|---|---|
| 401 | `Unauthorized` | `the server refused the exec connection (HTTP 401)` |
| 403 | `Forbidden` | `the server refused the exec connection (HTTP 403); a shell needs get and create on pods/exec` |
| 404 | `Api { code: 404 }` | `pod or container not found` |
| other | `Api { code }` | `the exec connection was refused (HTTP {code})` |

- Drive (private, generic so tests use `tokio::io::duplex`):
  `fn drive(owner: AttachedProcess, stdout: impl AsyncRead, stdin: impl AsyncWrite, resize: impl Sink<TerminalSize>, status: impl Future<Output = Option<Status>>, input) -> impl Stream<Item = ShellUpdate>`.
  `pod_shell` takes `stdout()`, `stdin()`, `terminal_size()`, `take_status()` from the `AttachedProcess` and moves the process itself into `drive`'s `stream::unfold` state (`owner`; tests pass a unit guard through a generic `O: Send`). kube's message-loop task belongs to that process, whose `Drop` aborts it, so **dropping the stream ends the session**; `pod_shell.rs` calls no `spawn`.
  It reads stdout into a **64 KiB** buffer and yields each read as one `Output` (whatever is ready; no timer); writes `Bytes` to stdin; forwards `Resize` (latest wins when the sink is full); on stdout EOF awaits the status and yields `Exited`. `tty = true` merges stderr into stdout.
- `fn shell_exit(status: Option<Status>) -> ShellExit` (pure): `Success` → code 0; `NonZeroExitCode` → the `ExitCode` cause; otherwise `code: None` with the status message.
- `fn argv(shell: ShellCommand) -> Vec<&'static str>` (pure): `Auto` → `sh`, `-c`, and the script `for s in bash ash sh; do if command -v "$s" >/dev/null 2>&1; then printf '\033]7770;%s\007' "$s"; exec "$s"; fi; done` (the private OSC 7770 names the picked shell for the header, [shell-tab.md](shell-tab.md)); `Bash` → `bash`; `Sh` → `sh` (bare names, resolved through the container's `PATH`).
- A container without `sh` (distroless) fails at start with "executable file not found"; the tab shows "No shell in this container" and points to "Debug container…" (0037).
- Traces carry the action name, target names, durations, byte counts, and the exit code only.

## RBAC: both exec verbs (decision 13)

API servers before 1.35 authorize a WebSocket exec as verb **`get`** on `pods/exec` (kubernetes#78741); KEP-4006 adds the `create` check in 1.35. UAT runs 1.29.5.

- New `AccessCheck::GetPodExec` = `("get", "", "pods", Some("exec"), true)`, added to `ALL` (one more SSAR per session).
- `OpenShell` is allowed only when **both** `GetPodExec` and `CreatePodExec` are allowed. Unknown, checking, or one denial → disabled (fail closed). Reason when either is denied: `Not permitted: get and create pods/exec`.
- UAT live check: record both SSAR answers from the session report (trace) and confirm the gate's text; **never attempt an exec** (no exec request in the trace, whatever the answers are).

## Gating through 0030 (one guarded core)

| 0030 piece | 0036 use |
|---|---|
| `action_availability(ResourceAction::OpenShell, &ClusterGuard)` | `ActionGate` with both checks above, `mutates: true`, shipped in step 4. Order: `Checking permissions…` → `Not permitted: get and create pods/exec` → `{cluster} is read-only` |
| `guard_for(&cluster)` | the target pod's own cluster (0027 contract) |
| `confirm_step(guard.confirm, ActionRisk::Change, guard.display_name)` | always a dialog: PROD types the cluster name; STG, DEV, LOCAL click Confirm |
| `run_guarded(GuardedIntent { kind: Connect(..) })` (0030 write-flow.md) | no dry-run (`NotSupported`); after the lock re-check, the connect callback opens the tab |
| `audit_entry(&GuardedIntent, ..)` | one line per session start: action `Open shell`, object `{ Pod, ns, name }`, fields `container`, `command`; outcome `applied` when `Started` arrives, `failed` with the error otherwise; never stream bytes |
| allow-list (write-path.md rule 4) | row `pods/exec`, `GET` + WebSocket upgrade, `/api/v1/namespaces/{ns}/pods/{pod}/exec`, dry-run no, 0036; the grep expects `access_review.rs`, `object_write.rs`, `pod_shell.rs` |

```rust
pub(crate) struct ConnectIntent { pub(crate) object: AuditObject, pub(crate) fields: Vec<AuditField>,
    pub(crate) open: Box<dyn FnOnce(ExecPermit, &mut Window, &mut App)> }
impl AppShell { // thin wrapper, like start_write
    pub(crate) fn start_connect(&mut self, intent: GuardedIntent, window: &mut Window, cx: &mut Context<Self>);
}
```

- `run_guarded` takes the `ExecPermit` from the guard's `AccessReport` right before `open`; no permit → the gate reason, nothing opens. 0035 decides how its own permit enters `Connect` (it may generalize `open`).
- Locking a cluster does not end shells already open; Reconnect runs `start_connect` again and is blocked while locked. On a `Live` tab, Reconnect asks no question of its own: the 0030 tier dialog always opens (0030 decision 9; [shell-tab.md](shell-tab.md)).
- The allowed path is verified by fake-transport tests and `--screen shell-fixture`; a live allowed-path run needs a write-capable cluster (risks R2).
