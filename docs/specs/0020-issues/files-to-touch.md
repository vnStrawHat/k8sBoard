# 0020 · Files to touch

[Back to index](README.md). **S** is the step. Each step passes the gate on its own; every new item has a production user in its step (1a: sidebar and title bar read the board, and `refresh` compares `Vec<Issue>` with `PartialEq`, which reads every field; 1b: the screen). If clippy still reports an unread item in 1a, move it to 1b, never `#[allow]`. No `Cargo.toml` or `Cargo.lock` change.

## `crates/cluster` (step 1a)

| File | Change |
|---|---|
| `src/pod.rs` (+ `pod_tests.rs`) | `PodCondition.changed_at: Option<jiff::Timestamp>` from `lastTransitionTime` (same doc as `NodeCondition.changed_at`) |
| `src/event.rs` (+ `event_tests.rs`) | `truncate_message` (the source of every event, pod, node, and workload message) hides URL userinfo before the cut, as 0018 does for custom objects. That is the only masking: an issue cause quotes the cleaned message otherwise as it is; tests `message_hides_url_userinfo`, `node_condition_message_hides_url_userinfo`, `pod_condition_and_waiting_messages_hide_url_userinfo` |
| `src/container_spec.rs` (+ tests) | `ProbeSummary.initial_delay_seconds` (`initialDelaySeconds`, default 0): the startup grace adds it |

## `crates/app`

| S | File | Change |
|---|---|---|
| 1a | `src/pod_diagnosis.rs` (+ tests) | `DiagnosisCause`, `PodDiagnosis.cause`; fixtures gain `changed_at` |
| 1a | `src/kind_row.rs` | `pub(crate) fn pod_workload(namespace: &str, controller: Option<&ControllerRef>) -> Option<IssueObject>` next to `is_deployment_replica_set` (shared hash alphabet), tests in module |
| 1a | `src/issue.rs` (new) | model types ([engine.md](engine.md)) |
| 1a | `src/issue_rules.rs` (new) + `issue_rules_tests.rs` | pod, node, volume, event rules; constants |
| 1a | `src/issue_board.rs` (new) + `issue_board_tests.rs` | `IssueInputs`, `evaluate`, `Grouped`, grouping, dedupe, grace, `IssueBoard`, `IssueSummary`, `RunReason` |
| 1a | `src/issue_feeds.rs` (new) + `issue_feeds_tests.rs` | `IssueFeed`, `FeedState`, `Coverage`, `WarningEvents`, `IssueFeeds` (events), core coverage |
| 1a | `src/cluster_runtime.rs` | `subscribe_silent`, private `UpdateNotice` helper |
| 1a | `src/cluster_session.rs` (+ tests) | `ClusterSession.{issues, is_issues_visible, _issue_tick}`, `issues()`, `set_issues_visible`, `refresh_issues`; `LiveCluster.issue_feeds`; `mark_dirty` from core list and metrics callbacks |
| 1a | `src/metrics_history.rs`, `src/kubelet_history.rs` (+ tests) | `latest_container_pair`, `pvc_usages` |
| 1a | `src/navigation.rs` (+ tests) | `NavigationCounts.{issue_total, issue_counts}`, toned suffixes, Issues total on the disabled item |
| 1a | `src/title_bar.rs` | `issues_button` (no click yet) |
| 1a | `src/main.rs` | `mod issue; mod issue_board; mod issue_feeds; mod issue_rules;` |
| 1b | `src/issue_table.rs` (new) + tests in module | table delegate, `TableRow for Issue`, menu |
| 1b | `src/app_shell.rs` | `Screen::Issues`, `issue_table` entity, row click → `reveal`, `show_screen` arm and `set_issues_visible` |
| 1b | `src/workspace.rs`, `src/table_view.rs` | Issues header, coverage text, state views; `default_filter` arm |
| 1b | `src/navigation.rs`, `src/title_bar.rs` | Issues item enabled; button click → `show_screen(Issues)` |
| 1b | `src/launch_options.rs` (+ tests), `src/screenshot.rs` | `--screen issues`, settle rule |
| 1b | `src/main.rs` | `mod issue_table;` |
| 2 | `src/issue_kind_rules.rs` (new) + tests in module | `object_finding`, NamespaceStuck, QuotaNearLimit, PvcPending, certificate rules |
| 2 | `src/issue_feeds.rs` (+ tests) | `ConditionFeed`, `CONDITION_KINDS`, `condition_plan`, All-scope filtering, Forbidden → Off, `restart_conditions`, coverage labels |
| 2 | `src/issue_board.rs` | `IssueInputs.objects` and `is_job_feed_live` wired into `evaluate` |
| 2 | `src/cluster_session.rs` (+ tests) | condition feeds on access review and scope change; `OpenWatches.issue_feeds`; `open_watch_count` |
| 2 | `src/main.rs` | `mod issue_kind_rules;` |

Step 1a starts on committed code plus 0011 (`KubeletHistory`). Step 2 needs 0012 (`kind_diagnosis`, `KindObject`, `OpenWatches`), 0013 (HPA/PDB/quota summaries and boxes, `quota_tone`), 0014 (PVC summaries, VOLUME LOST), 0016 (`watch_tls_secrets`, `certificate_expiry.rs`, `KindObject::Secret`), 0018 step 5 (`deleting_since`, STUCK).

## Docs (step 2, by the coder)

- `docs/roadmap/inventory-screens.md`: I1 → Done (0020); O2 → Partial (engine 0020, panel 0021).
- `docs/roadmap/inventory-shell.md`: T8 → Done; N4 → Done; N5 → Issues Done.
- `docs/roadmap/cross-cutting.md`: C13 → settled by 0020 decisions 1–11 with the measured numbers.
- `docs/roadmap/README.md`: 0020 status.
- [decisions.md](decisions.md) "Budget measurements" filled by coder-lite.

## As built (steps 1b and 2)

- `Issue.subject` is the object the rule fired on (the representative pod of a group); View logs reads it, because `shown` is the Deployment for a group.
- `resource_actions::view_logs_item` is `pub(crate)` and takes the container hint; `quota_ratio`, `quota_tone`, and `scope_multiplicity` are `pub(crate)` for reuse.
- Issues has no checkbox: a plain mouse click on a row reveals (`AppShell::reveal_issue`); the arrow keys only move the highlight, so they do not jump away. There is no Enter binding (the toolkit has none).
- `AppShell::reveal` selects after the tables' `ClearSelection` events, which `show_screen` queues and which would otherwise erase the selection of a list that is still loading. Steps that build on the selection (`open_drawer_tab`, `open_helm_values`, `run_secret_action`) run inside the same deferred closure through `reveal_then`; `open_helm_values` sets the revision and layout after the selection, which forgets them.
- `--screen issues-drawer` reveals the first issue (used for screenshots). Column widths: 80, 170, 110, 260, 120, fill (min 200), 64, 60. The Kind column shows `HPA`, `PDB`, `PVC` for the long policy kinds; the full name still matches the quick filter.
- Ages: NamespaceStuck uses `deleting_since`, a failed Job `finished_at`, CertExpired the `not_after` date, CertExpiring `not_after` minus `EXPIRY_WARNING`. The other condition summaries carry no transition time, so their age is the first sighting.
- A stuck namespace outside the picked namespaces is not listed. A failed Job is dropped when a newer Job of the same owner has completed. A PDB that blocks because its pods are unhealthy waits `ROLLOUT_GRACE`.
- `ProbeSummary.initial_delay_seconds` is in the cluster crate (see above).
