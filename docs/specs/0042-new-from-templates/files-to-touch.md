# 0042 · Files to touch

[Back to index](README.md)

## Step 1 (cluster crate only)

| File | Change |
|---|---|
| `crates/cluster/src/object_create.rs` (new) + `object_create_tests.rs` (new) | `ObjectDraft`, `DraftError`, `DraftWarning`, `ObjectKind::is_creatable`, `is_consistent` |
| `crates/cluster/src/object_edit.rs` | `parse_mapping`, `SERVER_METADATA` → `pub(crate)` (reuse only) |
| `crates/cluster/src/object_yaml.rs` | `api_resource` → `pub(crate)` (reuse only) |
| `crates/cluster/src/object_write.rs` | `CreateObject` variant, `name`, `checked_operation` arm, `fitting_access_check`, `is_safe_path`, `changed_fields`, `supports_dry_run`, `send` arm, the 409 arm in `write_error` |
| `crates/cluster/src/object_write_create_tests.rs` (new) | fake-transport tests, wired like the other `object_write_*_tests` modules (`#[allow(clippy::disallowed_methods)]` on the test module, as they are) |
| `crates/cluster/src/access_review.rs` | `AccessCheck::Create(ObjectKind)` (lazy, not in `ALL`), its target (verb `create`) |
| `crates/cluster/src/lib.rs` | module and exports (`ObjectDraft`, `DraftError`, `DraftWarning`) |
| `docs/specs/0030-guardrails-write-path/write-path.md` | allow-list row `CreateObject` |

## Step 2 (app crate)

| File | Change |
|---|---|
| `crates/app/src/object_templates.rs` (new) + tests in a `#[cfg(test)] mod` | `template_text` |
| `crates/app/src/object_create_view.rs` (new) + `object_create_view_tests.rs` (new) | `ObjectCreateView`, `CreateCheck`, render, flow, fixture |
| `crates/app/src/edit_yaml_flow.rs` | `OpenEdit::Create`; `open_create`, `create_commit_finished`; the discard prompt and slot helpers cover the new variant |
| `crates/app/src/write_flow.rs` | `commit_write` calls `create_commit_finished` for `CreateObject(_)` |
| `crates/app/src/audit_log.rs` | none expected (`audit_action` falls to `intent.button`); verify `Create` |
| `crates/app/src/resource_actions.rs` | `ResourceAction::CreateObject`, gate, label, risk |
| `crates/app/src/kind_access.rs` | `lazy_checks` adds `Create(kind)` for creatable kinds |
| `crates/app/src/workspace.rs` | `New` / `New namespace` header buttons on the five screens |
| `crates/app/src/leaving_work.rs` | `Unsaved new {Kind}` line |
| `crates/app/src/app_shell.rs` | render arm for `OpenEdit::Create`, module declarations |
| `crates/app/src/launch_options.rs`, `screenshot.rs` | `--screen new-config-map` |
| `crates/app/src/app_shell_create_tests.rs` (new) | shell flows over a fake cluster |

## Step 3

No code. UAT run and ui-verifier; as-built notes in the README; gap audit row "W7 5 kinds · New" updated.

## Do not touch

`clippy.toml` (no new exception), `confirm_dialog.rs` (renders `changed_fields()` and warnings already), `yaml_edit.rs` (decision 3), `values_edit*.rs`, `docs/specs/0040-*`.
