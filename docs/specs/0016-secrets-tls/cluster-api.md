# 0016 · Cluster crate: secrets, certificates, values, probe

[Back to index](README.md) · Step 1 · The 0001/0002 rules apply: summaries only, no kube/k8s-openapi/x509-cert/zeroize type in a public signature, no spawned task. Safety rules: [secret-safety.md](secret-safety.md). The app's log filter (M1a) is in [secret-safety.md](secret-safety.md) "Log filter".

## Cargo

Root `[workspace.dependencies]`: `x509-cert = { version = "0.3", default-features = false }`, `pem = "3"`, `zeroize = "1"`; `crates/cluster/Cargo.toml`: the three, `.workspace = true`.

`pem` 3.0.6 and `zeroize` 1.9 are already in `Cargo.lock`. Expected new packages: `x509-cert`, `der`, `der_derive`, `spki`, `flagset`; the coder records the actual list (AC 4). App Cargo changes (step 3, `windows`) are in [secret-clipboard.md](secret-clipboard.md).

## `certificate.rs` (new; tests in `certificate_tests.rs`)

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CertificateInfo {
    pub subject: String,            // RFC 4514 text of the subject DN, e.g. "CN=shop.example.test,O=Example"
    pub issuer: String,             // same for the issuer
    pub alt_names: Vec<String>,     // SAN dNSName as is, iPAddress as `IpAddr` text; other kinds skipped; file order
    pub not_before: jiff::Timestamp,
    pub not_after: jiff::Timestamp,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CertificateIssue { Missing /* no or empty tls.crt */, Unparsed /* no CERTIFICATE block, or a block the parser rejects */ }
pub(crate) const CHAIN_LIMIT: usize = 10;
/// Leaf first, in file order, never empty; at most `CHAIN_LIMIT`.
pub(crate) fn parse_certificates(pem_text: &[u8]) -> Result<Vec<CertificateInfo>, CertificateIssue>;
```

- `pem::parse_many(pem_text)`; keep blocks tagged `CERTIFICATE`, take `CHAIN_LIMIT`; none or a PEM error → `Unparsed`.
- Each block: `x509_cert::Certificate::from_der(contents)` with the **default (RFC 5280) profile** (decision 24); any error (DER, time conversion) → `Unparsed` for the whole chain.
- Names: the `Name` `Display` impl (RFC 4514). SANs: the `SubjectAltName` extension, `GeneralName::DnsName` and `IpAddress` (4 or 16 octets → `std::net::IpAddr`, else skipped). Validity: `to_unix_duration()` → `jiff::Timestamp::from_second`.
- Nothing else is kept (no key, signature, serial, other extensions). x509-cert 0.3 moved fields behind accessors; follow its docs.
- Fixtures: `crates/cluster/tests/fixtures/tls/` — `leaf.pem` (CN and SANs `shop.example.test`, `10.0.0.1`), `chain.pem` (leaf + CA, the CA expiring **before** the leaf), `garbage.pem`. Made once with Git Bash `openssl` in `.tmp/`; only certificates are committed, never keys; tests assert the dates `openssl x509 -noout -dates` printed.

## `secret.rs` (new; tests in `secret_tests.rs`)

```rust
#[derive(Clone, Debug, PartialEq, Eq)]   // Debug is safe: no value is held
pub struct SecretSummary {
    pub namespace: String, pub name: String, pub created_at: Option<jiff::Timestamp>,
    pub labels: Vec<String>,          // `label_terms`
    pub secret_type: String,          // `type` as written; "Opaque" when unset
    pub keys: Vec<SecretKey>,         // `data` keys, sorted
    pub details: SecretDetails,
    pub is_immutable: bool,
    pub is_owned: bool,               // ownerReferences non-empty
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SecretKey { pub name: String, pub size_bytes: usize, pub is_binary: bool /* not UTF-8; same polarity as ConfigMapKey */ }
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SecretDetails {
    None,
    Certificate { chain: Vec<CertificateInfo> },      // kubernetes.io/tls with a parsed tls.crt; leaf first, non-empty
    NoCertificate(CertificateIssue),                  // kubernetes.io/tls without one
    Registries(Vec<String>),                          // kubernetes.io/dockerconfigjson | dockercfg; sorted, deduped
    ServiceAccountToken { account: Option<String> },  // kubernetes.io/service-account-token
}

/// Plaintext of one key. Wiped on drop; no Debug, Display, Clone, or PartialEq.
pub struct SecretValue { key: String, bytes: zeroize::Zeroizing<Vec<u8>> }
impl SecretValue {
    pub fn new(key: String, bytes: Vec<u8>) -> Self;   // also the app's test constructor
    pub fn key(&self) -> &str;
    pub fn size_bytes(&self) -> usize;
    pub fn as_text(&self) -> Option<&str>;             // None when not UTF-8
}
impl ClusterConnection {
    pub fn watch_secrets(&self, scope: NamespaceScope) -> impl Stream<Item = WatchUpdate<SecretSummary>> + Send + 'static;     // "watching secrets"
    pub fn watch_tls_secrets(&self, scope: NamespaceScope) -> impl Stream<Item = WatchUpdate<SecretSummary>> + Send + 'static; // "watching TLS secrets"
    /// One GET, decoded here (never by kube); every value, sorted by key. "reading secret values"
    pub async fn secret_values(&self, namespace: &str, name: &str) -> Result<Vec<SecretValue>, ClusterError>;
}
pub(crate) fn secret_summary(secret: &Secret) -> SecretSummary;
```

| Detail | Rule |
|---|---|
| TLS | `data["tls.crt"]` missing or empty → `NoCertificate(Missing)`; else `parse_certificates` → `Certificate { chain }` or `NoCertificate(Unparsed)` |
| `Registries` | `.dockerconfigjson`: `serde_json::from_slice::<Value>` → keys of `auths`; `.dockercfg`: top-level keys. Missing or unparsable → empty list |
| `ServiceAccountToken` | annotation `kubernetes.io/service-account.name`, empty → `None` |
| other types (Helm `helm.sh/release.v1` included: 0017 owns its decoding) | `None` |

### Watches (decision 25)

Private `fn secret_watch_config() -> watcher::Config` = `watcher::Config::default().list_semantic(ListSemantic::MostRecent).page_size(50)`, shared by both watches.

- `watch_secrets`: 0012's `selected_summary_watch(self, self.scoped_apis(&scope), secret_watch_config(), ..)`.
- `watch_tls_secrets`: same with `secret_watch_config().fields("type=kubernetes.io/tls")`.
- `MostRecent` leaves `resourceVersion` unset, so the API server pages the initial list from etcd (≤ 50 objects per response) instead of answering from the watch cache in one response, which ignores `limit`.

### `secret_values` (M1b: kube must never deserialize the body)

```rust
let request = Request::new(Secret::url_path(&(), Some(namespace))).get(name, &GetParams::default())
    .map_err(|_| unexpected("the secret request could not be built"))?;
let body = Zeroizing::new(self.run(ACTION, self.client().request_text(request)).await?);
let mut secret: Secret = serde_json::from_str(&body).map_err(|_| unexpected("the secret could not be decoded"))?;
```

- `Client::request` and `Api::get` are not used: on a decode failure they `tracing::warn!` the whole body (kube-client 4.2 `client/mod.rs`). `request_text` returns the body unparsed; errors keep `self.run`'s timeout and `classify_error` (403, 404).
- `unexpected(source)` → `ClusterError::UnexpectedResponse { source }` with fixed text; the serde error is dropped (it can quote content).
- Then: take `data`; move each `ByteString.0` into a `SecretValue` (no copy); zeroize every annotation value (`String: Zeroize`; `last-applied-configuration` holds the base64 data); drop the rest. `body` is wiped on drop.
- No `tracing::` call in `secret.rs` or `certificate.rs`.

## Other changes

| Item | Change |
|---|---|
| `pod.rs` | `PodSummary.image_pull_secrets: Vec<String>` (`spec.imagePullSecrets[].name`, empty names dropped); every `PodSummary` and `Projected` fixture gains the fields (crate and app tests) |
| `container_spec.rs` | `VolumeSource::Projected { config_maps, secrets: Vec<String> }` (`projected.sources[].secret.name`, in order) |
| `object_yaml.rs` | `ObjectKind::Secret` (namespaced, "Secret"), `api_resource` arm; masking unchanged (keyed on response `kind`) |
| `object_count.rs` (0012) | `Secret` arm (`list_metadata`, limit 1) |
| `access_review.rs` | none: `ListSecrets` exists; no get check (decision 7) |
| `lib.rs` | `mod certificate; mod secret;` export `CertificateInfo`, `CertificateIssue`, `SecretSummary`, `SecretKey`, `SecretDetails`, `SecretValue` |

## Probe (`examples/probe.rs`)

- `--watch-seconds`: lines `secrets` and `tls secrets` after the 0015 lines. `--counts`: `count secrets`.
- New `--secrets` (after the yaml section, before the watch section), from the first `watch_secrets(scope)` snapshot (15 s timeout):
  - `secrets {n}: {type} {count} · …` (types sorted by count, then name);
  - `tls certificates {parsed}/{tls} parsed, earliest leaf not-after {RFC 3339} ({ns}/{name})`;
  - `first tls secret {ns}/{name}` (the ui-verifier filter);
  - one `secret_values` call on that secret: `secret values {ns}/{name}: {k} keys, {bytes} bytes` or the error `Display`.
- Never prints a value, a SAN list, or a registry host. Update `USAGE`, the module doc, and 0001 `probe-example.md`.
