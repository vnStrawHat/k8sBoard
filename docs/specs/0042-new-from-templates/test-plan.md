# 0042 · Test plan

[Back to index](README.md) · All tests offline; no test talks to a real cluster.

## Step 1 · `object_create_tests.rs`

| Test | Checks |
|---|---|
| `only_five_kinds_are_creatable` | `is_creatable` true for the five, false for every other `ObjectKind::ALL` entry |
| `draft_of_other_kind_is_refused` | `ObjectDraft::new(Secret, ..)` → `NotCreatable` |
| `draft_refuses_wrong_kind_and_api_version` | `kind: Secret` in a ConfigMap draft; `apiVersion: policy/v1beta1` for a PDB |
| `draft_refuses_server_fields` | `status`, `metadata.uid`, `resourceVersion`, `creationTimestamp`, `managedFields`, `ownerReferences` |
| `draft_refuses_generate_name_and_missing_name` | `GenerateName`, `MissingName` |
| `draft_name_rules_per_kind` | Namespace `a.b` → `InvalidName`; ConfigMap `a.b` ok; RoleBinding `system:x` ok; RoleBinding `a/b` refused |
| `draft_namespace_rules` | ConfigMap without namespace → `MissingNamespace`; Namespace with one → `UnexpectedNamespace` |
| `draft_refuses_hidden_placeholder` | `data.K: <hidden>` → `Placeholder { "data.K" }` |
| `draft_reports_syntax_and_size` | bad YAML → `Text(Syntax)`; > 2 MiB → `Text(TooLarge)`; `0755` → `Text(LeadingZero)` |
| `role_binding_warnings` | ClusterRole `cluster-admin` → `PowerfulRole`; Group `system:authenticated` → `BroadSubject`; `view` → none |
| `draft_debug_holds_no_content` | `Debug` has kind, namespace, name; not a data value |

## Step 1 · `object_write_create_tests.rs` (fake transport)

| Test | Checks |
|---|---|
| `dry_run_request_shape` | `POST /api/v1/namespaces/payments/configmaps`, query `dryRun=All&fieldManager=k8sboard`, `application/json`, body equals the draft |
| `commit_request_has_no_dry_run` | query `fieldManager=k8sboard` only; outcome `Created`, `created_name`, `uid` |
| `namespace_create_posts_to_cluster_collection` | `POST /api/v1/namespaces` |
| `each_kind_posts_to_its_collection` | PDB `/apis/policy/v1/…/poddisruptionbudgets`, ResourceQuota, RoleBinding paths |
| `blocked_policy_sends_nothing` | `WritesBlocked`, zero requests |
| `request_needs_matching_target_and_creatable_kind` | another target, or a draft built for a creatable kind used on a Secret target → `None` |
| `checked_operation_refuses_inconsistent_draft` | a draft whose body name differs from its target (built in-crate) → `None` |
| `access_check_is_create_of_kind` | `Create(ConfigMap)`; text `create configmaps`; namespaced flag per kind |
| `already_exists_reads_as_invalid_name` | 409 `AlreadyExists` → `Invalid { "ConfigMap new-config already exists", ["metadata.name"] }` |
| `changed_fields_never_hold_config_map_values` | ConfigMap: `data[KEY]` paths with `None`; RoleBinding: `roleRef`, `subjects` |
| `commit_timeout_is_outcome_unknown` | transport error on commit → `OutcomeUnknown` |
| `allow_list_matches_the_operations` | this module's own table test (as in the other write test modules): method, path, query, content type, body of `CreateObject` |

## Step 2 · app

| Test | File | Checks |
|---|---|---|
| `every_template_is_a_valid_draft` | `object_templates.rs` | each of the five parses with `ObjectDraft::new` for `payments` |
| `template_uses_single_scoped_namespace_else_default` | same | `Named(["payments"])` → `payments`; `All` → `default` |
| `new_buttons_on_the_five_screens` | `object_create_view_tests.rs` or workspace tests | labels `New namespace` / `New`; none on Secrets |
| `new_button_disabled_with_gate_reason` | same | `Checking permissions…`, `Not permitted: create configmaps`, read-only lock |
| `lazy_checks_ask_create_for_creatable_kinds` | `kind_access_tests.rs` | ConfigMap → Update, Patch, Delete, Create; Deployment → no Create |
| `local_error_blocks_before_request` | `object_create_view_tests.rs` | `Failed(Local)`, zero requests |
| `ctrl_s_during_dry_run_does_nothing` | same | no second request |
| `changed_text_needs_new_dry_run` | same | `Passed` stale after an edit; Ctrl S runs a dry-run, not the dialog |
| `passed_dry_run_opens_confirm_with_warnings` | `app_shell_create_tests.rs` | `WriteIntent` action, label, button `Create`, warnings |
| `commit_closes_and_reveals` | same | view closed, notice `Created ConfigMap payments/new-config`, reveal key |
| `outcome_unknown_keeps_view_without_retry` | same | footer text; no Retry |
| `dirty_cancel_asks_clean_cancel_closes` | same | prompt only when the text differs from the template |
| `cluster_switch_lists_unsaved_new_object` | `leaving_work` tests | `Unsaved new ConfigMap` |
| `audit_line_for_create` | `audit_log_tests.rs` | action `Create`, ConfigMap field values absent |

## Step 3 · live and visual

- UAT (debug build, `readonly@Monitor`, `RUST_LOG=cluster=debug`): open each of the five screens; every `New` reads `Not permitted: create {resource}`; the trace shows only GETs, LISTs, watches, and SSAR POSTs; no POST to a collection.
- ui-verifier: `--screen new-config-map` and the five screen headers, light and dark, against W7 and W10.
