//! One stored Helm revision, read on demand: masked values, a masked manifest, a notes line
//! count, and (after an explicit reveal) the values and notes themselves.
//!
//! Every text here can hold passwords. Nothing in this module logs or traces, texts live in
//! `HelmText` (wiped on drop), and errors are fixed texts because serde, flate2, base64, and
//! YAML errors can quote the payload.

use k8s_openapi::api::core::v1::Secret;
use serde::Deserialize;
use serde_json::Value;
use serde_saphyr::Options;
use serde_saphyr::budget::Budget;
use zeroize::{Zeroize, Zeroizing};

use crate::connection::{ClusterConnection, ClusterError};
use crate::helm_release::{
    ChartMetadata, HelmChart, HelmRevisionRef, PayloadIssue, RELEASE_DATA_KEY, RELEASE_TYPE,
    release_json,
};
use crate::helm_values_diff::{HelmValuesDiff, ValueVisibility, values_diff};
use crate::object_yaml::{
    EnvValues, HIDDEN, mask_env_values, mask_manifest_annotations, mask_secret_data,
    with_hidden_header, yaml_text,
};

const DETAIL_ACTION: &str = "reading a helm release";
const REVEAL_ACTION: &str = "reading helm values";
const DIFF_ACTION: &str = "comparing helm values";
/// A manifest of a large chart is several MiB, so the YAML budget is raised well above the
/// default. A document over budget is hidden, which is a visible, safe failure.
const MANIFEST_MAX_NODES: usize = 2_000_000;
const MANIFEST_MAX_EVENTS: usize = 8_000_000;
const MANIFEST_MAX_DEPTH: usize = 128;
const SOURCE_PREFIX: &str = "# Source: ";
const UNREADABLE_DOCUMENT: &str = "# k8sBoard could not read this document, so it is hidden.\n";
const VALUES_CONVERSION_FAILURE: &str = "the values could not be converted to YAML";

/// Text that may hold secret values. Wiped on drop; no `Debug`, `Display`, `Clone`, or
/// `PartialEq`, so it cannot reach a log or a comparison by accident.
pub struct HelmText(Zeroizing<String>);

