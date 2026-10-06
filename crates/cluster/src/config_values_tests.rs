use serde_json::{Value, json};

use super::*;
use crate::fake_api::FakeApi;
use crate::object_write::WritePolicy;

const SECRET_VALUE: &str = "S3cr3t-0047-ZZ";
const SECRET_BASE64: &str = "UzNjcjN0LTAwNDctWlo=";

fn secret_ref() -> ObjectRef {
    ObjectRef::new(
        ObjectKind::Secret,
        Some("payments".to_owned()),
        "api-db".to_owned(),
    )
    .expect("a secret has a namespace")
}

fn config_map_ref() -> ObjectRef {
    ObjectRef::new(
        ObjectKind::ConfigMap,
        Some("payments".to_owned()),
        "api-config".to_owned(),
    )
    .expect("a config map has a namespace")
}

fn secret(extra: Value) -> Secret {
    let mut object = json!({
        "apiVersion": "v1", "kind": "Secret",
        "metadata": {"name": "api-db", "namespace": "payments", "resourceVersion": "42"},
        "type": "Opaque",
        "data": {
            "DB_PASSWORD": STANDARD.encode(SECRET_VALUE),
            "DB_HOST": STANDARD.encode("db.internal"),
            "ca.der": STANDARD.encode([0xff, 0xfe, 0x00]),
        },
    });
    merge(&mut object, extra);
    serde_json::from_value(object).expect("a secret")
}

fn config_map(extra: Value) -> ConfigMap {
    let mut object = json!({
        "apiVersion": "v1", "kind": "ConfigMap",
        "metadata": {"name": "api-config", "namespace": "payments", "resourceVersion": "7"},
        "data": {"app.yaml": "a: 1\n", "mode": "fast"},
        "binaryData": {"BLOB": STANDARD.encode([1, 2, 3, 4])},
    });
    merge(&mut object, extra);
    serde_json::from_value(object).expect("a config map")
}

/// Shallow merge for the fixtures; `metadata` merges one level deeper.
fn merge(object: &mut Value, extra: Value) {
    let Some(extra) = extra.as_object() else {
        return;
    };
    for (key, value) in extra {
        if key == "metadata" {
            for (inner, inner_value) in value.as_object().into_iter().flatten() {
                object["metadata"][inner] = inner_value.clone();
            }
        } else {
            object[key] = value.clone();
        }
    }
}

fn secret_base_fixture() -> ValuesBase {
    secret_base("fake", secret(json!({})), &secret_ref()).expect("an editable secret")
}

fn config_map_base_fixture() -> ValuesBase {
    config_map_base("fake", config_map(json!({})), &config_map_ref())
        .expect("an editable config map")
}

fn new_value(text: &str) -> NewValue {
    NewValue::new(Zeroizing::new(text.to_owned()))
}

fn add(key: &str, text: &str) -> KeyChange {
    KeyChange::Add {
        key: key.to_owned(),
        value: new_value(text),
    }
}

fn set(key: &str, text: &str) -> KeyChange {
    KeyChange::Set {
        key: key.to_owned(),
        value: new_value(text),
    }
}

fn remove(key: &str) -> KeyChange {
    KeyChange::Remove {
        key: key.to_owned(),
    }
}

fn names(base: &ValuesBase) -> Vec<&str> {
    base.keys().iter().map(|key| key.name.as_str()).collect()
}

#[test]
fn secret_base_keeps_names_sizes_and_binary_flags() {
    let base = secret_base_fixture();
    assert_eq!(base.resource_version(), "42");
    assert!(base.is_secret());
    assert_eq!(names(&base), ["DB_HOST", "DB_PASSWORD", "ca.der"]);
    let password = &base.keys()[1];
    assert_eq!(password.size_bytes, SECRET_VALUE.len());
    assert!(matches!(password.content, KeyContent::Hidden));
    assert!(matches!(base.keys()[2].content, KeyContent::Binary));
    assert_eq!(base.keys()[2].size_bytes, 3);
    assert!(
        base.keys()
            .iter()
            .all(|key| key.field == DataField::Data && !matches!(key.content, KeyContent::Text(_)))
    );
}

#[test]
fn secret_base_debug_holds_no_value() {
    let text = format!("{:?}", secret_base_fixture());
    assert!(!text.contains(SECRET_VALUE), "{text}");
    assert!(!text.contains(SECRET_BASE64), "{text}");
    assert!(text.contains("api-db"), "{text}");
}

#[test]
fn helm_release_secret_is_refused() {
    let release = secret(json!({"type": "helm.sh/release.v1"}));
    let error = secret_base("fake", release, &secret_ref()).expect_err("a release record");
    assert!(matches!(error, ValuesBaseError::HelmRelease), "{error}");
}

