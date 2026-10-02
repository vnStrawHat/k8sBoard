# 0017 · Cluster crate: release detail, masking, values diff

[Back to index](README.md) · Step 1 · Modules: `helm_release_detail.rs` (new) + `helm_release_detail_tests.rs`, `helm_values_diff.rs` (new) + `helm_values_diff_tests.rs`, `object_yaml.rs`. Safety: [helm-safety.md](helm-safety.md).

## Public API

```rust
/// Text that may hold secret values. Wiped on drop; no Debug, Display, Clone, or PartialEq.
pub struct HelmText(Zeroizing<String>);
impl HelmText { pub fn new(text: String) -> Self; /* also the app's test constructor */ pub fn as_str(&self) -> &str; }
pub struct HelmReleaseDetail {           // masked; derives nothing
    pub chart: Option<HelmChart>,        // of this revision (decision 27: it can be older than the row)
    pub user_values: HelmText,           // masked YAML (decision 11); "{}" when the release has none
    pub hidden_user_values: usize,
    pub computed_values: HelmText,       // masked YAML of coalesce(chart.values, config)
    pub manifest: HelmText,              // masked per document
    pub hidden_env_values: usize,        // 0 when EnvValues::Shown
    pub notes_lines: usize,              // 0 = no notes; the text itself only in HelmRevealed (decision 14)
}
pub struct HelmRevealed { pub user: HelmText, pub computed: HelmText, pub notes: HelmText } // one GET; revealed YAML and notes
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ValueVisibility { #[default] Masked, Revealed }
pub struct HelmValuesDiff { pub user: Vec<ValueChange>, pub computed: Vec<ValueChange>, pub omitted_user: usize, pub omitted_computed: usize }
/// Added: `before` None; removed: `after` None; changed: both. `path` holds keys only.
pub struct ValueChange { pub path: String, pub before: Option<HelmText>, pub after: Option<HelmText> }
impl ClusterConnection {
    pub async fn helm_release_detail(&self, revision: &HelmRevisionRef, env: EnvValues) -> Result<HelmReleaseDetail, ClusterError>; // "reading a helm release"
    pub async fn helm_revealed(&self, revision: &HelmRevisionRef) -> Result<HelmRevealed, ClusterError>;                        // "reading helm values"
    pub async fn helm_values_diff(&self, before: &HelmRevisionRef, after: &HelmRevisionRef, visibility: ValueVisibility)
        -> Result<HelmValuesDiff, ClusterError>;                                                                                  // "comparing helm values"; two GETs, one after the other (decision 16)
}
```

## Fetch (`fn fetch_release(&self, revision, action) -> Result<ReleaseBody, ClusterError>`, private)

1. `self.secret_text(ns, &revision.secret_name(), action)` (0016 extraction; `request_text`, never `Api::get`).
2. `serde_json::from_str::<Secret>`; `type_` must be `helm.sh/release.v1`; move `data["release"].0` into `Zeroizing`.
3. `release_json` ([cluster-api.md](cluster-api.md)); then `serde_json::from_slice::<ReleaseBody>` with only `config: Option<Value>`, `manifest: Option<String>`, `info.notes: Option<String>`, `chart.metadata`, `chart.values: Option<Value>`. `manifest` and `notes` move into `Zeroizing<String>` at once. Ceiling (S2): serde builds these strings through scratch buffers that are freed, not wiped; the decompressed JSON can be up to 64 MiB.
4. Errors → `ClusterError::UnexpectedResponse` with fixed text; library errors are dropped:

| Failure | `source` text |
|---|---|
| Secret JSON | the release secret could not be decoded |
| other `type` | the secret is not a Helm release |
| `PayloadIssue::{Missing, NotBase64, NotGzip, TooLarge}` | the release secret has no release data / the release data is not valid base64 / the release data could not be decompressed / the release data is larger than 64 MiB |
| release JSON | the release could not be decoded |
| YAML serializer | the values could not be converted to YAML |

## `object_yaml.rs` extractions (behavior unchanged; existing tests keep passing)

