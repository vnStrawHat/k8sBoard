# 0016 · Test plan

[Back to index](README.md). Unit tests are offline and deterministic; fixtures are k8s-openapi structs with `..Default::default()` and the committed PEM certificates ([cluster-api.md](cluster-api.md)). Names are binding (AC 2). "Distinctive value" = a literal like `s3cr3t-0016-VALUE` that appears nowhere else.

## Step 1 — `crates/cluster`

| Module | Tests |
|---|---|
| `certificate_tests.rs` | `leaf_fields_are_read` (subject, issuer, SANs DNS + IP, both dates equal the `openssl` output), `chain_keeps_file_order`, `chain_is_cut_at_limit`, `non_certificate_blocks_are_skipped` (a `PRIVATE KEY` block before the cert), `garbage_is_unparsed`, `empty_input_is_unparsed`, `one_bad_block_makes_chain_unparsed` |
| `secret_tests.rs` | `secret_summary_drops_values` (`format!("{summary:?}")` lacks the distinctive value, for Opaque, TLS, docker, token types), `keys_sorted_with_sizes_and_binary_flag` (one non-UTF-8 key), `type_defaults_to_opaque`, `tls_details_parse_tls_crt` (`Certificate { chain }`, non-empty), `tls_without_crt_is_missing` (`NoCertificate(Missing)`), `tls_key_is_never_read` (a `tls.key` holding the distinctive value; summary `Debug` lacks it), `docker_config_json_keeps_hosts_only` (auth/password distinctive values absent), `docker_cfg_keeps_top_level_hosts`, `bad_docker_config_gives_no_hosts`, `token_account_from_annotation`, `other_annotations_are_ignored`, `immutable_and_owned_flags`, `secret_watch_config_pages_most_recent` (`list_semantic` MostRecent, `page_size` 50; TLS variant adds the field selector) |
| `secret_tests.rs` (values) | `secret_value_text_and_binary` (`as_text` `Some`/`None`, `size_bytes`), `values_are_sorted_by_key` (pure helper that converts a decoded `Secret` into `Vec<SecretValue>`, also used by `secret_values`), `undecodable_body_gives_fixed_error` (pure `decode_secret(`values_are_sorted_by_key` (pure helper that converts a `Secret` into `Vec<SecretValue>`, also used by `secret_values`)str)`: a body holding the distinctive value fails with `UnexpectedResponse` whose `Display` lacks it), `secret_request_targets_namespaced_path` (`Request::get` URI is `/api/v1/namespaces/{ns}/secrets/{name}`) |
| `pod.rs` / `container_spec.rs` tests | `image_pull_secret_names_are_kept`, `projected_volume_keeps_secret_names` |
| `object_yaml_tests.rs` | `secret_kind_has_name_and_scope`, `secret_yaml_hides_data_values` (existing rule, now through `ObjectKind::Secret`) |

Review items (no test possible): `SecretValue` derives nothing and has private fields; no `tracing::` in `secret.rs`, `certificate.rs`; `secret_values` uses `request_text` (never `Api::get` or `Client::request`), keeps the body in `Zeroizing`, zeroizes annotations, and moves `ByteString` bytes without a copy; the probe installs no tracing subscriber.

| `crates/app` (step 1) | Tests |
|---|---|
| `main.rs` tests | `kube_client_body_warning_is_suppressed` ([secret-safety.md](secret-safety.md) "Log filter"), `other_warnings_still_pass_the_filter` |

## Step 2 — Secrets kind

| Module | Tests |
|---|---|
| `secret_rows_tests.rs` | `secret_row_cells_match_column_count`, `secret_status_is_type_or_not_parsed`, `data_section_is_live_only` (the Data section is exactly `[Live(SecretData)]`, C7), `masked_rows_show_mask_size_and_binary`, `token_secret_links_service_account`, `registries_as_chips` |
| `certificate_expiry.rs` | `expiry_uses_leaf_not_after` (an earlier intermediate does not change the state), `intermediate_expiring_first_is_reported`, `later_intermediate_is_not_reported`, `expired_state_and_label`, `expiring_within_fourteen_days`, `fifteen_days_is_valid`, `not_yet_valid_state` |
| `kind_diagnosis_tests.rs` | `secret_certificate_expired_box`, `secret_certificate_expiring_box`, `secret_not_parsed_and_missing_boxes`, `valid_certificate_has_no_box`, `opaque_secret_has_no_box` |
| `kind_join_tests.rs` | `joined_column_indices_name_their_columns` (adds `SECRET_USED_BY`), `secret_users_by_env_env_from_volume_projected_pull`, `secret_users_include_ingress_tls`, `secret_unused_only_when_eligible` (Opaque unused; TLS, Helm, token, owned never), `secret_used_by_absent_until_lists_ready` |
| `live_sections_tests.rs` | `certificate_section_rows`, `certificate_section_warns_early_intermediate`, `certificate_alt_names_capped`, `secret_used_by_note_when_unused` |
| `kind_table.rs` tests | `expiry_cell_sorts_by_not_after` |
| `cluster_session_tests.rs` | `companion_plan_per_kind` (adds Secrets → Ingresses, denied), `open_watch_count_with_secret_companion` (≤ `3N + 4`) |
| `navigation.rs` | enabled list adds Secrets |

