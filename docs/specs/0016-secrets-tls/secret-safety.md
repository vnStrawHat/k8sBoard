# 0016 · Secret safety (C1 applied)

[Back to index](README.md) · All steps. This page is the reviewer's checklist; every rule maps to a test or a review item in [test-plan.md](test-plan.md).

## Where plaintext exists, and for how long

| Place | Holds | Lifetime | Wiped? |
|---|---|---|---|
| kube/hyper/serde buffers of the Secrets watch | every Secret in scope, ≤ 50 per list page (decision 25) | one event or one page | no (freed) |
| `secret_summary(&Secret)` input | one object, borrowed from the watcher | one call | no (owned by kube) |
| dockerconfig JSON parse | one `serde_json::Value` with `auths` | one call | no (freed) |
| TLS-secrets companion (Ingresses screen) | every TLS Secret in scope, `tls.key` included, ≤ 50 per page | one event or page | no (freed) |
| YAML tab GET (0007) | one object | until masked, same task | no (freed) |
| `secret_values` body | the response text (`Zeroizing<String>` from `request_text`) | one call | **wiped** (hyper's chunk buffers and the `Bytes` → `String` copy inside kube are freed) |
| `secret_values` decoded `Secret` | data and annotations | one call | data moved into `SecretValue`; annotation strings zeroized; rest freed |
| `SecretValue` in `SecretValuesView` | revealed keys only | ≤ 30 s, or until the view drops | yes, on drop |
| GPUI paint of a revealed value | `SharedString` copies, shaped-line cache | a few frames after hide | no (outside our control) |
| OS clipboard after Copy | one value | ≤ 30 s, then cleared if unchanged ([secret-clipboard.md](secret-clipboard.md)) | cleared, not wiped |
| Windows clipboard history / cloud clipboard | nothing: excluded by the three formats | — | — |
| macOS / Linux clipboard managers | may keep a copy | outside our control | no |

**Ceiling, stated honestly**: zeroize wipes only the copies k8sBoard owns (the GET body, the decoded values, the 30 s reveal store). Watch transport buffers, deserialization temporaries, GPUI text caches, and clipboard managers outside Windows keep freed or foreign copies. The design minimizes **how long** and **where** values are held: never in summaries, never cached, never across a subject change, clipboard cleared after 30 s.

## Log filter (M1a, step 1)

kube-client 4.2 `Client::request` logs the whole body at `warn` when JSON decoding fails; `Api::get`, `Api::list`, and the watcher's initial list go through it. A Secret list that fails to decode would print every value.

- `crates/app/src/main.rs`: `fn log_filter(env: EnvFilter) -> EnvFilter` returns `env.add_directive(KUBE_CLIENT_BODY_GUARD)` with `const KUBE_CLIENT_BODY_GUARD: &str = "kube_client::client=error"` parsed once; `main` passes `EnvFilter::from_default_env()`. Added **after** the env filter, so `RUST_LOG` cannot lower it (same-target directives are replaced).
- Test `kube_client_body_warning_is_suppressed` (main.rs tests): with env `kube_client::client=trace,info`, a `tracing::warn!(target: "kube_client::client", ..)` under a buffer-writing subscriber built from `log_filter` produces no output; a `warn!` with another target still does.
- `examples/probe.rs` installs **no** tracing subscriber, so kube's events are discarded. Review item: the probe keeps no subscriber; if one is ever added it must use the same directive.

## Rules (each is a review item)

1. **Summaries hold no value.** `SecretSummary` keeps names, sizes, `is_binary`, type, and decisions 9–12 details; it derives `Debug`, and a test proves a distinctive value is absent from `format!("{summary:?}")`.
2. **`SecretValue`** (cluster): no `Debug`, `Display`, `Clone`, `Serialize`, `PartialEq`; private fields; bytes in `Zeroizing<Vec<u8>>`. The app converts it to `String` only for the paint copy and the clipboard write.
3. **No logging**: no `tracing::` call in `secret.rs`, `certificate.rs` (cluster), `secret_rows.rs`, `secret_values.rs`, `secret_clipboard.rs`, `certificate_expiry.rs` (app). `CertificateIssue` has no payload; `secret_values` errors are `run`'s `ClusterError` or fixed `UnexpectedResponse` text. kube's own body log is filtered (above).
4. **No persistence**: nothing in these modules writes a file; no value reaches settings, presets, or the audit log (C10).
5. **Masked by default everywhere**: tables never show values; Data rows show `••••••••••`; the YAML tab masks `data`/`stringData` and manifest annotations (0007 rules 2–3, keyed on the response `kind`).
6. **Reveal lifetime**: each value hides at `shown_at + 30 s`; a 1 s ticker drops it. The view (and every value) drops on subject, tab, screen, scope, or context change and on drawer close.
7. **No cache**: every Reveal and Copy is a fresh GET; nothing from a previous action is kept.
8. **Copy without reveal**: Copy writes the clipboard (Windows: excluded from history and cloud) and drops the values in the same update; the row stays masked; the 30 s clear is armed in `AppShell`.
9. **Screenshots**: one source, `AppShell.secret_value_access` (decision 22), blocks Reveal and Copy in the view and the menus under `--screenshot`. No launch option reveals or copies. The ui-verifier never clicks Reveal or Copy.
10. **Probe**: `--secrets` prints counts, types, dates, and `{ns}/{name}` only; `secret values` prints key count and total bytes.
11. **Annotations**: `secret_summary` reads exactly one, `kubernetes.io/service-account.name` (decision 12).
12. **Docker configs**: only host keys are copied. **TLS**: the summarizer reads only `tls.crt`.

## Screenshot rules for ui-verifier

- Allowed screens: `secrets`, `secrets-drawer`, `secrets-yaml`, `ingresses`, `ingresses-drawer` (masked state only; `--screenshot` blocks reveal in code).
- `secrets-drawer` uses `--filter <first TLS secret name from the probe>` so the drawer shows the Certificate section.
- Any visible value in a Data cell is a **high-severity defect**; report it without attaching the image to any other agent and delete the file from `.tmp/`.

## Live checks of Reveal and Copy (coder, step 3)

On UAT with the first TLS secret: Reveal `tls.crt` (public); confirm it hides after 30 s and when the drawer closes. Copy `tls.crt`; `head -c 27 /dev/clipboard` (Git Bash) prints `-----BEGIN CERTIFICATE-----`; after 30 s it is empty; Win+V history does not list it. Copy, then copy other text within 30 s: the other text survives. Never reveal or copy `tls.key`, tokens, or Opaque values; no screenshots.
