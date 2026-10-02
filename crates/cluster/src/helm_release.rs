//! Helm 3 releases from their `helm.sh/release.v1` Secrets: one summary per release, its
//! revision history, and the payload decoder shared with the detail read.
//!
//! A release payload holds user values, rendered manifests, and notes, any of which can carry
//! passwords. Nothing in this module logs or traces, summaries hold no value, and decode errors
//! are fixed texts because serde, flate2, and base64 errors can quote bytes.

use std::collections::BTreeMap;
use std::io::Read;
use std::str::FromStr;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use flate2::read::GzDecoder;
use futures::future::Either;
use futures::{Stream, StreamExt, future, stream};
use k8s_openapi::api::core::v1::Secret;
use kube::Api;
use kube::core::PartialObjectMeta;
use kube::runtime::watcher::{self, ListSemantic};
use serde::Deserialize;
use zeroize::Zeroizing;

use crate::connection::ClusterConnection;
use crate::namespace::NamespaceScope;
use crate::resource_watch::{WatchUpdate, metadata_summary_watch, selected_summary_watch};

pub(crate) const RELEASE_TYPE: &str = "helm.sh/release.v1";
pub(crate) const RELEASE_DATA_KEY: &str = "release";
/// Largest decompressed release accepted: a gzip bomb guard.
pub(crate) const RELEASE_SIZE_LIMIT: u64 = 64 * 1024 * 1024;
/// A release Secret can be about 1 MiB, so 10 per page caps the plaintext in flight.
const WATCH_PAGE_SIZE: u32 = 10;
const GZIP_MAGIC: [u8; 2] = [0x1f, 0x8b];
/// The most a DEFLATE stream can expand: zlib documents about 1032:1.
const MAX_DEFLATE_RATIO: u64 = 1032;
const RELEASES_ACTION: &str = "watching helm releases";
const HISTORY_ACTION: &str = "watching helm history";

/// Helm's release status. `Unknown` carries any text this build does not know, including
/// Helm's own `unknown`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HelmStatus {
    Deployed,
    Failed,
    PendingInstall,
    PendingUpgrade,
    PendingRollback,
    Uninstalling,
    Uninstalled,
    Superseded,
    Unknown(String),
}

impl HelmStatus {
    /// Helm's own text, for example `pending-upgrade`.
    pub fn label(&self) -> &str {
        match self {
            Self::Deployed => "deployed",
            Self::Failed => "failed",
            Self::PendingInstall => "pending-install",
            Self::PendingUpgrade => "pending-upgrade",
            Self::PendingRollback => "pending-rollback",
            Self::Uninstalling => "uninstalling",
            Self::Uninstalled => "uninstalled",
            Self::Superseded => "superseded",
            Self::Unknown(text) => text,
        }
    }
}

fn helm_status(text: &str) -> HelmStatus {
    match text {
        "deployed" => HelmStatus::Deployed,
        "failed" => HelmStatus::Failed,
        "pending-install" => HelmStatus::PendingInstall,
        "pending-upgrade" => HelmStatus::PendingUpgrade,
        "pending-rollback" => HelmStatus::PendingRollback,
        "uninstalling" => HelmStatus::Uninstalling,
        "uninstalled" => HelmStatus::Uninstalled,
        "superseded" => HelmStatus::Superseded,
        other => HelmStatus::Unknown(other.to_owned()),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HelmChart {
    pub name: String,
    pub version: String,
    pub app_version: Option<String>,
}

/// One release, from its highest current revision. `Debug` is safe: no value is held, though
/// `description` is Helm's own text and can quote a field value, so never trace a summary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HelmReleaseSummary {
    pub namespace: String,
    pub name: String,
    pub revision: u32,
    pub status: HelmStatus,
    /// `None` when the payload does not decode.
    pub chart: Option<HelmChart>,
    /// `info.last_deployed`, else the Secret's creation time.
    pub updated_at: Option<jiff::Timestamp>,
    /// `info.description`; empty reads as `None`.
    pub description: Option<String>,
    /// An older revision that is still `deployed`; `None` when the row itself is deployed.
    pub deployed_revision: Option<u32>,
}

/// One history entry, from labels and metadata only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HelmRevision {
    pub revision: u32,
    pub status: HelmStatus,
    pub updated_at: Option<jiff::Timestamp>,
}

