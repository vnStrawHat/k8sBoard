use std::collections::BTreeMap;
use std::io::Write;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use flate2::Compression;
use flate2::write::GzEncoder;
use k8s_openapi::ByteString;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use serde_json::json;

use super::*;

/// Literals that appear nowhere else, so a leak is easy to find.
const CONFIG_SECRET: &str = "config-secret-value";
const MANIFEST_SECRET: &str = "manifest-secret-value";
const NOTES_SECRET: &str = "notes-secret-text";

const MANIFEST: &str = "---
# Source: api/templates/secret.yaml
# echoes manifest-secret-value in a comment
apiVersion: v1
kind: Secret
metadata:
  name: credentials
data:
  token: bWFuaWZlc3Qtc2VjcmV0LXZhbHVl
stringData:
  plain: manifest-secret-value
---
# Source: api/templates/config.yaml
apiVersion: v1
kind: ConfigMap
data:
  mode: fast
";

fn release_value() -> Value {
    json!({
        "info": {"notes": format!("{NOTES_SECRET}\nsecond line\n")},
        "chart": {
            "metadata": {"name": "api", "version": "1.2.3", "appVersion": "4.5.6"},
            "values": {"replicas": 2, "image": {"tag": "stable", "pull": true}},
        },
        "config": {"password": CONFIG_SECRET, "image": {"tag": "edge"}, "debug": false},
        "manifest": MANIFEST,
    })
}

/// A release Secret body as the API server returns it.
fn secret_body(secret_type: &str, release: Option<&[u8]>) -> String {
    let secret = Secret {
        metadata: ObjectMeta {
            namespace: Some("shop".to_owned()),
            name: Some("sh.helm.release.v1.api.v1".to_owned()),
            ..Default::default()
        },
        type_: Some(secret_type.to_owned()),
        data: release.map(|bytes| {
            BTreeMap::from([(RELEASE_DATA_KEY.to_owned(), ByteString(bytes.to_vec()))])
        }),
        ..Default::default()
    };
    serde_json::to_string(&secret).expect("secret serializes")
}

fn payload_of(release: &Value) -> Vec<u8> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(release.to_string().as_bytes())
        .expect("write to memory");
    STANDARD
        .encode(encoder.finish().expect("finish gzip"))
        .into_bytes()
}

fn body_of(release: &Value) -> ReleaseBody {
    decode_release_secret(&secret_body(RELEASE_TYPE, Some(&payload_of(release))))
        .expect("fixture decodes")
}

fn text_of(text: &HelmText) -> &str {
    text.as_str()
}

#[test]
fn mask_values_hides_strings_and_numbers() {
    let mut value = json!({
        "name": "x", "count": 3, "flag": true, "none": null,
        "empty_map": {}, "empty_list": [], "list": ["a", 1, false],
    });
    let hidden = mask_values(&mut value);
    assert_eq!(hidden, 4);
    assert_eq!(
        value,
        json!({
            "name": HIDDEN, "count": HIDDEN, "flag": true, "none": null,
            "empty_map": {}, "empty_list": [], "list": [HIDDEN, HIDDEN, false],
        })
    );
}

#[test]
fn masked_user_values_have_hidden_header() {
    let release = json!({"config": {"a": "x", "b": 2, "c": {"d": "y"}, "e": true}});
    let detail = masked_detail(body_of(&release), EnvValues::Hidden).expect("masks");
    assert_eq!(detail.hidden_user_values, 3);
    let text = text_of(&detail.user_values);
    assert!(
        text.starts_with("# k8sBoard hid 3 values as <hidden>.\n"),
        "{text}"
    );
    assert!(text.contains("e: true"), "{text}");
}

