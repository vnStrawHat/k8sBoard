# 0039 · Files to touch

[Back to index](README.md). **S** is the step. Each step passes the gate on its own; every new item has a production user in its step. No `Cargo.toml` change (`k8s-openapi` already has `LimitRange`; `futures::stream::select` is in use). Baseline main `a50264c`.

## `crates/cluster`

| S | File | Change |
|---|---|---|
| 3 | `src/object_yaml.rs` (+ `object_yaml_tests.rs`) | `pod_template_yaml`, `pod_template_text` |
| 4 | `src/limit_range.rs` (new, tests in module) | `LimitRangeSummary`, `LimitRangeLimit`, `watch_limit_ranges`, `limit_range_summary` |
| 4 | `src/lib.rs` | `mod limit_range;` and the two `pub use` |
| 4 | `src/access_review.rs` (+ tests) | `AccessCheck::ListLimitRanges`, `ALL` + 1 |

## `crates/app`

| S | File | Change |
|---|---|---|
| 1 | `src/resource_kind.rs` | ResourceQuotas `read_only_actions: &[]` |
| 1 | `src/live_sections.rs` (+ `live_sections_tests.rs`) | `restart_hint`; the note in `used_by_rows` |
| 1 | `src/resource_actions.rs` (+ `resource_actions_tests.rs`) | `LogsMenu`, `LogsMenuState`, `LogChoice`; `PodMenuItems.view_logs`; `container_menu`, `container_shell_availability` |
| 1 | `src/pod_drawer.rs`, `src/pod_table.rs` | build `LogsMenu` before borrowing the session (both pod menus); `containers_tab` builds the container ⋯ button |
| 1 | `src/container_detail.rs` | `ContainerDetailInput.menu`; the button in the header |
| 2 | `src/log_target.rs` (+ tests) | `last_job_owner` |
| 2 | `src/resource_actions.rs` (+ tests) | `subject_action` ViewLogs for workload kinds; `key_availability` kind path; CronJob menu item; `workload_logs_item` hint |
| 2 | `src/app_shell.rs` | `selected_log_target` CronJob arm |
| 3 | `src/revision_diff.rs` (new, tests in module) | `RevisionSide`, `RevisionDiffRequest`, `diff_request`, `RevisionDiffView`, `DiffState` |
| 3 | `src/yaml_edit_panels.rs` | `diff_row_element` extracted, used by both views |
| 3 | `src/live_sections.rs` (+ tests) | `Diff` button in `revision_element` |
| 3 | `src/app_shell.rs`, `src/main.rs` | `open_revision_diff`; `mod revision_diff;` |
| 3 | `src/launch_options.rs` (+ tests), `src/screenshot.rs` (+ tests) | `--screen revision-diff` (fixture, no connection) |
| 4 | `src/related_objects.rs`, `src/cluster_session.rs` (+ tests) | `RelatedList::NamespaceLimits`, `RelatedUpdate::LimitRanges`, `namespace_limits`, `RelatedObjects::start(.., access, ..)` with the selected stream |
| 4 | `src/live_sections.rs` (+ tests) | LimitRange rows in `namespace_quota_rows`, `limit_range_text` |

## Docs

| S | File | Change |
|---|---|---|
| 4 | `docs/roadmap/wireframe-gap-audit.md` | rows W4 n2, W4b n3, W7 Deployments (diff), CronJobs, ConfigMaps (hint), ResourceQuotas, Namespaces (LimitRange) → Done (0039) |
| 4 | `docs/roadmap/inventory-kinds.md`, `inventory-screens.md` | the same rows |
| 2 | `docs/specs/0028-keyboard-map/row-actions.md` | L (`ViewLogs`) also on workload kinds and CronJobs |