/// One stored revision. Its Secret is `sh.helm.release.v1.{release}.v{revision}`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct HelmRevisionRef {
    pub namespace: String,
    pub release: String,
    pub revision: u32,
}

impl HelmRevisionRef {
    pub(crate) fn secret_name(&self) -> String {
        format!("sh.helm.release.v1.{}.v{}", self.release, self.revision)
    }
}

impl ClusterConnection {
    /// Watches the current revision of every release in `scope`, like `helm list --all`.
    /// Yields batched snapshots ordered by (namespace, name).
    pub fn watch_helm_releases(
        &self,
        scope: NamespaceScope,
    ) -> impl Stream<Item = WatchUpdate<HelmReleaseSummary>> + Send + 'static {
        selected_summary_watch(
            self,
            self.scoped_apis(&scope),
            release_watch_config(),
            RELEASES_ACTION,
            revision_head,
        )
        .map(group_update)
    }

    /// Watches every revision of one release, newest first, from metadata only: no payload is
    /// downloaded. An empty `release`, which would select every release, yields one empty
    /// snapshot and ends.
    pub fn watch_helm_history(
        &self,
        namespace: &str,
        release: &str,
    ) -> impl Stream<Item = WatchUpdate<HelmRevision>> + Send + 'static {
        if release.is_empty() {
            return Either::Left(stream::once(future::ready(WatchUpdate::Snapshot(
                Vec::new(),
            ))));
        }
        let api = Api::<PartialObjectMeta<Secret>>::namespaced(self.client().clone(), namespace);
        Either::Right(
            metadata_summary_watch(
                self,
                api,
                history_watch_config(release),
                HISTORY_ACTION,
                history_revision,
            )
            .map(history_update),
        )
    }
}

fn release_watch_config() -> watcher::Config {
    watcher::Config::default()
        .labels("owner=helm,status!=superseded")
        .fields(&format!("type={RELEASE_TYPE}"))
        .list_semantic(ListSemantic::MostRecent)
        .page_size(WATCH_PAGE_SIZE)
}

fn history_watch_config(release: &str) -> watcher::Config {
    watcher::Config::default()
        .labels(&format!("owner=helm,name={release}"))
        .fields(&format!("type={RELEASE_TYPE}"))
}

/// Why a release payload could not be decoded. Each has a fixed text in the detail read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PayloadIssue {
    Missing,
    NotBase64,
    NotGzip,
    TooLarge,
}

/// `data["release"]` (already decoded once by the Secret) to the release JSON: base64, then
/// gzip. The JSON sits in a buffer that is wiped on drop.
pub(crate) fn release_json(release: &[u8]) -> Result<Zeroizing<Vec<u8>>, PayloadIssue> {
    release_json_within(release, RELEASE_SIZE_LIMIT)
}

fn release_json_within(release: &[u8], limit: u64) -> Result<Zeroizing<Vec<u8>>, PayloadIssue> {
    if release.is_empty() {
        return Err(PayloadIssue::Missing);
    }
    let bytes = Zeroizing::new(
        STANDARD
            .decode(release)
            .map_err(|_| PayloadIssue::NotBase64)?,
    );
    // Helm accepts a payload that was never compressed.
    if !bytes.starts_with(&GZIP_MAGIC) {
        return if bytes.len() as u64 > limit {
            Err(PayloadIssue::TooLarge)
        } else {
            Ok(bytes)
        };
    }
    // One reservation, so no outgrown copy of the plaintext is freed unwiped. A wrong ISIZE
    // makes the buffer grow anyway; that regrowth is the known ceiling.
    let mut json = Zeroizing::new(Vec::with_capacity(gzip_output_capacity(&bytes, limit)));
    GzDecoder::new(&bytes[..])
        .take(limit.saturating_add(1))
        .read_to_end(&mut json)
        .map_err(|_| PayloadIssue::NotGzip)?;
    if json.len() as u64 > limit {
        return Err(PayloadIssue::TooLarge);
    }
    Ok(json)
}

