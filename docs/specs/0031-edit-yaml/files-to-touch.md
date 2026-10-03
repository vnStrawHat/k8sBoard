# 0031 · Files to touch

[Back to index](README.md). **S** is the step. Each step passes the gate on its own, and every new item has a production user in its own step. Baseline: main `2c7dc08`. Steps 0–2 need only the merged 0030 step 1; steps 3–4 need 0030 steps 2b + 4 merged and the `RowAction` key layer (0032 actions-ui.md, built by whichever of 0031 step 3 / 0032 step 2a-i lands first). **C3: Approved by the user on 2026-10-02 (one approval for all mutating specs). Steps 0–3 send no mutating-verb request.**

## Cargo

| S | File | Change |
|---|---|---|
| 1 | root `Cargo.toml` | `serde-saphyr` features `["serialize", "deserialize"]`. The locked 1.3.0 already lists the deserialize packages, so `Cargo.lock` does not change; the coder confirms |
| 3 | root `[workspace.dependencies]` + `crates/app/Cargo.toml` | `similar = "3"` (C6: needs the user's approval). `Cargo.lock` gains exactly `similar`; anything else: stop and report |

## `crates/cluster`

| S | File | Change |
|---|---|---|
| 0 | `src/object_yaml.rs` (+ tests) | `mask_object`, `MaskCount`, `to_yaml_text`, `get_object` split out of `mask_to_yaml` / `object_yaml`; golden test `masked_yaml_output_is_unchanged` with its captured fixture texts. `HIDDEN` moves to `edit_placeholders.rs` in step 1 |
| 1 | `src/object_yaml.rs` | `ObjectKind::{ALL, resource, is_editable}` |
| 1 | `src/object_edit.rs` (new) + `object_edit_tests.rs` | `EditBase` (+ `edit_base`), `ObjectEdit`, `EditError`, strip, identity, Secret lock, leading-zero scan |
| 1 | `src/edit_placeholders.rs` (new) + `edit_placeholders_tests.rs` | `HIDDEN`, `counterpart` (unique-name rule), `check` |
| 1 | `src/edit_preview.rs` (new) + `edit_preview_tests.rs` | `FieldPath`, `PathSegment`, `field_paths` |
| 1 | `src/access_review.rs` (+ tests) | `AccessCheck::Update(ObjectKind)` (not in `ALL`), `review_access_for(checks, scope)`; `review_access` delegates; `AccessReport::all_of` takes the reviewed list |
| 1 | `src/lib.rs` | `mod object_edit; mod edit_placeholders; mod edit_preview;` and exports |
| 2 | `src/edit_placeholders.rs` | `restore`, `Restored { moved }` |
| 2 | `src/edit_preview.rs` | `EditPreview`, `FieldChange`, `EditCheck`, `field_changes`, `build_preview` (markers, no header) |
| 2 | `src/object_write.rs` (+ tests) | `WriteOperation::ReplaceObject`, `WriteEffect::Replaced`, per-operation effect seam, name rule (decision 27), `ChangedField.path` → `Cow`, the sequence, the `replace` arm, manual `Debug` |
| 3 | `src/object_edit.rs` | `format_yaml` |
| 4 | `src/object_edit.rs` | `rebase`, `Rebased` |

## `crates/app`

| S | File | Change |
|---|---|---|
| 3 | `src/cluster_session.rs` (+ tests) | `kind_access` map, `request_kind_access(kind)` on first screen show, cleared on scope change; `guard()` passes it |
| 3 | `src/write_guard.rs` (+ tests) | `ClusterGuard.kind_access` |
| 3 | `src/resource_actions.rs` (+ tests), `src/resource_kind.rs` | `EditYaml(ObjectKind)` gate `Update(kind)` (**not shipped**), lazy `permission_reason`, `Edit YAML` items, ConfigMaps `Edit` keyed; `subject_action` resolution |
| 3 | `src/keyboard_navigation.rs` (0028) | `EditYaml(_)` arm → `open_edit(subject)`; `run_row_key` inert while editing |
| 3 | `src/yaml_edit.rs` (new) + `yaml_edit_tests.rs` | `YamlEditView` (load, editor, tabs, diff list, side panel, footer, Env values, Format, local checks), `fixture` |
| 3 | `src/yaml_diff.rs` (new, tests in module) | `DiffRow`, `DiffRowKind`, `diff_rows` |
| 3 | `src/app_shell.rs`, `src/app_shell_view.rs`, `src/workspace.rs` | `AppShell.edit`, `open_edit`, discard prompt before navigation, `leaving_work` + `ReleaseCheck` at `switch_cluster` / `view_clusters` / `remove_from_view`, editor closed by `release_slot` / `release_all`, workspace swap |
| 3 | `src/keymap.rs` (+ `keymap_tests.rs`) | `secondary-s` → `ApplyEdit` in `YamlEdit`; out of `RESERVED_KEYS`; sheet row |
| 3 | `src/launch_options.rs` (+ tests), `src/screenshot.rs`, `src/main.rs` | `--screen edit-yaml-diff`; `USAGE`; `mod yaml_edit; mod yaml_diff;` |
| 4 | `src/resource_actions.rs` | `EditYaml` shipped |
| 4 | `src/write_flow.rs` (+ tests), `src/confirm_dialog.rs` (0030 step 4) | `preview_write` (through `checked_write`), `GuardedIntent.on_commit`, close after commit, `ReplaceObject` dry-run line and object row |
| 4 | `src/yaml_edit.rs` | server preview, Apply, `on_commit`, banners, rebase, `server_changed`, `Failed(Invalid)` |
| 4 | palette (0029) | nothing to change: the entry follows `key_availability_of` and dispatches the E key action |

## Docs

| S | File | Change |
|---|---|---|
| 2 | `docs/specs/0030-guardrails-write-path/write-path.md` | allow-list row `ReplaceObject` is already listed; note the name rule of decision 27 |
| 3 | `docs/specs/0028-keyboard-map/keymap.md` | Ctrl S (W10) bound in `YamlEdit` |
| 4 | `docs/roadmap/gap-plan-local-and-mutating.md`, `inventory-screens.md` | 0031 done; W10 partial (history, snapshot, templates, quota) |

## Parallel work (lane W1, with 0032 → 0033 → 0032b)

- Disjoint from lane W2 (0036 → 0035 → 0037 → 0034): every new file above, `object_yaml.rs`, `edit_*.rs`, `yaml_*.rs`.
- Shared with lane W2, append-only blocks: `object_write.rs` (enum, `send`, `Debug`, `changed_fields` arms), `access_review.rs`, `keymap.rs`, `keyboard_navigation.rs` (own arm), `resource_actions.rs`, `write_flow.rs`, `confirm_dialog.rs`, `app_shell.rs`, `launch_options.rs`, `screenshot.rs`, `main.rs`. Rebase on main before each step.
