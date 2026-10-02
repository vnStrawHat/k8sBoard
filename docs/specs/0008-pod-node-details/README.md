# 0008 — Pod and Node drawer details (read-only)

Status: amended after advisor review. Crates: `crates/cluster`, `crates/app`. Requires 0006 (committed at `9851346`) and 0007 merged (`DrawerTab`; Pod tabs Overview · Containers · YAML · Events; Node tabs Overview · YAML · Events). Wireframes: W4 (Overview, WHY box), W4b (Containers master-detail), W5 (node columns; no node drawer is drawn). Applies C1.

## Goal

- Pod Overview: a **WHY box** that explains a failing or not-ready pod and links to the container; Node and owner `→` links.
- Container detail: **Info · Env · Mounts** sub-tabs with digest, pull policy, ports (disabled Forward), requests/limits, probes with their current result, "next retry in", and env/mount **names and sources only**.
- Pod menu: **Copy kubectl command**.
- Node Overview: conditions, addresses, system info, capacity/allocatable, pods on the node, labels.

## Non-goals

Usage numbers and bars (0010), Monitor and Logs sub-tabs (0010, 0019), live port-forward (0035), PVC/Secret/ServiceAccount links (0014–0016), "View pods on node" menu filter (0009), `[` `]` keys (0028), the container ⋯ menu, env values anywhere outside the 0007 YAML toggle, annotations, a two-column Info grid.

## Implementation steps

| Step | Scope | ACs |
|---|---|---|
| 1 | `crates/cluster`: pod, container, event, and node summary fields; three `StatusReason` variants | 1, 2, 3, 4, 5 |
| 2 | App pod drawer: `pod_diagnosis`, WHY box, links, container sub-tabs, Copy kubectl command | 1, 2, 3, 6, 7, 9 |
| 3 | App node drawer: sections, `related_pods.rs`, `PodOwner::Node`, full ui-verifier run | 1, 2, 3, 8, 9 |

## Files

| File | Contents |
|---|---|
| [cluster-pod-fields.md](cluster-pod-fields.md) | step 1: pod/container fields, new spec types, mapping and secret rules, `EventSummary.container` |
| [cluster-node-fields.md](cluster-node-fields.md) | step 1: node conditions, addresses, system info, resources, labels |
| [pod-diagnosis.md](pod-diagnosis.md) | step 2: WHY rules, probe results, next retry (pure) |
| [pod-drawer.md](pod-drawer.md) | step 2: shared pieces, Overview, Containers sub-tabs, Env/Mounts, menu |
| [node-drawer.md](node-drawer.md) | step 3: node Overview sections, pods on node |
| [files-to-touch.md](files-to-touch.md) | modules per crate and step, doc updates |
| [test-plan.md](test-plan.md) | unit tests, live checks, ui-verifier |

## Decisions (architect defaults)

