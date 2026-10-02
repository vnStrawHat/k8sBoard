# 0016 · TLS expiry: rules, Ingress TLS, Overview contract

[Back to index](README.md) · Steps 2 (expiry model, Secret box) and 4 (Ingresses) · Modules: `certificate_expiry.rs` (new, tests in module), `kind_table.rs`, `kind_drawer.rs`, `kind_diagnosis.rs`, `network_rows.rs`, `kind_join.rs`, `live_sections.rs`, `cluster_session.rs`, `resource_kind.rs`

## Expiry model (`certificate_expiry.rs`, step 2, pure; decision 13)

Expiry is the **leaf's** not-after everywhere (cell, box, Ingress join, 0020 contract). An intermediate that expires first gets its own Warn note; it never replaces the leaf date.

```rust
pub(crate) const EXPIRY_WARNING: jiff::SignedDuration = jiff::SignedDuration::from_hours(14 * 24);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ExpiryState { Valid, ExpiringSoon, Expired, NotYetValid }
/// `leaf` is `chain[0]`; `SecretDetails::Certificate` guarantees a non-empty chain.
pub(crate) fn expiry_state(leaf: &CertificateInfo, now: jiff::Timestamp) -> ExpiryState;
pub(crate) fn expiry_label(not_after: jiff::Timestamp, now: jiff::Timestamp) -> StatusLabel;
/// The earliest not-after among `chain[1..]` when it is before the leaf's.
pub(crate) fn intermediate_expires_first(chain: &[CertificateInfo]) -> Option<jiff::Timestamp>;
```

| Case (first match) | State | `expiry_label` |
|---|---|---|
| `leaf.not_after <= now` | `Expired` | Bad `expired {format_age(not_after, now)} ago` |
| `leaf.not_before > now` | `NotYetValid` | (label by not-after as below) |
| `leaf.not_after - now <= EXPIRY_WARNING` | `ExpiringSoon` | Warn `expires in {format_age(now, not_after)}` |
| else | `Valid` | Ok `{format_age(now, not_after)} left` |

- `KindCell::Expiry { not_after }` paints `expiry_label(not_after, now)` as a toned cell (table and drawer field); `kind_table.rs` `value` → `Number(not_after.as_second())`.
- Dates in box texts: UTC `Oct 7, 2026` (`%b %-d, %Y`); fields: UTC `YYYY-MM-DD HH:MM UTC` (the suffix is part of the text).
- The Secrets **table** shows no expiry tone (decision 19); until 0020/0021 an expiring certificate is visible only in its drawer box and in the Ingresses TLS column.

## Secret CERTIFICATE box (`kind_diagnosis.rs`, step 2)

Arm `KindObject::Secret` with `Certificate` or `NoCertificate`; title `CERTIFICATE`; placed like 0012 WHY boxes; no link.

| Case | Tone | Text |
|---|---|---|
| `Expired` | Bad | `Expired {date} ({n} ago).` |
| `ExpiringSoon` | Warn | `Expires {date} (in {n}).` |
| `NotYetValid` | Warn | `Not valid until {not_before date}.` |
| `NoCertificate(Unparsed)` | Warn | `tls.crt could not be parsed as an X.509 certificate.` |
| `NoCertificate(Missing)` | Warn | `The secret has no tls.crt.` |
| `Valid` | — | no box |

Separately, the **Certificate** section gets a Warn field `Intermediate` → `expires {date}, before the leaf` when `intermediate_expires_first` is `Some` (no box: the leaf date stays the headline).
## Ingresses (step 4)

### TLS-secrets companion (`cluster_session.rs`)

```rust
pub(crate) enum CompanionLists { /* … */ TlsSecrets(LiveList<SecretSummary>) }
enum CompanionUpdate { /* … */ TlsSecrets(WatchUpdate<SecretSummary>) }
pub(crate) enum CompanionKind { /* … */ TlsSecrets }
```

`companion_plan(Ingresses, access)`: `Denied(ListSecrets)` when `Known` and denied, else `Start(TlsSecrets)` with `watch_tls_secrets(scope)`. `OpenWatches.companion = N`; Ingresses stay within the session bound `3N + 4` (`2 + N + N + N + 1`, no related subject).

### Column (`resource_kind.rs`, `network_rows.rs`)

Ingresses: Class 100 · Hosts 260 · Address 180 · **TLS 140** · Age (Ports removed, decision 17). Builder: `tls` empty → `Absent`; else `Text("yes")` until joined.

### Join (`kind_join.rs`, column `INGRESS_TLS`; triggers: explorer and companion)

Index the companion by `(namespace, name)` once per call. Per ingress with TLS, over the distinct `tls[].secret_name` values (`None` entries ignored):

| Case (first match) | Cell |
|---|---|
| companion not Ready or denied | builder `Text("yes")` |
| every entry has no secret name | `Text("default cert")` |
| a named secret is not in the list | Warn `no TLS secret` |
| a named secret is `NoCertificate(_)` | Warn `not parsed` |
| else | `Expiry { earliest leaf not_after over the named secrets }` |

### Drawer

- **TLS** section becomes `Live(IngressTls)` (new `LiveContent`). Per TLS entry: Hosts (`Chips`, `*` when empty); Secret → `Link` to the Secrets row (or `Text` "default certificate"); when the secret is in the Ready companion as `Certificate { chain }`: leaf Subject, Issuer, Not after (`Expiry`), and the Intermediate Warn field as above; not in the list → `Note("No TLS secret {name} in {ns} (missing, or not of type kubernetes.io/tls).")`; denied → `Note("Not permitted: list secrets")`; loading → `Note("Loading…")`.
- **CERTIFICATE box**: `DiagnosisInputs` gains `tls_secrets: Option<&'a [SecretSummary]>` (Ready companion only). Arm `KindObject::Ingress`: the referenced secret with the worst state (Expired before ExpiringSoon; then earliest leaf not-after) → the Secret box text prefixed `{secret}: `; a missing secret → Warn `No TLS secret {name} in {ns}.`; then a link `Open secret {name} →` (`reveal` to the Secrets row, when the kind is enabled).

### ServiceAccount links (0015, step 4)

`access_rows.rs` `service_account_row`: the Secrets section `Field { name, "token reference" | "image pull secret" }` rows become `Link { label: "Token reference" | "Image pull secret", text: name }` to the Secrets row (`ResourceKey` of `(ns, name)`), so the tooltip reads "Open {name}"; the note "Secret contents are never read" stays. Test `service_account_secret_names_link_to_secrets` (`access_rows_tests.rs`).

## Contract for 0020/0021 (not built here)

- Data: `SecretSummary` with `SecretDetails::Certificate { chain }` gives `(namespace, name, leaf)`; `expiry_state(leaf, now)` gives the rule; the Overview row is "`{ns} / {name}` · Expires in {n} ({date})" with an **Open Secret** action (`reveal`).
- Recommendation: **one always-on `watch_tls_secrets(All)`** per live session (one connection, small summaries), started by 0020 with the other always-on watches, rather than a periodic list: a list every 15 min re-transits every private key each time, while a watch transits them once plus on renewal. RBAC denied → the rule is off with a note. 0020 decides under the C13 budget (+1 watch).
