use std::collections::BTreeMap;
use std::io::Write;

use flate2::Compression;
use flate2::write::GzEncoder;
use k8s_openapi::ByteString;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{ObjectMeta, Time};
use kube::core::PartialObjectMetaExt;
use serde_json::json;

use super::*;

/// Literals that appear nowhere else, so a leak is easy to find.
const CONFIG_SECRET: &str = "config-secret-value";
const MANIFEST_SECRET: &str = "manifest-secret-value";
const NOTES_SECRET: &str = "notes-secret-text";

fn release_body(last_deployed: &str) -> serde_json::Value {
    json!({
        "name": "api",
        "info": {
            "first_deployed": "2024-01-01T00:00:00Z",
            "last_deployed": last_deployed,
            "description": "Upgrade complete",
            "status": "deployed",
            "notes": NOTES_SECRET,
        },
        "chart": {
            "metadata": {"name": "api", "version": "1.2.3", "appVersion": "4.5.6"},
            "values": {"replicas": 2},
            "templates": [{"name": "templates/secret.yaml", "data": "e30="}],
        },
        "config": {"password": CONFIG_SECRET},
        "manifest": format!("kind: Secret\ndata:\n  token: {MANIFEST_SECRET}\n"),
    })
}

fn gzip(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(bytes).expect("write to memory");
    encoder.finish().expect("finish gzip")
}

/// What `data["release"]` holds: base64 of gzip of the JSON.
fn payload(json: &serde_json::Value) -> Vec<u8> {
    STANDARD
        .encode(gzip(json.to_string().as_bytes()))
        .into_bytes()
}

fn labels(name: &str, version: u32, status: &str) -> BTreeMap<String, String> {
    [
        ("owner", "helm".to_owned()),
        ("name", name.to_owned()),
        ("version", version.to_string()),
        ("status", status.to_owned()),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_owned(), value))
    .collect()
}

fn timestamp(text: &str) -> jiff::Timestamp {
    text.parse().expect("test timestamp")
}

/// A release Secret as the API server stores it.
fn release_secret(
    namespace: &str,
    name: &str,
    revision: u32,
    status: &str,
    payload: Option<Vec<u8>>,
) -> Secret {
    Secret {
        metadata: ObjectMeta {
            namespace: Some(namespace.to_owned()),
            name: Some(format!("sh.helm.release.v1.{name}.v{revision}")),
            labels: Some(labels(name, revision, status)),
            creation_timestamp: Some(Time(timestamp("2024-05-01T00:00:00Z"))),
            ..Default::default()
        },
        type_: Some(RELEASE_TYPE.to_owned()),
        data: payload
            .map(|bytes| BTreeMap::from([(RELEASE_DATA_KEY.to_owned(), ByteString(bytes))])),
        ..Default::default()
    }
}

fn head(secret: &Secret) -> RevisionHead {
    revision_head(secret).expect("identity labels")
}

fn simple_head(namespace: &str, name: &str, revision: u32, status: &str) -> Option<RevisionHead> {
    revision_head(&release_secret(namespace, name, revision, status, None))
}

#[test]
fn release_json_decodes_base64_gzip() {
    let json = release_body("2024-02-03T04:05:06Z");
    let decoded = release_json(&payload(&json)).expect("decodes");
    assert_eq!(decoded.as_slice(), json.to_string().as_bytes());
}

#[test]
fn release_json_accepts_plain_json() {
    let plain = STANDARD.encode(b"{\"name\":\"api\"}").into_bytes();
    let decoded = release_json(&plain).expect("decodes");
    assert_eq!(decoded.as_slice(), b"{\"name\":\"api\"}");
}

#[test]
fn release_json_rejects_bad_base64_and_bad_gzip() {
    assert_eq!(
        release_json(b"not base64 !!").err(),
        Some(PayloadIssue::NotBase64)
    );
    let mut broken = gzip(b"{\"name\":\"api\"}");
    broken.truncate(broken.len() / 2);
    let payload = STANDARD.encode(broken).into_bytes();
    assert_eq!(release_json(&payload).err(), Some(PayloadIssue::NotGzip));
    assert_eq!(release_json(b"").err(), Some(PayloadIssue::Missing));
}

