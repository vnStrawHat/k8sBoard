# 0017 — Helm releases (read-only)

Status: amended after the advisor review (HEAD `ef69712`: 0010 done, 0011 step 1 in progress). **Not startable yet: 0012 and 0016 must be implemented first** (0013–0015 too, since 0016 depends on them); `secret_text` extracts code that 0016 creates. Crates: `crates/cluster` (step 1), `crates/app` (steps 2–3). Requires (`KindObject`, `Live`, `kind_diagnosis.rs`, related subjects, `selected_summary_watch`, `ObjectKind::Secret`, `ListSecrets`, `secret.rs` GET via `request_text`, `zeroize`, the `kube_client::client=error` log guard, `AppShell.secret_value_access`, `REVEAL_DURATION`, `write_private_text`, `SecretCopied`). Wireframes: W7 `k("Releases")`, sidebar group **Helm**. Applies **C1** (blocking), C6, C7, C11. C12 (Helm writes) stays with 0038.

## Goal

- **Releases** explorer kind (sidebar Helm › Releases): Name, Chart, App version, Revision, Status, Updated, read from Helm 3 release Secrets (`helm.sh/release.v1`); every status, like `helm list --all`.
- Drawer **Overview**: status box (UPGRADE / INSTALL / ROLLBACK FAILED, PENDING), release facts, **Values changed in rev N** (masked path list, Reveal), **History** of every revision (labels only) with Values and Diff buttons.
- Drawer **Values** (user-supplied or computed, masked until revealed for 30 s, diff with the previous revision), **Manifest** (masked like the YAML tab), **Notes** (masked until revealed). Revealed copies use the 0016 private clipboard.

## Non-goals

Roll back, Uninstall, upgrade (0038, C12; menu items disabled "Read-only mode"); the ConfigMap and SQL storage drivers; Helm 2; chart repositories; diff between two arbitrary revisions; a line diff (path list instead, decision 21); subchart values not stored in the release; the YAML and Events tabs for releases.

## Implementation steps

| Step | Scope | ACs |
|---|---|---|
| 1 | `crates/cluster`: `helm_release.rs` (summary, payload decode, two watches), `helm_release_detail.rs` (GET, values, manifest, notes), `helm_values_diff.rs`; `object_yaml.rs` and `secret.rs` extractions; metadata watch helper; Cargo features; probe `--helm`. **Run the UAT probe**, fill [decisions.md](decisions.md) "UAT probe" | 1, 2, 3, 4, 5 |
| 2 | App: Releases kind (rows, columns, Overview facts, status box, History related watch, menus), screenshots `releases`, `releases-drawer` | 1, 2, 3, 6, 8, 9 |
| 3 | App: `HelmReleaseView` (Overview "Values changed", Values, Manifest, Notes, reveal, diff, private copy), History buttons, View values / View manifest menu items, screenshots `releases-drawer` (again), `releases-values`, `releases-manifest`; doc updates | 1, 2, 3, 5, 7, 8, 9 |

## Files

| File | Contents |
|---|---|
| [decisions.md](decisions.md) | numbered decisions with rationale, ceilings, UAT probe table |
| [helm-safety.md](helm-safety.md) | C1 applied: where plaintext lives, rules, screenshots, review checklist |
| [cluster-api.md](cluster-api.md) | step 1: Cargo, summaries, payload decode, watches, probe |
| [release-detail.md](release-detail.md) | step 1: one-revision GET, values masking, computed values, manifest masking, values diff |
| [releases-kind.md](releases-kind.md) | step 2: kind spec, columns, Overview sections, box, History, menus |
| [release-view.md](release-view.md) | step 3: `HelmReleaseView` entity, async contract, tabs, render, AppShell wiring |
| [files-to-touch.md](files-to-touch.md) | modules and Cargo changes per step, doc updates |
| [test-plan.md](test-plan.md) | unit tests per step, live checks, ui-verifier checklist |

## Acceptance criteria

- [ ] 1. The quality gate passes, plus `cargo clippy -p k8sboard --features screenshot --all-targets -- -D warnings`. No new `#[allow]`.
- [ ] 2. Every test of the step in [test-plan.md](test-plan.md) exists under that name and passes offline.
- [ ] 3. No kube, k8s-openapi, flate2, base64, or zeroize type in a public signature; the crate spawns no task; the app gains no kube dependency; the 0001 read-only grep finds only the SSAR `create`; new requests are `list`/`watch`/`get` only.
- [ ] 4. `Cargo.lock` gains **no package** (`flate2` 1.1, `base64` 0.22, serde-saphyr `deserialize` deps, `serde` are locked); only the `k8sboard-cluster` dependency list changes. Anything else: stop and report.
- [ ] 5. Helm safety ([helm-safety.md](helm-safety.md) checklist): no `tracing::` in the listed modules; summaries hold no values, manifest, or notes (tests) and are never traced; detail GETs use `request_text`; `HelmText` has no `Debug`, `Display`, `Clone`; the probe prints counts and metadata only.
- [ ] 6. On UAT (or an empty state when the probe finds no release): Releases shows live rows with Chart and Status; a drawer shows History newest first.
- [ ] 7. Values, the Overview diff, and Notes open masked; Reveal shows them and re-masks after 30 s; a tab change hides at once; closing the drawer or changing the subject drops the view; Ctrl+C on revealed text writes the private clipboard (cleared after 30 s, out of Win+V history); Manifest shows Secret `data` as `<hidden>`; with `--screenshot`, every Reveal is disabled.
- [ ] 8. Watches per session stay at most `3N + 4` (explorer N + history 1). The 0003 AC4 color-literal grep is clean.
- [ ] 9. The step's screenshots exist, show masked values only, and the ui-verifier reports no high-severity defect against W7.

## Open items

1. "Compare with any revision" (W7 shows only "changed in rev N"); add a base picker if users ask.
2. The Overview diff costs 2 GETs per drawer open (decision 21); measure on a large release before 0021 reuses it.
