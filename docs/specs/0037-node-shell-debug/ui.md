# 0037 · Entry points, dialogs, settings, tabs

[Back to index](README.md) · Steps 2–3 · Modules: `resource_actions.rs`, `debug_dialogs.rs` (new), `write_guard.rs` (0030), `cluster_registry.rs` (0024), `settings_window.rs` (0025), `shell_tab.rs`, `dock.rs` (0036), `keyboard_navigation.rs` (0028), `screenshot.rs`. Decisions 5, 6, 14–18, 25.

## Entry points

| Where | Item | Action |
|---|---|---|
| Pod row / drawer ⋯ "Open shell ▸" submenu (W4 note 2) | last item after a separator: `Debug container…` with muted `ephemeral` | `DebugContainer` |
| 0036 Shell tab ended with `NoShell` | header button `Debug container…` | same, prefilled with that container |
| Node row / drawer ⋯ (W5) | `Open node shell` (S) | `OpenNodeShell` |
| S on a node row (0028), palette `>` Open node shell (0029) | the `OpenNodeShell` arm of `run_available_row_key` for the cursor node (its own slot) | same |
| Session goes Live with leftovers of other instances | notice `{n} leftover node shell pods` · `Review…` | sweep dialog ([session-flow.md](session-flow.md)) |

`ResourceAction::DebugContainer` is new (with a `RowAction` and an unbound unit action); `OpenNodeShell` exists (on main `Mutating { CreatePodExec, is_shipped: false }`, replaced). Both use 0036's multi-check `Mutating` gate, shipped in steps 2 and 3 (decisions 28–29).

## Gate rows (0030 `action_availability`, order kept)

| # | `DebugContainer` | `OpenNodeShell` |
|---|---|---|
| shipped | step 2 | step 3 |
| checking / unknown | 0030 texts | 0030 texts |
| SSAR | `patch pods/ephemeralcontainers`, `watch pods`, attach pair | `create pods`, `delete pods`, `watch pods`, attach pair |
| subject | no running container → `The pod has no running container` | Windows node → `Node shell needs a Linux node` |
| setting | — | `allow_node_shell == false` → `Node shell is off for {cluster} (Settings › Clusters › Safety)` |
| lock | `{cluster} is read-only` | same |

## Options dialogs (`debug_dialogs.rs`, kit `Dialog`, width 460; Continue starts `run_guarded`)

**Debug container on {pod}?**

| Field | Content |
|---|---|
| Target container | select of running containers (Main and Sidecar tags), default the first running Main or the prefilled one |
| Image | input, default `profile.debug_image` or `DEFAULT_DEBUG_IMAGE` (digest-pinned busybox); validated (decision 25) |
| Warning (warning token) | `Ephemeral containers cannot be removed. It stays in the pod spec until the pod is deleted. Each Reconnect adds another container.` |
| Note (muted) | `Closing the tab ends the shell and everything started from it.` |
| Buttons | `Cancel` / `Continue` |

**Open a privileged shell on node {node}?**

| Field | Content |
|---|---|
| Warning (danger token) | `Creates a privileged pod with host PID access on {node}. Anything you run affects the node.` |
| Namespace | input, default `profile.node_shell_namespace` or `kube-system` |
| Image | as above; must provide `nsenter` and `sh` (muted hint) |
| Note (muted) | `Closing the tab ends the shell and everything started from it. The pod is deleted when the shell ends.` |
| Buttons | `Cancel` / `Continue` (danger) |

Continue → `run_guarded(GuardedIntent { cluster, action, label, risk, expected_name, kind: CreateThenAttach { .. } })`:

| Action | `label` | `risk` | `expected_name` |
|---|---|---|---|
| Debug container | `Add debug container to {pod}` | `Change` | `None` (cluster name when TypeName) |
| Node shell | `Open node shell on {node}` | `Privileged` | `Some(node)` |

`confirm_step` (merged in 0030 step 2a, extended): `Privileged` → `DialogConfirm::TypeName { expected }` for every mode; the dialog primary button uses the danger variant. The 0030 confirm dialog shows the dry-run line and `changed_fields` (e.g. `spec.hostPID → true`, `…privileged → true`). After a successful start, `debug_image` and `node_shell_namespace` are written to the cluster's registry entry when they differ from the stored value.

## Settings (0024 keys, 0025 W2 Clusters › Safety)

| Key | Type, default | UI |
|---|---|---|
| `registry.clusters[].allow_node_shell` | `Option<bool>`; `None` = ON for LOCAL, ON for DEV and STG only when `environment` is set in the entry, else OFF (decision 15) | W2 toggle `Allow node shell` below `Confirm changes by`, hint `Creates a privileged debug pod on the node. Off by default for production.` (muted) |
| `registry.clusters[].debug_image` | `Option<String>` | options dialogs only (W2 draws no field) |
| `registry.clusters[].node_shell_namespace` | `Option<String>` | node shell dialog only |

`ClusterProfile` gains `allow_node_shell: bool`, `debug_image: String`, `node_shell_namespace: String` (resolved). The 0024 allow-list test gains the three keys. Reset clears them with the entry.

## Shell tab variants (0036 `ShellTab`)

| Source | Tab label (W8, W5) | Header | Shell picker |
|---|---|---|---|
| Exec (0036) | `›_ shell · {pod suffix}/{container}` | 0036 | yes |
| Debug container | `›_ debug · {pod suffix}/{container}` | `›_ debug {name} → {container} · {pod} · {image} · {context}` | hidden |
| Node shell | `›_ node shell · {node} (debug pod)` (W5) | `›_ node {node} · pod {ns}/{name} · {image} · {context}` | hidden |

- `Connecting` shows `Starting debug container…` / `Starting node shell pod…`, then the waiting reason when known (`Pulling image…`).
- Reconnect (header) re-opens the options dialog prefilled; it never reuses a pod or container.
- The 8-tab cap of 0036 counts these tabs too.

## Screenshot (`--screen node-shell-confirm`, `screenshot` feature only)

Opens the 0030 confirm dialog for the first fixture node with `risk: Privileged`, `DryRunState::Passed { 388 ms }`, typed-name field empty, seeded PROD entry, and the node shell `changed_fields`; no connection call (as 0030 `cordon-confirm`). Listed in `USAGE`.
