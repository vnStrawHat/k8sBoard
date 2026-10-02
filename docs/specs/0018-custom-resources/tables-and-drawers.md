# 0018 · Tables and drawers

[Back to index](README.md) · Steps 2a (CRDs kind), 2b (custom rows, cells, status), 3 (object drawer, menus) · Modules: `crd_rows.rs` (new) + `crd_rows_tests.rs`, `custom_rows.rs` (new) + `custom_rows_tests.rs`, `kind_row.rs`, `kind_table.rs`, `kind_drawer.rs`, `kind_diagnosis.rs`, `live_sections.rs`, `related_objects.rs`, `cluster_session.rs`, `resource_actions.rs`

## Row model (`kind_row.rs`)

```rust
pub(crate) enum KindObject { /* … */ Crd(CrdSummary), Custom(CustomObjectSummary) }
pub(crate) enum LiveContent { /* … */ CustomConditions, CustomStatus, CustomSpec }   // step 3
pub(crate) enum KindCell { /* … */
    /// A printer-column date: `5d` in the past, `in 6d` in the future, painted at render. Sorts by time.
    Date { at: jiff::Timestamp, rule: DateRule } }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DateRule { Plain, Expiry }
```

`kind_table.rs` `value`: `Date { at, .. }` → `Number(at.as_second())`. `cell_element` and `kind_drawer.rs` `field_value`: past → `format_age(Some(at), now)`, future → `in {format_age(Some(now), at)}` (kubectl shows `<invalid>`); the drawer adds `{at} (…)` like `Age`. `DateRule::Expiry` colors the text with `expiry_label(at, now).tone` (0016 `certificate_expiry.rs`) only when it is Warn (≤ 14 days) or Bad (expired); otherwise no tone, like W7 `in 64d`.

## CRDs kind (`crd_rows.rs`)

| Column | Width | Cell |
|---|---|---|
| Group | 200 | `Mono(group)` |
| Version | 90 | preferred served version (`Mono`), none → `Absent` |
| Scope | 110 | `Text("Namespaced" / "Cluster")` |
| Instances | 90 r | built `Absent`; filled by the cluster-wide counts join (step 4, [counts-and-namespaces.md](counts-and-namespaces.md)) |
| Age | — | `AGE_COLUMN` |

Status: `Established` → Ok "Established"; `NamesNotAccepted` → Bad "Names not accepted"; `NotEstablished` → Warn "Not established"; `Terminating` → Info "Terminating". `crd_row(&CrdSummary) -> KindRow`, object `KindObject::Crd`. Sections are pre-built in step 2a (definitions only, no instance data):

| # | Section | Rows |
|---|---|---|
| 1 | **Definition** | Kind (Mono), Plural (Mono `plural / singular`), Scope; not Established → `Note` with the reason or message |
| 2 | **Versions** | per version: label `name` (`name (preferred)` for the preferred one), `Toned`: served + storage → Ok `served · storage`; served → Ok `served`; deprecated → Warn `deprecated` (then `Note(deprecation_warning)` when set); not served → Done `not served` |
| 3 | **Printer columns** (preferred version) | `Chips` of column names, unsupported as `{name} (unsupported)`; none → `Note("No printer columns: the table shows Name and Age.")` |
| 4 | **Schema** (preferred version) | `Code`: `spec:` then `  {name}: {type or "any"}` lines, same for `status:`; `… {omitted} more` last; empty → `Note("No schema.")` |

Menu (step 3): **Browse instances** (enabled when `live.crds.kinds` holds a kind with this `crd_name`, opens `Screen::Kind(Custom(kind))`; else disabled "Not established or not served"), then the kind items (View YAML, Copy name, Delete CRD… disabled "Read-only mode").

## Custom object rows (`custom_rows.rs`)

```rust
pub(crate) fn custom_object_row(kind: CustomKind, summary: &CustomObjectSummary) -> KindRow;
pub(crate) fn custom_status(conditions: &[ObjectCondition], phase: Option<&str>) -> StatusLabel;
pub(crate) fn condition_label(status_text: &str) -> Option<StatusLabel>;   // "True" Ok, "False" Bad, "Unknown" Warn
```

| `ColumnValue` | Cell |
|---|---|
| `Absent` | `Absent` |
| `Hidden` | `Text("<hidden>")` |
| `Text(t)` | rule `ConditionStatus` and `condition_label(t)` is `Some` → `Toned`; else `Text` (object values arrive as `Absent`, shown `—`) |
| `Integer(n)` | `n >= 0` → `Quantity { text, value: n, tone: None }`; else `Text` |
| `Number(t)`, `Boolean(b)` | `Text` |
| `Date(at)` | `Date { at, rule: Expiry }` for rule `Expiry`, else `Plain` |

Rules come from `kind.column_rules()`. No printer columns → one `Age` cell. `object: KindObject::Custom(summary.clone())`; sections (C7, built at render): `[Live(CustomConditions), Live(CustomStatus), Live(CustomSpec)]`. Labels as chips.

