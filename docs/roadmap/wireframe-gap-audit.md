# Wireframe gap audit — main `735f658`

[Back to index](README.md). Date 2026-10-03. Sources: `docs/k8sboard-wireframes.html` v0.6 (HTML source, W1–W11, notes, `k(...)` kind definitions, keyboard map, tokens), every `docs/specs/*/README.md`, scoped greps of `crates/app/src` and `crates/cluster/src`. Out of scope: 0034 Node maintenance (being built in `.tmp/wt-e`) and 0038 Helm writes (deferred by the user). Built and verified items are not listed.

Status: **missing**, **partial**, **in build** (0034), **backlog** (no spec, user decision). Size: S ≤ 1 coder step, M 2–3 steps, L a spec of 4+ steps.

## 1. Wireframe elements not built or partly built

| Wireframe | Element | Status | Evidence | Size |
|---|---|---|---|---|
| W1 n6, W3, W11 | Overview, Issues (with `⚑ N`, sidebar count, Cluster column), Topology over several viewed clusters | dropped (user decision 2026-10-03: single cluster only) | 0045 dropped; multi-cluster mode removed by 0046 | — |
| W2 nav | Pages General, Terminal & Shell, Logs | missing | `settings_window.rs` `SettingsPage` has 5 pages; owners in 0025 `other-pages.md` (0019, 0036) never added them | M |
| W2 n3 | Drag to reorder clusters (drives Ctrl 1–9), ⌕ Search | missing | `clusters_page.rs` | M |
| W2 form | Color swatches, Proxy | missing | `clusters_page.rs`, `cluster_registry.rs` (no color field) | M |
| W2 n2 | Watch a kubeconfig folder | missing (disabled "later") | `clusters_page.rs:737` | M |
| Tokens | Row density 28 / 36 px (Appearance) | missing | Appearance has Theme only | M |
| W3 n4 | Recent changes from managedFields (ConfigMap keys, "who"); click opens a diff | partial | `recent_changes.rs` (rollouts, HPA, nodes, namespaces); 0021 open item 1 | M |
| W4 n1 | Pod menu: Attach (A), Restart pod, Evict | missing | `resource_actions.rs::pod_menu`; 0033 moved them out, 0034 evicts only inside drain | M |
| W4 n2 | View logs ▸ container submenu (MAIN/SIDECAR/INIT) | Done (0039) | `LogsMenu` in `resource_actions.rs` | — |
| W4b n3 | Container ⋯ menu: logs, shell, attach, copy image | Done (0039) except Attach (0040) | `container_menu` in `resource_actions.rs`, button in `container_detail.rs` | — |
| W5, W6 | Edit taints/labels, Drain dialog, bulk Cordon/Uncordon/Drain, dock drain tab | in build (0034) | `node_menu`, `row_selection.rs` `NODE_ACTIONS` | — |
| W6 n2 | Skip PodDisruptionBudgets option | missing | 0034 non-goal, open item 1 | S |
| W5 n7 dock | `logs · kubelet · node` tab | missing, blocked (`NodeLogQuery` needs ≥ 1.30; UAT is 1.29.5) | 0019 open item 4; `kubelet_stats.rs` `KubeletPath` | M |
| W5 header | Edit labels for several nodes | missing | 0034 open item 2 | S |
| W7 Deployments | Revision diff ("history with diff and rollback") | Done (0039; rollback 0032) | `revision_diff.rs`, `pod_template_yaml` | — |
| W7 CronJobs | View logs of last job | Done (0039; key L on the workload kinds too) | `last_job_owner` in `kind_join.rs` | — |
| W7 ConfigMaps | Compare with previous | missing | 0031 non-goals (the "restart the workload" hint is Done in 0039) | M |
| W7 5 kinds | New (Namespace, ConfigMap, ResourceQuota, PDB, RoleBinding) | missing | 0031 non-goal ("templates") | M |
| W7 Secrets, ConfigMaps | Edit values (E on the two screens; Edit YAML stays in the menu) | Done (0047; masked write-only Secret fields, merge patch with the base `resourceVersion`) | [as-built](../specs/0047-config-secret-values/README.md), `values_edit.rs`, `config_values.rs` | — |
| W7 ResourceQuotas | `Edit` shown disabled next to a working `Edit YAML` | Done (0039: the placeholder is gone) | `resource_kind.rs` RESOURCE_QUOTAS | — |
| W7 Namespaces | Quota section LimitRange row | Done (0039) | `limit_range.rs`, `AccessCheck::ListLimitRanges`, `namespace_quota_rows` | — |
| W7 Namespaces, PDBs | Menu items Show remaining resources, Show selected pods | partial (sections exist) | `namespace_rows.rs`, `policy_rows.rs` | S |
| W7 Certificates | Renew now | missing | 0018 open item 5 | S–M |
| W8 n1–2 | Dashed 60 % line while dragging, double-click reset, remembered height | missing | `dock.rs`; 0004 non-goals; `dock.height` key unused | S |
| W8 | `SYS` marker lines (container restart, pod joined) | missing | 0019 follow-up | S |
| W8b | Histogram brush window; Pop out | missing | 0019 non-goals | S + M |
| W9 n1, n3 | Action × resource results (`> rest pay` → Restart rollout · deployment/payments-api) | missing (cursor row only) | 0029 decision 10, open item 2 | M |
| W9 | Matched-character underline; `@` keeps the namespace | missing | 0029 open items 3, 5 | S |
| W10 | Revision history tab | missing | 0031 non-goal | M |
| W10 n5 | Pre-apply snapshot, one-step rollback (ConfigMaps too) | missing | 0031 non-goal; needs a C1 decision | M |
| W10 n2 | Quota check ("Namespace quota OK") | missing | 0031 non-goal | S |
| W10 header | Hide managedFields toggle, Format | missing (always hidden) | `object_yaml.rs:474` | S |
| W11 | RBAC layer chip | done (0022 steps 4a, 4b: working chip, off by default; account → binding → role access row, three access checks) | [as-built-rbac](../specs/0022-topology/as-built-rbac.md) | — |
| W11 n1, W4c n1, W2 | Traffic mode; Prometheus with 30-day ranges and Settings › Metrics; Extensions; cloud scans | backlog | `topology_view.rs:1077`, `monitor_tab.rs:32`, `clusters_page.rs:737` | L each |
| W10 n1 | YAML LSP with the cluster schema | not planned (C6: server dry-run instead) | cross-cutting C6 | — |

