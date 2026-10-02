# 0031 · Files to touch

[Back to index](README.md). **S** is the step. Each step passes the gate on its own, and every new item has a production user in its own step. Prerequisites: 0030 code merged with the 0032 architect's one-line amendments (`GuardedIntent.warnings`, `WriteOutcome.effect` / `WriteEffect`, `ObjectKind::{ALL, resource}`, "no SSA / no Force"), the 0036 `run_guarded`, 0032 `checked_write`, 0028, 0007; 0016 before Secret edits. **C3: Approved by the user on 2026-10-02 (one approval for all mutating specs). Steps 0–3 send no mutating-verb request.**

## Cargo

| S | File | Change |
|---|---|---|
| 1 | root `Cargo.toml` | `serde-saphyr` features `["serialize", "deserialize"]`. The locked 1.3.0 already lists the deserialize packages, so `Cargo.lock` does not change; the coder confirms |
| 3 | root `[workspace.dependencies]` + `crates/app/Cargo.toml` | `similar = "3"`. `Cargo.lock` gains exactly `similar`; anything else: stop and report |

## `crates/cluster`

| S | File | Change |
|---|---|---|
| 0 | `src/object_yaml.rs` (+ tests) | `mask_object`, `to_yaml_text`, `get_object` split out; `HIDDEN` moves to `edit_placeholders.rs` in step 1; golden test `masked_yaml_output_is_unchanged` with its captured fixture texts |
| 1 | `src/object_yaml.rs` | `ObjectKind::is_editable` |
| 1 | `src/object_edit.rs` (new) + `object_edit_tests.rs` | `EditBase` (+ `edit_base`), `ObjectEdit`, `EditError`, strip, identity, Secret lock, leading-zero scan |
| 1 | `src/edit_placeholders.rs` (new) + `edit_placeholders_tests.rs` | `HIDDEN`, `counterpart` (unique-name rule), `check` |
| 1 | `src/edit_preview.rs` (new) + `edit_preview_tests.rs` | `FieldPath`, `PathSegment`, `field_paths` |
| 1 | `src/access_review.rs` (+ tests) | `AccessCheck::Update(ObjectKind)` (not in `ALL`), `review_checks(checks, scope)`; `review_access` delegates to it |
| 1 | `src/lib.rs` | `mod object_edit; mod edit_placeholders; mod edit_preview;` and exports |
| 2 | `src/edit_placeholders.rs` | `restore`, `Restored { moved }` |
| 2 | `src/edit_preview.rs` | `EditPreview`, `FieldChange`, `EditCheck`, `field_changes`, `build_preview` (markers, no header) |
| 2 | `src/object_write.rs` (+ tests) | `WriteOperation::ReplaceObject`, `WriteEffect::Replaced`, the sequence, the `replace` arm, manual `Debug` |
| 3 | `src/object_edit.rs` | `format_yaml` |
| 4 | `src/object_edit.rs` | `rebase`, `Rebased` |

## `crates/app`

| S | File | Change |
|---|---|---|
| 1 | `src/cluster_session.rs` (+ tests) | `kind_access` map, `request_kind_access(kind)` on first screen show, cleared on scope change |
| 1 | `src/resource_actions.rs` | the gate reads lazy checks from `kind_access` (`Checking permissions…` until known) |
| 3 | `src/yaml_edit.rs` (new) + `yaml_edit_tests.rs` | `YamlEditView` (load, editor, tabs, diff list, side panel, footer, Env values, Format, local checks), `fixture` |
| 3 | `src/yaml_diff.rs` (new, tests in module) | `DiffRow`, `DiffRowKind`, `diff_rows` |
| 3 | `src/app_shell.rs`, `src/workspace.rs` | `AppShell.edit`, `open_edit`, discard prompt before navigation and switches, workspace swap |
| 3 | `src/resource_actions.rs`, `src/resource_kind.rs`, `src/keyboard_navigation.rs` | `EditYaml` gate `Update(kind)` (**not shipped**), `Edit YAML` items, `Edit` `KindAction`s and E call `open_edit` |
| 3 | `src/keymap.rs` (+ tests) | `secondary-s` → `ApplyEdit` in `YamlEdit` |
| 3 | `src/launch_options.rs` (+ tests), `src/screenshot.rs`, `src/main.rs` | `--screen edit-yaml-diff`; `USAGE`; `mod yaml_edit; mod yaml_diff;` |
| 4 | `src/resource_actions.rs` | `EditYaml` shipped |
| 4 | `src/write_flow.rs` (+ tests), `src/confirm_dialog.rs` | `preview_write` (through `checked_write`), `GuardedIntent.on_commit`, close after commit, `ReplaceObject` dry-run line and object row |
| 4 | `src/yaml_edit.rs` | server preview, Apply, `on_commit`, banners, rebase, `server_changed`, `Failed(Invalid)` |
| 4 | palette (0029) | `Edit YAML` enabled through the same gate |

## Docs

| S | File | Change |
|---|---|---|
| now | `docs/roadmap/cross-cutting.md` C8 | YAML edits: replace with `resourceVersion`, no SSA, no Force (done with this amendment) |
| 2 | `docs/specs/0030-guardrails-write-path/write-path.md` | allow-list row `ReplaceObject` (requested from the 0032 architect) |
| 3 | `docs/specs/0028-keyboard-map/keymap.md` | Ctrl S (W10) bound in `YamlEdit` |
| 4 | `docs/roadmap/gap-plan-local-and-mutating.md`, `inventory-screens.md` | 0031 done; W10 partial (history, snapshot, templates, quota) |
