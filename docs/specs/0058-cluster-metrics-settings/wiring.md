# 0058 · Wiring: Clusters page, Settings window, helper, screens, texts

[Back to index](README.md)

## Clusters page (`clusters_page.rs`)

| Change | Detail |
|---|---|
| New field | `metrics: Entity<ClusterMetricsSection>`, created in `ClustersPage::new` with `catalog.clone()` |
| New accessor | `pub(crate) fn metrics_section(&self) -> Entity<ClusterMetricsSection>` (for the kit group) |
| Retarget | end of `sync_form` (runs only when a new form is built: selection change, `forget_form_and_test`): `self.metrics.update(cx, \|section, cx\| section.show_cluster(Some(row), window, cx))`. In `render`, when `selected_row` is `None` and `self.metrics.read(cx).cluster().is_some()`: `show_cluster(None, …)` |
| Visibility | `select` becomes `pub(crate)` (the helper selects the open cluster) |
| Form | delete the `metrics_row` and `.child(section("Metrics", [metrics_row], cx))`; the form ends with Safety, then Reset / Remove as today |
| Deleted items | `METRICS_HINT`, `METRICS_SERVER_ONLY`, `set_metrics_source`, `metrics_source_label`, `metrics_menu`; imports that become unused (`show_metrics_page`, `MetricsSourceFields`, `StoredMetrics`, `ActiveConnection`, `disabled_menu_item` if no other user: the compiler decides) |

## Settings window (`settings_window.rs`)

```rust
/// The kit group of the Clusters page that holds the Metrics section (decision 5).
const CLUSTER_METRICS_GROUP: usize = 1;
pub(crate) struct SettingsWindow { first_page: SettingsPage, first_group: Option<usize>, /* … */ }
/// Settings › Clusters with the open cluster selected and its Metrics section scrolled into view;
/// opens the window or brings it forward. `None` when the window could not open.
pub(crate) fn show_cluster_metrics(cx: &mut App) -> Option<AnyWindowHandle>;
impl SettingsWindow {
    /// first_page = Clusters, first_group = Some(CLUSTER_METRICS_GROUP), page_generation += 1,
    /// then `clusters.select(cluster)` when `cluster` is `Some`.
    fn select_cluster_metrics(&mut self, cluster: Option<ClusterRef>, cx: &mut Context<Self>);
}
#[cfg(feature = "screenshot")]
pub(crate) fn is_cluster_metrics_pending(cx: &App) -> bool; // first_group is the Metrics group and !section.is_settled()
```

- `show_cluster_metrics`: `cluster = cx.try_global::<ActiveConnection>().map(|a| a.cluster.clone())`; `let window = open_settings_window(SettingsPage::Clusters, SettingsSize::Standard, cx)?;` then update the view (the `add_cluster` pattern) with `select_cluster_metrics(cluster, cx)`; return `Some(window)`.
- `show_page` sets `first_group = None` (every other way in keeps today's top-of-page behaviour).
- `Render`: `default_selected_index(SelectIndex { page_ix: self.first_page.index(), group_ix: self.first_group })`.
- `clusters_page(page, cx)`: after the existing group, `.group(SettingGroup::new().item(SettingItem::render(move |_, _, _| section.clone())))` with `section = page.read(cx).metrics_section()`. Always present (decision 4), so index 1 exists on the first frame.
- Deleted: `SettingsPage::Metrics` (variant, `PAGES` entry, title `Metrics`, icon `ChartLine`), `PAGES: [SettingsPage; 9]`, `metrics: Entity<MetricsPage>` and its construction, `fn metrics_page`, `show_metrics_page`, `is_metrics_page_pending`, `use crate::metrics_page::MetricsPage`.

## Launch screen (`launch_options.rs`, `main.rs`, `screenshot.rs`, `app_shell.rs`)

| Item | Change |
|---|---|
| `LaunchScreen` | delete `SettingsMetrics`, `SettingsMetricsFixture`; add `SettingsClusterMetricsFixture` parsed from `settings-cluster-metrics-fixture`; `USAGE` text updated (drop `settings-metrics|settings-metrics-fixture`, add `settings-cluster-metrics-fixture`) |
| `screen()` | `SettingsClusterMetricsFixture` → `Screen::Overview` (as the other settings screens) |
| `settings_screen()` | `SettingsClusterMetricsFixture` → `Some((SettingsPage::Clusters, SettingsSize::Standard))` |
| new | `pub(crate) fn opens_cluster_metrics(self) -> bool` — true for `SettingsClusterMetricsFixture` only |
| `main.rs` | set `cluster_metrics_section::ClusterMetricsFixture` for the new screen (replaces `MetricsFixture`); after the settings window opens: `if options.screen.opens_cluster_metrics() { settings_window::show_cluster_metrics(cx); }` (the returned handle is the same window) |
| `screenshot.rs` | `SettleInput.is_metrics_page_pending` → `is_cluster_metrics_pending`; the settings branch of `is_screen_settled` reads it |
| `app_shell.rs` | the settle input calls `settings_window::is_cluster_metrics_pending(cx)` |
| `mod` lines | `main.rs`: `mod metrics_page;` → `mod cluster_metrics_section;` |

The fixture screen needs a kubeconfig with at least one context (any; ui-verifier uses the UAT file with `--context readonly@Monitor`, as for `settings`); the section draws the fixed data for the selected row.

## Texts that point at the old page

| File | Old → new |
|---|---|
| `monitor_tab.rs` `SHORT_HISTORY_TIP` | `… in Settings › Metrics …` → `… in Settings › Clusters › Metrics …` |
| `topology_view.rs` `traffic_button` | `Choose a metrics source in Settings › Metrics` → `… in Settings › Clusters › Metrics` |
| `cluster_metrics.rs` `source_note` | both `(Settings › Metrics)` → `(Settings › Clusters › Metrics)` |
| `active_session.rs` doc of `ActiveConnection` | "the Metrics page reads it and never opens a connection of its own" → "the Metrics section of the Clusters page reads it for the open cluster" |
| `crates/cluster/src/service.rs:71` doc | "Settings › Metrics page" → "Metrics section of Settings › Clusters" |
