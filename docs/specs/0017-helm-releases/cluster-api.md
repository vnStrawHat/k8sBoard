# 0017 · Cluster crate: summaries, payload decode, watches, probe

[Back to index](README.md) · Step 1 · The 0001/0002 rules apply: summaries only, no kube/k8s-openapi/flate2/base64/zeroize type in a public signature, no spawned task. Safety: [helm-safety.md](helm-safety.md). Detail GET, masking, diff: [release-detail.md](release-detail.md).

## Cargo

| File | Change |
|---|---|
| root `Cargo.toml` | `flate2 = "1"`, `base64 = "0.22"`; `serde-saphyr` features `["serialize", "deserialize"]` |
| `crates/cluster/Cargo.toml` | `flate2`, `base64`, `serde` (derive), `.workspace = true` (`zeroize` comes from 0016) |

All are locked (`flate2` 1.1.10 with `miniz_oxide`/`crc32fast`; `base64` 0.22.1 via kube-client; serde-saphyr 1.3 `deserialize` deps `granit-parser`, `annotate-snippets`, `smallvec`, `encoding_rs_io`). AC 4: no new package; anything else, stop and report.

## Types (`helm_release.rs`, tests in `helm_release_tests.rs`)

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HelmStatus { Deployed, Failed, PendingInstall, PendingUpgrade, PendingRollback,
    Uninstalling, Uninstalled, Superseded, Unknown(String) }
impl HelmStatus { pub fn label(&self) -> &str; } // Helm's text: "deployed", "pending-upgrade", the unknown text as is
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HelmChart { pub name: String, pub version: String, pub app_version: Option<String> }
/// One release, from its highest current revision. Debug is safe: no value is held.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HelmReleaseSummary {
    pub namespace: String, pub name: String,
    pub revision: u32,
    pub status: HelmStatus,                    // label `status`
    pub chart: Option<HelmChart>,              // None when the payload does not decode (decision 8)
    pub updated_at: Option<jiff::Timestamp>,   // info.last_deployed, else the Secret's creationTimestamp
    pub description: Option<String>,           // info.description, empty → None
    pub deployed_revision: Option<u32>,        // decision 7; None when the row itself is deployed
}
/// One history entry, from labels and metadata only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HelmRevision { pub revision: u32, pub status: HelmStatus, pub updated_at: Option<jiff::Timestamp> }
/// One stored revision. Its Secret is `sh.helm.release.v1.{release}.v{revision}`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct HelmRevisionRef { pub namespace: String, pub release: String, pub revision: u32 }
impl HelmRevisionRef { pub(crate) fn secret_name(&self) -> String; }
impl ClusterConnection {
    pub fn watch_helm_releases(&self, scope: NamespaceScope) -> impl Stream<Item = WatchUpdate<HelmReleaseSummary>> + Send + 'static; // "watching helm releases"
    /// Every revision of one release, newest first.
    pub fn watch_helm_history(&self, namespace: &str, release: &str) -> impl Stream<Item = WatchUpdate<HelmRevision>> + Send + 'static; // "watching helm history"
}
```

## Payload decode (shared with the detail GET)

```rust
pub(crate) const RELEASE_SIZE_LIMIT: u64 = 64 * 1024 * 1024;
pub(crate) enum PayloadIssue { Missing, NotBase64, NotGzip, TooLarge } // fixed texts in release-detail.md
/// `data["release"]` bytes (already base64-decoded once by k8s-openapi) → release JSON bytes.
pub(crate) fn release_json(release: &[u8]) -> Result<Zeroizing<Vec<u8>>, PayloadIssue>; // = release_json_within(release, RELEASE_SIZE_LIMIT)
```

- `base64::engine::general_purpose::STANDARD.decode` into `Zeroizing<Vec<u8>>`; error → `NotBase64`.
- Starts with `1f 8b` → read the gzip ISIZE trailer (last 4 bytes, little-endian), `Zeroizing::new(Vec::with_capacity(min(isize, limit)))` once (decision 9), then `GzDecoder::new(&bytes[..]).take(limit + 1).read_to_end(&mut out)`; error → `NotGzip`; over the limit → `TooLarge`. Otherwise the bytes are the JSON (decision 10), same limit. Ceiling: a wrong ISIZE regrows `out` and frees unwiped copies.
- No step logs or keeps an error (they can quote bytes).

## Summarizer and grouping (private)

```rust
#[derive(Clone, PartialEq)] // no Debug needed; holds no value
struct RevisionHead { namespace: String, name: String, revision: u32, status: HelmStatus,
    created_at: Option<jiff::Timestamp>, chart: Option<HelmChart>, last_deployed: Option<jiff::Timestamp>, description: Option<String> }
