# 0001 · Test plan (kubeconfig, connection, namespaces, nodes, access, integration)

[Back to index](README.md) · Pod tests: [test-plan-pods.md](test-plan-pods.md)

## Conventions

- Tests are deterministic and offline.
- k8s-openapi objects are built with struct literals plus `..Default::default()`, through small fixture functions local to each test file.
- No shared fixture module, no JSON fixtures, no new dev-dependencies.
- `collect_pages` tests run with `futures::executor::block_on`, so they need no tokio runtime.

## Fixture `tests/fixtures/kubeconfig.yaml`

Fake credentials only:

- `current-context: missing-context`, mirroring the UAT file.
- Clusters `alpha` and `beta`: `server: https://127.0.0.1:1`, `insecure-skip-tls-verify: true` (no dependency on the host trust store; no test does a TLS handshake).
- Users:
  - `alpha-user`: `token: fixture-token-do-not-print`
  - `beta-user`: `username: fixture-user`, `password: fixture-password-do-not-print`
- Contexts:
  - `alpha` (alpha, alpha-user, namespace `team-a`)
  - `beta` (beta, beta-user)
  - `broken` with no `context:` body

Unit tests load the fixture through `include_str!("../tests/fixtures/kubeconfig.yaml")` → `kube::config::Kubeconfig::from_yaml` → the private `Kubeconfig::from_document`. Inline YAML variants are fine for other current-context cases.

## `src/kubeconfig_tests.rs`

| Test | Pins |
|---|---|
| `contexts_list_usable_contexts_in_file_order` | alpha, beta; `broken` skipped |
| `context_summary_carries_cluster_user_and_namespace` | field mapping; beta namespace `None` |
| `contexts_keep_first_of_duplicate_names` | first entry wins |
| `current_context_returns_raw_value_even_if_missing` | `missing-context` |
| `current_context_treats_empty_value_as_unset` | `""` → `None` |
| `resolve_context_prefers_requested_name` | `Some("beta")` wins over current-context |
| `resolve_context_falls_back_to_current_context` | valid current-context + `None` |
| `resolve_context_reports_unknown_requested_context` | `ContextNotFound { origin: Requested, available: [alpha, beta] }` |
| `resolve_context_reports_invalid_current_context` | UAT case, `origin: CurrentContext` |
| `resolve_context_without_current_context_reports_no_context_selected` | with available list |
| `context_not_found_message_names_context_and_lists_available` | Display has `current-context 'missing-context'` and `alpha, beta` |
| `context_list_renders_none_when_empty` | `(none)` |
| `debug_output_never_contains_credentials` | no `do-not-print` in `{:?}`; contains `alpha` |
| `parse_error_does_not_quote_file_content` | broken YAML holding the token → `load_error` → no `do-not-print` in Display or source chain |
| `load_reports_missing_file_with_path` | `Read`; message has the path |

## `src/connection.rs` (inline)

| Test | Pins |
|---|---|
| `api_status_401_maps_to_unauthorized` | `Status::failure(..).with_code(401)` |
| `api_status_403_maps_to_forbidden` | message preserved |
| `api_status_other_maps_to_api_with_code` | 503 → `Api { code: 503 }` |
| `auth_error_maps_to_credentials_unavailable` | any public `kube::client::AuthError`; no detail in Display |
| `cluster_error_message_names_context_and_action` | Display text |
| `list_all_follows_continue_tokens_until_empty` | 3 scripted pages, the last token `""`; order kept; later requests carry the token; `limit == 500` |
| `list_all_restarts_once_when_continue_token_expires` | 410 on page 2 → restart without a token; no duplicate items |
| `list_all_surfaces_second_expired_continue_token` | 410 on page 2 twice → `Err(Api { code: 410 })` |
| `list_all_does_not_retry_first_page_error` | first-page error returned; fetch called once |

## `src/namespace.rs` (inline)

| Test | Pins |
|---|---|
| `namespace_phase_maps_active_terminating_and_unknown` | Active, Terminating, other text, missing |
| `namespace_summary_reads_name_and_creation_time` | field mapping |

## `src/node_tests.rs`

| Test | Pins |
|---|---|
| `ready_condition_true_is_ready` / `ready_condition_false_is_not_ready` | readiness |
| `ready_condition_unknown_is_unknown` / `missing_ready_condition_is_unknown` | `Unknown` |
| `unschedulable_node_has_scheduling_disabled` | `Some(true)` → `Disabled`; `Some(false)`/`None` → `Enabled` |
| `roles_come_from_node_role_label_suffixes_sorted` | sorted suffixes |
| `roles_include_kubernetes_io_role_label_value` | legacy label value, deduplicated |
| `roles_ignore_empty_suffix_and_unrelated_labels` | empty suffix ignored |
| `roles_are_empty_without_role_labels` | `[]`, no `worker` inference |
| `taint_with_value_displays_key_value_effect` / `taint_without_value_displays_key_effect` | Display; empty value treated as `None` |
| `internal_ip_is_first_internal_ip_address` | skips Hostname/ExternalIP; `None` if absent |
| `node_summary_reads_kubelet_version_and_creation_time` | missing nodeInfo → `""` |

## `src/metrics_api.rs` and `src/access_review.rs` (inline)

| Test | Pins |
|---|---|
| `metrics_group_uses_preferred_version` / `metrics_group_falls_back_to_first_version` / `metrics_group_absent_returns_none` | group-version choice |
| `all_checks_cover_nine_distinct_permissions` | `ALL.len() == 9`, all distinct |
| `pod_log_check_uses_get_on_pods_log` / `exec_and_port_forward_checks_use_create` | attributes |
| `named_scope_sets_namespace_on_namespaced_checks` / `all_scope_leaves_namespace_unset` / `cluster_scoped_checks_ignore_scope` | namespace handling |
| `allowed_status_is_allowed` / `denied_status_carries_reason` / `evaluation_error_is_used_when_reason_is_empty` / `missing_status_is_denied_without_reason` | decision mapping |
| `report_is_allowed_looks_up_check` / `check_display_matches_kubectl_wording` | report and Display |

## Integration `tests/connection.rs` (public API, no cluster)

| Test | Pins |
|---|---|
| `open_fixture_context_succeeds_without_network` | `#[tokio::test]`; alpha → namespace `team-a`; beta → `default` |
| `open_unknown_context_returns_kubeconfig_error` | `ClusterError::Kubeconfig(ContextNotFound { .. })` |
| `connection_and_query_futures_are_send` | public types are `Send + Sync + 'static`; the `list_pods` and `review_access` futures are `Send` (created, then dropped unpolled) |
| `connection_debug_hides_credentials` | no `do-not-print` in `{:?}` |
| `server_version_against_closed_port_is_unreachable` | `127.0.0.1:1` → `Unreachable` (about 2 s on Windows) |