#[test]
fn release_json_stops_at_size_limit() {
    let at_limit = vec![b' '; 1024];
    let over_limit = vec![b' '; 1025];
    for (bytes, expected) in [
        (gzip(&at_limit), None),
        (gzip(&over_limit), Some(PayloadIssue::TooLarge)),
        (at_limit.clone(), None),
        (over_limit.clone(), Some(PayloadIssue::TooLarge)),
    ] {
        let payload = STANDARD.encode(bytes).into_bytes();
        assert_eq!(release_json_within(&payload, 1024).err(), expected);
    }
}

#[test]
fn gzip_capacity_reads_isize_capped_at_limit() {
    let bytes = gzip(&vec![b' '; 300]);
    assert_eq!(gzip_output_capacity(&bytes, 1024), 300);
    assert_eq!(gzip_output_capacity(&bytes, 100), 100);
    assert_eq!(gzip_output_capacity(&[0x1f, 0x8b], 1024), 0);
}

#[test]
fn gzip_capacity_ignores_a_lying_isize() {
    // A small stream whose trailer claims 4 GiB - 1 must not reserve the whole limit.
    let mut bytes = gzip(b"{}");
    let end = bytes.len();
    bytes[end - 4..].copy_from_slice(&u32::MAX.to_le_bytes());
    let capacity = gzip_output_capacity(&bytes, RELEASE_SIZE_LIMIT);
    assert!(capacity <= bytes.len() * 1032, "{capacity}");
    assert!(capacity < 64 * 1024, "{capacity}");
}

#[test]
fn revision_head_reads_labels_and_chart() {
    let json = release_body("2024-02-03T04:05:06.123456789+07:00");
    let secret = release_secret("shop", "api", 7, "deployed", Some(payload(&json)));
    let head = head(&secret);
    assert_eq!(head.namespace, "shop");
    assert_eq!(head.name, "api");
    assert_eq!(head.revision, 7);
    assert_eq!(head.status, HelmStatus::Deployed);
    assert_eq!(
        head.chart,
        Some(HelmChart {
            name: "api".to_owned(),
            version: "1.2.3".to_owned(),
            app_version: Some("4.5.6".to_owned()),
        })
    );
    assert_eq!(
        head.last_deployed,
        Some(timestamp("2024-02-02T21:05:06.123456789Z"))
    );
    assert_eq!(head.description.as_deref(), Some("Upgrade complete"));
}

#[test]
fn revision_head_without_identity_labels_is_none() {
    let mut no_name = release_secret("shop", "api", 1, "deployed", None);
    no_name.metadata.labels = Some(BTreeMap::from([("version".to_owned(), "1".to_owned())]));
    let mut bad_version = release_secret("shop", "api", 1, "deployed", None);
    if let Some(labels) = bad_version.metadata.labels.as_mut() {
        labels.insert("version".to_owned(), "one".to_owned());
    }
    let mut no_labels = release_secret("shop", "api", 1, "deployed", None);
    no_labels.metadata.labels = None;
    for secret in [no_name, bad_version, no_labels] {
        assert!(revision_head(&secret).is_none());
    }
}

#[test]
fn revision_head_with_bad_payload_keeps_row() {
    let secret = release_secret("shop", "api", 2, "failed", Some(b"%%%".to_vec()));
    let head = head(&secret);
    assert_eq!(head.chart, None);
    assert_eq!(head.last_deployed, None);
    assert_eq!(head.description, None);
    let summary = group_releases(vec![Some(head)]);
    assert_eq!(
        summary[0].updated_at,
        Some(timestamp("2024-05-01T00:00:00Z"))
    );
}

#[test]
fn go_zero_time_reads_as_none() {
    assert_eq!(real_time("0001-01-01T00:00:00Z"), None);
    assert_eq!(real_time("not a time"), None);
    assert!(real_time("2024-02-03T04:05:06Z").is_some());
}

#[test]
fn helm_status_parses_every_helm_text() {
    let cases = [
        ("deployed", HelmStatus::Deployed),
        ("failed", HelmStatus::Failed),
        ("pending-install", HelmStatus::PendingInstall),
        ("pending-upgrade", HelmStatus::PendingUpgrade),
        ("pending-rollback", HelmStatus::PendingRollback),
        ("uninstalling", HelmStatus::Uninstalling),
        ("uninstalled", HelmStatus::Uninstalled),
        ("superseded", HelmStatus::Superseded),
        ("unknown", HelmStatus::Unknown("unknown".to_owned())),
        ("whatever", HelmStatus::Unknown("whatever".to_owned())),
    ];
    for (text, status) in cases {
        assert_eq!(helm_status(text), status);
        assert_eq!(status.label(), text);
    }
}

