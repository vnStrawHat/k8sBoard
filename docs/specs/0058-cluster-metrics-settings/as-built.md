# 0058 · As built

[Back to index](README.md)

Differences from the spec, found while implementing against gpui-component 0.7.0 (via gpui-kit 0.7.0).

1. **The kit does not scroll to a default group.** `SettingPage::render` scrolls its list to `selected_index.group_ix` only when `deferred_scroll_group_ix` is set (a click on a titled group's sidebar item) or when the page's query or visible-group list changed. The state of the page list is keyed under the `Settings` id, so a `page_generation` bump creates a fresh list at the top and neither condition holds. Result: `first_group` and `CLUSTER_METRICS_GROUP` are passed to the kit as the spec says, but the Metrics section is not scrolled into view; it sits below the form and the user scrolls to it. Headless proof: with a 1000x620 window, `metrics-detect` is not drawn after `select_cluster_metrics`. Options (not done): a kit change that honours the default group on first render; or a titled group (adds a `Clusters > Metrics` sidebar item whose click scrolls, which decision 5 rejected).
2. **Tests that need the section drawn use a tall window.** The kit list draws only what is in view, so `cluster_metrics_redirect_detects_only_when_shown` resizes the window to 2400 px before `show_cluster_metrics`.
3. **`set_value` raises no change event**, so tests that fill `Other service` call `on_other_changed` (what typing does).
4. **`launch_options_tests.rs` still names `settings-metrics`** once, to assert it no longer parses (test-plan); the grep of AC 3 matches that one line.