/// The uncompressed size a gzip stream declares in its trailer (ISIZE, little-endian), capped
/// at `limit` and at the largest ratio DEFLATE can reach (1032:1), because the trailer is
/// attacker-controlled: a tiny Secret must not reserve (and later wipe) the whole limit on every
/// watch event. A stream too short to have a trailer declares nothing.
fn gzip_output_capacity(gzip: &[u8], limit: u64) -> usize {
    let Some(trailer) = gzip.last_chunk::<4>() else {
        return 0;
    };
    let declared = u64::from(u32::from_le_bytes(*trailer));
    let possible = (gzip.len() as u64).saturating_mul(MAX_DEFLATE_RATIO);
    usize::try_from(declared.min(limit).min(possible)).unwrap_or(0)
}

/// The summary fields of one revision, before grouping. Holds no value.
#[derive(Clone, PartialEq)]
pub(crate) struct RevisionHead {
    namespace: String,
    name: String,
    revision: u32,
    status: HelmStatus,
    created_at: Option<jiff::Timestamp>,
    chart: Option<HelmChart>,
    last_deployed: Option<jiff::Timestamp>,
    description: Option<String>,
}

#[derive(Deserialize)]
struct ReleaseHead {
    info: Option<InfoHead>,
    chart: Option<ChartHead>,
}

#[derive(Deserialize)]
struct InfoHead {
    last_deployed: Option<String>,
    description: Option<String>,
}

#[derive(Deserialize)]
struct ChartHead {
    metadata: Option<ChartMetadata>,
}

/// The `chart.metadata` fields every Helm read needs.
#[derive(Deserialize)]
pub(crate) struct ChartMetadata {
    name: Option<String>,
    version: Option<String>,
    #[serde(rename = "appVersion")]
    app_version: Option<String>,
}

impl ChartMetadata {
    /// `None` when the chart has no name.
    pub(crate) fn into_chart(self) -> Option<HelmChart> {
        let name = self.name.filter(|name| !name.is_empty())?;
        Some(HelmChart {
            name,
            version: self.version.unwrap_or_default(),
            app_version: self.app_version.filter(|version| !version.is_empty()),
        })
    }
}

/// The watch's summarizer. `None` without valid `name` and `version` labels: the labels are the
/// identity of a revision. A payload that does not decode still gives a row, without chart facts.
pub(crate) fn revision_head(secret: &Secret) -> Option<RevisionHead> {
    let labels = secret.metadata.labels.as_ref()?;
    let name = labels.get("name").filter(|name| !name.is_empty())?;
    let revision = labels.get("version")?.parse().ok()?;
    let status = helm_status(labels.get("status").map_or("unknown", String::as_str));
    let head = payload_head(secret);
    let (info, chart) = (head.info, head.chart);
    Some(RevisionHead {
        namespace: secret.metadata.namespace.clone().unwrap_or_default(),
        name: name.clone(),
        revision,
        status,
        created_at: secret
            .metadata
            .creation_timestamp
            .as_ref()
            .map(|time| time.0),
        chart: chart
            .and_then(|chart| chart.metadata)
            .and_then(ChartMetadata::into_chart),
        last_deployed: info
            .as_ref()
            .and_then(|info| info.last_deployed.as_deref())
            .and_then(real_time),
        description: info
            .and_then(|info| info.description)
            .filter(|text| !text.is_empty()),
    })
}

struct PayloadHead {
    info: Option<InfoHead>,
    chart: Option<ChartHead>,
}

