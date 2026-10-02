# 0037 · Decisions

[Back to index](README.md). Architect defaults; items marked (user) need confirmation.

## Debug container

| # | Decision | Rationale |
|---|---|---|
| 1 | Strategic merge patch of `spec.ephemeralContainers` on the `ephemeralcontainers` subresource (`Api::patch_ephemeral_containers` shape) through `ClusterConnection::write` | the merge key `name` appends without a prior GET; dry-run supported; one write path (0030) |
| 2 | `targetContainerName` = the chosen container | shares its process namespace: `ps` and `/proc/1/root` reach a distroless container |
| 3 | `command: ["sh"]`, `stdin`, `tty`, **`stdinOnce: true`**; attach, not exec | the shell ends when the attach ends, so closing the tab stops the process (ephemeral containers cannot be removed) |
| 4 | Reconnect adds a new ephemeral container (new name), through the gate again; the dialog says `Each Reconnect adds another container.` | a terminated ephemeral container cannot restart |
| 5 | Risk `Change` (cluster tier), after an options dialog that always opens | it needs a container and image choice; it is irreversible, so the dialog says so |
| 6 | Default image pinned by digest: `docker.io/library/busybox:1.36.1@sha256:<DIGEST>` (the coder fills `<DIGEST>` from the registry; open item 6); the cluster's `debug_image` overrides it, free-form | small, has `sh` and `nsenter` (verified); a digest cannot be re-pointed; air-gapped clusters need a mirror |

## Node shell

| # | Decision | Rationale |
|---|---|---|
| 7 | A pod with `nodeName`, `hostPID`, `privileged`, command `nsenter -t 1 -m -u -i -n -p -- sh -c <0036 Auto script>` | W5 note 7 (`hostPID`, `nsenter` into PID 1); the host shell, not the image's |
| 8 | Not `hostNetwork`/`hostIPC`; `nsenter -n -i` reaches them through PID 1 | fewer host namespaces declared on the pod |
| 9 | Toleration `operator: Exists`; no resources; `restartPolicy: Never`; `terminationGracePeriodSeconds: 0`; `automountServiceAccountToken: false`; `enableServiceLinks: false` | schedules on tainted nodes; no token reaches a privileged pod |
| 10 | Client-side name `k8sboard-node-shell-{node}-{5 chars}` (≤ 63) | the name is known before the create, for the tab and the audit |
| 11 | `activeDeadlineSeconds: 14400` (4 h) | if the app dies before attaching, the privileged process still ends; `// ponytail: 4 h cap; make it a setting if long sessions need it` |
| 12 | Delete with a `uid` precondition and `gracePeriodSeconds: 0` when the shell exits, the tab closes, the cluster switches, the start fails after the create, and when the main window closes (the close waits for it); at quit only best effort within GPUI's 200 ms | 0030 decision 14; nothing privileged outlives its session where we control the timing |
| 13 | Cleanup runs through `write` as a **commit only** (no dry-run; recorded in 0030 decision 5) with **no** gate, lock, or dialog, only for `DeleteNodeShellPod` of this run's pod or a sweep row | removing a k8sBoard privileged pod must work after a lock, a switch, and inside the shutdown window; the `uid` precondition makes a dry-run add nothing |
| 14 | Risk `Privileged` (new 0030 `ActionRisk` variant): always a TypeName dialog with the **node name**, for every tier and trigger | the strongest tier the task asks for; W6 types the node name for drain |
| 15 | Per-cluster `allow_node_shell` (W2 Safety). Default ON for DEV and LOCAL (set or guessed), for STG only when the environment is set explicitly in the registry; OFF for PROD and for a guessed STG or unknown environment (C5 fallback). Checked in the gate after RBAC, before the lock | W2 hint "Off by default for production"; a guess must never enable a privileged pod |
| 16 | Namespace: the cluster's `node_shell_namespace`, default `kube-system`, editable in the options dialog | usually exempt from Pod Security enforcement and where admins expect system pods; other namespaces often reject `privileged` |
| 17 | Linux nodes only (`nodeInfo.operatingSystem == "linux"`) | `nsenter` and `sh` exist only there |
| 18 | Leftover sweep: each run labels its pods `k8sboard.io/instance: {run id}`; at session start a read-only list finds node shell pods of other instances in any phase and shows a notice with `Review…`; deletion only on the user's click, never automatic (session-flow.md) | covers crashes and quits past 200 ms without a background job; running pods may be someone else's |

## Both

| # | Decision | Rationale |
|---|---|---|
| 19 | `AttachPermit` (not `Clone`), only from `AccessReport::attach_permit` (both `get` and `create` `pods/attach`); taken **before** the commit | no pod is created or patched that the user cannot attach to; KEP-4006 verbs |
| 20 | New `GuardedKind::CreateThenAttach { request, open }`: the dry-run, dialog, and lock re-check are 0030's; `open` gets the permit and the `WriteOutcome` | reuses the one guarded core; no parallel path |
| 21 | `WriteOutcome.uid: Option<String>`: the uid the server returned on a commit | the delete precondition needs it |
| 22 | The session uses 0036 `ShellTab`, `TerminalSession`, and `drive`; a new source `Attach` beside `Exec` | one terminal; the task forbids a parallel one |
| 23 | Wait = 1 s GET poll, up to 120 s, behind the transport seam (no watcher); fail fast on image-pull and create-config errors, and on exit 126/127 (`the node has no shell; node shell needs sh on the host`) | image pulls are slow; a bad image or shell-less host must not hang; polls are trivially fake-testable |
| 24 | Audit: one line per create/patch commit and one per cleanup delete; fields name image, node, container, privileged; never bytes | C10; secrets never (C1) |
| 25 | Image text: non-empty, no whitespace, ≤ 255 chars; stored per cluster after a successful start (`debug_image`) | air-gapped mirrors; no registry parsing |
| 26 | Kill switch: the 0030 `WritePolicy` blocks every create, patch, and delete in debug builds; the attach never runs without a created object | C3 first-call approval |
| 27 | Both options dialogs say `Closing the tab ends the shell and everything started from it.` | `stdinOnce` ends the shell; its children get a hangup |