## Step 3 — Reveal and Copy

| Module | Tests |
|---|---|
| `secret_values_tests.rs` | `reveal_all_keeps_every_value`, `reveal_one_keeps_only_that_key`, `copy_keeps_only_that_key`, `missing_key_is_an_error`, `reveal_replaces_same_key_and_resets_timer`, `expire_drops_only_due_values`, `expire_reports_no_change`, `display_cuts_at_char_boundary` (multi-byte char at 4096), `binary_value_displays_size`, `seconds_left_rounds_up` |
| `secret_values_tests.rs` (gpui) | `blocked_view_ignores_actions` (headless `TestAppContext`: `run` on a `Blocked` view starts no request), `set_keys_drops_values_of_removed_keys` |
| `secret_clipboard_tests.rs` | `mark_matches_same_text_only`, `marks_use_distinct_keys` (two marks of one text hash differently), `clear_skipped_when_clipboard_changed`, `clear_runs_when_clipboard_unchanged` (headless `TestAppContext` clipboard; non-Windows path), `clear_delay_is_thirty_seconds` |
| `app_shell_tests.rs` | `secret_view_drops_on_tab_change`, `secret_view_drops_on_subject_change`, `pending_action_runs_for_its_subject_only`, `copy_arms_clipboard_clear_and_survives_drawer_close`, `new_copy_replaces_armed_clear` |
| `resource_actions_tests.rs` | `secret_menu_order`, `copy_submenu_disables_binary_keys`; `#[cfg(feature = "screenshot")]` `secret_menu_blocked_in_screenshot_runs` (launch options with a screenshot output → `value_access` is `Blocked` → menu Reveal and Copy disabled; the same field builds the view) |

Review items: the Windows FFI writes text and the three formats in one `OpenClipboard` session, closes it on every path (guard `Drop`), fails closed with no `write_to_clipboard` fallback, and carries the only `#[allow(unsafe_code)]` with `// SAFETY:` comments; `value_access` is the only reader of the screenshot option for secrets. Run `cargo test -p k8sboard --features screenshot` for the gated test.

## Step 4 — Ingresses TLS

| Module | Tests |
|---|---|
| `network_rows_tests.rs` | `ingress_row_cells_match_column_count` (TLS replaces Ports), `ingress_tls_cell_before_join` |
| `kind_join_tests.rs` | `ingress_tls_earliest_leaf_expiry`, `ingress_tls_missing_secret`, `ingress_tls_not_parsed`, `ingress_tls_default_cert`, `ingress_tls_unjoined_without_companion` |
| `kind_diagnosis_tests.rs` | `ingress_certificate_box_worst_secret`, `ingress_missing_tls_secret_box`, `ingress_box_waits_for_companion` |
| `live_sections_tests.rs` | `ingress_tls_rows_link_secret`, `ingress_tls_note_when_denied` |
| `cluster_session_tests.rs` | `companion_plan_per_kind` (adds Ingresses → TlsSecrets) |
| `access_rows_tests.rs` | `service_account_secret_names_link_to_secrets` |

## Live checks (coder-lite, UAT `readonly@Monitor`)

1. Step 1: `probe --watch-seconds 5 --counts --secrets`; fill [decisions.md](decisions.md) "UAT probe"; the AC7 credential script reports 0. Inspect the probe output for any value: there must be none (counts, dates, names only).
2. Step 2: `secrets`, `secrets-drawer --filter {first tls secret}`, `secrets-yaml` (masked header present).
3. Step 3: the coder's Reveal/Copy check in [secret-safety.md](secret-safety.md) (no screenshots); coder-lite only confirms `--screenshot` shows disabled Reveal/Copy.
4. Step 4: `ingresses`, `ingresses-drawer` (`--filter` an ingress with TLS if UAT has one; else record "no TLS ingress on UAT" and rely on unit tests).

## ui-verifier (steps 2–4)

Screens above in light, `secrets-drawer` in dark. Check against W7: Secrets columns Type, Keys, Used by, Age; muted `unused`; drawer sections Secret, Data (dots, sizes, disabled Reveal/Copy with tooltip), Certificate, Used by; CERTIFICATE box tone; Ingress TLS column tones (`expires in 6d` Warn, `64d left` Ok); TLS section Secret link; badge `Se`; no hardcoded colors. **Any visible value in a Data cell is high severity** ([secret-safety.md](secret-safety.md)).
