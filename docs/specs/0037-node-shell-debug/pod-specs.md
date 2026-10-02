# 0037 · Exact requests, RBAC, admission

[Back to index](README.md) · Step 1 · Module: `object_write.rs` (0030) + `object_write_tests.rs`. Decisions 1–3, 7–12, 19, 21. Bodies are pinned byte for byte by `allow_list_matches_the_operations` (AC 3); key order as written.

## New `WriteOperation` variants (0030 allow-list)

```rust
pub enum WriteOperation {
    SetNodeSchedulable { schedulable: bool },                       // 0030
    AddDebugContainer { name: String, image: String, target_container: String },
    CreateNodeShellPod { node: String, image: String, user: Option<String> },
    DeleteNodeShellPod { uid: String },
}
pub struct WriteOutcome { pub mode: WriteMode, pub elapsed: Duration, pub uid: Option<String> } // uid: commit only
```

`WriteRequest::new` returns `None` unless the target kind is `Pod` and (for the node shell variants) the name starts with `k8sboard-node-shell-`. Manual `Debug` prints the variant name, kind, namespace, name only.

| Operation | HTTP | Path | Content type | Dry-run | `AccessCheck` | Spec |
|---|---|---|---|---|---|---|
| `AddDebugContainer` | PATCH | `/api/v1/namespaces/{ns}/pods/{pod}/ephemeralcontainers` | `application/strategic-merge-patch+json` | yes | `PatchPodEphemeralContainers` | 0037 |
| `CreateNodeShellPod` | POST | `/api/v1/namespaces/{ns}/pods` | `application/json` | yes | `CreatePods` | 0037 |
| `DeleteNodeShellPod` | DELETE | `/api/v1/namespaces/{ns}/pods/{name}` | `application/json` | **no** (commit only, 0030 decision 5) | `DeletePods` | 0037 |
| attach | GET + WebSocket upgrade | `/api/v1/namespaces/{ns}/pods/{pod}/attach?container={c}&stdin=true&stdout=true&tty=true` | — | no | `GetPodAttach` + `CreatePodAttach` (permit) | 0037 |

All writes carry `fieldManager=k8sboard`; dry-runs add `dryRun=All` (0030).

## Debug container patch

```json
{"spec":{"ephemeralContainers":[{"name":"k8sboard-debug-x7k2q","image":"docker.io/library/busybox:1.36.1@sha256:<DIGEST>",
  "command":["sh"],"stdin":true,"stdinOnce":true,"tty":true,"targetContainerName":"api",
  "imagePullPolicy":"IfNotPresent","terminationMessagePolicy":"File"}]}}
```

- Name: `k8sboard-debug-{5}` with `{5}` from `random_suffix()` (5 chars `[a-z0-9]`, `RandomState::new().hash_one(Instant::now())` in base 36; no new dependency).
- No `securityContext`: the pod's own applies (open item 4). `changed_fields`: `spec.ephemeralContainers[].name`, `.image`, `.targetContainerName` (values recorded).

## Node shell pod

```json
{"apiVersion":"v1","kind":"Pod","metadata":{"name":"k8sboard-node-shell-wk-03-x7k2q","namespace":"kube-system",
  "labels":{"app.kubernetes.io/managed-by":"k8sboard","k8sboard.io/purpose":"node-shell","k8sboard.io/node":"wk-03","k8sboard.io/instance":"q4m7x2k9pa"},
  "annotations":{"k8sboard.io/user":"readonly"}},
 "spec":{"nodeName":"wk-03","hostPID":true,"restartPolicy":"Never","terminationGracePeriodSeconds":0,
  "activeDeadlineSeconds":14400,"automountServiceAccountToken":false,"enableServiceLinks":false,
  "tolerations":[{"operator":"Exists"}],
  "containers":[{"name":"shell","image":"docker.io/library/busybox:1.36.1@sha256:<DIGEST>",
   "command":["nsenter","-t","1","-m","-u","-i","-n","-p","--","sh","-c","<AUTO>"],
   "stdin":true,"stdinOnce":true,"tty":true,"securityContext":{"privileged":true},
   "imagePullPolicy":"IfNotPresent","terminationMessagePolicy":"File"}]}}
```

- `<AUTO>` = 0036's `Auto` script (`pod_shell::AUTO_SHELL_SCRIPT`, made `pub(crate)`), so the host's bash, ash, or sh runs.
- Name: `k8sboard-node-shell-{node}-{5}`; `{node}` lowercased, non-`[a-z0-9-]` → `-`, cut so the whole name is ≤ 63 chars, no trailing `-` (pure `node_shell_pod_name`).
- `k8sboard.io/node` label value: the node name if it is a valid label value, else omitted. `k8sboard.io/user`: the kubeconfig user entry name (0030 decision 21), omitted when `None`. `k8sboard.io/instance`: the run id (10 chars `[a-z0-9]`, two `random_suffix` calls), fixed for the app run (sweep, decision 18).
- `changed_fields`: `spec.nodeName`, `spec.hostPID` = `true`, `spec.containers[0].securityContext.privileged` = `true`, `spec.containers[0].image`.

## Delete

Commit only (no `dryRun`). `DeleteParams { grace_period_seconds: Some(0), preconditions: Some(Preconditions { uid: Some(uid), resource_version: None }), .. }` (`kube-core-4.2.0/src/params.rs:763, 951`). 404 counts as done (already gone); 409 (uid differs) → never retried, logged as `failed` (another object took the name).

## Verified APIs

| API | Where |
|---|---|
| `Api::patch_ephemeral_containers` path `ephemeralcontainers`, `Patch::Strategic` | `kube-client-4.2.0/src/api/subresource.rs:276` |
| `Api::attach(name, &AttachParams) -> AttachedProcess` (`ws`); `AttachParams { container, stdin, stdout, stderr, tty, .. }`, `interactive_tty()` | `kube-client-4.2.0/src/api/subresource.rs:683`; `kube-core-4.2.0/src/subresource.rs:139` |
| `EphemeralContainer.{stdin_once, target_container_name}`; `PodSpec.{host_pid, node_name, tolerations, active_deadline_seconds}` | `k8s-openapi-0.28.0/src/v1_32/api/core/v1/` |
| Wait: `Api::<Pod>::get` once per second (read-only GET) under a 120 s cap | `kube-client-4.2.0/src/api/core_methods.rs` (`get`) |

## RBAC (all required; fail closed)

| Action | Checks |
|---|---|
| Debug container… | `patch pods/ephemeralcontainers`, `watch pods` (exists), `get pods/attach`, `create pods/attach` |
| Open node shell | `create pods`, `delete pods`, `watch pods`, `get pods/attach`, `create pods/attach`; `list nodes` (exists, for the menu) |
| Leftover sweep | `list pods` (exists), `delete pods` |

New `AccessCheck`s (namespaced, core group): `CreatePods` (`create pods`), `DeletePods` (`delete pods`), `PatchPodEphemeralContainers` (`patch pods/ephemeralcontainers`), `GetPodAttach` (`get pods/attach`), `CreatePodAttach` (`create pods/attach`). Gate reasons: the first missing check, e.g. `Not permitted: create pods`; the attach pair reads `Not permitted: get and create pods/attach`.

## Admission

- Pod Security Admission: `hostPID` and `privileged` violate `baseline`; the create dry-run returns 403 `violates PodSecurity "baseline:latest": …` → `WriteError::Denied`, shown in the dialog dry-run line; the user picks another namespace in the options dialog.
- Webhooks rejecting dry-run block the action (0030 decision 26).
- ResourceQuota requiring limits rejects the pod at dry-run (no resources are set; decision 9).