#[test]
fn owner_helm_label_is_refused_on_both_kinds() {
    let labels = json!({"metadata": {"labels": {"owner": "helm"}}});
    let error =
        secret_base("fake", secret(labels.clone()), &secret_ref()).expect_err("a helm record");
    assert!(matches!(error, ValuesBaseError::HelmRelease), "{error}");
    let error =
        config_map_base("fake", config_map(labels), &config_map_ref()).expect_err("a helm record");
    assert!(matches!(error, ValuesBaseError::HelmRelease), "{error}");
}

#[test]
fn service_account_token_secret_is_refused() {
    let token = secret(json!({"type": "kubernetes.io/service-account-token"}));
    let error = secret_base("fake", token, &secret_ref()).expect_err("a token");
    assert!(
        matches!(error, ValuesBaseError::ServiceAccountToken),
        "{error}"
    );
}

#[test]
fn immutable_secret_and_config_map_are_refused() {
    let error = secret_base("fake", secret(json!({"immutable": true})), &secret_ref())
        .expect_err("immutable");
    assert!(matches!(error, ValuesBaseError::Immutable), "{error}");
    let error = config_map_base(
        "fake",
        config_map(json!({"immutable": true})),
        &config_map_ref(),
    )
    .expect_err("immutable");
    assert!(matches!(error, ValuesBaseError::Immutable), "{error}");
}

#[test]
fn missing_resource_version_is_unusable() {
    let mut object = secret(json!({}));
    object.metadata.resource_version = None;
    let error = secret_base("fake", object, &secret_ref()).expect_err("no version");
    assert!(matches!(error, ValuesBaseError::Cluster(_)), "{error}");
}

#[tokio::test]
async fn other_kind_is_refused() {
    let (connection, api) = FakeApi::connection(WritePolicy::Blocked, |_| (500, String::new()));
    let deployment = ObjectRef::new(
        ObjectKind::Deployment,
        Some("payments".to_owned()),
        "api".to_owned(),
    )
    .expect("a deployment has a namespace");
    let error = connection
        .values_base(&deployment)
        .await
        .expect_err("not editable");
    assert!(matches!(error, ValuesBaseError::NotEditable), "{error}");
    assert!(api.requests().is_empty());
}

#[test]
fn config_map_base_keeps_text_and_marks_large_and_binary() {
    let large = "x".repeat(MAX_INLINE_VALUE + 1);
    let base = config_map_base(
        "fake",
        config_map(json!({"data": {"big": large, "mode": "fast"}})),
        &config_map_ref(),
    )
    .expect("an editable config map");
    assert_eq!(names(&base), ["BLOB", "big", "mode"]);
    assert!(matches!(base.keys()[0].content, KeyContent::Binary));
    assert_eq!(base.keys()[0].field, DataField::BinaryData);
    assert_eq!(base.keys()[0].size_bytes, 4);
    assert!(matches!(base.keys()[1].content, KeyContent::TooLarge));
    assert_eq!(base.keys()[1].size_bytes, MAX_INLINE_VALUE + 1);
    assert!(matches!(&base.keys()[2].content, KeyContent::Text(text) if text == "fast"));
}

#[test]
fn base_notes_read_helm_label_and_owner() {
    let object = config_map(json!({"metadata": {
        "labels": {"app.kubernetes.io/managed-by": "Helm"},
        "ownerReferences": [{"apiVersion": "apps/v1", "kind": "Deployment", "name": "api", "uid": "u"}],
    }}));
    let base = config_map_base("fake", object, &config_map_ref()).expect("editable");
    assert!(base.notes().is_helm_managed);
    assert_eq!(base.notes().owner.as_deref(), Some("Deployment/api"));
    assert!(!config_map_base_fixture().notes().is_helm_managed);
}

#[test]
fn wipe_secret_clears_values_and_annotations() {
    let mut object = secret(json!({"metadata": {"annotations": {"last": SECRET_VALUE}}}));
    wipe_secret(&mut object);
    assert!(object.data.iter().flatten().all(|(_, v)| v.0.is_empty()));
    assert!(
        object
            .metadata
            .annotations
            .iter()
            .flatten()
            .all(|(_, v)| v.is_empty())
    );
}

#[tokio::test]
async fn values_base_reads_a_secret_with_one_get() {
    let body = serde_json::to_string(&secret(json!({}))).expect("json");
    let (connection, api) = FakeApi::connection(WritePolicy::Blocked, move |_| (200, body.clone()));
    let base = connection
        .values_base(&secret_ref())
        .await
        .expect("the base is read");
    assert_eq!(names(&base), ["DB_HOST", "DB_PASSWORD", "ca.der"]);
    let requests = api.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "GET");
    assert_eq!(
        requests[0].path,
        "/api/v1/namespaces/payments/secrets/api-db"
    );
}

