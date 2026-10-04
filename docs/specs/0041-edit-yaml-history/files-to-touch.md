# 0041 · Files to touch

[Back to index](README.md)

## Step 1 (cluster crate only)

| File | Change |
|---|---|
| `crates/cluster/src/replica_set.rs` | `ClusterConnection::deployment_revisions` (one LIST, owner filter) |
| `crates/cluster/src/deployment.rs` | `FieldWriter`, `DeploymentSummary.template_change`, the `managedFields` reader in `deployment_summary` |
| `crates/cluster/src/deployment_tests.rs` | managedFields fixtures |
| `crates/cluster/src/quota_demand.rs` (new) + `quota_demand_tests.rs` (new) | `WorkloadDemand`, `DemandChange`, `QuotaResource`, `QuotaCheck`, `QuotaShortfall`, `workload_demand`, `quota_check` |
| `crates/cluster/src/edit_preview.rs` | `EditPreview.demand`, set in `build_preview` |
| `crates/cluster/src/edit_preview_tests.rs` | demand set / not set |
| `crates/cluster/src/connection_tests.rs` or a new `replica_set_tests.rs` | `deployment_revisions` over `FakeApi` |
| `crates/cluster/src/lib.rs` | module and exports (`FieldWriter`, `WorkloadDemand`, `DemandChange`, `QuotaResource`, `QuotaCheck`, `QuotaShortfall`, `quota_check`) |

## Step 2 (app: History tab)

| File | Change |
|---|---|
| `crates/app/src/revision_history.rs` (new) + `revision_history_tests.rs` (new) | `RevisionHistory` entity, `HistoryState`, list and selection |
| `crates/app/src/revision_diff.rs` | `revision_list`, `latest_pair`; root sized `size_full` (height moves to the dialog wrapper) |
| `crates/app/src/revision_diff_tests.rs` | the two helpers |
| `crates/app/src/yaml_edit.rs` | `EditTab::History`, `history` field, `show_tab` |
| `crates/app/src/yaml_edit_panels.rs` | third tab for Deployments, History body |
| `crates/app/src/app_shell.rs` | `open_revision_diff` puts the height share on the dialog wrapper |
| `crates/app/src/launch_options.rs`, `screenshot.rs` | `--screen edit-yaml-history` |

## Step 3 (app: quota line)

| File | Change |
|---|---|
| `crates/app/src/issue_feeds.rs` | `IssueFeeds::condition`, `ConditionFeed::off_reason` (read-only twins) |
| `crates/app/src/yaml_edit.rs` | `PassedPreview.quota`, computed when a preview passes; warnings to `WriteIntent.warnings` |
| `crates/app/src/yaml_edit_panels.rs` | Checks line and tone |
| `crates/app/src/yaml_edit_tests.rs` | quota line texts per state |
| `crates/app/src/screenshot.rs` | `edit-yaml-diff` fixture gets the `Fits` line |

## Step 4 (app: timeline)

| File | Change |
|---|---|
| `crates/app/src/recent_changes.rs` + `recent_changes_tests.rs` | `ChangeInputs.deployments`, `ActorSource`, the window rule |
| `crates/app/src/overview.rs` | pass the Deployments feed; row click per kind; tooltip suffix |
| `crates/app/src/overview_report.rs` | unchanged texts; passes `deployments: None` or the feed (report shows the actor it gets) |
| `crates/app/src/app_shell.rs` | `open_latest_revision_diff`, `revision_lookup` task field |
| `crates/app/src/revision_diff.rs` | `go_to: Option<ResourceKey>`, `Go to deployment` button |
| `crates/app/src/app_shell_tests.rs` (or a new `app_shell_revision_tests.rs`) | click flow over a fake cluster |

## Docs (coder, when built)

- This README: status, AC ticks, as-built notes.
- `docs/specs/0021-overview/README.md` open item 1: "who" and click-to-diff for Deployments done by 0041; ConfigMap rows dropped (user, 2026-10-03).
- `docs/specs/0031-edit-yaml/README.md` open item 3: Revision history and quota check done by 0041; snapshots dropped.
- `docs/roadmap/wireframe-gap-audit.md`: rows W10, W10 n2, W10 n5, W3 n4, W7 ConfigMaps Compare.

## Do not touch

`object_write.rs`, `clippy.toml`, `access_review.rs` (no new check), `docs/specs/0040-*`.
