# 0018 — CRDs and generic custom resources (read-only)

Status: implemented (steps 1a to 5); UAT-probed, ui-verifier review pending. Amended after the advisor review (HEAD `156ceb9`, 0011 in progress). **Requires 0012, 0014, 0016 merged** (`KindObject`, `Live`, `kind_diagnosis.rs`, related subjects, `selected_summary_watch`, `kind_join.rs`, counts, `is_secret_parameter` (now `is_secret_key`), Secrets kind, `expiry_label`); 0017 not required. Crates: `crates/cluster` (1a, 1b, 5), `crates/app` (2a-5). Wireframes: sidebar **Custom Resources**, W7 `k("CRDs")`, `k("Certificates")`, `k("Namespaces")` STUCK box. Applies C1, C6 (no new dependency), C7, C11. **Amendment 2026-10-04: step 6 (Certificate Renew now) is mutating** ([renew-now.md](renew-now.md)): one allow-listed operation, `RenewCertificate`; C3 covered by the user's approval of 2026-10-02 for all mutating specs; debug builds block writes unless `K8SBOARD_ALLOW_WRITES=1`, and UAT checks stay denied-path-only.

## Goal

- **CRDs** kind: Name, Group, Version, Scope, Instances, Age; drawer Versions, Printer columns, Schema outline, Browse instances.
- Every Established CRD is a sidebar item under Custom Resources › {API group}, at its preferred served version.
- A generic **custom object** table from `additionalPrinterColumns` (JSONPath subset) plus a small built-in column table (Certificates **Expires**, toned like 0016 TLS expiry), and a drawer: status box, Conditions, Status and Spec fields (live), YAML (masked), Events. cert-manager Certificates is the worked example.
- Namespaces: **STUCK** box and Remaining resources from deletion conditions.

## Non-goals

Any mutation other than step 6 Renew now (Delete and Edit of custom objects stay absent); column overrides beyond `BUILT_IN_COLUMNS` (one entry); schema descriptions in the object drawer; Discovery API fallback when CRDs are not listable; sidebar search or pinning; listing every object of a stuck namespace; conversion webhooks; Go to object from events for custom kinds.

## Implementation steps

| Step | Scope | ACs |
|---|---|---|
| 1a | `crates/cluster`: CRD watch and summary, `column_path.rs`, `ListCustomResourceDefinitions`, `ObjectKind::CustomResourceDefinition`, `is_secret_key` (moved, widened); probe `--crds` (CRD lines). **Run the probe**, fill [decisions.md](decisions.md) | 1, 2, 3, 4 |
| 1b | `crates/cluster`: custom object watch, fields watch, masking (S2–S4), `review_custom_access`, `count_custom_objects`, closure summarizers, `ObjectRef::custom`; probe access/count/watch/yaml lines | 1, 2, 3, 4, 5 |
| 2a | App: `KindApi` conversion, the Crds kind fed by the session CRD watch (table, pre-built drawer); screenshots `crds`, `crds-drawer` | 1, 2, 3, 6, 8, 9 |
| 2b | App: `CustomKind` + cache, `BUILT_IN_COLUMNS`, `ResourceKind::Custom`, gate, custom tables, sidebar groups, remap, launch `custom:…`; screenshot `custom` | 1, 2, 3, 6, 7, 8, 9 |
| 3 | App: custom object drawer (box, Conditions, fields watch, links, YAML, Events, menus), Browse instances; screenshots `custom-drawer`, `custom-yaml` | 1, 2, 5, 6, 8, 9 |
| 4 | App: Instances counts and custom sidebar counts | 1, 2, 8, 9 |
| 5 | Cluster + app: Namespace deletion conditions, STUCK box, Remaining resources; doc updates | 1, 2, 9 |
| 6 | Amendment (mutating): `RenewCertificate` (`certificates/status` PUT adding `Issuing=True`, `cmctl renew` parity), `AccessCheck::UpdateCertificateStatus`, `certificate_renewal.rs`; app `Renew now` item and `Renew` header on the cert-manager kind; UAT trace; ui-verifier | 1, 2, 10–15 |

## Files

| File | Contents |
|---|---|
| [decisions.md](decisions.md) | numbered decisions with rationale, ceilings, UAT probe table |
| [cluster-api.md](cluster-api.md) | 1a/1b: types, CRD watch, custom object watches, SSAR, counts, YAML, probe |
| [column-path.md](column-path.md) | 1a: JSONPath subset grammar, evaluation, column typing |
| [custom-object-safety.md](custom-object-safety.md) | C1 for unknown objects: `is_secret_key`, masking rules, review checklist |
| [app-model.md](app-model.md) | 2a/2b: `KindApi`, `CustomKind`, cache, built-in columns, session, gate, sidebar, launch |
| [tables-and-drawers.md](tables-and-drawers.md) | 2a–3: CRDs kind, custom rows, status tone, drawers, Certificates example |
| [counts-and-namespaces.md](counts-and-namespaces.md) | 4–5: Instances counts, STUCK namespaces |
| [files-to-touch.md](files-to-touch.md) | modules per step, doc updates |
| [renew-now.md](renew-now.md) | step 6: what renew means, operation, request, app gate and texts, tests |
| [test-plan.md](test-plan.md) | unit tests per step, live checks, ui-verifier checklist |

