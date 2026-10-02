# 0005 — Kind Explorer, batch 1 (read-only)

Status: amended after advisor review. Crates: `crates/cluster` and `crates/app`. Based on HEAD `7e36125` (0004 merged). Wireframes: W7, anatomy, tokens.

## Goal

Make ten more sidebar items live: **Namespaces, Deployments, StatefulSets, DaemonSets, ReplicaSets, Jobs, CronJobs, Services, Ingresses, ConfigMaps**. Each kind gets:

- its own screen, with a live table whose columns follow `kubectl get`;
- an overlay drawer with an Overview only;
- the same actions in the row menu and the ⋯ menu, with mutating actions disabled as "Read-only mode".

When RBAC denies `list`, the sidebar item is disabled and shows the reason.

## Batch rationale

These kinds answer the daily DevOps questions: is the rollout healthy, what ran and failed, what a service or ingress exposes, and which config it uses. All of them are plain watches in core, `apps`, `batch`, or `networking.k8s.io`.

Deferred: **Secrets** (the watch carries values; waits for secret rules), **Events** (needs the 0002 store cap), HPAs, PDBs, NetworkPolicies, ResourceQuotas (need pod labels or metrics), storage, RBAC, and ServiceAccounts (same pattern, later), Helm, CRDs, and Port Forwarding (own designs).

## Non-goals

Monitor, YAML, and Events tabs; workload logs, Topology, Open URL, palette, filter chips, sort, column toggles, list-level buttons; executing any mutation or port-forward (shown disabled only); Service endpoints, ConfigMap "Used by", CronJob next run, ingress cert expiry, and the "WHY" box.

## Implementation steps

Each step is one coder pass and must pass the full quality gate on its own. Nothing is added before its first user (dead-code rule).

| Step | Scope | ACs |
|---|---|---|
| 1 | All of `crates/cluster`: `workload.rs`, `PodController` → `ControllerRef`, `scoped_api`, the 9 summaries and watches, the 10 access checks, the probe | 1, 2, 3, 4, 5 |
| 2 | App infrastructure plus **Namespaces and Deployments**. Covers the generic table and drawer, navigation gating, the single-watch lifecycle, related pods, ports with Forward, and screenshot wiring | 1, 2, 3, 6, 7, 9 |
| 3 | The remaining 8 row builders, `ResourceKind` variants, and screens, plus the full ui-verifier run | 1, 2, 6, 8, 9 |

## Files

| File | Contents |
|---|---|
| [decisions.md](decisions.md) | numbered decisions with a rationale for each, and known ceilings |
| [cluster-workloads.md](cluster-workloads.md) | shared helpers; Deployment, StatefulSet, DaemonSet, ReplicaSet, Job, and CronJob |
| [cluster-network-config.md](cluster-network-config.md) | Service, Ingress, ConfigMap; access checks; `lib.rs`; probe |
| [explorer-model.md](explorer-model.md) | `ResourceKind`, `KindRow`, lazy watch lifecycle |
| [kind-columns.md](kind-columns.md) | columns, cells, status tones per kind |
| [kind-drawers.md](kind-drawers.md) | drawer sections, related pods, Forward buttons |
| [navigation-and-actions.md](navigation-and-actions.md) | sidebar gating, screens, selection, menus, screenshots |
| [files-to-touch.md](files-to-touch.md) | modules per crate and step |
| [test-plan.md](test-plan.md) | unit tests per step, live checks, ui-verifier checklist |

## Acceptance criteria

- [ ] 1. The quality gate passes, and so does `cargo clippy -p k8sboard --features screenshot --all-targets -- -D warnings`. No new `#[allow]`.
- [ ] 2. Every test of the step in [test-plan.md](test-plan.md) exists under that name and passes, offline.
- [ ] 3. No kube or k8s-openapi type in a public signature. The 0001 read-only grep finds only the SSAR `create`. The crate never spawns tasks. The app has no kube dependency.
- [ ] 4. `ConfigMapSummary` and `TemplateContainer` retain no value text. Verified by type review plus `config_map_summary_drops_values` and `template_containers_keep_only_name_image_and_ports`.
- [ ] 5. The probe `--watch-seconds 5` prints 12 watch lines and 19 access lines. coder-lite records UAT allow/deny per kind. The AC7 credential script reports 0.
- [ ] 6. On UAT, each allowed kind of the step shows live rows and a drawer. Denied kinds are disabled with "Not permitted: list …". Forward buttons are disabled with the port-forward reason.
- [ ] 7. At most one explorer watch runs. Leaving a kind screen drops it (review of `set_explorer_kind`).
- [ ] 8. All 0005 screenshots exist, and the ui-verifier reports no high-severity defect against W7.
- [ ] 9. The 0003 AC4 color-literal grep is clean.

## Open items

1. Related pods for Services, PDBs, and NetworkPolicies need `labels` on `PodSummary`. Done in 0012 (`PodSummary.labels`, Services); PDBs and NetworkPolicies wait for 0013.
2. The CronJob next run needs a cron crate. Done in 0012: an own port of the controller grammar, no crate.
3. Service Endpoints need an EndpointSlice watch. Done in 0012.
4. ConfigMap "Used by" needs pod-spec references. Done in 0012.
5. Hiding inactive ReplicaSets waits for filter chips.
6. If re-listing on a kind switch feels slow, keep the last watch alive as a cache.
