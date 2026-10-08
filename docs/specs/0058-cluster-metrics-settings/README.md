# 0058 — Metrics source settings per cluster (Settings › Clusters › Metrics)

Status: **draft 2026-10-08** against main `058218d`. User request: "Move Settings → Metrics into the per-cluster configuration inside Settings → Clusters." Crate: `crates/app` only. **Read-only:** the same GETs as 0048 (`list services`, `query`), plus a one-time client for a cluster that is not open, opened the way Test connection already does. No new Kubernetes call, no new setting key, no write path. Builds on 0043 (Clusters page) and 0048 (Metrics page).

## Goal

- The Settings sidebar loses its **Metrics** page. Its content (radio list of detected services, `metrics-server only`, `Other service` fields, Detect, Test, Save, saved line) becomes the **Metrics section of the selected cluster** on the Clusters page.
- Works for every cluster in the list, not only the open one: the open cluster detects on its own as today; another cluster detects and tests on click, through a one-time connection.
- One helper opens Settings › Clusters on the open cluster with the Metrics section scrolled into view.

## Non-goals

A direct URL or credential (0048 decision 3); any change to `registry.clusters[].metrics`, `SourceState`, the Monitor or Topology readers; clickable links in the Monitor range tooltip or the Topology Traffic tooltip (they are tooltips on disabled controls; only their text changes); a live `--screen` for this section (see [decisions.md](decisions.md) 7); a kit change for the sidebar highlight (open item 1).

## Files

| File | Contents |
|---|---|
| [decisions.md](decisions.md) | numbered decisions with rationale (layout, reach of non-open clusters, scroll, entry points, settings shape) |
| [section.md](section.md) | `ClusterMetricsSection`: what moves from `MetricsPage`, state, reach, behaviour table, texts |
| [wiring.md](wiring.md) | Clusters page, Settings window, `show_cluster_metrics`, launch screen, text pointers |
| [files-to-touch.md](files-to-touch.md) | per file: new, moved, changed, deleted; doc follow-ups |
| [test-plan.md](test-plan.md) | tests moved, renamed, new, deleted; ui-verifier and live checks |

## Acceptance criteria

- [ ] 1. Gate passes, plus `cargo clippy -p k8sboard --features screenshot --all-targets -- -D warnings`; no new `#[allow]`, no `unsafe`, no new dependency; `crates/cluster` unchanged except one doc comment (`service.rs`).
- [ ] 2. Every test in [test-plan.md](test-plan.md) exists under its name and passes offline; no test reaches a cluster (fake API, or `https://127.0.0.1:1`).
- [ ] 3. The Settings sidebar reads General, Clusters, Environments, Appearance, Keyboard Shortcuts, Safety, Terminal & Shell, Logs, About (9 pages). `SettingsPage::Metrics`, `metrics_page.rs`, `show_metrics_page`, and `settings-metrics[-fixture]` are gone (grep clean).
- [ ] 4. The Clusters page has two kit groups: list + form (group 0) and the Metrics section of the selected cluster (group 1, full page width). The form no longer has a `Metrics · Source ▾` row.
- [ ] 5. For the open cluster the section detects at its first draw, preselects the saved source, tests and saves exactly as the 0048 page did, and shows `Saved: … · {state}` from the session.
- [ ] 6. For a cluster that is not open the section shows its saved source and the note of [section.md](section.md); nothing is sent until Detect or Test is clicked; then one client is opened with that cluster's kubeconfig and stored proxy (`open_cluster`, 15 s open deadline) and only `list services` or the check `query` GET is sent.
- [ ] 7. Selecting another row, Reset to defaults, or removing the row resets the section (inputs from the new saved entry, running detection and test dropped).
- [ ] 8. Save writes `registry.clusters[<selected>].metrics` only (shape unchanged); `metrics-server only` clears it; the open session re-checks when the saved cluster is the open one (0048 rule, unchanged).
- [ ] 9. `show_cluster_metrics` opens or brings forward Settings on Clusters, selects the open cluster when there is one, and scrolls to group 1. `--screen settings-cluster-metrics-fixture` uses it and draws fixed data with no request.
- [ ] 10. Tooltips and notes that said `Settings › Metrics` say `Settings › Clusters › Metrics` (Monitor range tip, Topology Traffic tip, `source_note`).
- [ ] 11. Theme tokens only (0003 color-literal grep clean); English only.
- [ ] 12. ui-verifier (light, dark): `settings-cluster-metrics-fixture` and `settings` show no high-severity defect against W2; the live UAT check of [test-plan.md](test-plan.md) lists the two VictoriaMetrics rows for `readonly@Monitor`.

## Open items

1. Kit limitation: with `group_ix: Some(1)` on a page of two untitled groups, the kit sidebar does not highlight the Clusters item (`is_page_active` needs `group_ix` `None` or one group) until the user picks a page. Accepted as low severity; the fix is a kit change (treat an untitled group as its page). See [decisions.md](decisions.md) 5.