Accepted deviations, not gaps: confirm tiers (user, 2026-10-02), no Follow toggle (0019), row click switches the drawer subject (0028 decision 14), bulk actions in the selection bar instead of header buttons.

## 2. Unticked acceptance criteria

Excluded: 0034 (13, in build) and 0038 (10, deferred).

| Reason | Count | ACs |
|---|---|---|
| Needs a write-capable (or larger) cluster | 3 | 0036 AC 8, 12; 0022 AC 10 (no UAT namespace above `POD_GROUP_LIMIT`) |
| Needs a manual check by the user | 5 | 0016 AC 7, 0017 AC 7 (reveal, clipboard); 0019 AC 6 (spot check); 0014 AC 6, 0022 AC 9 (kubectl cross-check, kubectl not installed) |
| Needs a ui-verifier run | 18 | 0015 AC 6; 0021 AC 7, 8, 11; 0022 AC 12; 0022b AC 9, 10; 0025 AC 4, 7; 0028 AC 9, 10, 12, 13; 0030 AC 14; 0031 AC 12; 0035 AC 12; 0036 AC 11; 0037 AC 13 |
| Needs a live UAT rerun by coder-lite | 6 | credential script: 0014 AC 4, 0015 AC 4, 0016 AC 5; request trace: 0025 AC 9, 0028 AC 7, 0029 AC 6 |
| Real gap | 0 | every gap in section 1 is a spec non-goal or open item, not an AC |
| Bookkeeping | 72 | 0001–0013: 38 of 125 ACs ticked, 16 marked superseded (0030, 0036, watch-budget growth); 71 left open because they need a UAT run, screenshots, ui-verifier, or renamed tests are unmatched; 0029 AC 2 stays open, name mapping added |

