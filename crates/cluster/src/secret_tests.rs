use std::collections::BTreeMap;

use k8s_openapi::ByteString;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{ObjectMeta, OwnerReference};

use super::*;

/// A literal that appears nowhere else, so a leak is easy to find.
const DISTINCTIVE: &str = "s3cr3t-0016-VALUE";
const LEAF: &[u8] = include_bytes!("../tests/fixtures/tls/leaf.pem");

fn secret_of(secret_type: Option<&str>, data: &[(&str, &[u8])]) -> Secret {
    Secret {
        metadata: ObjectMeta {
            namespace: Some("shop".to_owned()),
            name: Some("credentials".to_owned()),
            ..Default::default()
        },
        type_: secret_type.map(str::to_owned),
        data: Some(
            data.iter()
                .map(|(key, value)| ((*key).to_owned(), ByteString(value.to_vec())))
                .collect(),
        ),
        ..Default::default()
    }
}

fn key(name: &str, size_bytes: usize, is_binary: bool) -> SecretKey {
    SecretKey {
        name: name.to_owned(),
        size_bytes,
        is_binary,
    }
}

#[test]
fn secret_summary_drops_values() {
    let docker = format!(r#"{{"auths":{{"registry.example.test":{{"auth":"{DISTINCTIVE}"}}}}}}"#);
    let cases = [
        secret_of(None, &[("password", DISTINCTIVE.as_bytes())]),
        secret_of(
            Some(TLS_TYPE),
            &[("tls.crt", LEAF), ("tls.key", DISTINCTIVE.as_bytes())],
        ),
        secret_of(
            Some(DOCKER_CONFIG_JSON_TYPE),
            &[(".dockerconfigjson", docker.as_bytes())],
        ),
        secret_of(
            Some(SERVICE_ACCOUNT_TOKEN_TYPE),
            &[("token", DISTINCTIVE.as_bytes())],
        ),
    ];
    for secret in &cases {
        let text = format!("{:?}", secret_summary(secret));
        assert!(!text.contains(DISTINCTIVE), "{text}");
    }
}

#[test]
fn keys_sorted_with_sizes_and_binary_flag() {
    let secret = secret_of(
        None,
        &[("zeta", b"abc"), ("alpha", &[0xff, 0xfe]), ("empty", b"")],
    );
    assert_eq!(
        secret_summary(&secret).keys,
        [
            key("alpha", 2, true),
            key("empty", 0, false),
            key("zeta", 3, false)
        ]
    );
}

#[test]
fn type_defaults_to_opaque() {
    assert_eq!(secret_summary(&secret_of(None, &[])).secret_type, "Opaque");
    assert_eq!(
        secret_summary(&secret_of(Some(""), &[])).secret_type,
        "Opaque"
    );
    let basic = secret_summary(&secret_of(Some("kubernetes.io/basic-auth"), &[]));
    assert_eq!(basic.secret_type, "kubernetes.io/basic-auth");
    assert_eq!(basic.details, SecretDetails::None);
}

#[test]
fn tls_details_parse_tls_crt() {
    let secret = secret_of(Some(TLS_TYPE), &[("tls.crt", LEAF), ("tls.key", b"k")]);
    let SecretDetails::Certificate { chain } = secret_summary(&secret).details else {
        panic!("expected a certificate");
    };
    assert_eq!(chain.len(), 1);
    assert_eq!(chain[0].subject, "CN=shop.example.test,O=Example");
}

#[test]
fn tls_without_crt_is_missing() {
    for data in [&[][..], &[("tls.crt", &b""[..])], &[("tls.key", &b"k"[..])]] {
        let summary = secret_summary(&secret_of(Some(TLS_TYPE), data));
        assert_eq!(
            summary.details,
            SecretDetails::NoCertificate(CertificateIssue::Missing)
        );
    }
}

#[test]
fn tls_garbage_crt_is_unparsed() {
    let garbage = secret_of(Some(TLS_TYPE), &[("tls.crt", b"not a certificate")]);
    assert_eq!(
        secret_summary(&garbage).details,
        SecretDetails::NoCertificate(CertificateIssue::Unparsed)
    );
}

#[test]
fn tls_key_is_never_read() {
    let secret = secret_of(
        Some(TLS_TYPE),
        &[("tls.crt", LEAF), ("tls.key", DISTINCTIVE.as_bytes())],
    );
    let text = format!("{:?}", secret_summary(&secret));
    assert!(text.contains("tls.key"));
    assert!(!text.contains(DISTINCTIVE));
}

#[test]
fn docker_config_json_keeps_hosts_only() {
    let json = format!(
        r#"{{"auths":{{"b.example.test":{{"auth":"{DISTINCTIVE}"}},"a.example.test":{{"username":"u","password":"{DISTINCTIVE}"}}}}}}"#
    );
    let secret = secret_of(
        Some(DOCKER_CONFIG_JSON_TYPE),
        &[(".dockerconfigjson", json.as_bytes())],
    );
    let summary = secret_summary(&secret);
    assert_eq!(
        summary.details,
        SecretDetails::Registries(vec![
            "a.example.test".to_owned(),
            "b.example.test".to_owned()
        ])
    );
    assert!(!format!("{summary:?}").contains(DISTINCTIVE));
}

#[test]
fn docker_cfg_keeps_top_level_hosts() {
    let json = format!(r#"{{"old.example.test":{{"auth":"{DISTINCTIVE}"}}}}"#);
    let secret = secret_of(Some(DOCKER_CONFIG_TYPE), &[(".dockercfg", json.as_bytes())]);
    assert_eq!(
        secret_summary(&secret).details,
        SecretDetails::Registries(vec!["old.example.test".to_owned()])
    );
}

#[test]
fn bad_docker_config_gives_no_hosts() {
    let bad = secret_of(
        Some(DOCKER_CONFIG_JSON_TYPE),
        &[(".dockerconfigjson", b"{not json")],
    );
    assert_eq!(
        secret_summary(&bad).details,
        SecretDetails::Registries(vec![])
    );
    let missing = secret_of(Some(DOCKER_CONFIG_TYPE), &[]);
    assert_eq!(
        secret_summary(&missing).details,
        SecretDetails::Registries(vec![])
    );
}

fn token_secret(annotations: &[(&str, &str)]) -> Secret {
    let mut secret = secret_of(Some(SERVICE_ACCOUNT_TOKEN_TYPE), &[("token", b"t")]);
    secret.metadata.annotations = Some(
        annotations
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect(),
    );
    secret
}

#[test]
fn token_account_from_annotation() {
    let named = token_secret(&[(SERVICE_ACCOUNT_NAME_ANNOTATION, "builder")]);
    assert_eq!(
        secret_summary(&named).details,
        SecretDetails::ServiceAccountToken {
            account: Some("builder".to_owned())
        }
    );
    for annotations in [&[][..], &[(SERVICE_ACCOUNT_NAME_ANNOTATION, "")]] {
        assert_eq!(
            secret_summary(&token_secret(annotations)).details,
            SecretDetails::ServiceAccountToken { account: None }
        );
    }
}

#[test]
fn other_annotations_are_ignored() {
    let mut secret = secret_of(None, &[]);
    secret.metadata.annotations = Some(BTreeMap::from([(
        "kubectl.kubernetes.io/last-applied-configuration".to_owned(),
        DISTINCTIVE.to_owned(),
    )]));
    let text = format!("{:?}", secret_summary(&secret));
    assert!(!text.contains(DISTINCTIVE));
}

#[test]
fn immutable_and_owned_flags() {
    let plain = secret_summary(&secret_of(None, &[]));
    assert!(!plain.is_immutable);
    assert!(!plain.is_owned);
    let mut secret = secret_of(None, &[]);
    secret.immutable = Some(true);
    secret.metadata.owner_references = Some(vec![OwnerReference::default()]);
    let flagged = secret_summary(&secret);
    assert!(flagged.is_immutable);
    assert!(flagged.is_owned);
}

#[test]
fn secret_watch_config_pages_most_recent() {
    let config = secret_watch_config();
    assert_eq!(config.list_semantic, ListSemantic::MostRecent);
    assert_eq!(config.page_size, Some(50));
    assert_eq!(config.field_selector, None);
    let tls = tls_secret_watch_config();
    assert_eq!(tls.list_semantic, ListSemantic::MostRecent);
    assert_eq!(tls.page_size, Some(50));
    assert_eq!(
        tls.field_selector.as_deref(),
        Some("type=kubernetes.io/tls")
    );
}

#[test]
fn secret_value_text_and_binary() {
    let text = SecretValue::new("user".to_owned(), b"admin".to_vec());
    assert_eq!(text.key(), "user");
    assert_eq!(text.as_text(), Some("admin"));
    assert_eq!(text.size_bytes(), 5);
    let binary = SecretValue::new("blob".to_owned(), vec![0xff, 0xfe, 0x00]);
    assert_eq!(binary.as_text(), None);
    assert_eq!(binary.size_bytes(), 3);
}

#[test]
fn values_are_sorted_by_key() {
    let secret = secret_of(None, &[("zeta", b"3"), ("alpha", b"1"), ("middle", b"2")]);
    let values = values_of(secret);
    let keys: Vec<_> = values.iter().map(SecretValue::key).collect();
    assert_eq!(keys, ["alpha", "middle", "zeta"]);
    assert_eq!(values[0].as_text(), Some("1"));
}

#[test]
fn values_of_secret_without_data_is_empty() {
    assert!(values_of(Secret::default()).is_empty());
}

#[test]
fn undecodable_body_gives_fixed_error() {
    let body = format!(r#"{{"data":"{DISTINCTIVE}"}}"#);
    let Err(error) = decode_secret("uat", &body) else {
        panic!("a body of the wrong shape must not decode");
    };
    assert!(matches!(error, ClusterError::UnexpectedResponse { .. }));
    let source = std::error::Error::source(&error).map(ToString::to_string);
    for text in [
        error.to_string(),
        format!("{error:?}"),
        source.unwrap_or_default(),
    ] {
        assert!(!text.contains(DISTINCTIVE), "{text}");
    }
}

#[test]
fn invalid_utf8_error_is_replaced_by_fixed_text() {
    let utf8_error = String::from_utf8(DISTINCTIVE.bytes().chain([0xff]).collect()).unwrap_err();
    let original = ClusterError::UnexpectedResponse {
        context: "uat".to_owned(),
        action: READ_ACTION,
        source: Box::new(kube::Error::FromUtf8(utf8_error)),
    };
    let replaced = without_body_bytes("uat", original);
    let source = std::error::Error::source(&replaced).expect("fixed source");
    assert!(source.downcast_ref::<kube::Error>().is_none());
    assert_eq!(
        source.to_string(),
        "the secret response was not valid UTF-8"
    );
}

#[test]
fn other_unexpected_errors_get_the_generic_fixed_text() {
    let original = ClusterError::UnexpectedResponse {
        context: "uat".to_owned(),
        action: READ_ACTION,
        source: Box::new(kube::Error::TlsRequired),
    };
    let replaced = without_body_bytes("uat", original);
    let source = std::error::Error::source(&replaced).expect("fixed source");
    assert!(source.downcast_ref::<kube::Error>().is_none());
    assert_eq!(source.to_string(), "the secret response could not be read");
}

#[test]
fn other_errors_pass_through_unchanged() {
    let forbidden = ClusterError::Forbidden {
        context: "uat".to_owned(),
        action: READ_ACTION,
        message: "no".to_owned(),
    };
    assert!(matches!(
        without_body_bytes("uat", forbidden),
        ClusterError::Forbidden { .. }
    ));
}

#[test]
fn secret_request_targets_namespaced_path() {
    let request = secret_collection("shop")
        .get("credentials", &GetParams::default())
        .expect("request builds");
    assert_eq!(request.uri(), "/api/v1/namespaces/shop/secrets/credentials");
}
