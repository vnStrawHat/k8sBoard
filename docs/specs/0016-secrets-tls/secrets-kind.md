# 0016 · App: the Secrets kind

[Back to index](README.md) · Step 2 (menus in step 3) · Modules: `resource_kind.rs`, `kind_row.rs`, `secret_rows.rs` (new) + `secret_rows_tests.rs`, `kind_join.rs`, `live_sections.rs`, `kind_diagnosis.rs`, `cluster_session.rs`, `config_map_rows.rs`. Anything not listed works unchanged for a new `ResourceKind` (0013 [app-model.md](../0013-policy-kinds/app-model.md)): sidebar gating, counts, Events and YAML tabs, toolkit, screenshots.

## `KindSpec`

| Variant | label | object | singular / plural | badge | ns | access | read-only actions | delete label |
|---|---|---|---|---|---|---|---|---|
| `Secrets` | Secrets | Secret | secret / secrets | Se | yes | `ListSecrets` | Edit | Delete secret… |

`NameColumn::Flexible`, `has_labels: true`, no port-forward; appended to `ALL` after the 0015 kinds. `watch_rows`: `rows(update, secret_row)` over `watch_secrets(scope)`.

## Row model (`kind_row.rs`)

```rust
pub(crate) enum KindObject { /* … */ Secret(SecretSummary) }
pub(crate) enum LiveContent { /* … */ SecretData, Certificate }   // `UsedBy` (0012) gains the Secret arm
pub(crate) enum KindCell { /* … */
    /// Paint-time expiry of a certificate (tls-expiry.md). Sorts by `not_after`.
    Expiry { not_after: jiff::Timestamp },
}
```

`format_bytes` (`config_map_rows.rs`) becomes `pub(crate)` for sizes.

## Columns and cells (`secret_rows.rs`)

| Column | Width | Cell |
|---|---|---|
| Type | 220 | `Mono(secret_type)` |
| Keys | 70 r | `Text(keys.len())`, like ConfigMaps' Data |
| Used by | 220 | joined (`SECRET_USED_BY`), built `Absent` |
| Expires | 130 | `Expiry { leaf.not_after }` for a parsed certificate, `Absent` for other secrets; sortable by date (UX walk H5) |
| Age | — | `Age { at: created_at, tone: None }` (decision 19) |

**Status** (drawer subtitle): `Ok` with the type text; `NoCertificate(Unparsed)` → Warn "Certificate not parsed"; `NoCertificate(Missing)` → Warn "No certificate". Time-dependent states live in the box. Helm release secrets (`helm.sh/release.v1`) are plain rows; 0017 owns their decoding.

## Drawer sections (C7: no value is ever in `sections`)

| # | Section | Rows |
|---|---|---|
| — | CERTIFICATE box | `kind_diagnosis` arm, [tls-expiry.md](tls-expiry.md) |
| 1 | **Secret** | Type (Mono); Immutable `yes`/`no`; Service account → `Link` to the ServiceAccounts row (`ResourceKey` of `(ns, account)`), or `Text` when the kind has no screen; Registries → `Chips` (none → `Note("No registry hosts")`) |
| 2 | **Data** | `Live(SecretData)`; empty `keys` → `Note("No data")` |
| 3 | **Certificate** (TLS only) | `Live(Certificate)` |
| 4 | **Used by** | `Live(UsedBy)` |
| 5 | Labels | as 0005 |

### `Live(SecretData)` (step 2 form; step 3 hands it to the view)

`secret_data_rows(keys: &[SecretKey]) -> Vec<MaskedKeyRow>` (pure): per key, name (Mono), `MASK = "••••••••••"`, size (`format_bytes`), and `binary` muted suffix when `is_binary`. Step 3 renders these rows inside `SecretValuesView` and adds buttons ([secret-values-view.md](secret-values-view.md)).

### `Live(Certificate)`

