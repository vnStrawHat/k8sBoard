# 0058 · Files to touch

[Back to index](README.md). One step, one commit; base main `058218d`. All paths under `crates/app/src/` unless stated.

## Moved (`git mv`, then edited)

| From | To | Edit |
|---|---|---|
| `metrics_page.rs` (764 lines) | `cluster_metrics_section.rs` | [section.md](section.md): rename types, `catalog` field, `show_cluster`, `Reach`, `connect`, `follow_session`, header and note texts, `#[path]` of its tests |
| `metrics_page_tests.rs` (527 lines) | `cluster_metrics_section_tests.rs` | [test-plan.md](test-plan.md): `open_section` builds the section and calls `show_cluster`; tests renamed and added |

## Changed

| File | Change |
|---|---|
| `clusters_page.rs` | `metrics` field, `metrics_section()`, `select` `pub(crate)`, retarget in `sync_form` / `render`; delete the form's Metrics row and its five items ([wiring.md](wiring.md)) |
| `clusters_page_tests.rs` | delete the `// ---- The Metrics section ----` block (`metrics_fields`, `source_button_names_metrics_server_or_the_saved_service`, `form_dropdown_clears_the_source`); add the page tests of [test-plan.md](test-plan.md) |
| `settings_window.rs` | delete `SettingsPage::Metrics` and everything named in [wiring.md](wiring.md); add `CLUSTER_METRICS_GROUP`, `first_group`, `show_cluster_metrics`, `select_cluster_metrics`, `is_cluster_metrics_pending`, the second group |
| `settings_window_tests.rs` | `pages_follow_w2_order`, `default_page_is_clusters`; replace the Metrics page test ([test-plan.md](test-plan.md)) |
| `launch_options.rs`, `launch_options_tests.rs` | screen variant, parse, `USAGE`, `settings_screen`, `opens_cluster_metrics`; test rename |
| `main.rs` | `mod cluster_metrics_section;`, fixture global, `show_cluster_metrics` call |
| `screenshot.rs` | `is_cluster_metrics_pending` field and its tests |
| `app_shell.rs` | settle input field name and call (line ~4358) |
| `monitor_tab.rs`, `topology_view.rs`, `cluster_metrics.rs`, `active_session.rs` | texts and doc comment ([wiring.md](wiring.md)) |
| `topology_view_tests.rs`, `cluster_metrics_tests.rs` | expected texts `Settings › Clusters › Metrics` |
| `crates/cluster/src/service.rs` | doc comment only |

## Deleted

`metrics_page.rs` and `metrics_page_tests.rs` (by the move); `SettingsPage::Metrics`, its title and icon; `SettingsWindow.metrics`; `fn metrics_page`; `show_metrics_page`; `is_metrics_page_pending`; `MetricsFixture`; `LaunchScreen::{SettingsMetrics, SettingsMetricsFixture}`; in `clusters_page.rs` `METRICS_HINT`, `METRICS_SERVER_ONLY`, `set_metrics_source`, `metrics_source_label`, `metrics_menu`; the tests named in [test-plan.md](test-plan.md).

## Not touched

`settings.rs`, `settings_store.rs`, `cluster_registry.rs` (shape, allow-list, reset), `cluster_session.rs`, `app_shell_monitor_source.rs`, `monitor_source.rs`, every file of `crates/cluster` except the doc comment, `object_write.rs`, `clippy.toml`, `Cargo.toml`, `Cargo.lock`.

## Doc follow-ups (same commit)

| File | Change |
|---|---|
| `docs/specs/0048-prometheus-metrics/settings-page.md` | one line under the title: `Superseded by [0058](../0058-cluster-metrics-settings/README.md): the page is now the Metrics section of Settings › Clusters.` |
| `docs/specs/0043-settings-pages/pages.md` | drop `Metrics` from the list of non-resettable pages; the nav has no Metrics page (0058) |
| `docs/specs/0058-cluster-metrics-settings/README.md` | tick the ACs; add `as-built.md` if anything differs |