#[test]
fn masked_detail_hides_every_secret_text() {
    let detail = masked_detail(body_of(&release_value()), EnvValues::Hidden).expect("masks");
    for text in [
        &detail.user_values,
        &detail.computed_values,
        &detail.manifest,
    ] {
        let text = text_of(text);
        for secret in [CONFIG_SECRET, MANIFEST_SECRET, NOTES_SECRET] {
            assert!(!text.contains(secret), "{text}");
        }
    }
    assert!(text_of(&detail.manifest).contains("name: credentials"));
}

#[test]
fn revealed_payload_has_values_and_notes_unmasked() {
    let revealed = revealed_texts(body_of(&release_value())).expect("reveals");
    let user = text_of(&revealed.user);
    assert!(user.contains(CONFIG_SECRET), "{user}");
    assert!(!user.contains("k8sBoard hid"), "{user}");
    let computed = text_of(&revealed.computed);
    assert!(computed.contains("replicas: 2"), "{computed}");
    assert!(computed.contains("tag: edge"), "{computed}");
    assert_eq!(
        text_of(&revealed.notes),
        format!("{NOTES_SECRET}\nsecond line\n")
    );
}

#[test]
fn detail_holds_notes_line_count_only() {
    let detail = masked_detail(body_of(&release_value()), EnvValues::Hidden).expect("masks");
    assert_eq!(detail.notes_lines, 2);
    assert_eq!(
        detail.chart.map(|chart| chart.version),
        Some("1.2.3".to_owned())
    );
    let without = masked_detail(body_of(&json!({})), EnvValues::Hidden).expect("masks");
    assert_eq!(without.notes_lines, 0);
}

#[test]
fn missing_config_reads_as_empty_values() {
    for release in [
        json!({}),
        json!({"config": null, "chart": {"values": null}}),
    ] {
        let detail = masked_detail(body_of(&release), EnvValues::Hidden).expect("masks");
        assert_eq!(text_of(&detail.user_values), "{}\n");
        assert_eq!(text_of(&detail.computed_values), "{}\n");
        assert_eq!(detail.hidden_user_values, 0);
    }
}

#[test]
fn coalesce_merges_user_over_defaults() {
    let defaults = json!({"a": {"b": 1, "c": 2}, "keep": "x"});
    let user = json!({"a": {"b": 9}, "new": true});
    assert_eq!(
        coalesce_values(defaults, &user),
        json!({"a": {"b": 9, "c": 2}, "keep": "x", "new": true})
    );
}

#[test]
fn coalesce_user_null_removes_default() {
    let defaults = json!({"a": {"b": 1}, "c": 2});
    let user = json!({"a": null, "gone": null});
    assert_eq!(coalesce_values(defaults, &user), json!({"c": 2}));
}

#[test]
fn coalesce_replaces_arrays_whole() {
    let defaults = json!({"hosts": ["a", "b", "c"], "port": 80});
    let user = json!({"hosts": ["z"], "port": "http"});
    assert_eq!(
        coalesce_values(defaults, &user),
        json!({"hosts": ["z"], "port": "http"})
    );
}

#[test]
fn manifest_masks_secret_data_per_document() {
    let (text, hidden_env) = masked_manifest(MANIFEST, EnvValues::Hidden).expect("masks");
    assert_eq!(hidden_env, 0);
    assert!(!text.contains(MANIFEST_SECRET), "{text}");
    assert!(!text.contains("bWFuaWZlc3Qt"), "{text}");
    assert!(
        text.starts_with("# k8sBoard hid 2 values as <hidden>.\n"),
        "{text}"
    );
    assert!(text.contains("token: <hidden>"), "{text}");
    assert!(text.contains("plain: <hidden>"), "{text}");
    assert!(text.contains("mode: fast"), "{text}");
}

#[test]
fn manifest_keeps_only_source_comments() {
    let (text, _) = masked_manifest(MANIFEST, EnvValues::Hidden).expect("masks");
    assert!(
        text.contains("# Source: api/templates/secret.yaml\n"),
        "{text}"
    );
    assert!(
        text.contains("# Source: api/templates/config.yaml\n"),
        "{text}"
    );
    assert!(!text.contains("echoes"), "{text}");
    assert_eq!(text.matches("---\n").count(), 1, "{text}");
}