From `SecretDetails::Certificate { chain }`, leaf = `chain[0]`: Subject (Mono), Issuer (Mono), Alt names (`Chips`, at most 20, then `+{n}`), Not before (`YYYY-MM-DD HH:MM UTC`), Not after (`Expiry { leaf.not_after }`, shown as `Dec 25, 2026 (81d left)` or `Sep 1, 2026 (expired 35d ago)`), Intermediate (Warn, only when `intermediate_expires_first`; [tls-expiry.md](tls-expiry.md)), Chain `{n} certificates` when `n > 1`. `NoCertificate(Missing)` → `Note("The secret has no tls.crt.")`; `NoCertificate(Unparsed)` → `Note("tls.crt could not be parsed as an X.509 certificate.")`.

## Used by (`kind_join.rs`, `live_sections.rs`)

```rust
pub(crate) fn secret_users(pods: &[PodSummary], ingresses: &[IngressSummary]) -> SecretUsers; // ns → name → Vec<UsedBy>
```

- Ways (0012 `UsedBy.ways`): `env` (`EnvSource::SecretKey`), `env from` (`EnvFromSource::Secret`), `volume` (`VolumeSource::Secret`, `Projected.secrets`), `image pull` (`image_pull_secrets`), all container kinds; owners mapped as 0012 decision 9. Ingresses: owner `ingress/{name}`, way `tls`, target its Ingresses key, for every `tls[].secret_name`. Service-account token: owner `serviceaccount/{account}`, way `token`, added per row from `details`.
- Cell (`SECRET_USED_BY`): first owner, plus ` +{n}`; no users and both lists Ready → `Toned(Done, "none found")`; else `Absent`. Pods not Ready → `Absent`.
- Section: one `Link` row per user (owner text → target, ways joined `, ` as value). No users: Ready → `Note("No pod or ingress in this namespace uses it. Workloads with no running pod, CronJob templates, Gateway API and Istio references, and readers through the API are not checked.")`; not Ready → `Note("Loading…")`; companion denied → the same note starting "No pod in this namespace uses it." (ingresses were not checked) plus "Not permitted: list ingresses"; companion failed → that note plus "Ingresses are unavailable".
- Triggers (0012 `join_explorer`): explorer, pods, and companion snapshots or failures while Secrets is shown.

## Ingresses companion (`cluster_session.rs`)

```rust
pub(crate) enum CompanionLists { /* … */ Ingresses(LiveList<IngressSummary>) }
enum CompanionUpdate { /* … */ Ingresses(WatchUpdate<IngressSummary>) }
pub(crate) enum CompanionKind { /* … */ Ingresses }
```

`companion_plan(Secrets, access)`: `Denied(ListIngresses)` when `Known` and denied, else `Start(Ingresses)` with `watch_ingresses(scope)`. `OpenWatches.companion = N`. Secrets stay within the session bound `3N + 4` (`2 + N + N + N + 1`, no related subject).

## Menus (`resource_actions.rs`, step 3)

Order: **Reveal values (30s)** · **Copy value ▸** (submenu: `Copy {key}` per key; binary keys disabled "Binary value"; no keys → one disabled "No data"), then the existing kind items (Edit disabled "Read-only mode", View YAML, Delete secret… disabled) in their current order. Reveal and Copy call `AppShell::run_secret_action(key, action)` ([secret-values-view.md](secret-values-view.md)); when `AppShell.secret_value_access` is `Blocked` (the one source the view also uses) both are disabled with "Disabled in screenshot runs".

## W7 parts not rendered

List-level "Reveal all" (decision 4); drawer `meta` "values hidden"; Age warn tone (decision 19); "New" (0031).

## Restart all (walk H12)

The Used by section of a Secret or ConfigMap drawer gets a `Restart all N` button above its rows when more than one workload reads the value through env (volume-only users, Jobs, CronJobs, and bare pods are not counted). It calls `restart_consumers`, so each workload kind opens its own Restart rollout batch with the normal confirm; it is off with the first reason when a kind of the set is not allowed.