impl HelmText {
    /// Also the constructor for tests in other crates.
    pub fn new(text: String) -> Self {
        Self(Zeroizing::new(text))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The masked content of one revision. Derives nothing, so it cannot be logged.
pub struct HelmReleaseDetail {
    /// The chart of this revision, which can be older than the release's row.
    pub chart: Option<HelmChart>,
    /// Masked YAML; `{}` when the release has no user values.
    pub user_values: HelmText,
    pub hidden_user_values: usize,
    /// Masked YAML of the chart defaults merged with the user values.
    pub computed_values: HelmText,
    /// Masked YAML, one document per rendered resource.
    pub manifest: HelmText,
    /// Env literals hidden in the manifest; 0 when `EnvValues::Shown`.
    pub hidden_env_values: usize,
    /// 0 means no notes. The text itself is only in `HelmRevealed`.
    pub notes_lines: usize,
}

/// Values and notes as stored, after an explicit reveal.
pub struct HelmRevealed {
    pub user: HelmText,
    pub computed: HelmText,
    pub notes: HelmText,
}

impl ClusterConnection {
    /// One GET of a revision, with values and the manifest masked.
    pub async fn helm_release_detail(
        &self,
        revision: &HelmRevisionRef,
        env: EnvValues,
    ) -> Result<HelmReleaseDetail, ClusterError> {
        let body = self.fetch_release(revision, DETAIL_ACTION).await?;
        masked_detail(body, env).map_err(|text| unexpected(self.context(), DETAIL_ACTION, text))
    }

    /// One GET of a revision, with user values, computed values, and notes unmasked.
    pub async fn helm_revealed(
        &self,
        revision: &HelmRevisionRef,
    ) -> Result<HelmRevealed, ClusterError> {
        let body = self.fetch_release(revision, REVEAL_ACTION).await?;
        revealed_texts(body).map_err(|text| unexpected(self.context(), REVEAL_ACTION, text))
    }

    /// What changed in the values from `before` to `after`: two GETs, one after the other.
    pub async fn helm_values_diff(
        &self,
        before: &HelmRevisionRef,
        after: &HelmRevisionRef,
        visibility: ValueVisibility,
    ) -> Result<HelmValuesDiff, ClusterError> {
        let (before_user, before_computed) =
            self.fetch_release(before, DIFF_ACTION).await?.into_values();
        let (after_user, after_computed) =
            self.fetch_release(after, DIFF_ACTION).await?.into_values();
        let (user, omitted_user) = values_diff(&before_user, &after_user, visibility);
        let (computed, omitted_computed) =
            values_diff(&before_computed, &after_computed, visibility);
        Ok(HelmValuesDiff {
            user,
            computed,
            omitted_user,
            omitted_computed,
        })
    }

    /// One GET through `secret_text`, never `Api::get`: kube logs the whole body when a decode
    /// fails.
    async fn fetch_release(
        &self,
        revision: &HelmRevisionRef,
        action: &'static str,
    ) -> Result<ReleaseBody, ClusterError> {
        let text = self
            .secret_text(&revision.namespace, &revision.secret_name(), action)
            .await?;
        decode_release_secret(&text).map_err(|source| unexpected(self.context(), action, source))
    }
}

/// The parts of a release that the reads need.
struct ReleaseBody {
    chart: Option<HelmChart>,
    config: Value,
    defaults: Value,
    manifest: Zeroizing<String>,
    notes: Zeroizing<String>,
}

impl ReleaseBody {
    /// The user values and the computed values (defaults merged with the user values).
    fn into_values(self) -> (Value, Value) {
        let computed = coalesce_values(self.defaults, &self.config);
        (self.config, computed)
    }
}

#[derive(Deserialize)]
struct ReleaseJson {
    config: Option<Value>,
    manifest: Option<String>,
    info: Option<NotesInfo>,
    chart: Option<ChartBody>,
}

#[derive(Deserialize)]
struct NotesInfo {
    notes: Option<String>,
}

#[derive(Deserialize)]
struct ChartBody {
    metadata: Option<ChartMetadata>,
    values: Option<Value>,
}

/// The release Secret body to its release content. The error is a fixed text.
fn decode_release_secret(body: &str) -> Result<ReleaseBody, &'static str> {
    let mut secret: Secret =
        serde_json::from_str(body).map_err(|_| "the release secret could not be decoded")?;
    if secret.type_.as_deref() != Some(RELEASE_TYPE) {
        return Err("the secret is not a Helm release");
    }
    let payload = secret
        .data
        .as_mut()
        .and_then(|data| data.remove(RELEASE_DATA_KEY))
        .map(|payload| payload.0)
        .unwrap_or_default();
    let payload = Zeroizing::new(payload);
    let json = release_json(&payload).map_err(payload_text)?;
    let release: ReleaseJson =
        serde_json::from_slice(&json).map_err(|_| "the release could not be decoded")?;
    let (metadata, values) = match release.chart {
        Some(chart) => (chart.metadata, chart.values),
        None => (None, None),
    };
    Ok(ReleaseBody {
        chart: metadata.and_then(ChartMetadata::into_chart),
        config: release.config.unwrap_or_else(empty_values),
        defaults: values.unwrap_or_else(empty_values),
        manifest: Zeroizing::new(release.manifest.unwrap_or_default()),
        notes: Zeroizing::new(release.info.and_then(|info| info.notes).unwrap_or_default()),
    })
}

fn payload_text(issue: PayloadIssue) -> &'static str {
    match issue {
        PayloadIssue::Missing => "the release secret has no release data",
        PayloadIssue::NotBase64 => "the release data is not valid base64",
        PayloadIssue::NotGzip => "the release data could not be decompressed",
        PayloadIssue::TooLarge => "the release data is larger than 64 MiB",
    }
}

fn empty_values() -> Value {
    Value::Object(serde_json::Map::new())
}

/// Fixed text only.
fn unexpected(context: &str, action: &'static str, source: &'static str) -> ClusterError {
    ClusterError::UnexpectedResponse {
        context: context.to_owned(),
        action,
        source: source.into(),
    }
}

fn masked_detail(body: ReleaseBody, env: EnvValues) -> Result<HelmReleaseDetail, &'static str> {
    let ReleaseBody {
        chart,
        config: mut user,
        defaults,
        manifest,
        notes,
    } = body;
    let notes_lines = notes.lines().count();
    let (manifest, hidden_env_values) = masked_manifest(&manifest, env)?;
    let mut computed = coalesce_values(defaults, &user);
    let hidden_user_values = mask_values(&mut user);
    let hidden_computed_values = mask_values(&mut computed);
    Ok(HelmReleaseDetail {
        chart,
        user_values: masked_values_text(user, hidden_user_values)?,
        hidden_user_values,
        computed_values: masked_values_text(computed, hidden_computed_values)?,
        manifest: HelmText::new(manifest),
        hidden_env_values,
        notes_lines,
    })
}

fn masked_values_text(mut values: Value, hidden: usize) -> Result<HelmText, &'static str> {
    values.sort_all_objects();
    let body = yaml_text(&values).map_err(|_| VALUES_CONVERSION_FAILURE)?;
    Ok(HelmText::new(with_hidden_header(body, hidden)))
}