#[tokio::test]
async fn values_base_reads_a_config_map() {
    let body = serde_json::to_string(&config_map(json!({}))).expect("json");
    let (connection, api) = FakeApi::connection(WritePolicy::Blocked, move |_| (200, body.clone()));
    let base = connection
        .values_base(&config_map_ref())
        .await
        .expect("the base is read");
    assert_eq!(base.resource_version(), "7");
    assert!(!base.is_secret());
    assert_eq!(api.requests()[0].method, "GET");
}

#[test]
fn edit_refuses_bad_key_names() {
    let base = secret_base_fixture();
    let too_long = "k".repeat(254);
    for name in ["", ".", "..", "a/b", "sp ace", too_long.as_str()] {
        let error = base.edit(vec![add(name, "v")]).expect_err("a bad key");
        assert_eq!(error, ValuesEditError::InvalidKey(name.to_owned()));
    }
    assert!(base.edit(vec![add("a-b_c.d", "v")]).is_ok());
}

#[test]
fn edit_refuses_add_of_existing_key() {
    let base = config_map_base_fixture();
    for key in ["mode", "BLOB"] {
        let error = base.edit(vec![add(key, "v")]).expect_err("exists");
        assert_eq!(error, ValuesEditError::DuplicateKey(key.to_owned()));
    }
}

#[test]
fn edit_refuses_set_or_remove_of_unknown_key() {
    let base = secret_base_fixture();
    let error = base.edit(vec![set("nope", "v")]).expect_err("unknown");
    assert_eq!(error, ValuesEditError::UnknownKey("nope".to_owned()));
    let error = base.edit(vec![remove("nope")]).expect_err("unknown");
    assert_eq!(error, ValuesEditError::UnknownKey("nope".to_owned()));
}

#[test]
fn edit_refuses_text_set_on_binary_or_large_key() {
    let large = "x".repeat(MAX_INLINE_VALUE + 1);
    let base = config_map_base(
        "fake",
        config_map(json!({"data": {"big": large}})),
        &config_map_ref(),
    )
    .expect("editable");
    for key in ["BLOB", "big"] {
        let error = base.edit(vec![set(key, "v")]).expect_err("remove only");
        assert_eq!(error, ValuesEditError::BinaryValue(key.to_owned()));
        assert!(base.edit(vec![remove(key)]).is_ok());
    }
    let secret = secret_base_fixture();
    let error = secret.edit(vec![set("ca.der", "v")]).expect_err("binary");
    assert_eq!(error, ValuesEditError::BinaryValue("ca.der".to_owned()));
}

#[test]
fn edit_refuses_value_over_one_mebibyte() {
    let base = config_map_base_fixture();
    let error = base
        .edit(vec![add("big", &"x".repeat(MAX_VALUE_BYTES + 1))])
        .expect_err("too large");
    assert_eq!(error, ValuesEditError::TooLarge("big".to_owned()));
    // A value of exactly 1 MiB passes the value check; the whole object then no longer fits.
    let error = base
        .edit(vec![add("big", &"x".repeat(MAX_VALUE_BYTES))])
        .expect_err("the object is over the limit");
    assert_eq!(error, ValuesEditError::ObjectTooLarge);
}

#[test]
fn edit_refuses_object_over_one_mebibyte() {
    let base = config_map_base_fixture();
    let chunk = "x".repeat(400 * 1024);
    let error = base
        .edit(vec![add("a", &chunk), add("b", &chunk), add("c", &chunk)])
        .expect_err("over the object limit");
    assert_eq!(error, ValuesEditError::ObjectTooLarge);
    assert!(base.edit(vec![add("a", &chunk), add("b", &chunk)]).is_ok());
}

#[test]
fn removing_keys_frees_room_for_the_estimate() {
    let big = "y".repeat(900 * 1024);
    let base = config_map_base(
        "fake",
        config_map(json!({"data": {"huge": big}})),
        &config_map_ref(),
    )
    .expect("editable");
    let chunk = "x".repeat(600 * 1024);
    assert_eq!(
        base.edit(vec![add("a", &chunk)]).expect_err("too big"),
        ValuesEditError::ObjectTooLarge
    );
    assert!(base.edit(vec![remove("huge"), add("a", &chunk)]).is_ok());
}

#[test]
fn secret_set_is_a_change_even_if_equal() {
    let base = secret_base_fixture();
    let edit = base
        .edit(vec![set("DB_PASSWORD", SECRET_VALUE)])
        .expect("a typed value is a change");
    let fields: Vec<_> = edit.change_paths().collect();
    assert_eq!(fields, ["data[DB_PASSWORD] value changed"]);
}

