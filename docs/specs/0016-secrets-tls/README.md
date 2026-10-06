# 0016 — Secrets and TLS expiry (read-only)

Status: done (steps 1-4, read-only); amended after the security review (HEAD `6de733c`); architect defaults, the user asked not to stop for questions. Crates: `crates/cluster` (step 1), `crates/app` (steps 2–4). Requires 0012–0015 merged (`KindObject`, `Live`, joins, general `KindList.companion`, `kind_diagnosis.rs`, `selected_summary_watch`, counts, `Projected { config_maps }`, `go_to_item`, ServiceAccounts). Wireframes: W7 `k("Secrets")`, `k("Ingresses")` (TLS column, CERTIFICATE box, TLS section). Applies **C1** (blocking), C6, C7, C11. Risk: High (real credentials; UAT allows `list secrets`).

## Goal

- **Secrets** explorer kind: Type, Keys, **Used by** (pods and ingresses), **unused** flag, Age; drawer with masked **Data** (key names and sizes), per-key **Reveal** for 30 s, **Reveal all** per drawer, **Copy** without reveal (private on Windows, auto-cleared after 30 s); type views: TLS certificate (subject, issuer, SANs, validity), docker registry hosts, service-account link; **CERTIFICATE** box.
- **Ingresses**: **TLS** column with expiry (replaces Ports, as W7), TLS section with secret link, issuer, not-after, and a **CERTIFICATE** box.
- **Ingresses** also get a **NO ADDRESS** box (Warn) before the certificate box: older than 5 min with no load-balancer address, `No address: no ingress controller has picked it up (class X: no controller reports it)` or `(no ingressClassName; only a default IngressClass would pick it up)`. The default IngressClass is not read.
- A parsed-certificate model the Overview/Issues feed (0020/0021) reuses.

## Non-goals

Edit, Delete, New (0031/0033; disabled); revealing in the YAML tab (0007 decision 11); the Overview "Cert expiring" feed itself (0020/0021, contract in [tls-expiry.md](tls-expiry.md)); cert-manager facts ("has not renewed", issuer resource name: 0018); Helm release decoding (0017); an expiry signal in the Secrets list (0020/0021).

## Implementation steps

| Step | Scope | ACs |
|---|---|---|
| 1 | `crates/cluster`: `certificate.rs`, `secret.rs` (summary, two watches, `secret_values`), pod `image_pull_secrets`, `Projected.secrets`, `ObjectKind::Secret`, count arm, paged watches, GET decoded in-crate, app log filter `kube_client::client=error`, probe `--secrets`; deps `x509-cert`, `pem`, `zeroize`. **Run the UAT probe** and fill [decisions.md](decisions.md) "UAT probe" | 1, 2, 3, 4, 5 |
| 2 | App: Secrets kind (rows, columns, masked Data, Certificate, Used by with the Ingresses companion, CERTIFICATE box, `KindCell::Expiry`). No Reveal or Copy yet | 1, 2, 3, 6, 8 |
| 3 | App: `SecretValuesView` (Reveal, Reveal all, Copy, timers), private clipboard write and 30 s auto-clear (`windows` FFI), menus, pending actions, one screenshot gate | 1, 2, 3, 7, 8 |
| 4 | App: Ingress TLS column, TLS-secrets companion, TLS section, Ingress CERTIFICATE box, ServiceAccount secret links; full ui-verifier run; doc updates | 1, 2, 3, 6, 8, 9 |

## Files

| File | Contents |
|---|---|
| [decisions.md](decisions.md) | numbered decisions (1–30, not in table order) with rationale, ceilings, UAT probe table |
| [secret-safety.md](secret-safety.md) | C1 applied: where plaintext lives, for how long, hygiene ceiling, logging, screenshots |
| [cluster-api.md](cluster-api.md) | step 1: summaries, certificate parsing, watches, `secret_values`, deps, probe |
| [secrets-kind.md](secrets-kind.md) | step 2: kind spec, columns, sections, Used by join, companion, box, menus |
| [secret-values-view.md](secret-values-view.md) | step 3: reveal/copy entity, async contract, timers, render, AppShell wiring |
| [secret-clipboard.md](secret-clipboard.md) | step 3: private clipboard write (Windows formats), auto-clear, ceilings |
| [tls-expiry.md](tls-expiry.md) | steps 2 and 4: expiry rules, `Expiry` cell, Ingress TLS, 0020/0021 contract |
| [files-to-touch.md](files-to-touch.md) | modules and Cargo changes per step, doc updates |
| [test-plan.md](test-plan.md) | unit tests per step, live checks, ui-verifier checklist |