Superseded early ACs (mark, do not tick): 0002 AC 4 and 0004 AC 3 (`kube` without `ws`; 0035/0036 enabled it), 0001 AC 4 (read-only grep; now the 0030 allow-list), 0001 AC 8 and 0002 AC 7 ("`crates/app` unchanged", point in time).

## 3. Roadmap vs code

Fixed in this pass (docs only):

- `README.md`: baseline line and the status table (Events, mutations, Ctrl ⏎, kinds count, next specs).
- `inventory-shell.md`: T5, H1, H2, H4, H5, H6, D4, D5 were Missing or Partial but are built (0008, 0009, 0012–0018); H9 named 0025, which did not ship density; P1 and P2 still listed Ctrl ⏎ and the mutating letters as pending.
- `inventory-screens.md`: W5-3 and W5-6 (0009 built them); W4-3 and O5 pointed Evict, Restart pod, and the timeline diff at specs that do not cover them.
- `inventory-kinds.md`: Events rows said Planned(0006); Nodes drawer Partial; ConfigMaps, Namespaces, Quotas, PDBs, RoleBindings, Secrets named 0031 for New, Compare, and value edits (0031 non-goals); "Live today (12)".
- `gap-plan-local-and-mutating.md`: 0033 said Evict moves to 0034 (0034 drains only).

Not fixed (spec files, owner decision): status lines still read "draft" on built specs 0011, 0030, 0031, 0035; 0025 `other-pages.md` still assigns the Logs page to 0019 and Terminal & Shell to 0036.

## 4. Recommended order

| # | Spec | Gaps | New or amend | Kind | Size |
|---|---|---|---|---|---|
| 0 | housekeeping | tick 0001–0013 ACs, run the 18 ui-verifier and 6 coder-lite checks | no spec | read-only | S |
| 1 | 0039 Drawer and menu completions (done, except Show remaining / Show selected items) | Deployment revision diff, CronJob last-job logs, Logs ▸ submenu, container ⋯ menu (no Attach), LimitRange row, Show remaining / Show selected items, ConfigMap restart hint, drop the stale Quotas `Edit` | new | read-only | M |
| 2 | 0040 Pod lifecycle | Evict (0034 `EvictPod`), Restart pod (controller-owned only, 0033 delete), Attach (A, `create pods/attach` in `pod_shell.rs`), drain Skip PDBs, bulk node labels | new, after 0034 merges | mutating | M |
| 3 | 0046 One cluster at a time | remove multi-cluster mode (0027 multi view); 0045 dropped (user decision 2026-10-03) | new | local, no new request | M |
| 4 | 0041 Edit YAML II | Revision history, snapshot and rollback, ConfigMap Compare with previous, quota check, managedFields toggle, Format, Overview timeline diff and "who" | new; C1 decision on snapshots first | mutating + local files | L |
| 5 | 0042 Create from templates | New for the 5 kinds; a `Create` `WriteOperation` | new | mutating | M |
| 6 | 0043 Settings completions | General, Logs, Terminal & Shell pages (content needs the user), density, drag order, Search, color, Proxy, watch folder (C6 dependency) | new | local | L |
| 7 | 0044 Dock completions | dashed line, double-click reset, saved `dock.height`, `SYS` lines, histogram brush, Pop out | new | local | M |
| 8 | 0029 step 2 | action × resource results, highlight, `@` namespace carry | amend 0029 | local | M |
| 9 | 0022 steps 4a, 4b Topology RBAC layer (done) | ServiceAccount → binding → role edges behind the RBAC chip | amend 0022 | read-only | M |
| 10 | 0018 step 6 | Certificate Renew now (allow-listed `certificates/status` patch) | amend 0018 | mutating | S–M |
| 11 | 0047 Secret and ConfigMap value editing (done) | Edit values with masking and the C1 rules | new; user decision | mutating | M |
| — | backlog | kubelet logs (≥ 1.30), Prometheus and Metrics page, cloud scans, Traffic, Extensions, LSP, 0038 | user decision | — | L |
