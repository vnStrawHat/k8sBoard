# 0016 · Files to touch

[Back to index](README.md). **S** is the step. Each step passes the gate on its own; nothing lands before its first user (the probe is the crate's first user in step 1; `SecretValue` and `secret_values` are used by the probe).

## Cargo (step 1)

| File | Change |
|---|---|
| `Cargo.toml` (root) | workspace deps `x509-cert` (0.3, no default features), `pem` 3, `zeroize` 1 |
| `crates/cluster/Cargo.toml` | the three, `.workspace = true` |
| `Cargo.lock` | expected new packages: `x509-cert`, `der`, `der_derive`, `spki`, `flagset` (AC 4) |

The app gains no cluster-side dependency (it uses `SecretValue` through the cluster crate). Step 3 adds the target-only `windows` 0.62 dependency to `crates/app/Cargo.toml` ([secret-clipboard.md](secret-clipboard.md)); it is already locked, so `Cargo.lock` does not change.

## `crates/cluster` (step 1)

| File | Change |
|---|---|
| `src/certificate.rs` (new) + `src/certificate_tests.rs` | `CertificateInfo`, `CertificateIssue`, `parse_certificates`, `CHAIN_LIMIT` |
| `src/secret.rs` (new) + `src/secret_tests.rs` | `SecretSummary`, `SecretKey`, `SecretDetails`, `SecretValue`, `secret_summary`, `watch_secrets`, `watch_tls_secrets`, `secret_values` |
| `tests/fixtures/tls/leaf.pem`, `chain.pem`, `garbage.pem` (new) | certificates only, made with Git Bash `openssl` in `.tmp/` |
| `src/pod.rs` | `image_pull_secrets` |
| `src/container_spec.rs` | `Projected.secrets` |
| `src/object_yaml.rs` (+ tests) | `ObjectKind::Secret`, `api_resource` arm |
| `src/object_count.rs` | `Secret` arm |
| `src/lib.rs` | modules and exports |
| `examples/probe.rs` | `secrets`, `tls secrets` watch lines, `count secrets`, `--secrets`, `USAGE` |

## `crates/app`

| S | File | Change |
|---|---|---|
| 1 | `src/main.rs` (+ tests in module) | `log_filter`, `KUBE_CLIENT_BODY_GUARD` (`kube_client::client=error` after the env filter); the crate step 1 needs this app change because it protects every existing watch |
| 2 | `src/resource_kind.rs` | `Secrets` spec, `ALL`, `watch_rows` |
| 2 | `src/kind_row.rs` | `KindObject::Secret`, `LiveContent::{SecretData, Certificate}`, `KindCell::Expiry` |
| 2 | `src/secret_rows.rs` (new) + `secret_rows_tests.rs` | `secret_row`, `secret_data_rows`, `MASK` |
| 2 | `src/certificate_expiry.rs` (new, tests in module) | `EXPIRY_WARNING`, `ExpiryState`, `expiry_state` (leaf), `expiry_label`, `intermediate_expires_first` |
| 2 | `src/config_map_rows.rs` | `format_bytes` → `pub(crate)` |
| 2 | `src/kind_table.rs`, `src/kind_drawer.rs` | `Expiry` cell paint and sort value; `Live(SecretData)` masked rows |
| 2 | `src/kind_join.rs` (+ tests) | `SECRET_USED_BY`, `secret_users`, `may_be_unused`, Secrets arm and triggers |
| 2 | `src/live_sections.rs` (+ tests) | `SecretData` (masked), `Certificate`, `UsedBy` Secret arm |
| 2 | `src/kind_diagnosis.rs` (+ tests) | Secret CERTIFICATE arm |
| 2 | `src/cluster_session.rs` (+ tests) | `Ingresses` companion variants, `companion_plan` arm, watch count |
| 2 | `src/main.rs` | `mod secret_rows; mod certificate_expiry;` |
| 3 | `src/secret_values.rs` (new) + `secret_values_tests.rs` | `SecretValuesView`, `SecretAction`, `ValueAccess`, `value_access`, `SecretCopied`, pure core |
| 3 | `src/drawer.rs` | `DrawerState.secret_values`, `pending_secret_action` |
| 3 | `src/app_shell.rs` (+ tests) | `secret_value_access` (one source), `sync_secret_values`, `run_secret_action`, `clipboard_clear`, `arm_clipboard_clear`, `on_app_quit` clear |
| 3 | `src/kind_drawer.rs` | place the view for `Live(SecretData)` |
| 3 | `src/resource_actions.rs` (+ tests) | Reveal values (30s), Copy value ▸ |
| 3 | `src/main.rs` | `mod secret_values; mod secret_clipboard;` |
| 3 | `src/secret_clipboard.rs` (new) + `secret_clipboard_tests.rs` | `ClipboardMark`, `write_private_text`, `clear_if_unchanged`, `#[cfg(windows)] mod windows_clipboard` (the only `#[allow(unsafe_code)]`) |
| 3 | `Cargo.toml` (app) | `[target.'cfg(windows)'.dependencies] windows = "0.62"` with four `Win32_*` features |
| 4 | `src/resource_kind.rs`, `src/network_rows.rs` (+ tests) | Ingress TLS column replaces Ports; `Live(IngressTls)` section |
| 4 | `src/kind_row.rs` | `LiveContent::IngressTls` |
| 4 | `src/kind_join.rs` (+ tests) | `INGRESS_TLS`, Ingresses arm, companion trigger |
| 4 | `src/live_sections.rs` (+ tests) | `IngressTls` |
| 4 | `src/kind_diagnosis.rs` (+ tests) | `DiagnosisInputs.tls_secrets`, Ingress CERTIFICATE arm |
| 4 | `src/cluster_session.rs` (+ tests) | `TlsSecrets` companion variants, `companion_plan` arm |
| 4 | `src/access_rows.rs` (0015) | ServiceAccount secret rows → `Link` |

## Docs (with the last step)

- `docs/roadmap/inventory-kinds.md`: Secrets → Done (Edit 0031, Delete 0033); Ingresses TLS column and CERTIFICATE box → Done.
- `docs/roadmap/cross-cutting.md`: C1 row notes "0016: per-key reveal, Copy without reveal (private on Windows, cleared after 30 s), zeroize ceiling; open question settled"; C6 x509 row → `x509-cert` 0.3; UAT table: secrets probe results.
- `docs/roadmap/inventory-screens.md` O2: cert-expiry data source is 0016 (`watch_tls_secrets`).
- `docs/specs/0005-kind-explorer/kind-columns.md` Ingresses row: Ports replaced by TLS in 0016.
- `docs/roadmap/README.md` status rows; 0001 `probe-example.md`: watch lines and `--secrets`.