## Acceptance criteria

- [x] 1. The quality gate passes, plus `cargo clippy -p k8sboard --features screenshot --all-targets -- -D warnings`. No new `#[allow]`.
- [x] 2. Every test of the step in [test-plan.md](test-plan.md) exists under that name and passes offline. Tests that need a live session (gate flow, CRD snapshot feed, launch resolution, Browse instances) are covered through pure helpers: `gate_outcome`, `explorer_action`, `feed_crds`, `resolve_custom_launch`, `browse_target`.
- [x] 3. No kube, k8s-openapi, or `serde_json` type in a public signature; the crate spawns no task; the app gains no kube dependency; the 0001 read-only grep finds only the SSAR `create`; new requests are `list`/`watch`/`get` only. `Cargo.lock` unchanged.
- [x] 4. On UAT the probe prints the CRD access line, `crds` count, per-CRD column support (1a), then access, count, watch, yaml lines (1b); results copied into [decisions.md](decisions.md).
- [x] 5. Safety ([custom-object-safety.md](custom-object-safety.md) checklist): no `tracing::` call with object content; summaries and fields hold only the listed data; masking tests pass; the probe prints counts only.
- [x] 6. On UAT (or an empty state when no CRD exists): CRDs lists every CRD; each Established CRD is a sidebar item under its group; opening one shows live rows with printer columns.
- [x] 7. Unsupported printer-column paths show `—`, never a wrong value; a CRD without printer columns shows Name and Age; `format: password` columns read `<hidden>`.
- [x] 8. Watches per session stay at most `3N + 5` (`open_watch_count` test). The 0003 AC4 color-literal grep is clean.
- [x] 9. The step's screenshots exist; the ui-verifier reports no high-severity defect against W7 (accepted deviations in [test-plan.md](test-plan.md)). Screenshots `v42-*`, `v43-*`, and `v44-*` exist. The ui-verifier ran on 2026-10-02 (`v43v-*`) and found no high-severity defect. The typed date and Expires tone, the Ready, Available, and failing-condition status boxes, and the `secretName` link have no UAT data (cert-manager is not installed): they are unit-tested only.
- [ ] 10. (step 6) Request shape: one GET, then `PUT /apis/cert-manager.io/v1/namespaces/{ns}/certificates/{name}/status?dryRun=All&fieldManager=k8sboard` (a commit has no `dryRun`); body = the fresh object with its `resourceVersion`, no `managedFields`, spec and other conditions unchanged, and exactly one `Issuing` condition (`True`, `ManuallyTriggered`, `lastTransitionTime` = `requested_at`, `observedGeneration`).
- [ ] 11. (step 6) `WriteRequest::new` fits `RenewCertificate` only to a namespaced custom target of `cert-manager.io/v1` `certificates`; every other target, and every other operation on a custom target, is `None`. No new clippy exception.
- [ ] 12. (step 6) A Certificate already `Issuing=True`, or being deleted, is refused after the GET with zero PUTs (`Invalid`, field `status.conditions[Issuing]`).
- [ ] 13. (step 6) `Renew now` (menus) and `Renew` (header, cursor row) appear only on the `certificates.cert-manager.io` kind; disabled with the gate reason (`Not permitted: update certificates/status`, lock) or `Needs cert-manager.io/v1`. `UpdateCertificateStatus` is in `AccessCheck::ALL` (one more SSAR per namespace at connect).
- [ ] 14. (step 6) The 0030 flow runs: dry-run, tier (PROD types the cluster name), the two warnings (rate limits, key rotation), commit, one audit line (action `Renew`, field `status.conditions[Issuing]`), notice `Renewal requested for {ns}/{name}`.
- [ ] 15. (step 6) Every test of [renew-now.md](renew-now.md) exists and passes offline. UAT (no cert-manager): no Renew item; the trace adds only the SSAR; no PUT. ui-verifier: the item in the `custom-drawer` fixture, no high-severity defect against W7 Certificates.

## Open items

1. Discovery API fallback for users who may list some custom resources but not CRDs (probe first).
2. More `BUILT_IN_COLUMNS` entries (other well-known CRDs) if users ask.
3. The 0029 command palette should index custom kinds (large CRD sets).
4. Event Go to object for custom kinds needs a session-aware `ResourceKey::of_object`.
5. Certificate `Renew now` (W7 Certificates, moved here from 0032 by 0032b decision 16): designed as step 6, [renew-now.md](renew-now.md) (amendment 2026-10-04). Not testable live: cert-manager is not on UAT and UAT is read-only (R2); fake-transport tests only.