| # | Decision | Rationale |
|---|---|---|
| 1 | Spec data lives in the **live summaries**, not a per-drawer GET | pod specs are immutable, data stays live with the watch, no new async path; roadmap says "new summary fields" |
| 2 | WHY rules are **pure app code** (`pod_diagnosis.rs`) over `PodSummary` + the drawer's object events | the crate exposes data, the app explains it; 0020 can lift the rules into an engine later |
| 3 | Pod-level causes (unschedulable, gated, evicted) beat container causes; Bad beats Warn; then lifecycle order | the scheduler/eviction message is the real cause; init failures block everything after them |
| 4 | Probe result = ready/started flags plus `Unhealthy` events tied by `fieldPath`; "Failing · ×N" (newest event count) replaces the wireframe's "3/3"; liveness and readiness read "Waiting for startup" until a startup probe passes | the API has no per-probe status; consecutive-failure counts are kubelet-internal; the kubelet does not run them before startup passes |
| 5 | `EventSummary.container` from `involvedObject.fieldPath` | needed to tie probe events to a container; one small parser |
| 6 | Node `lastHeartbeatTime` and pod condition probe/transition times are not kept | heartbeats would emit a snapshot on every kubelet status update |
| 7 | Messages (waiting, condition, pod, node condition) are cut like event messages (1 KiB) | bounded memory; same rule as 0006 |
| 8 | Terminated `message` (termination log) is never kept. Cost: the WHY box loses the line that names the cause (e.g. a panic); it shows reason and exit code only. Revisit in 0019: the WHY box links to the container's previous logs | it can be application output, including secrets |
| 9 | Env: names and sources only; probes: no exec command, no HTTP headers, HTTP query string dropped | C1; these are the places literal credentials appear |
| 10 | Links only where a screen exists: Node, owner (ReplicaSet, Job, StatefulSet, DaemonSet), ConfigMap sources. ServiceAccount, Secret, PVC are text | no dead links; 0014–0016 turn them into links via `ResourceKey::of_object` |
| 11 | One shared `ResourceKey::of_object` (from 0006's `object_key`) | one kind → screen mapping for events, owners, and sources |
| 12 | WHY box uses the kit `gpui_kit::component::Alert` (error/warning); the "Open container →" link is its sibling (no child slot) | theme colors with no new styling code |
| 13 | Sub-tabs Info · Env n · Mounts n only; Logs and Monitor are omitted, not disabled | they belong to 0019/0010; disabled tabs add noise |
| 14 | `container_tab` survives container and subject changes; `show_screen` resets it | same rule as the drawer tab (0007 decision 21) |
| 15 | Info is one column at both widths | smallest layout that fits 420 px; the W4b grid is a later refinement |
| 16 | Copy kubectl command = `kubectl --context C -n NS describe pod NAME`, shell-quoted when needed; no `--kubeconfig` | `describe` is the read-only triage command; a path is machine-specific |
| 17 | Quantities shown as written; resources and capacity as text, no bars | bars need usage (0010) and a quantity parser |
| 18 | Node drawer adds a Pods section (pods of the current scope on that node) instead of the "View pods on node" filter | reuses the related-pods renderer; the filter needs 0009 |
| 19 | Node condition tone: Ready True is Ok; for every other type True is Bad | pressure and node-problem-detector conditions are problem-positive |
| 20 | No new launch screens | `pod-drawer`, `pod-containers`, `node-drawer` cover the views; sub-tab content is unit-tested |
| 21 | Add `InvalidImageName`, `ErrImageNeverPull`, `CreateContainerError` to `StatusReason`; WHY rules C1/C5 use the shared `is_bad_reason` | the WHY box and the state label can never disagree; no string matching on `Other` |
| 22 | `status_message` is kept only for phase Failed or reason Evicted | P3 must not fire on transient messages of live pods; less memory |
| 23 | Wireframe-shown items stay: the "Other containers are healthy" suffix, `next retry in`, env/mount source summaries. Cut (not in the wireframe): liveness-failing-while-ready and recent-OOM WHY rules | user goal is everything in the wireframe, nothing beyond it |

Known ceilings: memory ≈ 3 × 3–6 KiB per container for the new fields (roughly 10–20 MiB at 1,000 pods; env-heavy pods drive it), because the watch store, the `LiveList`, and a transient clone each hold a copy, and every batch deep-clones the whole list. Upgrade path: `Arc<ContainerSpec>` for the immutable spec part (cheap clones, same API shape), not an on-demand fetch. The `PartialEq` dedupe means any churning field causes snapshots; every chosen field is stable (no heartbeats, no probe times). Probe "×N" is one event series and may include failures from before the current run when the series started earlier.

## Acceptance criteria

- [ ] 1. The quality gate passes, and so does `cargo clippy -p k8sboard --features screenshot --all-targets -- -D warnings`. No new `#[allow]`.
- [ ] 2. Every test of the step in [test-plan.md](test-plan.md) exists under that name and passes, offline.
- [ ] 3. No kube or k8s-openapi type in a public signature; the app gains no kube or serde dependency; no `Cargo.lock` package change; the 0001 read-only grep still finds only the SSAR `create`.
- [ ] 4. Secret safety: no summary field holds an env literal, exec command, HTTP header, query string, termination message, or annotation (tests prove distinctive values are absent from `Debug`). `container_spec.rs` and `pod_diagnosis.rs` contain no `tracing::` call.
- [ ] 5. Node summaries ignore `lastHeartbeatTime` (`node_conditions_ignore_heartbeat`).
- [ ] 6. On UAT, a pod drawer shows working Node and owner links, Info sections, Env and Mounts names with sources, and Copy kubectl command; no env value is visible outside the YAML tab toggle.
- [ ] 7. The WHY box appears only when `pod_diagnosis` is Some, and its container link opens that container on the Containers tab.
- [ ] 8. On UAT, a node drawer shows conditions, addresses, system info, resources, pods (with the scope note when a namespace is picked), and labels.
- [ ] 9. The step's screenshots exist; the ui-verifier reports no high-severity defect against W4/W4b; the 0003 AC4 color-literal grep is clean.

## Open items

1. `pod-containers` shows Info only; screenshots of Env/Mounts need a launch screen for a container sub-tab, if the ui-verifier asks.
2. A container stuck in `ContainerCreating` (e.g. FailedMount) gets no WHY box; a rule using Warning events and the pod's age could add it.
3. Copy kubectl command omits `--kubeconfig`; add it if users launch with `--kubeconfig` and paste into a shell without `KUBECONFIG`.
4. Human-readable quantities (`15.6Gi`): resolved in 0010.
