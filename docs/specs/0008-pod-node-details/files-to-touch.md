# 0008 · Files to touch

[Back to index](README.md). Based on 0006 and 0007 merged. **S** is the step; each step passes the gate on its own, and nothing lands before its first user.

## Cargo

No changes. No new `[[package]]` in `Cargo.lock` in any step (`git diff Cargo.lock` is empty). The kit's `Alert` and `TabBar::segmented` already exist in gpui-component 0.7.

## `crates/cluster` (step 1)

| File | Change |
|---|---|
| `src/container_spec.rs` (new) + `src/container_spec_tests.rs` | types and mapping fns of [cluster-pod-fields.md](cluster-pod-fields.md) |
| `src/pod.rs` (+ `pod_tests.rs`) | `status_message`; `PodCondition.reason`/`message`; `Waiting.message`; `ContainerSummary` additions; volumes passed to `container_summary` |
| `src/workload.rs` (+ tests) | `container_ports` extracted; `template_containers` uses it |
| `src/event.rs` (+ `event_tests.rs`) | `EventSummary.container`, `field_path_container`; `truncate_message` → `pub(crate)` |
| `src/node.rs` (+ `node_tests.rs`) | the [cluster-node-fields.md](cluster-node-fields.md) fields and types |
| `src/lib.rs` | `mod container_spec;`; exports `ContainerResource`, `ContainerProbes`, `ProbeSummary`, `ProbeAction`, `EnvEntry`, `EnvSource`, `EnvFromEntry`, `EnvFromSource`, `MountEntry`, `VolumeSource`, `ConditionStatus`, `NodeAddress`, `NodeCondition`, `NodeResource`, `NodeSystemInfo` |
| `src/pod_status.rs` (+ `pod_status_tests.rs`) | `StatusReason::{InvalidImageName, ErrImageNeverPull, CreateContainerError}` in the enum, `from_api`, `Display` |
| app compile fixes (step 1) | `ContainerState::Waiting { reason, .. }` in `status_tone.rs`; `is_bad_reason` gains the three variants; test fixtures that build summaries get the new fields |

## `crates/app`

| S | File | Change |
|---|---|---|
| 2 | `src/pod_diagnosis.rs` (new) + `pod_diagnosis_tests.rs` | [pod-diagnosis.md](pod-diagnosis.md) |
| 2 | `src/status_tone.rs` | `is_bad_reason` → `pub(crate)` |
| 2 | `src/container_detail.rs` (new) + `container_detail_tests.rs` | container detail header, sub-tab bar, Info/Env/Mounts bodies, text builders, `SourceRow`, `env_rows`, `mount_rows`, `resource_label` |
| 2 | `src/pod_drawer.rs` (+ tests) | WHY box, Overview links and condition tooltips; `container_detail` moves out; Containers tab calls `container_detail` |
| 2 | `src/drawer.rs` | `ContainerTab`, `DrawerState.container_tab`; receives `link_text`, `port_row`, `chips` |
| 2 | `src/kind_drawer.rs` | uses the moved `link_text`, `port_row`, `chips` |
| 2 | `src/table_selection.rs` (+ tests) | `ResourceKey::of_object` |
| 2 | `src/event_rows.rs` | `object_key` delegates to `of_object` |
| 2 | `src/app_shell.rs` | `set_container_tab`; `show_screen` resets `container_tab` |
| 2 | `src/resource_actions.rs` (+ tests) | `pod_menu(.., context, ..)`, Copy kubectl command, `kubectl_describe_command` |
| 2 | `src/pod_table.rs` | passes the context to `pod_menu` |
| 2 | `src/main.rs` | `mod pod_diagnosis; mod container_detail;` |
| 3 | `src/related_pods.rs` (new) | moved `pods_section`, `related_pod_row`, `PodRowDetail` (+ `NamespaceAndStatus`), `MAX_RELATED_PODS`; scope note |
| 3 | `src/kind_drawer.rs` | imports `pods_section` |
| 3 | `src/kind_row.rs` (+ tests) | `PodOwner::Node`, `owns_pod` arm |
| 3 | `src/node_drawer.rs` | Overview sections of [node-drawer.md](node-drawer.md) |
| 3 | `src/status_tone.rs` (+ tests) | `node_condition_tone`, `condition_status_text` |
| 3 | `src/main.rs` | `mod related_pods;` |

All new app items are private or `pub(crate)`. The app gains no kube, k8s-openapi, or serde dependency.

## Docs (in the step that changes the behavior)

| S | Change |
|---|---|
| 1 | 0003 `cluster-additions.md`: one line pointing to 0008 for the new fields |
| 2 | 0003 `drawer.md`: Overview links, WHY box, container sub-tabs; 0003 `actions.md`: Copy kubectl command |
| 3 | 0003 `drawer.md`: node Overview sections |
| 3 | `docs/roadmap/`: flip W4-3 (Copy kubectl command), W4-5, W4-6, W4-8, W4-9 (Info/Env/Mounts), W4-10 (disabled), W4-11 (spec values), W4-12, W4-13, W5-4 (spec data) to Done/Partial; status table row for Pods and Nodes |
