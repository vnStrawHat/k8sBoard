use std::path::PathBuf;

use cluster::WritePolicy;
use cluster::fake_api::FakeApi;
use serde_json::json;

use super::*;
use crate::cluster_registry::ClusterRef;

const RSA_CERT: &str = include_str!("../../cluster/tests/fixtures/tls/pair-rsa.crt");
const RSA_KEY: &str = include_str!("../../cluster/tests/fixtures/tls/pair-rsa-pkcs8.key");
const OTHER_KEY: &str = include_str!("../../cluster/tests/fixtures/tls/other-rsa-pkcs8.key");
const EC_CERT: &str = include_str!("../../cluster/tests/fixtures/tls/pair-ec.crt");
/// A well-formed PKCS#8 Ed25519 key: its public part is not in the file, so it cannot be compared.
const ED_KEY: &str = "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIH8vUU1hAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA\n-----END PRIVATE KEY-----\n";

fn cluster() -> ClusterRef {
    ClusterRef {
        kubeconfig: PathBuf::from("/home/me/.kube/config"),
        context: "prod-ctx".to_owned(),
    }
}

fn scope(cluster: &ClusterRef) -> NodeScope<'_> {
    NodeScope {
        cluster,
        cluster_name: "prod-a",
    }
}

fn fields<'a>(name: &'a str, password: &'a str) -> RegistryFields<'a> {
    RegistryFields {
        name,
        namespace: "shop",
        server: "https://registry.example.test",
        username: "me",
        password,
        email: "",
    }
}

fn registry(fields: &RegistryFields<'_>) -> Result<WriteIntent, SharedString> {
    let cluster = cluster();
    registry_intent(&scope(&cluster), fields)
}

fn audited_paths(intent: &WriteIntent) -> Vec<String> {
    intent
        .request
        .changed_fields()
        .iter()
        .map(|field| field.path.to_string())
        .collect()
}

#[test]
fn a_registry_secret_is_a_create_of_a_dockerconfigjson_secret() {
    let intent = registry(&fields("regcred", "p@ss")).expect("an intent");
    assert_eq!(
        intent.action,
        ResourceAction::CreateObject(ObjectKind::Secret)
    );
    assert_eq!(intent.label, "Create Secret shop/regcred");
    assert_eq!(intent.button, "Create");
    assert_eq!(intent.request.target().name(), "regcred");
    assert_eq!(intent.request.target().namespace(), Some("shop"));
    assert_eq!(
        audited_paths(&intent),
        [
            "metadata.name",
            "metadata.namespace",
            "type",
            "data[.dockerconfigjson]"
        ]
    );
}

#[test]
fn a_registry_secret_never_shows_the_password_or_the_login() {
    let intent = registry(&fields("regcred", "hunter2-pw")).expect("an intent");
    let shown = format!(
        "{:?} {:?} {:?}",
        intent.request.changed_fields(),
        intent.change_lines,
        intent.request
    );
    assert!(!shown.contains("hunter2-pw"));
    // The base64 of `me:hunter2-pw`, which `auth` holds.
    assert!(!shown.contains("bWU6aHVudGVyMi1wdw"));
    assert!(
        intent
            .change_lines
            .iter()
            .any(|line| line.contains("user me") && line.contains("registry.example.test"))
    );
}

#[test]
fn a_registry_secret_needs_every_field_but_the_email() {
    for (name, namespace, server, username, password, problem) in [
        ("", "shop", "s", "me", "pw", "Enter a name"),
        ("n", "", "s", "me", "pw", "Enter a namespace"),
        ("n", "shop", "", "me", "pw", "Enter the registry server"),
        ("n", "shop", "s", "", "pw", "Enter the user name"),
        ("n", "shop", "s", "me", "", "Enter the password"),
    ] {
        let given = RegistryFields {
            name,
            namespace,
            server,
            username,
            password,
            email: "",
        };
        assert_eq!(registry(&given).err().as_deref(), Some(problem));
    }
}

#[test]
fn a_bad_name_is_refused_by_the_draft_without_quoting_it() {
    let text = registry(&fields("Bad Name", "pw"))
        .err()
        .expect("a refusal");
    assert!(text.contains("not a valid Secret name"), "{text}");
    assert!(!text.contains("Bad Name"));
}

fn tls(certificate: &str, key: &str) -> Result<WriteIntent, SharedString> {
    let cluster = cluster();
    tls_create_intent(&scope(&cluster), "api-tls", "shop", certificate, key)
}

#[test]
fn a_tls_secret_needs_a_pair_that_matches() {
    let intent = tls(RSA_CERT, RSA_KEY).expect("an intent");
    assert_eq!(intent.request.target().name(), "api-tls");
    assert!(intent.warnings.is_empty());
    assert!(
        intent
            .change_lines
            .iter()
            .any(|line| line.contains("CN=api.example.test"))
    );
    assert_eq!(
        tls(RSA_CERT, OTHER_KEY).err().as_deref(),
        Some("The key does not match the certificate")
    );
}