#[test]
fn manifest_hides_unreadable_document() {
    let manifest = format!("# Source: a.yaml\nkey: [{MANIFEST_SECRET}\n---\nkind: ConfigMap\n");
    let (text, _) = masked_manifest(&manifest, EnvValues::Hidden).expect("masks");
    assert!(!text.contains(MANIFEST_SECRET), "{text}");
    assert!(text.contains(UNREADABLE_DOCUMENT), "{text}");
    assert!(
        text.starts_with("# k8sBoard hid 1 value as <hidden>.\n"),
        "{text}"
    );
    assert!(text.contains("kind: ConfigMap"), "{text}");
}

#[test]
fn manifest_env_literals_follow_env_values() {
    let manifest = "kind: Deployment\nspec:\n  template:\n    spec:\n      containers:\n        - name: app\n          env:\n            - name: TOKEN\n              value: env-literal-value\n";
    let (hidden, hidden_count) = masked_manifest(manifest, EnvValues::Hidden).expect("masks");
    assert_eq!(hidden_count, 1);
    assert!(!hidden.contains("env-literal-value"), "{hidden}");
    let (shown, shown_count) = masked_manifest(manifest, EnvValues::Shown).expect("masks");
    assert_eq!(shown_count, 0);
    assert!(shown.contains("env-literal-value"), "{shown}");
}

#[test]
fn manifest_masks_last_applied_annotation() {
    let manifest = format!(
        "kind: ConfigMap\nmetadata:\n  annotations:\n    kubectl.kubernetes.io/last-applied-configuration: '{MANIFEST_SECRET}'\n    keep: me\n"
    );
    let (text, _) = masked_manifest(&manifest, EnvValues::Hidden).expect("masks");
    assert!(!text.contains(MANIFEST_SECRET), "{text}");
    assert!(text.contains("keep: me"), "{text}");
}

#[test]
fn manifest_options_raise_budget() {
    let budget = manifest_options().budget.expect("budget is set");
    assert_eq!(budget.max_nodes, 2_000_000);
    assert_eq!(budget.max_events, 8_000_000);
    assert_eq!(budget.max_depth, 128);
}

fn failure_text(result: Result<ReleaseBody, &'static str>) -> &'static str {
    result.err().expect("decoding fails")
}

#[test]
fn detail_errors_have_fixed_text() {
    let valid = payload_of(&release_value());
    let not_json = STANDARD.encode(b"not json").into_bytes();
    let cases = [
        (
            failure_text(decode_release_secret("{broken")),
            "the release secret could not be decoded",
        ),
        (
            failure_text(decode_release_secret(&secret_body("Opaque", Some(&valid)))),
            "the secret is not a Helm release",
        ),
        (
            failure_text(decode_release_secret(&secret_body(RELEASE_TYPE, None))),
            "the release secret has no release data",
        ),
        (
            failure_text(decode_release_secret(&secret_body(
                RELEASE_TYPE,
                Some(b"%%%"),
            ))),
            "the release data is not valid base64",
        ),
        (
            failure_text(decode_release_secret(&secret_body(
                RELEASE_TYPE,
                Some(&STANDARD.encode([0x1f, 0x8b, 0, 0]).into_bytes()),
            ))),
            "the release data could not be decompressed",
        ),
        (
            failure_text(decode_release_secret(&secret_body(
                RELEASE_TYPE,
                Some(&not_json),
            ))),
            "the release could not be decoded",
        ),
        (
            payload_text(PayloadIssue::TooLarge),
            "the release data is larger than 64 MiB",
        ),
    ];
    for (text, expected) in cases {
        assert_eq!(text, expected);
    }
    let error = unexpected("uat", DETAIL_ACTION, "the release data is not valid base64");
    assert!(!error.to_string().contains(CONFIG_SECRET));
}
