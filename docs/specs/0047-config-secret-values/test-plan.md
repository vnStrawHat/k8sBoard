# 0047 · Test plan

[Back to index](README.md) · All tests offline. Fixtures use a distinctive value `S3cr3t-0047-ZZ` (base64 `UzNjcjN0LTAwNDctWlo=`); every leak test searches for both.

## Step 1 · `config_values_tests.rs`

| Test | Checks |
|---|---|
| `secret_base_keeps_names_sizes_and_binary_flags` | keys sorted, `Hidden`/`Binary`, sizes, `resourceVersion` |
| `secret_base_debug_holds_no_value` | `format!("{base:?}")` lacks the value and its base64 |
| `helm_release_secret_is_refused` | `type: helm.sh/release.v1` → `HelmRelease`, no base |
| `owner_helm_label_is_refused_on_both_kinds` | a ConfigMap and a Secret labelled `owner=helm` → `HelmRelease`, no base |
| `service_account_token_secret_is_refused` | → `ServiceAccountToken` |
| `immutable_secret_and_config_map_are_refused` | → `Immutable` |
| `other_kind_is_refused` | a Deployment target → `NotEditable` |
| `config_map_base_keeps_text_and_marks_large_and_binary` | `Text`, `TooLarge` over 128 KiB, `binaryData` → `Binary` |
| `base_notes_read_helm_label_and_owner` | `is_helm_managed`, `owner = "Deployment/api"` |
| `edit_refuses_bad_key_names` | empty, `.`, `..`, `a/b`, 254 bytes → `InvalidKey` |
| `edit_refuses_add_of_existing_key` | in `data` and in `binaryData` → `DuplicateKey` |
| `edit_refuses_set_or_remove_of_unknown_key` | → `UnknownKey` |
| `edit_refuses_text_set_on_binary_or_large_key` | → `BinaryValue` |
| `edit_refuses_value_over_one_mebibyte` | → `TooLarge` |
| `edit_refuses_object_over_one_mebibyte` | many keys under 1 MiB each, estimated total over → `ObjectTooLarge` |
| `secret_set_is_a_change_even_if_equal` | a `Set` is never dropped as unchanged for a Secret |
| `newlines_are_kept_in_body` | `a\nb\n` and `a\r\nb` base64-decode from the body byte for byte |
| `edit_refuses_no_change_and_two_changes_of_one_key` | → `NoChange`, `DuplicateKey` |
| `edit_debug_shows_counts_only` | `ValuesEdit`, `KeyChange`, `FieldChange` `Debug` lack the value |
| `secret_patch_base64_encodes_data` | body `data.K` = standard base64; no `stringData`, no `binaryData` |
| `config_map_patch_sends_text_as_typed` | `data.K` = text |
| `removal_is_null_in_its_field` | `binaryData.BLOB: null` for a ConfigMap binary key |
| `patch_holds_only_version_and_changed_keys` | top-level keys exactly `metadata` (+ `data`/`binaryData`); `metadata` only `resourceVersion` |

## Step 1 · `object_write_values_tests.rs` (fake transport)

| Test | Checks |
|---|---|
| `dry_run_request_shape` | `PATCH`, path, query `dryRun=All&fieldManager=k8sboard`, content type `application/merge-patch+json`, body |
| `commit_request_has_no_dry_run` | query `fieldManager=k8sboard` only |
| `blocked_policy_sends_nothing` | `WritesBlocked`, zero requests |
| `request_needs_matching_target_and_kind` | `WriteRequest::new` → `None` for another target or kind |
| `access_check_is_patch_of_kind` | `Patch(Secret)`, `Patch(ConfigMap)` |
| `changed_fields_are_names_and_markers` | `data[K] added`, `value changed`, `removed`; `value: None`; no value text |
| `conflict_maps_to_conflict` | 409 → `Conflict` |
| `secret_invalid_message_is_redacted` | 422 with the value in its message → reason and paths only |
| `request_debug_holds_no_value` | `format!("{request:?}")` lacks value and base64 |
| `patch_check_targets_patch_verb` | `access_review` `CheckTarget` of `Patch(kind)`: verb `patch`, core group, resource |

## Step 2 · `values_edit_tests.rs` (view model, no window where possible)

| Test | Checks |
|---|---|
| `secret_fields_start_masked_and_empty` | masked, empty, state `unchanged` |
| `typing_makes_a_set_and_empty_keeps` | change list |
| `config_map_set_equal_to_base_is_no_change` | ConfigMap only |
| `pasted_newlines_survive_mask_round_trip` | paste `a\nb\n` and `a\r\nb` (while masked and while unmasked), mask, unmask: `text()` unchanged; the Apply `NewValue` equals it |
| `secret_field_is_one_textarea_for_life` | the same `TextareaState` entity before and after mask toggles |
| `masked_field_renders_placeholder_with_char_count` | `•••• N chars`; `unchanged` when empty; no textarea element |
| `paste_while_masked_fills_hidden_field` | text inserted, still masked |
| `dirtiness_tracks_change_events_and_length` | `InputEvent::Change` → dirty; no `value()` call path |
| `apply_copies_rope_once_into_zeroizing` | the copy has `capacity == len` |
| `unmask_re_masks_after_thirty_seconds` | ticker with a fake clock |
| `apply_re_masks_every_field` | |
| `copy_and_cut_in_secret_field_are_dropped` | clipboard port untouched |
| `binary_and_large_rows_offer_remove_only` | |
| `screenshot_access_disables_the_eye` | `ValueAccess::Blocked` |
| `reload_keeps_changes_by_key_and_lists_dropped` | decision 12 |
| `local_error_shows_under_its_key` | nothing sent |