#[test]
fn releases_group_by_highest_revision() {
    let heads = vec![
        simple_head("shop", "web", 2, "deployed"),
        simple_head("shop", "api", 4, "deployed"),
        simple_head("blue", "zeta", 1, "deployed"),
        simple_head("shop", "api", 5, "deployed"),
        None,
    ];
    let summaries = group_releases(heads);
    let keys: Vec<_> = summaries
        .iter()
        .map(|summary| {
            (
                summary.namespace.as_str(),
                summary.name.as_str(),
                summary.revision,
            )
        })
        .collect();
    assert_eq!(
        keys,
        [("blue", "zeta", 1), ("shop", "api", 5), ("shop", "web", 2)]
    );
}

#[test]
fn failed_latest_names_deployed_revision() {
    let heads = vec![
        simple_head("shop", "api", 3, "deployed"),
        simple_head("shop", "api", 4, "failed"),
        simple_head("shop", "api", 2, "deployed"),
        simple_head("shop", "web", 1, "failed"),
    ];
    let summaries = group_releases(heads);
    assert_eq!(summaries[0].status, HelmStatus::Failed);
    assert_eq!(summaries[0].revision, 4);
    assert_eq!(summaries[0].deployed_revision, Some(3));
    assert_eq!(summaries[1].deployed_revision, None);
    let deployed = group_releases(vec![simple_head("shop", "api", 4, "deployed")]);
    assert_eq!(deployed[0].deployed_revision, None);
}

#[test]
fn summary_debug_holds_no_payload_text() {
    let json = release_body("2024-02-03T04:05:06Z");
    let secret = release_secret("shop", "api", 7, "deployed", Some(payload(&json)));
    let text = format!("{:?}", group_releases(vec![Some(head(&secret))]));
    for secret in [CONFIG_SECRET, MANIFEST_SECRET, NOTES_SECRET] {
        assert!(!text.contains(secret), "{text}");
    }
}

#[test]
fn release_watch_config_selects_current_revisions() {
    let config = release_watch_config();
    assert_eq!(
        config.label_selector.as_deref(),
        Some("owner=helm,status!=superseded")
    );
    assert_eq!(
        config.field_selector.as_deref(),
        Some("type=helm.sh/release.v1")
    );
    assert_eq!(config.list_semantic, ListSemantic::MostRecent);
    assert_eq!(config.page_size, Some(10));
}

#[test]
fn history_watch_config_selects_one_release() {
    let config = history_watch_config("api");
    assert_eq!(
        config.label_selector.as_deref(),
        Some("owner=helm,name=api")
    );
    assert_eq!(
        config.field_selector.as_deref(),
        Some("type=helm.sh/release.v1")
    );
}

fn history_secret(revision: u32, modified_at: Option<&str>) -> PartialObjectMeta<Secret> {
    let mut labels = labels("api", revision, "superseded");
    if let Some(seconds) = modified_at {
        labels.insert("modifiedAt".to_owned(), seconds.to_owned());
    }
    ObjectMeta {
        labels: Some(labels),
        creation_timestamp: Some(Time(timestamp("2024-05-01T00:00:00Z"))),
        ..Default::default()
    }
    .into_response_partial::<Secret>()
}

#[test]
fn history_revision_reads_modified_at_label() {
    let revision = history_revision(&history_secret(3, Some("1700000000"))).expect("revision");
    assert_eq!(revision.revision, 3);
    assert_eq!(revision.status, HelmStatus::Superseded);
    assert_eq!(revision.updated_at, Some(timestamp("2023-11-14T22:13:20Z")));
    let created = history_revision(&history_secret(3, None)).expect("revision");
    assert_eq!(created.updated_at, Some(timestamp("2024-05-01T00:00:00Z")));
}

#[test]
fn history_sorts_newest_first() {
    let items = [2, 10, 9]
        .into_iter()
        .map(|revision| history_revision(&history_secret(revision, None)))
        .chain([None])
        .collect();
    let WatchUpdate::Snapshot(revisions) = history_update(WatchUpdate::Snapshot(items)) else {
        panic!("expected a snapshot");
    };
    let numbers: Vec<_> = revisions.iter().map(|revision| revision.revision).collect();
    assert_eq!(numbers, [10, 9, 2]);
}

#[test]
fn secret_name_of_revision() {
    let revision = HelmRevisionRef {
        namespace: "shop".to_owned(),
        release: "api".to_owned(),
        revision: 38,
    };
    assert_eq!(revision.secret_name(), "sh.helm.release.v1.api.v38");
}