fn revision_head(secret: &Secret) -> Option<RevisionHead>;          // the watch's `summarize`
fn group_releases(heads: Vec<Option<RevisionHead>>) -> Vec<HelmReleaseSummary>; // sorted by (namespace, name)
#[derive(Deserialize)] struct ReleaseHead { #[serde(default)] info: InfoHead, #[serde(default)] chart: ChartHead }
// InfoHead { last_deployed: Option<String>, description: Option<String> }
// ChartHead { metadata: Option<ChartMetadata { name, version, #[serde(rename = "appVersion")] app_version: Option<String> }> }
```

| Rule | Detail |
|---|---|
| identity | labels `name` (non-empty) and `version` (`u32`); missing or invalid → `None` (decision 8) |
| status | label `status` via `helm_status(text)`: the 8 Helm texts map to variants; anything else, including Helm's own `unknown`, → `Unknown(text)` |
| payload | `release_json` then `serde_json::from_slice::<ReleaseHead>`; any failure → `chart: None`, `last_deployed: None`, `description: None`; the buffer drops (wiped) before return |
| times | RFC 3339 with offset (`jiff::Timestamp` parse); before 1970 (Go zero time) → `None` |
| grouping | per `(namespace, name)`: the highest revision is the row; `deployed_revision` = highest other revision with `Deployed` when the row is not `Deployed` |

`watch_helm_releases` = 0012 `selected_summary_watch(self, self.scoped_apis(&scope), release_watch_config(), ACTION, revision_head)` mapped by `group_update` (`Snapshot` → `group_releases`, `Failed` passes through).

```rust
fn release_watch_config() -> watcher::Config {
    watcher::Config::default().labels("owner=helm,status!=superseded").fields("type=helm.sh/release.v1")
        .list_semantic(ListSemantic::MostRecent).page_size(10)
}
fn history_watch_config(release: &str) -> watcher::Config; // labels "owner=helm,name={release}", same field selector
```

## History watch (metadata only)

```rust
// resource_watch.rs
pub(crate) fn metadata_summary_watch<K, T>(connection: &ClusterConnection, api: Api<K>, config: watcher::Config,
    action: &'static str, summarize: fn(&PartialObjectMeta<K>) -> T) -> impl Stream<Item = WatchUpdate<T>> + Send + 'static;
```

- Body: `kube::runtime::metadata_watcher(api, config).default_backoff()` fed to the existing `batch_updates` (one api, no merge).
- `watch_helm_history`: `Api::<Secret>::namespaced`, `history_watch_config`, summarizer `history_revision(&PartialObjectMeta<Secret>) -> Option<HelmRevision>` (labels `version`, `status`; `updated_at` = label `modifiedAt` (Unix seconds) else `creationTimestamp`); the stream drops `None` and sorts by revision, newest first.
- An empty `release` emits `Snapshot(vec![])` once and ends (0012 empty-selector rule).

## Other changes

| Item | Change |
|---|---|
| `secret.rs` (created by 0016; **0016 step 1 must be merged first**) | extract `pub(crate) async fn secret_text(&self, namespace: &str, name: &str, action: &'static str) -> Result<Zeroizing<String>, ClusterError>` (the `Request::get` + `request_text` + `run` lines); `secret_values` calls it |
| `object_yaml.rs` | extractions listed in [release-detail.md](release-detail.md) |
| `lib.rs` | `mod helm_release; mod helm_release_detail; mod helm_values_diff;` export `HelmStatus`, `HelmChart`, `HelmReleaseSummary`, `HelmRevision`, `HelmRevisionRef`, `HelmText`, `HelmReleaseDetail`, `HelmRevealed`, `HelmValuesDiff`, `ValueChange`, `ValueVisibility` |
| `object_count.rs`, `access_review.rs` | none (decision 23; `ListSecrets` exists) |

## Probe (`examples/probe.rs`)

- `--watch-seconds`: line `helm releases` after the 0016 lines.
- New `--helm`, from the first `watch_helm_releases(scope)` snapshot (30 s timeout):
  - `helm releases {n}: {status} {count} · …; payload decoded {d}/{n}`;
  - `first helm release {ns}/{name} rev {r}` (the ui-verifier filter);
  - `helm history {ns}/{name}: {k} revisions` (first `watch_helm_history` snapshot);
  - `helm detail {ns}/{name} rev {r}: values {l} lines ({h} hidden), computed {l} lines, manifest {d} documents {l} lines ({e} env values hidden), notes {n} lines, description {c} chars` or the error `Display`;
  - with an earlier revision: `helm diff {ns}/{name} rev {p} → {r}: user {u} changes, computed {c} changes` (masked).
- Never prints values, manifest or notes text, description text, or chart files. Update `USAGE`, the module doc, 0001 `probe-example.md`.