## Step 2 · `app_shell_values_edit_tests.rs` (fake clusters)

| Test | Checks |
|---|---|
| `menu_offers_edit_values_on_config_maps_and_secrets_only` | not on Helm release rows or other kinds |
| `refused_objects_disable_the_item` | reasons of editor-view.md, including `owner=helm` on a ConfigMap row |
| `denied_patch_disables_with_reason` | `Not permitted: patch secrets` |
| `apply_runs_dry_run_then_confirm_then_commit` | requests in order; one audit line |
| `confirm_dialog_shows_names_and_markers_only` | dialog text lacks the value |
| `audit_line_holds_names_and_markers_only` | line lacks value and base64; action `Edit values` |
| `notices_hold_no_value` | success and failure notices |
| `prod_tier_needs_typed_name` | |
| `conflict_shows_banner` | |
| `one_edit_at_a_time_with_edit_yaml` | `OpenEdit` slot |
| `dirty_edit_asks_before_cluster_switch` | `leaving_work.unsaved_edit`; editor closed after confirm |
| `namespace_change_asks_then_drops_view` | entity released |
| `table_keys_inert_while_open` | |
| `e_opens_edit_values_on_config_maps_and_secrets` | E on a ConfigMap and a Secret row opens `ValuesEditView`, not Edit YAML |
| `e_keeps_edit_yaml_on_other_kinds` | E on a Deployment row opens Edit YAML |
| `e_does_nothing_on_helm_releases` | `NotOffered`, no editor |
| `e_on_denied_secret_shows_the_reason` | the gate's `Not permitted: patch secrets` notice |
| `edit_yaml_still_opens_from_menu_and_palette_on_secrets` | dispatching `EditYaml` opens Edit YAML |

## Step 2 · keymap and actions (`keymap_tests.rs`, `resource_actions_tests.rs`)

| Test | Checks |
|---|---|
| `e_binds_edit_values_only_in_values_screen` | binding of `e` per context: `ValuesScreen` → `EditValues`, otherwise `EditYaml` |
| `values_screen_context_follows_the_visible_screen` | ConfigMaps, Secrets → set; Releases, Deployments → not set |
| `edit_values_key_action_is_edit_values` | `RowAction::EditValues.key_action()` |
| `edit_values_resolves_on_two_kinds_only` | `subject_action(EditValues, ..)`: ConfigMaps, Secrets → `Some`; Releases, Pods, Nodes, custom kinds → `None` |
| `shortcut_rows_name_both_edit_keys` | `Edit values (ConfigMaps, Secrets)` and `Edit YAML (other kinds)` |

Existing tests that change: `resource_actions_tests` (Secrets placeholder gone; the 0031 "E offered on every editable kind" tests now exclude ConfigMaps and Secrets), `kind_access_tests` (`Patch` lazy checks), `keymap_tests` (`ValuesEdit` context, the `e` binding split), `launch_options_tests` (`values-edit`), edit-slot tests now through `OpenEdit`.

## Greps (review)

- `grep -n "tracing::" crates/cluster/src/config_values.rs crates/app/src/values_edit.rs crates/app/src/values_edit_flow.rs` → none.
- `grep -n "stringData" crates/cluster/src/config_values.rs` → none outside comments.
- `grep -n "derive(.*Debug" crates/cluster/src/config_values.rs` → only `DataField`, `BaseNotes`, `FieldChange` (no value inside). `ValuesBase`, `ValuesEdit`, `KeyChange` have manual `Debug`; `NewValue`, `ValueKey` none.
- `grep -nE "mask_toggle|set_masked|\.masked\(|\.value\(\)" crates/app/src/values_edit.rs` → none on Secret fields.

## Step 3 · live (agents)

1. Probe build with `readonly@Monitor`: `--screen secrets`, open a row menu: `Edit values…` disabled with the `patch` reason. No reveal, no copy, no typing of real values.
2. Run with `RUST_LOG=kube_client::client::builder=debug` and count from the app's own debug log only (the `HTTP` span: method and URL): only GET and SSAR POST; zero PATCH, PUT, DELETE. If that is not possible, a code review of the call sites stands in, as in 0026.
3. ui-verifier: `--screen values-edit` against W7 (Secrets drawer, Data rows) and W10 notes 3–4; masked state only.