### Status (`custom_status`, decision 22)

1. A **failing** condition: a type other than `Ready`/`Available` whose reason contains `fail` or `error` (ASCII case-insensitive).
2. `Ready`, else `Available`: True → Ok `Ready`/`Available` (Warn `{type}: {reason}` when a failing condition exists); False → Bad reason or `Not ready`/`Unavailable`; Unknown → Warn reason or `Unknown`.
3. Neither: a failing condition → Warn `{type}: {reason}`; else `phase` → Info phase; else Info `No status`.

## Custom object drawer (step 3)

Tabs Overview · YAML · Events (0007, 0006; `kind.object_ref`, `kind.object_kind()` cover custom kinds). Overview, built at render from `KindObject::Custom` and the related list:

| # | Section | Rows |
|---|---|---|
| — | **FROM STATUS** box (`kind_diagnosis` arm) | first match: Ready or Available False → Bad, its message (else `{type} is False: {reason}`); Ready Unknown → Warn, its message; a failing condition → Warn `{type}: {message or reason}`; else none |
| 1 | **Conditions** (`Live(CustomConditions)`) | per condition: label type, `Toned("{status}" or "{status} · {reason}")`: Ready/Available True Ok, False Bad, Unknown Warn; other types failing → Bad, else Info; then `Note(message)` when set. None → `Note("No conditions reported.")` |
| 2 | **Status** (`Live(CustomStatus)`) | the related fields list, `status` side |
| 3 | **Spec** (`Live(CustomSpec)`) | the same, `spec` side |
| 4 | Labels | as 0005 |

Fields rows: `Text(t)` → `Field { path, Mono(t) }` (`Stacked` when longer than 60 chars); `Hidden` → `Text("<hidden>")`; `Items(n)` → `Text("{n} items")`; `Fields(n)` → `Text("{n} fields")`; a `Text` entry whose last path segment is `secretName` → `Link { path, t, ResourceKey::of_object("Secret", namespace, t) }` (`Field` when `None`); `omitted > 0` → `Note("{n} more fields in the YAML tab.")`; empty side → `Note("No status fields.")` / `Note("No spec fields.")`. List states: Loading → `Note("Loading…")`; Failed → `Note(error_text)`; empty snapshot → `Note("The object no longer exists.")`.

### Related fields watch (`related_objects.rs`, `cluster_session.rs`, decision 9)

```rust
pub(crate) enum RelatedSubject { /* … */ CustomFields { kind: CustomKind, namespace: Option<String>, name: String } }
pub(crate) enum RelatedList { /* … */ CustomFields(LiveList<CustomObjectFields>) }
```

`related_subject` builds it from `ResourceKind::Custom` rows; stream `watch_custom_object_fields(&object_ref)`; no gate (the explorer gate already allowed list/watch). Lifecycle, debounce, and settle are 0012's.

### Menus

`kind_menu` unchanged: no read-only actions, View YAML, Copy name, `Delete {singular}…` disabled "Read-only mode". W7's cert-manager items (Renew now, Go to secret, Edit YAML) are not rendered; Go to secret is the `secretName` field link.

## Worked example: cert-manager Certificates (fixture tests and screenshots)

CRD `certificates.cert-manager.io`, served `v1` (storage), Namespaced; printer columns: Ready `.status.conditions[?(@.type == "Ready")].status` (string), Secret `.spec.secretName`, Issuer `.spec.issuerRef.name` (priority 1), Status `.status.conditions[?(@.type == "Ready")].message` (priority 1), Age `.metadata.creationTimestamp` (date); `BUILT_IN_COLUMNS` inserts Expires `.status.notAfter` (date, expiry rule) before Age. Object `ingress/tls-shop-example`: Ready True; Issuing False, reason `Failed`, message `Last renewal attempt failed: ACME challenge returned 404`; `status.notAfter`, `renewalTime`, `revision: 5`.

| Where | Expected |
|---|---|
| Sidebar | Custom Resources › cert-manager.io › Certificates (badge `Ce`) |
| Table | Name · Ready · Secret · Issuer · Status · **Expires** (built-in) · Age → `ingress/tls-shop-example` · Ok `True` · `tls-shop-example` · `letsencrypt-prod` · message · Warn `in 6d` · `300d` |
| Expires tone | `notAfter` in 64 days → no tone; in 6 days → Warn; 2 days ago → Bad `2d` |
| Row status | Warn `Issuing: Failed` |
| Box | Warn FROM STATUS `Issuing: Last renewal attempt failed: ACME challenge returned 404` |
| Conditions | Ready Ok `True`; Issuing Bad `False · Failed` + note |
| Status | `notAfter`, `renewalTime`, `revision 5` |
| Spec | `dnsNames`, `issuerRef.kind`, `issuerRef.name`, `secretName` → link to Secrets |

Accepted W7 deviations: an extra Status message column (printer column), no meta line (0013 convention), no Renew menu item.