#[test]
fn a_tls_secret_never_shows_the_key() {
    let intent = tls(RSA_CERT, RSA_KEY).expect("an intent");
    let shown = format!(
        "{:?} {:?} {:?}",
        intent.request.changed_fields(),
        intent.change_lines,
        intent.request
    );
    assert!(!shown.contains("BEGIN"));
    assert_eq!(
        audited_paths(&intent),
        [
            "metadata.name",
            "metadata.namespace",
            "type",
            "data[tls.crt]",
            "data[tls.key]"
        ]
    );
}

#[test]
fn a_tls_pair_without_blocks_is_a_problem_line() {
    assert_eq!(
        tls("", "").err().as_deref(),
        Some("Paste the certificate and the key")
    );
    assert_eq!(
        tls("nope", RSA_KEY).err().as_deref(),
        Some("The certificate has no CERTIFICATE block that parses")
    );
    assert_eq!(
        tls(RSA_CERT, "nope").err().as_deref(),
        Some("The key has no PRIVATE KEY block")
    );
}

#[test]
fn a_key_that_cannot_be_compared_is_sent_with_a_warning() {
    let intent = tls(EC_CERT, ED_KEY).expect("an intent");
    assert_eq!(intent.warnings.len(), 1);
    assert!(intent.warnings[0].contains("could not be compared"));
}

#[test]
fn the_report_follows_the_texts() {
    assert_eq!(tls_report("", " "), TlsReport::Empty);
    assert!(matches!(
        tls_report(RSA_CERT, RSA_KEY),
        TlsReport::Ready(info) if info.key == KeyCheck::Matches
    ));
    let TlsReport::Ready(info) = tls_report(RSA_CERT, OTHER_KEY) else {
        panic!("a readable pair");
    };
    let now: jiff::Timestamp = "2026-10-07T00:00:00Z".parse().expect("a timestamp");
    let lines = tls_report_lines(&info, now);
    assert_eq!(lines[0], "Subject: O=Example,CN=api.example.test");
    assert!(lines[1].starts_with("Not after: 2126-09-13 03:21 UTC ("));
    assert_eq!(lines[2], "The key does not match the certificate");
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .expect("a tokio runtime")
}

/// The base of a TLS Secret whose certificate is older than the new one.
fn tls_base(rt: &tokio::runtime::Runtime) -> ValuesBase {
    let secret = json!({
        "apiVersion": "v1", "kind": "Secret",
        "metadata": {"name": "api-tls", "namespace": "shop", "resourceVersion": "9"},
        "type": "kubernetes.io/tls",
        "data": {"tls.crt": "b2xk", "tls.key": "b2xk"},
    })
    .to_string();
    let connection = {
        let _guard = rt.enter();
        FakeApi::connection(WritePolicy::Blocked, move |_| (200, secret.clone())).0
    };
    let target = cluster::ObjectRef::new(
        ObjectKind::Secret,
        Some("shop".to_owned()),
        "api-tls".to_owned(),
    )
    .expect("a Secret");
    rt.block_on(connection.values_base(&target))
        .expect("an editable base")
}

#[test]
fn a_replace_sets_both_keys_and_says_old_to_new() {
    let rt = runtime();
    let base = tls_base(&rt);
    let cluster = cluster();
    let old: jiff::Timestamp = "2026-10-11T15:13:00Z".parse().expect("a timestamp");
    let intent = tls_replace_intent(&scope(&cluster), &base, RSA_CERT, RSA_KEY, Some(old))
        .expect("an intent");
    assert_eq!(intent.action, ResourceAction::ReplaceCertificate);
    assert_eq!(intent.label, "Replace certificate of secret api-tls");
    assert_eq!(intent.button, "Replace certificate");
    let WriteOperation::SetDataValues(edit) = intent.request.operation() else {
        panic!("a values edit");
    };
    let keys: Vec<&str> = edit
        .changes()
        .iter()
        .map(|change| change.change.key())
        .collect();
    assert_eq!(keys, ["tls.crt", "tls.key"]);
    assert_eq!(
        intent.change_lines[1],
        "not after: 2026-10-11 15:13 UTC → 2126-09-13 03:21 UTC"
    );
    assert!(intent.change_lines[2].contains("matches the certificate"));
    // The audit line names the keys, never their text.
    let audited = format!("{:?}", intent.request.changed_fields());
    assert!(!audited.contains("BEGIN"));
}

#[test]
fn a_replace_refuses_a_key_that_differs() {
    let rt = runtime();
    let base = tls_base(&rt);
    let cluster = cluster();
    let refused = tls_replace_intent(&scope(&cluster), &base, RSA_CERT, OTHER_KEY, None);
    assert_eq!(
        refused.err().as_deref(),
        Some("The key does not match the certificate")
    );
}
