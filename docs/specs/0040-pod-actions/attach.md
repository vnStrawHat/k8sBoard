# 0040 · Attach

[Back to index](README.md) · Steps 1–2 · Decisions 3–9. Wireframes: W4 menu `Attach A`, W4b note 3 (container ⋯ menu "logs, shell, attach, copy image"), W8/W8b tab.

## Cluster crate (step 1)

```rust
// pod.rs: from spec.stdin, spec.stdinOnce, spec.tty of the container (init and sidecar too).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContainerTerminal { None /* !(stdin && tty) */, Interactive, InteractiveOnce /* stdinOnce */ }
pub struct ContainerSummary { /* … */ pub terminal: ContainerTerminal }

// debug_shell.rs
pub enum AttachWait { NodeShellPod, EphemeralContainer, /** 0040: a running container of the pod's spec */ Container }
```

- `readiness(pod, container, AttachWait::Container)` looks in `status.containerStatuses` then `status.initContainerStatuses` (native sidecars). Running → `Running` (attach after the first read); terminated → `Failed("the container ended ({reason})")`; the 126/127 `NO_SHELL` mapping stays `NodeShellPod` only; the fatal waiting reasons and the 120 s cap are shared.
- The action words follow the wait: `Container` → `attaching to a container` / `waiting for the container`; the others keep their 0037 words. The module comment says the file attaches to a container k8sBoard created **or** to a running container with a terminal (0040).
- Nothing else changes: `attach_shell` keeps the kill switch first, `AttachPermit`, `AttachParams::interactive_tty()`, `run_raw`, `connect_error(ATTACH)`, `drive_process`. No new `pub` function.
- Every `ContainerSummary` literal (cluster and app fixtures, about 72) gains `terminal: ContainerTerminal::None`; the few attach fixtures set `Interactive`. Mechanical.

## Gate and blocks (step 2, `resource_actions.rs`)

| Item | Value |
|---|---|
| `ResourceAction::Attach`, `RowAction::Attach`, key action `Attach` | bound `a` in `WORKSPACE` (free today); pods only (`subject_action`: Pod → `Attach`, else `None`) |
| `gate()` | `Mutating { checks: [GetPodAttach, CreatePodAttach], is_shipped: true }`; denied text `Not permitted: get and create pods/attach` (the existing `VERB_PAIRS` row) |
| `action_risk`, `action_label` | `Change`, `Attach` |
| `attach_block(container) -> Option<SharedString>` | not running → `Container is not running; see Previous logs or Debug container`; `Init` kind → `Init containers cannot be attached`; `terminal == None` → `The container has no terminal (stdin and tty); use View logs` |
| `default_attach_container(pod) -> Result<&ContainerSummary, SharedString>` | first running Main with a terminal, else first running Sidecar with one; else `No running container has a terminal (stdin and tty); use View logs` |
| `key_availability_of` | `Attach` on a pod: the gate first, then `default_attach_container` (its `Err` is the disabled reason) |

## Entries

| Where | Item | Runs |
|---|---|---|
| Pod row menu and drawer ⋯ (`pod_menu`) | `Attach` with key hint A, after `Port-forward ▸` (W4); disabled with the gate or block reason | dispatches A |
| A, palette `> Attach` | the `Attach` arm of `run_available_row_key` → `attach_default(&subject)` | the default container, cursor's cluster |
| Container ⋯ menu (`container_menu`) | `Attach` after `Open shell`; `container_attach_item` with `on_click`, wrapped in `guarded(row, item)` | that container |

No Reconnect on an attach tab (decision 9): pressing A again opens a new tab through the same gate.

## Intent (`shell_open.rs`, `write_flow.rs`)

```rust
pub(crate) enum ConnectOpen { Exec(..), PortForward(..), CreateThenAttach(..), /** 0040 */ Attach(Rc<ContainerAttachOpen>) }
pub(crate) type ContainerAttachOpen =
    dyn Fn(&mut AppShell, AttachPermit, ClusterConnection, &mut Window, &mut Context<AppShell>);
impl AppShell {
    pub(crate) fn start_attach(&mut self, open: ShellOpen, window: &mut Window, cx: &mut Context<Self>);
    pub(crate) fn attach_default(&mut self, subject: &ClusterObject, window: &mut Window, cx: &mut Context<Self>);
}
```

- `ConnectOpen::granted`: `Attach` takes `attach_permit_of(access)?` like `CreateThenAttach`; `GrantedOpen::Attach(open, permit)` runs `open`; `create()` is `None`.
- `start_attach` mirrors `start_shell`: tab cap check, `guard_for(&open.cluster)` and `slot_label` (else `{context} is not open`), then `start_connect(ConnectIntent { .. })`:

| Field | Value |
|---|---|
| `action`, `risk`, `expected_name` | `Attach`, `Change`, `None` (TypeName types the cluster name) |
| `label`, `button` | `Attach to {pod}/{container}`, `Attach` |
| `warnings` | `What you type goes to the main process of {container}; Ctrl C, Ctrl D or exit may stop it, and the container restarts.` · when `InteractiveOnce`: `This container closes its input after one attach (stdinOnce): closing the tab ends its process.` |
| `object`, `fields` | Pod `{ns}/{pod}`; `container` |
| `open` | `Dock::open_attach(target, ShellKind::Attach, label, AttachGrant { connection, permit })`, then `watch_shell` and `begin_shell_start` (as `start_shell`) |

- The dialog line reads `Dry-run not supported for this action` (0036 `NotSupported`); `gate_block` is re-read on render and at commit, so a lock or a lost permission while the dialog stands refuses the confirm.
## Tab (`shell_tab.rs`)

| Item | Attach |
|---|---|
| `ShellKind::Attach` | new variant; `is_exec()` false; `connect_attach` maps it to `AttachWait::Container` |
| Tab label | `›_ attach · {pod suffix}/{container}` |
| Header | `›_ attach {pod} · {container} · {context}`; no shell picker, **no Reconnect**; Find and Clear as 0036 |
| Connecting | `Attaching…`; after `Started` the 0037 note `If you don't see a prompt, press Enter.` |
| Ended | `[process exited with code N]` or `[connection lost: …]`; no `Debug container…` button |
| Close, switch, quit | dropping the tab drops the stream: the WebSocket closes (detach). Counted in the 8-tab cap and in `leaving_work` shells |

## Audit (`shell_open.rs` `start_audit`)

`ShellKind::Attach` → action `Attach`, object Pod, field `container`; outcome `applied` on `Opened`, `failed` with the error on `OpenFailed`, `abandoned` through `ShellStarts` when the tab closes first. Each A press is its own start and line. Never stream bytes.

## Screen

`--screen attach-confirm` (screenshot feature, no cluster): the connect dialog of the shell fixture pod on the fixed Production cluster, `InteractiveOnce`, both warnings, the typed cluster name field empty; `show_fixture` makes the button dead (as `shell-confirm-fixture`).
