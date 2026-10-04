# 0018 · Step 6: Certificate "Renew now" (amendment, mutating)

[Back to index](README.md) · Amendment 2026-10-04 (open item 5). **Mutating**: one allow-listed operation, `RenewCertificate`. C3: covered by the user's one approval of 2026-10-02 for all mutating specs; debug builds block writes unless `K8SBOARD_ALLOW_WRITES=1` (agents never set it). Wireframe W7 `k("Certificates")`: header `Renew`, drawer `Renew now`.

## What "renew" means (decision)

| Option | Verdict |
|---|---|
| **`cmctl renew` equivalent**: add the condition `Issuing=True`, reason `ManuallyTriggered`, through `PUT …/certificates/{name}/status` | **chosen**. cert-manager's trigger controller treats it as a request to issue now; it is the documented manual-renewal path. One status write, no spec change. cert-manager's default `cert-manager-edit` aggregated role grants `update certificates/status` for this purpose, so `update` (not `patch`) is the verb users have. |
| Delete the TLS Secret | rejected: drops the serving certificate until reissue; touches a Secret |
| Annotation `cert-manager.io/issue-temporary-certificate` | rejected: it asks for a temporary self-signed certificate while issuing, not for a renewal |
| Edit `spec.renewBefore` / `spec.duration` | rejected: changes the user's spec to force a side effect |

Basis: cmctl `renew` (cert-manager v1.x): read the Certificate, skip it when an `Issuing=True` condition exists, else set that condition and `UpdateStatus`. Written from the cmctl behavior as known offline (no network in this run); the coder confirms against the vendored cmctl text if one is available, else the fake-transport tests pin our request.

Safe because: one subresource, one condition, a `resourceVersion` precondition (from the fresh GET), dry-run first, the 0030 gate, lock, tier, and audit. No Secret is read or written.

## Cluster crate

```rust
pub enum WriteOperation { /* … */
    /// `PUT` of a cert-manager `Certificate`'s `status` adding `Issuing=True` (`cmctl renew`), 0018 step 6.
    /// `requested_at` is the condition time, fixed so the dry-run and the commit send one body.
    RenewCertificate { requested_at: jiff::Timestamp },
}
// access_review.rs: in ALL (custom kinds have no `ObjectKind`, so no lazy map entry fits)
AccessCheck::UpdateCertificateStatus => ("update", "cert-manager.io", "certificates", Some("status"), true)
// certificate_renewal.rs (new, pure)
pub(crate) enum RenewRefusal { AlreadyIssuing, Deleting, Unreadable }
pub(crate) fn renewal_status_body(fresh: Value, requested_at: jiff::Timestamp) -> Result<Value, RenewRefusal>;
```