fn revealed_texts(body: ReleaseBody) -> Result<HelmRevealed, &'static str> {
    let ReleaseBody {
        config: mut user,
        defaults,
        notes,
        ..
    } = body;
    let mut computed = coalesce_values(defaults, &user);
    user.sort_all_objects();
    computed.sort_all_objects();
    let convert = |values: &Value| {
        yaml_text(values)
            .map(HelmText::new)
            .map_err(|_| VALUES_CONVERSION_FAILURE)
    };
    Ok(HelmRevealed {
        user: convert(&user)?,
        computed: convert(&computed)?,
        notes: HelmText(notes),
    })
}

/// Every string and number leaf becomes `<hidden>`; returns how many. Keys, booleans, `null`,
/// and empty maps and arrays stay: they show structure and switches, never a secret.
fn mask_values(value: &mut Value) -> usize {
    match value {
        Value::String(text) => {
            text.zeroize();
            *value = Value::from(HIDDEN);
            1
        }
        Value::Number(_) => {
            *value = Value::from(HIDDEN);
            1
        }
        Value::Object(map) => map.values_mut().map(mask_values).sum(),
        Value::Array(items) => items.iter_mut().map(mask_values).sum(),
        Value::Bool(_) | Value::Null => 0,
    }
}

/// Helm's `CoalesceValues`: maps merge recursively, the user wins, a user `null` removes the
/// key, and arrays and scalars replace whole.
fn coalesce_values(defaults: Value, user: &Value) -> Value {
    let (Value::Object(mut merged), Value::Object(overrides)) = (defaults, user) else {
        return user.clone();
    };
    for (key, value) in overrides {
        let Some(existing) = merged.remove(key) else {
            if !value.is_null() {
                merged.insert(key.clone(), value.clone());
            }
            continue;
        };
        if value.is_null() {
            continue;
        }
        let value = if existing.is_object() && value.is_object() {
            coalesce_values(existing, value)
        } else {
            value.clone()
        };
        merged.insert(key.clone(), value);
    }
    Value::Object(merged)
}

/// Masks the rendered manifest document by document: annotations that embed an applied manifest,
/// Secret `data`, and (unless shown) env literals. Only `# Source:` comments survive, because
/// templates can echo values in other comments. Returns the text and the env literals hidden.
fn masked_manifest(manifest: &str, env: EnvValues) -> Result<(String, usize), &'static str> {
    let mut texts = Vec::new();
    let mut hidden_env = 0;
    let mut hidden_other = 0;
    for document in split_documents(manifest) {
        let sources: String = document
            .lines()
            .filter(|line| line.starts_with(SOURCE_PREFIX))
            .map(|line| format!("{line}\n"))
            .collect();
        let parsed = serde_saphyr::from_str_with_options::<Value>(&document, manifest_options());
        let mut value = match parsed {
            Ok(Value::Null) if sources.is_empty() => continue,
            Ok(Value::Null) => {
                texts.push(sources);
                continue;
            }
            Ok(value) => value,
            Err(_) => {
                hidden_other += 1;
                texts.push(format!("{sources}{UNREADABLE_DOCUMENT}"));
                continue;
            }
        };
        hidden_other += mask_manifest_annotations(&mut value, false) + mask_secret_data(&mut value);
        if env == EnvValues::Hidden
            && let Some(spec) = value.get_mut("spec")
        {
            hidden_env += mask_env_values(spec);
        }
        value.sort_all_objects();
        let body = yaml_text(&value).map_err(|_| VALUES_CONVERSION_FAILURE)?;
        texts.push(format!("{sources}{body}"));
    }
    let text = with_hidden_header(texts.join("---\n"), hidden_env + hidden_other);
    Ok((text, hidden_env))
}

/// Documents split at lines that read `---`. Empty documents are dropped by the caller.
fn split_documents(manifest: &str) -> Vec<Zeroizing<String>> {
    let mut documents = vec![Zeroizing::new(String::new())];
    for line in manifest.lines() {
        if line.trim_end() == "---" {
            documents.push(Zeroizing::new(String::new()));
            continue;
        }
        if let Some(current) = documents.last_mut() {
            current.push_str(line);
            current.push('\n');
        }
    }
    documents
}

/// `Budget` and `Options` are non-exhaustive, so they start from `default()`.
fn manifest_options() -> Options {
    let mut budget = Budget::default();
    budget.max_nodes = MANIFEST_MAX_NODES;
    budget.max_events = MANIFEST_MAX_EVENTS;
    budget.max_depth = MANIFEST_MAX_DEPTH;
    let mut options = Options::default();
    options.budget = Some(budget);
    options
}

#[cfg(test)]
#[path = "helm_release_detail_tests.rs"]
mod helm_release_detail_tests;
