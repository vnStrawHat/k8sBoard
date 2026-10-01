# 0005 · Files to touch

[Back to index](README.md). Based on HEAD `7e36125`, with 0004 merged. **S** is the implementation step ([README](README.md)). Each step passes the gate on its own. An item lands in the step of its first user, so no dead code is ever added.

## Cargo

There are no Cargo changes:

- k8s-openapi `v1_32` already has the `apps`, `batch`, and `networking` types;
- `futures` (`StreamExt::map`, `BoxStream`, `select_all`) is already in both crates.

## `crates/cluster` (all in step 1)

| File | Change |
|---|---|
| `src/workload.rs` (new) | `ControllerRef`, `WorkloadCondition`, `TemplateContainer`, `ContainerPort`, and the shared `pub(crate)` helpers. Inline tests |
| `src/deployment.rs` (new) | `DeploymentSummary`, `deployment_summary`, `watch_deployments` |
| `src/stateful_set.rs` (new) | `StatefulSetSummary`, `ClaimTemplate`, `watch_stateful_sets` |
| `src/daemon_set.rs` (new) | `DaemonSetSummary` (no conditions), `watch_daemon_sets` |
| `src/replica_set.rs` (new) | `ReplicaSetSummary`, `watch_replica_sets` |
| `src/job.rs` (new) + `src/job_tests.rs` | `JobSummary`, `JobStatus`, `watch_jobs` |
| `src/cron_job.rs` (new) | `CronJobSummary`, `watch_cron_jobs` |
| `src/service.rs` (new) | `ServiceSummary`, `ServicePortSummary` (Display), `watch_services` |
| `src/ingress.rs` (new) | `IngressSummary`, `IngressPath`, `IngressTls`, `watch_ingresses` |
| `src/config_map.rs` (new) | `ConfigMapSummary`, `ConfigMapKey`, `watch_config_maps` |
| `src/pod.rs` (lines 31, 39–43, 178–188) | `PodController` removed; `controller: Option<ControllerRef>` via `workload::controller_ref`; `pods_api` replaced by `scoped_api` |
| `src/pod_tests.rs` (line 328) | `PodController` becomes `ControllerRef`; the controller test moves to `workload.rs` |
| `src/connection.rs` | `pub(crate) fn scoped_api<K>` |
| `src/access_review.rs` | `group` on `CheckTarget`, 10 variants, `ALL: [_; 19]`, updated tests |
| `src/lib.rs` | modules and exports ([cluster-network-config.md](cluster-network-config.md)); `PodController` export removed |
| `examples/probe.rs` | `--watch-seconds` covers 12 kinds via `select_all` |

New modules use inline `mod tests`. Move them to a sibling `*_tests.rs` (`#[path]`, as in 0001) once they pass about 150 lines.

## `crates/app`

| S | File | Change |
|---|---|---|
| 2 | `src/status_tone.rs` | `StatusLabel` gains `#[derive(Clone, Debug, PartialEq, Eq)]` |
| 2 | `src/resource_kind.rs` (new) | `ResourceKind { Namespaces, Deployments }`, `KindColumn`, `Align`, the per-kind tables, `watch_rows`, private `rows`. Inline tests |
| 3 | `src/resource_kind.rs` | the other 8 variants and their table entries |
| 2 | `src/kind_row.rs` (new) | `KindRow`, `KindCell` (no `Duration`), `DetailSection`, `DetailRow`, `PodOwner::Deployment`, `owns_pod`. Inline tests |
| 3 | `src/kind_row.rs` | `KindCell::Duration`, `PodOwner::Controller` |
| 2 | `src/namespace_rows.rs` (new) | `namespace_row`. Inline tests |
| 2 | `src/workload_rows.rs` (new) + `src/workload_rows_tests.rs` | `deployment_row`, `replica_tone` |
| 3 | `src/workload_rows.rs` | `stateful_set_row`, `daemon_set_row`, `replica_set_row`, `job_row`, `cron_job_row`, `LastRun`, `last_run`, `last_run_tone`, `ordinal` |
| 3 | `src/network_rows.rs` (new) | `service_row`, `ingress_row`. Inline tests |
| 3 | `src/config_map_rows.rs` (new) | `config_map_row`, `format_bytes`. Inline tests |
| 2 | `src/kind_table.rs` (new) | `KindTableDelegate` |
| 2 | `src/kind_drawer.rs` (new) | `kind_drawer`, the section, chip, port, and Pods renderers, `kind_menu_button` |
| 2 | `src/cluster_session.rs` | `explorer_kind`, `KindList`, `LiveCluster.explorer`, `set_explorer_kind`, the `set_scope` restart, the `new` parameter |
| 2 | `src/app_shell.rs` | `Screen::Kind`, `kind_table`, `show_screen`, `reveal_pod`, `sync_selection`, `navigation_counts`, `apply_pending_launch_screen` and `settle_input` arms |
| 2 | `src/workspace.rs` | `render_header`, banner, `render_list`, and `render_drawer` arms in `render_upper`; `count_label(n, singular, plural)` (line 227 and its callers at 98 and 107) |
| 2 | `src/navigation.rs` | `screen_of`, counts, `kind_availability`, the denied suffix; `sidebar` takes `Option<&LiveCluster>` |
| 2 | `src/table_selection.rs` (+ tests) | `ResourceKey::Kind`, `of_row`, `is_row` |
| 2 | `src/resource_actions.rs` | `kind_menu`, `YAML_DEFERRED_REASON` |
| 2 | `src/drawer.rs` | the `detail_row` label becomes `impl Into<SharedString>` |
| 2 | `src/launch_options.rs` (+ tests) | `LaunchScreen::Kind` and `KindDrawer`, parsing via `from_plural`, `USAGE` |
| 2 | `src/screenshot.rs` (+ tests) | `is_screen_settled` covers `KindDrawer` |
| 2 | `src/main.rs` | the new `mod` lines (step 3 adds `network_rows` and `config_map_rows`) |

All new app items are private or `pub(crate)`. The app still has no kube or k8s-openapi dependency (`BoxStream` comes from `futures`). `pod_drawer.rs` needs no change, because it never names `PodController`.

## Docs (in the step that changes the behavior)

| Step | Change |
|---|---|
| 1 | 0001 `probe-example.md`: the `--watch-seconds` row lists 12 kinds |
| 2 | 0003 `screenshot-hook.md`: add `<plural>` and `<plural>-drawer` to the `--screen` list |
