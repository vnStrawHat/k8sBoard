# 0017 · Files to touch

[Back to index](README.md). **S** is the step. Each step passes the gate on its own; nothing lands before its first user (the probe is the crate's first user in step 1). **Prerequisite (M3): 0012 and 0016 (with 0013–0015) are implemented and merged before step 1**; at HEAD `ef69712` neither is. `secret.rs`, `secret_values`, `write_private_text`, `SecretCopied`, `arm_clipboard_clear`, `ValueAccess`, `REVEAL_DURATION`, `selected_summary_watch`, related subjects, and `kind_diagnosis.rs` are created by them.

## Cargo (step 1)

| File | Change |
|---|---|
| `Cargo.toml` (root) | workspace deps `flate2 = "1"`, `base64 = "0.22"`; `serde-saphyr` features `["serialize", "deserialize"]` |
| `crates/cluster/Cargo.toml` | `flate2`, `base64`, `serde` with `.workspace = true` |
| `Cargo.lock` | no new package (AC 4); only the `k8sboard-cluster` dependency list changes |

## `crates/cluster` (step 1)

| File | Change |
|---|---|
| `src/helm_release.rs` (new) + `src/helm_release_tests.rs` | `HelmStatus`, `HelmChart`, `HelmReleaseSummary`, `HelmRevision`, `HelmRevisionRef`, `RELEASE_SIZE_LIMIT`, `PayloadIssue`, `release_json`, `revision_head`, `group_releases`, `history_revision`, both watch configs, `watch_helm_releases`, `watch_helm_history` |
| `src/helm_release_detail.rs` (new) + `src/helm_release_detail_tests.rs` | `HelmText`, `HelmReleaseDetail`, `HelmRevealed` (user, computed, notes), `ValueVisibility`, `fetch_release`, `mask_values`, `coalesce_values`, `masked_manifest`, `helm_release_detail`, `helm_revealed`, `manifest_options` |
| `src/helm_values_diff.rs` (new) + `src/helm_values_diff_tests.rs` | `HelmValuesDiff`, `ValueChange`, `DIFF_LIMIT`, `DIFF_VALUE_CHARS`, `flatten`, `values_diff`, `leaf_text`, `helm_values_diff` |
| `src/resource_watch.rs` (+ tests) | `metadata_summary_watch` |
| `src/secret.rs` (created by 0016) | extract `secret_text`; `secret_values` uses it |
| `src/object_yaml.rs` | `HIDDEN`, `yaml_text`, `with_hidden_header`, three maskers `pub(crate)` (no behavior change) |
| `src/lib.rs` | modules and exports ([cluster-api.md](cluster-api.md)) |
| `examples/probe.rs` | `helm releases` watch line, `--helm`, `USAGE` |

Test fixtures are built in code (a release JSON → `flate2::write::GzEncoder` → base64 → `Secret`); no fixture files.

## `crates/app`

| S | File | Change |
|---|---|---|
| 2 | `src/resource_kind.rs` | `HelmReleases` spec, `ALL` (after Secrets), `watch_rows`, `has_count` |
| 2 | `src/kind_row.rs` | `KindObject::HelmRelease`, `LiveContent::{HelmRelease, HelmHistory}` |
| 2 | `src/helm_rows.rs` (new) + `helm_rows_tests.rs` | `helm_release_row`, `helm_status_label` |
| 2 | `src/live_sections.rs` (+ tests) | `HelmRelease`, `HelmHistory` (rows without buttons) |
| 2 | `src/kind_diagnosis.rs` (+ tests) | Helm status box arm |
| 2 | `src/related_objects.rs` (+ tests), `src/cluster_session.rs` (+ tests) | `RelatedSubject::HelmHistory`, `RelatedList::HelmHistory`, `RelatedUpdate::HelmHistory`; counts skip `!has_count()` |
| 2 | `src/drawer.rs` (+ tests) | `drawer_tabs` arm `&[Overview]` |
| 2 | `src/object_events.rs` (+ tests), `src/yaml_view.rs` (+ tests) | `HelmReleases` → `None` |
| 2 | `src/resource_actions.rs` (+ tests) | Roll back…, Uninstall release… disabled "Read-only mode" |
| 2 | `src/app_shell.rs` | `open_yaml` → `open_drawer_tab(key, tab)`; callers |
| 2 | `src/main.rs` | `mod helm_rows;` |
| 3 | `src/helm_release_view.rs` (new) + `helm_release_view_tests.rs` | `HelmReleaseView`, `HelmTab`, `ValuesSource`, `ValuesLayout`, `ShowLatest`, pure core incl. `private_copy`; Copy/Cut capture while revealed (M2) |
| 3 | `src/drawer.rs` (+ tests) | `DrawerTab::{Values, Manifest, Notes}`, titles, `drawer_tabs` arm `&[Overview, Values, Manifest, Notes]`, `DrawerState.{helm, helm_revision, pending_helm_layout}`, `helm_subject` |
| 3 | `src/app_shell.rs` | `sync_helm_view` (subscribes `ShowLatest`, `SecretCopied` → 0016 `arm_clipboard_clear`), `open_helm_values`, `helm_revision` reset on subject change |
| 3 | `src/live_sections.rs` (+ tests), `src/kind_row.rs`, `src/helm_rows.rs`, `src/kind_drawer.rs` | History **Values** / **Diff** buttons, `shown` mark; `LiveContent::HelmValuesChange` section placing the view on Overview |
| 3 | `src/resource_actions.rs` (+ tests) | View values, View manifest |
| 3 | `src/launch_options.rs` (+ tests), `src/screenshot.rs` (+ tests) | `-values`, `-manifest` slugs; settle on `is_loading` |
| 3 | `src/main.rs` | `mod helm_release_view;` |

## Docs (with the last step)

- `docs/roadmap/inventory-kinds.md`: Helm › Releases → Done (Roll back, Uninstall 0038).
- `docs/roadmap/cross-cutting.md`: C1 row note "0017: values masked by leaf, Reveal per drawer 30 s, manifests masked per document, notes masked until revealed, revealed copies through the private clipboard"; C6 Helm row → `flate2` 1 + `base64` 0.22 + serde-saphyr `deserialize` (all locked); Diff row: 0017 uses a path diff, `similar` left to 0031; UAT table: Helm probe results.
- `docs/roadmap/README.md` status row; 0001 `probe-example.md`: `helm releases` watch line and `--helm`.
