# 0042 · As built

[Back to index](README.md) · Steps 1-3, 2026-10-04, on main `277759a`. All ACs hold; the deviations are listed below.

## What was built

- **Cluster crate.** `object_create.rs`: `ObjectDraft`, `DraftError`, `DraftWarning`, `ObjectKind::is_creatable`, `missing_paths`, the capped `changed_fields`. `WriteOperation::CreateObject` (explicit `checked_operation` arm, `AccessCheck::Create(kind)`, name and namespace rules in `is_safe_path`, the `send` arm, `create_failure` for the 409 and 404 texts), `WriteOutcome.dropped_fields`, the 0030 allow-list row. `parse_mapping`, `refuse_leading_zero`, `SERVER_METADATA`, and `api_resource` are `pub(crate)` for reuse.
- **App.** `object_templates.rs` (five templates, `template_namespace`), `object_create_view.rs` (`ObjectCreateView`, `CreateCheck`, render, `create_intent`, `confirm_risk`), `OpenEdit::Create` with `discard_title` and `leaving_line` (`edit_yaml_flow.rs`, with `open_create` and `create_commit_finished`), the `New` header buttons (`workspace.rs`, `new_button_of`), `ResourceAction::CreateObject`, the lazy `Create(kind)` check, the `write_flow.rs` notice arm and Retry exclusion.
- **Fixture.** `--screen new-config-map` (screenshot builds): the ConfigMap template of `payments` with a passed dry-run of 212 ms, no request.

## Deviations from the spec text

1. **`ResourceAction::row_action` returns `Option<RowAction>`.** The `New` button is a header action with no row and no key; every other action keeps its row action. Menu items go through `keyed(item, action)`; the node shell item names `RowAction::OpenShell` directly. Tests that read `row_action()` unwrap it.
2. **`confirm_dialog.rs` was touched (three lines).** The spec lists it under "Do not touch" but also says the object row reads `{Kind} {ns/}{name} · new`; the row said `N fields changed`, which is wrong for a create, so a create shows `new` there. Nothing else changed in the dialog.
3. **`DraftError::InvalidMetadata` is new.** A `metadata` field of the wrong type (`labels: 5`) would otherwise fail inside `send` as "the object could not be used"; the draft now refuses it with a text that names the field group.
4. **Footer wording of the passed check.** `Dry-run OK · {ms} ms` (the Edit YAML footer adds `unchanged since you opened it`, which has no meaning for a template).
5. **`checked_operation_refuses_inconsistent_draft` lives in `object_create_tests.rs`.** It builds a draft with private fields in-crate, so it sits in the module that owns them.
6. **Test names and places.** `open_create_slot_has_no_object` and `cluster_switch_lists_unsaved_new_object` are in `app_shell_create_tests.rs` (they need a window); `new_buttons_on_the_five_screens` is in `workspace.rs`; `new_button_disabled_with_gate_reason` is in `resource_actions_tests.rs` (the pure gate) and `app_shell_create_tests.rs` covers it over a live fake cluster.
7. **A debug line.** `finish_kind_access` logs `is_create_allowed` next to the delete and patch answers, so the live check reads the real answer from the app's own log.
8. **The editor takes focus from `open_create`, not from `ObjectCreateView::new`.** A fixture that took focus in `new` tripped the leak check of the screenshot build at exit.
9. **`#[allow(clippy::disallowed_methods)]` on the new test module** `object_write_create_tests` in `object_write.rs`, like every `object_write_*_tests` module (0030 AC 9). No production `#[allow]` was added.
10. **Dropped fields list leaves, not roots.** A dropped subtree reports every leaf under it (`data.KEY`, `data.OTHER`); the confirm and the side panel show them as the spec words them.
11. **The ConfigMap `metadata.name` field has no value in the audit line.** `recordable_fields` (`PATH_ONLY_KINDS`) drops every value of a ConfigMap, the name included; the object column of the line holds the name.

## Live check (UAT, `readonly@Monitor`, writes blocked)

The app's own debug log (`RUST_LOG=kube_client::client::builder=debug,k8sboard::cluster_session=debug`) of `--screen namespaces|configmaps|resourcequotas|poddisruptionbudgets|rolebindings` on the screenshot build: per screen 114 GET, 116-120 SelfSubjectAccessReview POST, **0 other POST, 0 PATCH, 0 PUT, 0 DELETE**. The kind review logged `is_create_allowed=false` for Namespace, ConfigMap, ResourceQuota, PodDisruptionBudget, and RoleBinding, so each `New` is disabled with `Not permitted: create {resource}` (the gate text is tested offline). No UAT image or log was kept; the offline fixture screenshots are `v100-*-light|dark.png` (`new-config-map` and the five screens in their disconnected state).

## Not covered by a test

- The real `Create…` click path and the dialog's Retry button over a live window: the tests call `apply` and `press_confirm`.
- A commit transport failure end to end (the view's text is tested through `commit_failed`; the cluster crate tests the `OutcomeUnknown` mapping).
- The ui-verifier was not run.