## Acceptance criteria

- [x] 1. The quality gate passes, plus `cargo clippy -p k8sboard --features screenshot --all-targets -- -D warnings`. No new `#[allow]` except the one `#[allow(unsafe_code)]` on `mod windows_clipboard` ([secret-clipboard.md](secret-clipboard.md)), with `// SAFETY:` on every block.
- [x] 2. Every test of the step in [test-plan.md](test-plan.md) exists under that name and passes offline. Some certificate, ingress-row and menu tests live in the module that owns the code (secret_rows_tests.rs, network_rows_tests.rs, resource_actions_tests.rs) instead of the file the plan names.
- [x] 3. No kube, k8s-openapi, x509-cert, or zeroize type in a public signature; the crate spawns no task; the app gains no kube dependency; the 0001 read-only grep finds only the SSAR `create`; new requests are `list`/`watch`/`get` only.
- [x] 4. Step 1 `Cargo.lock` gains exactly the x509-cert set (expected 5: `x509-cert`, `der`, `der_derive`, `spki`, `flagset`); `pem`, `zeroize`, `const-oid` are already locked. Anything else: stop and report. Steps 2–4 leave `Cargo.lock` unchanged (`windows` 0.62 is locked). Actual: 6 lock entries; `base64ct` is a weak optional dependency of `spki`, never compiled (cargo #10801), accepted by the security review ([decisions.md](decisions.md)). Steps 2-4 added only dependency edges (`windows`, `zeroize`) for the app, no package.
- [ ] 5. Secret safety ([secret-safety.md](secret-safety.md) checklist): no `tracing::` in the listed modules; the log filter test passes; `secret_values` uses no `Api::get` or `Client::request`; `SecretValue` has no `Debug`, `Display`, or `Clone`; summaries hold no value (tests); the probe prints counts and metadata only; the AC7 credential script reports 0. The 0001 AC7 credential script was not run; the substitutes are the grep and test checks recorded in the step reports.
- [x] 6. On UAT: Secrets shows live rows with Type, Keys, Used by; a TLS secret drawer shows subject, SANs, not-after; Ingresses show the TLS column (or "—" when none use TLS).
- [ ] 7. Reveal shows a value and re-masks it after 30 s; leaving the drawer, tab, or screen drops it at once; Copy fills the clipboard without revealing, stays out of Win+V history, and is cleared after 30 s unless replaced (coder live check, never screenshotted). With `--screenshot`, Reveal and Copy are disabled. **Manual user step**: agents never touch the real clipboard or reveal a real value ([secret-safety.md](secret-safety.md)). Covered offline by the fake-port and gate tests; with `--screenshot` Reveal and Copy are disabled (verified).
- [x] 8. Watches per session stay at most `3N + 4` (`open_watch_count` covers both companions). The 0003 AC4 color-literal grep is clean.
- [x] 9. The step's screenshots exist, show masked values only, and the ui-verifier reports no high-severity defect against W7. Coder-viewed (v37-v40, light and dark); ui-verifier 2026-10-02 (v40v shots): no high-severity defect; SA secret links and Warn/expired tones have no UAT data and rely on unit tests.

## Open items

1. The Overview/Issues cert feed: always-on `watch_tls_secrets(All)` recommended ([tls-expiry.md](tls-expiry.md)); 0020 decides under the C13 budget.
2. Second allowlisted annotation `kubernetes.io/service-account.name` (decision 12); confirm with the user like 0014 decision 4.
3. macOS `org.nspasteboard.ConcealedType` for clipboard managers needs AppKit FFI; not done ([secret-clipboard.md](secret-clipboard.md) ceilings).