/// Decodes the payload once and keeps the head fields; the plaintext buffer is wiped before
/// returning. Any failure reads as an empty head.
fn payload_head(secret: &Secret) -> PayloadHead {
    let empty = PayloadHead {
        info: None,
        chart: None,
    };
    let Some(payload) = secret
        .data
        .as_ref()
        .and_then(|data| data.get(RELEASE_DATA_KEY))
    else {
        return empty;
    };
    let Ok(json) = release_json(&payload.0) else {
        return empty;
    };
    match serde_json::from_slice::<ReleaseHead>(&json) {
        Ok(head) => PayloadHead {
            info: head.info,
            chart: head.chart,
        },
        Err(_) => empty,
    }
}

/// RFC 3339 with an offset. Helm writes Go's zero time for "never", which reads as `None`.
fn real_time(text: &str) -> Option<jiff::Timestamp> {
    let time = jiff::Timestamp::from_str(text).ok()?;
    (time.as_second() >= 0).then_some(time)
}

/// One summary per release; the highest revision is the row, and an older `deployed`
/// revision is named when the row is not deployed itself. Ordered by (namespace, name).
fn group_releases(heads: Vec<Option<RevisionHead>>) -> Vec<HelmReleaseSummary> {
    let mut groups: BTreeMap<(String, String), Vec<RevisionHead>> = BTreeMap::new();
    for head in heads.into_iter().flatten() {
        groups
            .entry((head.namespace.clone(), head.name.clone()))
            .or_default()
            .push(head);
    }
    groups
        .into_values()
        .filter_map(|mut revisions| {
            revisions.sort_by_key(|head| head.revision);
            let row = revisions.pop()?;
            let deployed_revision = if row.status == HelmStatus::Deployed {
                None
            } else {
                revisions
                    .iter()
                    .rev()
                    .find(|head| head.status == HelmStatus::Deployed)
                    .map(|head| head.revision)
            };
            Some(HelmReleaseSummary {
                namespace: row.namespace,
                name: row.name,
                revision: row.revision,
                status: row.status,
                chart: row.chart,
                updated_at: row.last_deployed.or(row.created_at),
                description: row.description,
                deployed_revision,
            })
        })
        .collect()
}

fn group_update(update: WatchUpdate<Option<RevisionHead>>) -> WatchUpdate<HelmReleaseSummary> {
    match update {
        WatchUpdate::Snapshot(heads) => WatchUpdate::Snapshot(group_releases(heads)),
        WatchUpdate::Failed(error) => WatchUpdate::Failed(error),
    }
}

/// The history summarizer: labels and metadata only. `modifiedAt` is Helm's Unix-seconds
/// label, which wins over the creation time.
fn history_revision(secret: &PartialObjectMeta<Secret>) -> Option<HelmRevision> {
    let labels = secret.metadata.labels.as_ref()?;
    let revision = labels.get("version")?.parse().ok()?;
    let modified = labels
        .get("modifiedAt")
        .and_then(|seconds| seconds.parse::<i64>().ok())
        .and_then(|seconds| jiff::Timestamp::from_second(seconds).ok());
    let created = secret
        .metadata
        .creation_timestamp
        .as_ref()
        .map(|time| time.0);
    Some(HelmRevision {
        revision,
        status: helm_status(labels.get("status").map_or("unknown", String::as_str)),
        updated_at: modified.or(created),
    })
}

fn history_update(update: WatchUpdate<Option<HelmRevision>>) -> WatchUpdate<HelmRevision> {
    match update {
        WatchUpdate::Snapshot(revisions) => {
            let mut revisions: Vec<_> = revisions.into_iter().flatten().collect();
            revisions.sort_by_key(|revision| std::cmp::Reverse(revision.revision));
            WatchUpdate::Snapshot(revisions)
        }
        WatchUpdate::Failed(error) => WatchUpdate::Failed(error),
    }
}

#[cfg(test)]
#[path = "helm_release_tests.rs"]
mod helm_release_tests;
