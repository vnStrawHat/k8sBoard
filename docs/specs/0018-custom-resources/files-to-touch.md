# 0018 · Files to touch

[Back to index](README.md). **S** is the step. Each step passes the gate on its own; nothing lands before its first user (the probe is the crate's first user in 1a and 1b). Prerequisite: 0012, 0014, 0016 merged (0013–0015 with them). If 0017 is merged first, its `HelmReleases` static converts to `KindApi` too.

## Cargo

None. `kube::core::{Object, DynamicObject, ApiResource, Version}`, `serde`, `serde_json`, `jiff` are already dependencies; `Cargo.lock` must not change (AC 3).

## `crates/cluster`

| S | File | Change |
|---|---|---|
| 1a | `src/column_path.rs` (new) + `src/column_path_tests.rs` | `ColumnPath`, `Step`, `UnsupportedPath`, `parse`, `first_match`, `reads_metadata`, `is_condition_status`, `last_field`, `column_value` |
| 1a | `src/custom_resource_definition.rs` (new) + `src/custom_resource_definition_tests.rs` | `ResourceScope`, `CustomResourceType`, `CrdSummary`, `CrdState`, `CrdVersion`, `PrinterColumn` (+ `new`), `ColumnType`, `SchemaOutline`, `SchemaField`, lean serde heads, `crd_summary`, `preferred_version`, `resource`, `watch_crds`, shared `api_resource` |
| 1a | `src/object_yaml.rs` (+ `object_yaml_tests.rs`) | `ObjectKind::CustomResourceDefinition`; `is_secret_key` (moved from 0014, widened) |
| 1a | `src/storage_class.rs` (created by 0014) | call `object_yaml::is_secret_key`; its tests move with the function and keep their results |
| 1a | `src/access_review.rs` (+ tests) | `ListCustomResourceDefinitions` |
| 1a | `src/lib.rs`, `examples/probe.rs` | CRD exports; `--crds` CRD lines, `USAGE` |
| 1b | `src/custom_object.rs` (new) + `src/custom_object_tests.rs` | `CustomObjectSummary`, `ColumnValue`, `ObjectCondition`, `CustomObjectFields`, `FieldList`, `FieldEntry`, `FieldValue`, `custom_object_summary`, `object_fields`, `watch_custom_objects`, `watch_custom_object_fields`, `count_custom_objects` |
| 1b | `src/object_yaml.rs` (+ tests) | `ObjectTarget`, `ObjectRef::custom`, `mask_custom_object` (S2–S5) |
| 1b | `src/access_review.rs` (+ tests) | `review_custom_access` (`list` only), `&str` review builder |
| 1b | `src/resource_watch.rs` (+ tests) | `summarize: F` generic in every helper |
| 1b | `src/object_count.rs` (created by 0012) | private count params shared with `count_custom_objects` (`pub(crate)`) |
| 1b | `src/lib.rs`, `examples/probe.rs` | custom object exports; access, count, watch, yaml probe lines |
| 5 | `src/namespace.rs` (+ tests) | `deleting_since`, `deletion_conditions`, `NamespaceDeletionCondition`; export |

Fixtures are built in code with `serde_json::json!` (cert-manager, Argo CD, Strimzi CRDs and objects, a `ClusterSecret`); no fixture files.

## `crates/app`

| S | File | Change |
|---|---|---|
| 2a | `src/resource_kind.rs` | `KindSpec` `pub(crate)` with `api: KindApi`; every static converted; `Crds`; accessors (`access_check` → `Option`, `builtin_object`, `object_kind`, `object_ref`); `watch_rows` → `Option`; tests |
| 2a | `src/crd_rows.rs` (new) + `crd_rows_tests.rs` | `crd_row`, status, columns, the four pre-built sections |
| 2a | `src/cluster_session.rs` (+ tests) | `CrdWatch` (list only), start after the review, feed into the Crds explorer, `KindList._subscription: Option`, `OpenWatches.crds` |
| 2a | `src/kind_row.rs`, `src/yaml_view.rs` (+ tests), `src/navigation.rs`, 0012 counts | `KindObject::Crd`; `kind.object_ref`; `access_check()` as `Option`; counts use `builtin_object()` |
| 2a | `src/main.rs` | `mod crd_rows;` |
| 2b | `src/custom_kind.rs` (new) + `custom_kind_tests.rs` | `CustomKind`, `CustomKindSpec`, `ColumnRule`, `CustomKindCache`, `custom_kinds`, `BUILT_IN_COLUMNS`, `kind_label`, `kind_badge`, columns |
| 2b | `src/resource_kind.rs` | `Custom(CustomKind)`, `custom()`, `watch_rows` Custom arm, `rows` takes a closure |
| 2b | `src/custom_rows.rs` (new) + `custom_rows_tests.rs` | `custom_object_row`, `custom_status`, `condition_label` |
| 2b | `src/kind_row.rs`, `src/kind_table.rs`, `src/kind_drawer.rs` | `KindObject::Custom`, `KindCell::Date { at, rule }`, `DateRule`, value and paint (expiry tone via 0016 `expiry_label`) |
| 2b | `src/cluster_session.rs` (+ tests) | `CrdWatch.kinds`, `custom_kind_cache`, `take_custom_kind_cache`, `CustomGate`, gate flow, `custom_denied_reason` |
| 2b | `src/app_shell.rs` | cache hand-over in `start_session`; `follow_custom_kinds`, `remapped_screen` |
| 2b | `src/navigation.rs` (+ tests) | `screen_item`, group submenus, custom availability |
| 2b | `src/object_events.rs` (+ tests) | event subject via `object_kind()` (test a custom key) |
| 2b | `src/launch_options.rs` (+ tests), `src/screenshot.rs` (+ tests) | `LaunchScreen::Custom`, resolution, settle, slugs |
| 2b | `src/main.rs` | `mod custom_kind; mod custom_rows;` |
| 3 | `src/kind_row.rs`, `src/live_sections.rs` (+ tests) | `LiveContent::{CustomConditions, CustomStatus, CustomSpec}`, field rows, `secretName` link |
| 3 | `src/kind_diagnosis.rs` (+ tests) | status box arm (`custom_object_diagnosis`) |
| 3 | `src/related_objects.rs` (+ tests), `src/cluster_session.rs` | `RelatedSubject::CustomFields`, `RelatedList::CustomFields`, `RelatedUpdate::CustomFields` |
| 3 | `src/resource_actions.rs` (+ tests) | Browse instances for CRD rows |
| 4 | `src/cluster_session.rs` (+ tests), `src/kind_join.rs` (+ tests), `src/navigation.rs`, `src/app_shell.rs` | `CustomCounts`, `refresh_custom_counts`, `CRD_INSTANCES`, custom sidebar counts |
| 5 | `src/kind_row.rs`, `src/namespace_rows.rs` (+ tests), `src/kind_diagnosis.rs` (+ tests) | `KindObject::Namespace`, `STUCK_AFTER`, STUCK box, `remaining_entries`, Remaining resources section |

## Docs (with the last step)

- `docs/roadmap/inventory-kinds.md`: CRDs → Done (Instances, Browse instances); Certificates (any discovered CR) → Done for read (Expires built-in; Renew 0032); Namespaces STUCK → Done (object names not listed).
- `docs/roadmap/inventory-shell.md`: N6 → Done (grouped by API group).
- `docs/roadmap/cross-cutting.md`: C1 note "0018: custom objects shown like ConfigMaps plus key, kind, URL-userinfo, and password-format heuristics; `is_secret_key` shared with 0014; no reveal"; UAT table: CRD probe results.
- `docs/roadmap/README.md` status row; 0001 `probe-example.md`: `--crds` lines.