#[test]
fn newlines_are_kept_in_body() {
    let base = secret_base_fixture();
    let edit = base
        .edit(vec![add("A", "a\nb\n"), add("B", "a\r\nb")])
        .expect("valid");
    let body = values_patch(&edit);
    let decode = |key: &str| {
        let text = body["data"][key].as_str().expect("a string");
        String::from_utf8(STANDARD.decode(text).expect("base64")).expect("utf-8")
    };
    assert_eq!(decode("A"), "a\nb\n");
    assert_eq!(decode("B"), "a\r\nb");
}

#[test]
fn edit_refuses_no_change_and_two_changes_of_one_key() {
    let base = secret_base_fixture();
    assert_eq!(
        base.edit(Vec::new()).expect_err("no change"),
        ValuesEditError::NoChange
    );
    let error = base
        .edit(vec![set("DB_HOST", "a"), remove("DB_HOST")])
        .expect_err("two changes");
    assert_eq!(error, ValuesEditError::DuplicateKey("DB_HOST".to_owned()));
}

#[test]
fn edit_debug_shows_counts_only() {
    let base = secret_base_fixture();
    let change = set("DB_PASSWORD", SECRET_VALUE);
    let edit = base.edit(vec![change.clone()]).expect("valid");
    let paired = edit.changes()[0].clone();
    for text in [
        format!("{edit:?}"),
        format!("{change:?}"),
        format!("{paired:?}"),
        format!("{:?}", edit.changes()),
    ] {
        assert!(!text.contains(SECRET_VALUE), "{text}");
        assert!(!text.contains(SECRET_BASE64), "{text}");
    }
    assert_eq!(format!("{change:?}"), "Set(DB_PASSWORD)");
}

#[test]
fn secret_patch_base64_encodes_data() {
    let base = secret_base_fixture();
    let edit = base
        .edit(vec![set("DB_PASSWORD", SECRET_VALUE), add("NEW", "x")])
        .expect("valid");
    let body = values_patch(&edit);
    assert_eq!(body["data"]["DB_PASSWORD"], SECRET_BASE64);
    assert_eq!(body["data"]["NEW"], STANDARD.encode("x"));
    assert!(body.get("stringData").is_none());
    assert!(body.get("binaryData").is_none());
}

#[test]
fn config_map_patch_sends_text_as_typed() {
    let base = config_map_base_fixture();
    let edit = base.edit(vec![set("mode", "slow\n")]).expect("valid");
    assert_eq!(values_patch(&edit)["data"]["mode"], "slow\n");
}

#[test]
fn removal_is_null_in_its_field() {
    let base = config_map_base_fixture();
    let edit = base
        .edit(vec![remove("BLOB"), remove("mode")])
        .expect("valid");
    let body = values_patch(&edit);
    assert_eq!(body["binaryData"], json!({"BLOB": null}));
    assert_eq!(body["data"], json!({"mode": null}));
}

#[test]
fn patch_holds_only_version_and_changed_keys() {
    let base = secret_base_fixture();
    let edit = base.edit(vec![set("DB_HOST", "h")]).expect("valid");
    let body = values_patch(&edit);
    let mut top: Vec<_> = body.as_object().expect("an object").keys().collect();
    top.sort();
    assert_eq!(top, ["data", "metadata"]);
    assert_eq!(body["metadata"], json!({"resourceVersion": "42"}));
    assert_eq!(body["data"].as_object().expect("an object").len(), 1);
}

#[test]
fn change_paths_are_sorted_by_key_with_markers() {
    let base = config_map_base_fixture();
    let edit = base
        .edit(vec![remove("mode"), add("a", "1"), remove("BLOB")])
        .expect("valid");
    let paths: Vec<_> = edit.change_paths().collect();
    assert_eq!(
        paths,
        [
            "binaryData[BLOB] removed",
            "data[a] added",
            "data[mode] removed"
        ]
    );
}

#[test]
fn label_terms_and_pairs_agree_on_helm() {
    let terms = [
        "app=api".to_owned(),
        "app.kubernetes.io/managed-by=Helm".to_owned(),
    ];
    assert!(terms_are_helm_managed(&terms));
    assert!(!terms_are_helm_managed(&terms[..1]));
    assert!(!terms_are_helm_managed(&[
        "app.kubernetes.io/managed-by=helm".to_owned()
    ]));
    assert!(is_helm_managed([("app.kubernetes.io/managed-by", "Helm")]));
    assert!(!is_helm_managed([("other", "Helm")]));
}