```rust
pub(crate) const HIDDEN: &str = "<hidden>";
pub(crate) fn yaml_text(value: &Value) -> Result<String, &'static str>;  // today's SerializerOptions + serde_saphyr call
pub(crate) fn with_hidden_header(body: String, hidden: usize) -> String; // today's "# k8sBoard hid …" match
pub(crate) fn mask_manifest_annotations(..); pub(crate) fn mask_secret_data(..); pub(crate) fn mask_env_values(..); // visibility only
```

## Values (decisions 11, 15)

```rust
/// Every string and number leaf → HIDDEN; returns the count. Keys, bools, null, empty {} / [] stay.
fn mask_values(value: &mut Value) -> usize;
/// Helm CoalesceValues: objects merge recursively, user wins, user null removes the key, arrays/scalars replace.
fn coalesce_values(defaults: Value, user: &Value) -> Value;
```

- `config` null or absent → `{}`; `chart.values` null or absent → `{}`.
- Masked text: `mask_values` → `sort_all_objects` → `yaml_text` → `with_hidden_header`. Revealed text (`helm_revealed`): no mask, no header; notes as stored. `notes_lines` = line count of `info.notes` (0 when empty); the notes `String` is wiped in the masked path without being copied. Computed is coalesced before masking.

## Manifest (decision 13)

`fn masked_manifest(manifest: &str, env: EnvValues) -> Result<(String, usize /* env hidden */), &'static str>`:

| Step | Rule |
|---|---|
| split | documents at lines equal to `---` (after `trim_end`) |
| keep | lines starting with `# Source: ` verbatim, at the top of their document; every other comment is dropped (templates can echo values in comments) |
| parse | rest via `serde_saphyr::from_str_with_options::<Value>` with `manifest_options()` (below); `Null` (empty) → Source lines only, or skip when none |
| failure | Source lines + `# k8sBoard could not read this document, so it is hidden.`; counts 1 hidden |
| mask | 0007 rules 2 (annotations), 3 (Secret `data`/`stringData`, keyed on the document's `kind`), 4 (env literals under `spec`, only when `Hidden`); then `sort_all_objects`, `yaml_text` |
| join | documents joined with `---\n`; `with_hidden_header(text, env + other)` |

`fn manifest_options() -> Options` (decision 13, S3): `Budget` and `Options` are `non_exhaustive`, so start from `default()`, set `max_nodes = 2_000_000`, `max_events = 8_000_000`, `max_depth = 128`; other fields stay default. A document over budget is hidden (safe failure, a visible defect). If `serde_json::Value` cannot be deserialized by serde-saphyr, stop and report.

## Values diff (`helm_values_diff.rs`, decisions 16–18)

```rust
// ponytail: fixed caps; add paging or a per-subtree summary if users hit them.
pub(crate) const DIFF_LIMIT: usize = 500;
pub(crate) const DIFF_VALUE_CHARS: usize = 200; // ponytail: long values (PEM blocks) are cut; Reveal + Values tab shows them whole
/// Leaves keyed by path: scalars, empty objects, empty arrays.
fn flatten(value: &Value) -> BTreeMap<String, &Value>;
/// Sorted by path; at most DIFF_LIMIT changes, the rest counted in the second field.
pub(crate) fn values_diff(before: &Value, after: &Value, visibility: ValueVisibility) -> (Vec<ValueChange>, usize);
fn leaf_text(value: &Value, visibility: ValueVisibility) -> String;
```

- Paths: object keys matching `[A-Za-z0-9_-]+` join with `.` (no leading dot); other keys as `["…"]` (JSON-escaped); array items `[i]`. Example: `ingress.annotations["kubernetes.io/ingress.class"]`, `servers[0].host`.
- `leaf_text`: `Masked` and a string or number → `<hidden>`; strings as JSON string literals (quoted, escaped); others as JSON text; cut at `DIFF_VALUE_CHARS` chars plus `…`.
- Both sides hidden and different → still a change (`<hidden> → <hidden>`). Equal leaves are not listed.
- Applied to `config` (user) and to `coalesce_values` (computed) of both revisions.
