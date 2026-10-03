# 0047 · Files to touch

[Back to index](README.md)

## Step 1 (cluster crate only)

| File | Change |
|---|---|
| `crates/cluster/src/config_values.rs` (new) | `ValuesBase`, `ValueKey`, `DataField`, `KeyContent`, `BaseNotes`, `NewValue`, `KeyChange`, `FieldChange`, `ValuesEdit`, errors, `values_base`, `ValuesBase::edit`, `values_patch` |
| `crates/cluster/src/config_values_tests.rs` (new) | unit tests of the module |
| `crates/cluster/src/object_write.rs` | `SetDataValues` variant, `name`, `checked_operation`, `fitting_access_check`, `changed_fields`, `supports_dry_run`, `send` arm |
| `crates/cluster/src/object_write_values_tests.rs` (new) | fake-transport request tests, wired like the other `object_write_*_tests` modules |
| `crates/cluster/src/access_review.rs` | `AccessCheck::Patch(ObjectKind)` (lazy, not in `ALL`), its `CheckTarget` (verb `patch`) |
| `crates/cluster/src/secret.rs` | `decode_secret` becomes `pub(crate)` (reuse only); no behavior change |
| `crates/cluster/src/object_edit.rs` | `HELM_RELEASE_TYPE` to `pub(crate)` |
| `crates/cluster/src/lib.rs` | module and the public exports above |
| `docs/specs/0030-guardrails-write-path/write-path.md` | allow-list row for `SetDataValues` |
| `docs/specs/0031-edit-yaml/README.md` | open item 5: pointer "amended by 0047 decision 9" (done by the architect) |

## Step 2 (app crate)

| File | Change |
|---|---|
| `crates/app/src/values_edit.rs` (new) | `ValuesEditView`: rows, inputs, mask timer, local errors, conflict banner, fixture |
| `crates/app/src/values_edit_tests.rs` (new) | view model tests |
| `crates/app/src/values_edit_flow.rs` (new; child of `app_shell`, like `edit_yaml_flow.rs`) | `open_values_edit`, `values_commit_finished` |
| `crates/app/src/edit_yaml_flow.rs` | slot helpers take `OpenEdit` |
| `crates/app/src/app_shell.rs` | `edit: Option<OpenEdit>`, render arm, module declaration, `ValuesScreen` in the root key context while ConfigMaps or Secrets is visible |
| `crates/app/src/write_flow.rs` | the commit callback calls `values_commit_finished` for `EditValues(_)`, next to `edit_commit_finished` |
| `crates/app/src/resource_actions.rs` | `ResourceAction::EditValues`, `RowAction::EditValues` (+ `key_action` → `EditValues`), `subject_action` for ConfigMaps and Secrets only, gate, label, risk, row disable reasons, Helm release exclusion |
| `crates/app/src/resource_kind.rs` | drop the Secrets `Edit` placeholder; menu item on ConfigMaps and Secrets |
| `crates/app/src/kind_access.rs` | `lazy_checks` adds `Patch(kind)` for ConfigMap and Secret |
| `crates/app/src/keymap.rs` | `EditValues` action; `e` → `EditYaml` in `WORKSPACE && !ValuesScreen`, `e` → `EditValues` in `WORKSPACE && ValuesScreen` (decision 9); shortcut rows; `ValuesEdit` context, `secondary-s` → `ApplyEdit`, `WORKSPACE` adds `!ValuesEdit` |
| `crates/app/src/keyboard_navigation.rs` | `on_row_key::<EditValues>(root, RowAction::EditValues, cx)`; `run_available_row_key` arm `EditValues(_)` → `open_values_edit` |
| `crates/app/src/leaving_work.rs` | reads `OpenEdit` (no new text) |
| `crates/app/src/command_palette.rs`, `palette_search.rs` | `EditValues` entry and icon |
| `crates/app/src/launch_options.rs` | `--screen values-edit` |
| `crates/app/src/app_shell_values_edit_tests.rs` (new) | shell flow tests over fake clusters |

## Step 3

No code. UAT run and ui-verifier; As-built notes in the README.

## Do not touch

`audit_log.rs` (`PATH_ONLY_KINDS` and `recordable_fields` stay as they are), `confirm_dialog.rs` (renders `changed_fields()` already), `object_edit.rs` behavior (0031 keeps refusing Secret data), `docs/specs/0043-*`.