| Item | Rule |
|---|---|
| Target fit (`fitting_access_check`) | a **custom** target whose resource is group `cert-manager.io`, version `v1`, plural `certificates`, kind `Certificate`, namespaced → `UpdateCertificateStatus`. Every other custom target stays `None` for every operation. **Order matters**: the custom arm goes **before** the function's first line `let kind = target.builtin_kind()?;` (`object_write.rs`), which returns `None` for every custom target; e.g. `if let Some((resource, namespace, _)) = target.as_custom() { return (matches!(operation, RenewCertificate { .. }) && is_cert_manager_v1(resource) && namespace.is_some()).then_some(AccessCheck::UpdateCertificateStatus); }` |
| `checked_operation` | explicit arm, kept as is (the target rule carries the check) |
| `is_safe_path` | name and namespace DNS subdomains (the default rule) |
| `changed_fields` | `status.conditions[Issuing]` = `True (ManuallyTriggered)` |
| `supports_dry_run` | true |
| `renewal_status_body` | `metadata.deletionTimestamp` set → `Deleting`; a condition `type: Issuing, status: "True"` → `AlreadyIssuing`; no `metadata.resourceVersion` → `Unreadable`. Else: remove `metadata.managedFields` (the server keeps them when absent); in `status.conditions` (created if missing) drop any `Issuing`, append `{type: Issuing, status: "True", reason: ManuallyTriggered, message: "Certificate re-issuance manually triggered", lastTransitionTime: requested_at (RFC 3339, whole seconds), observedGeneration: metadata.generation}`. Spec and other conditions unchanged |
| `send` arm | `get_object(target, READ_ACTION)`; a `ClusterError::Api { code: 404, .. }` from this GET → `WriteError::NotFound` (other read errors stay `Cluster`); refusal → `AlreadyIssuing`: `Invalid { "the certificate is already being issued", ["status.conditions[Issuing]"] }`, `Deleting`: `Invalid { "the certificate is being deleted", [] }`, `Unreadable`: `unusable_object`; then `serde_json::from_value::<DynamicObject>(body)` (a failure → `unusable_object(mode)`), and `api.replace_status(name, &post_params(mode), &object)` in `run_raw` (kube 4.2 takes `&K`, here `&DynamicObject`); response dropped; `Answer::patched()` |
| Errors | the PUT through the merged mapping: 409 `Conflict` (0030 Retry re-reads), 404 `NotFound` (the GET's 404 is mapped in the arm above), 403 `Denied` / `Invalid`, 400 webhook `DryRunRejected` |

Allow-list row (added to the 0030 table by this step): `RenewCertificate` · GET, then PUT `application/json` · `/apis/cert-manager.io/v1/namespaces/{ns}/certificates/{name}/status?dryRun=All&fieldManager=k8sboard` (commit: `?fieldManager=k8sboard`) · fresh object, fresh `resourceVersion`, no `managedFields`, one added `Issuing` condition · dry-run yes · 0018. No new clippy exception (`Api::replace_status` is in the excepted `send` match).

## App

| Item | Rule |
|---|---|
| Presence check | offered only on the custom kind whose CRD is `certificates.cert-manager.io` (the `BUILT_IN_COLUMNS` key) **and** whose served version is `v1`; another version → item disabled `Needs cert-manager.io/v1`. No CRD → no kind, no item |
| Action | `ResourceAction::RenewCertificate`; gate `Mutating { checks: [UpdateCertificateStatus], is_shipped: true }`; risk `Change`; label `Renew now` |
| Menus | row menu and drawer ⋯ menu: `Renew now` in the change section (`kind_menu` arm for that custom kind) |
| Header | `Renew` on that screen: acts on the cursor row; disabled `Select a certificate` without one |
| Intent | `label: "Renew certificate {ns}/{name}"`, `button: "Renew"`, `expected_name: None`, warnings: `cert-manager requests a new certificate now; ACME issuers count it against their rate limits (Let's Encrypt: 5 duplicate certificates per week)` and `The private key changes too unless privateKey.rotationPolicy is Never` (cert-manager v1.18 made `Always` the default; the coder confirms against the cert-manager release notes if they are available offline) |
| After commit | notice `Renewal requested for {ns}/{name}`; the drawer's Conditions follow the fields watch (`Issuing True`) |
| Audit | action `Renew`; field `status.conditions[Issuing]` = `True (ManuallyTriggered)` |

## Tests (offline)

| Test | File | Checks |
|---|---|---|
| `renewal_body_adds_one_issuing_condition` | `certificate_renewal_tests.rs` | condition fields, `observedGeneration`; spec, other conditions, and other status fields (`notAfter`, `renewalTime`, `revision`) unchanged |
| `renewal_body_replaces_a_false_issuing_condition` | same | `Issuing=False` → replaced, one left |
| `renewal_refuses_issuing_and_deleting` | same | `AlreadyIssuing`, `Deleting` |
| `renewal_body_drops_managed_fields_keeps_version` | same | no `managedFields`; `resourceVersion` kept |
| `renew_get_not_found_is_not_found` | `object_write_certificate_tests.rs` | the GET answers 404 → `WriteError::NotFound`, zero PUTs |
| `renew_dry_run_request_shape` | `object_write_certificate_tests.rs` (`FakeApi`) | GET, then `PUT …/certificates/tls/status?dryRun=All&fieldManager=k8sboard`, body |
| `renew_commit_has_no_dry_run` | same | `?fieldManager=k8sboard` |
| `renew_already_issuing_sends_no_put` | same | `Invalid`, one GET, zero PUTs |
| `renew_needs_cert_manager_v1_target` | same | builtin target, other group, `v1alpha2`, cluster scope → `None` |
| `other_custom_targets_fit_no_operation` | same | `DeleteObject`, `ReplaceObject` on a custom target → `None` |
| `blocked_policy_sends_nothing` | same | `WritesBlocked`, zero requests |
| `renew_item_only_on_cert_manager_kind` | `resource_actions_tests.rs` | item on `certificates.cert-manager.io`; absent on Argo CD Applications |
| `renew_item_disabled_reasons` | same | `Checking permissions…`, `Not permitted: update certificates/status`, lock, `Needs cert-manager.io/v1` |
| `renew_intent_has_warnings_and_audit_field` | `app_shell_write_tests.rs` | intent texts; audit action `Renew` |

Live: cert-manager is not on UAT (decisions.md probe table), so no row offers Renew; the UAT run checks the new SSAR (`update certificates/status`, one per namespace of the scope) and no PUT. ui-verifier: `custom-drawer` fixture with the item, against W7 Certificates.

## Files to touch

Cluster: `certificate_renewal.rs` (new) + `certificate_renewal_tests.rs`, `object_write.rs` (variant, `name`, `checked_operation`, `fitting_access_check` custom arm, `changed_fields`, `supports_dry_run`, `send` arm), `object_write_certificate_tests.rs` (new, wired like the other write test modules), `access_review.rs` (`UpdateCertificateStatus` in `ALL`; the `ALL` length test `all_checks_cover_distinct_permissions` goes from 55 to 56; the stale `22 x N + 4 requests` comment on `review_access` is rewritten from the current cluster-scoped and namespaced counts), `object_yaml.rs` (`as_custom` reuse only), `lib.rs`. App: `custom_kind.rs` (`is_cert_manager_certificate`), `resource_actions.rs` (action, gate, label, `kind_menu` arm), `workspace.rs` (header `Renew`), `resource_actions_tests.rs`, `app_shell_write_tests.rs`. Docs: the 0030 `write-path.md` allow-list row; gap audit row "W7 Certificates · Renew now".

## As built (2026-10-04)

Built as written, with these differences. Only the first two touch behavior.

- **Item through the kind table, version through `kind_block`.** The Certificates `CustomKind` carries `Renew now` in its `read_only_actions` (set in `leak_kind` for `certificates.cert-manager.io`), so the row menu, the drawer ⋯ menu, the key action, and the palette share one path and no `kind_menu` arm is needed. `kind_block(action, kind)` (`resource_actions.rs`) is the one place that says `Needs cert-manager.io/v1`; it runs after the gate, in `kind_menu`, `key_availability_of`, and the start. `CustomKind::is_cert_manager_certificate` and `is_cert_manager_v1` are the two predicates.
- **Refusal after the read only.** The app does not pre-block a Certificate that is already issuing or being deleted: the dry-run reads it and the cluster crate refuses (`Invalid`, field `status.conditions[Issuing]`), so the dialog shows the reason with zero PUTs.
- **App module.** `crates/app/src/certificate_renewal.rs` (child of `app_shell`) holds `renew_intent`, `renewal_notice`, `start_renew_certificate`, `renew_header_state` (header `Renew`, same gate as the key, `Select a certificate` without a cursor Certificate), and the `--screen renew-confirm` fixture. The header button is `render_renew` in `workspace.rs`; the notice is wired in `finish_commit`. The key action `RenewCertificate` is unbound.
- **Texts.** The two warnings are exactly the spec's. The cert-manager release notes were not available offline, so the v1.18 default change is not claimed anywhere in the UI or here.
- **Tests beyond the table.** Cluster: `renew_conflict_is_a_conflict`, `renew_deleting_sends_no_put`, `dry_run_and_commit_send_one_body`, `renewal_refuses_unreadable_objects`, and the `ALL` and Display coverage in `access_review.rs`. App: `app_shell_certificate_tests.rs` runs the key over a fake server (dry-run, commit, one body, audit line), held Enter, a switched session at commit, a locked cluster, another CRD version, and the header states.
- **Screen.** `--screen renew-confirm` (screenshot builds): the Renew dialog of `shop/shop-tls` on a fixed Production cluster, offline.
- **One body for the check and the commit.** `requested_at` is taken (whole seconds) when the dialog opens, in `renew_intent`, so the dry-run and the commit send the same body; a Retry after a 409 reads the object again and reuses that time.
