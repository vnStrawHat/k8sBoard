//! Tests of the 0042 `CreateObject` write over the fake transport.

use serde_json::{Value, json};

use super::*;
use crate::fake_api::{Failure, FakeApi, RecordedRequest};

const CONFIG_MAP: &str = "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: new-config\n  namespace: payments\ndata:\n  KEY: value\n  OTHER: more\n";

fn draft_of(kind: ObjectKind, text: &str) -> ObjectDraft {
    ObjectDraft::new(kind, text).expect("a valid draft")
}

fn request_of(kind: ObjectKind, text: &str) -> WriteRequest {
    let draft = draft_of(kind, text);
    WriteRequest::new(
        draft.target().clone(),
        WriteOperation::CreateObject(Box::new(draft)),
    )
    .expect("a creatable kind fits")
}

fn config_map_request() -> WriteRequest {
    request_of(ObjectKind::ConfigMap, CONFIG_MAP)
}

fn body_of(request: &RecordedRequest) -> Value {
    serde_json::from_str(&request.body).expect("the body is JSON")
}

/// The server's answer to a create: the body it received plus a uid.
fn echoing() -> (ClusterConnection, FakeApi) {
    FakeApi::connection(WritePolicy::Allowed, |request| {
        let mut object = body_of(request);
        object["metadata"]["uid"] = json!("uid-1");
        (201, object.to_string())
    })
}

fn answering(code: u16, body: Value) -> (ClusterConnection, FakeApi) {
    let body = body.to_string();
    FakeApi::connection(WritePolicy::Allowed, move |_| (code, body.clone()))
}

fn status(code: u16, reason: &str, message: &str) -> Value {
    json!({
        "kind": "Status", "apiVersion": "v1", "status": "Failure",
        "message": message, "reason": reason, "code": code,
    })
}

#[tokio::test]
async fn dry_run_request_shape() {
    let request = config_map_request();
    let (connection, api) = echoing();
    let outcome = connection
        .write(&request, WriteMode::DryRun)
        .await
        .expect("accepted");
    assert_eq!(outcome.effect, WriteEffect::Created);
    assert_eq!(outcome.created_name, None);
    let sent = api.requests();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].method, "POST");
    assert_eq!(sent[0].path, "/api/v1/namespaces/payments/configmaps");
    assert!(sent[0].has_query("dryRun", "All"), "{}", sent[0].query);
    assert!(sent[0].has_query("fieldManager", "k8sboard"));
    assert_eq!(sent[0].content_type.as_deref(), Some("application/json"));
    assert_eq!(
        body_of(&sent[0]),
        json!({
            "apiVersion": "v1", "kind": "ConfigMap",
            "metadata": {"name": "new-config", "namespace": "payments"},
            "data": {"KEY": "value", "OTHER": "more"},
        })
    );
}

#[tokio::test]
async fn commit_request_has_no_dry_run() {
    let (connection, api) = echoing();
    let outcome = connection
        .write(&config_map_request(), WriteMode::Commit)
        .await
        .expect("accepted");
    let sent = &api.requests()[0];
    assert!(!sent.has_query_key("dryRun"), "{}", sent.query);
    assert!(sent.has_query("fieldManager", "k8sboard"));
    assert_eq!(outcome.effect, WriteEffect::Created);
    assert_eq!(outcome.created_name.as_deref(), Some("new-config"));
    assert_eq!(outcome.uid.as_deref(), Some("uid-1"));
}

#[tokio::test]
async fn namespace_create_posts_to_cluster_collection() {
    let text = "apiVersion: v1\nkind: Namespace\nmetadata:\n  name: team-a\n";
    let (connection, api) = echoing();
    connection
        .write(&request_of(ObjectKind::Namespace, text), WriteMode::Commit)
        .await
        .expect("accepted");
    let sent = &api.requests()[0];
    assert_eq!(
        (sent.method.as_str(), sent.path.as_str()),
        ("POST", "/api/v1/namespaces")
    );
}

#[tokio::test]
async fn each_kind_posts_to_its_collection() {
    let quota = "apiVersion: v1\nkind: ResourceQuota\nmetadata:\n  name: q\n  namespace: payments\nspec:\n  hard:\n    pods: \"20\"\n";
    let budget = "apiVersion: policy/v1\nkind: PodDisruptionBudget\nmetadata:\n  name: b\n  namespace: payments\nspec:\n  minAvailable: 1\n  selector:\n    matchLabels:\n      app: x\n";
    let binding = "apiVersion: rbac.authorization.k8s.io/v1\nkind: RoleBinding\nmetadata:\n  name: r\n  namespace: payments\nroleRef:\n  apiGroup: rbac.authorization.k8s.io\n  kind: ClusterRole\n  name: view\nsubjects:\n  - kind: ServiceAccount\n    name: default\n    namespace: payments\n";
    let cases = [
        (
            ObjectKind::ResourceQuota,
            quota,
            "/api/v1/namespaces/payments/resourcequotas",
        ),
        (
            ObjectKind::PodDisruptionBudget,
            budget,
            "/apis/policy/v1/namespaces/payments/poddisruptionbudgets",
        ),
        (
            ObjectKind::RoleBinding,
            binding,
            "/apis/rbac.authorization.k8s.io/v1/namespaces/payments/rolebindings",
        ),
    ];
    for (kind, text, path) in cases {
        let (connection, api) = echoing();
        connection
            .write(&request_of(kind, text), WriteMode::DryRun)
            .await
            .expect("accepted");
        assert_eq!(api.requests()[0].path, path);
    }
}

#[tokio::test]
async fn blocked_policy_sends_nothing() {
    let (connection, api) = FakeApi::connection(WritePolicy::Blocked, |_| (201, "{}".to_owned()));
    let error = connection
        .write(&config_map_request(), WriteMode::Commit)
        .await
        .expect_err("blocked");
    assert!(matches!(error, WriteError::WritesBlocked), "{error}");
    assert!(api.requests().is_empty());
}

#[test]
fn request_needs_matching_target_and_creatable_kind() {
    let draft = draft_of(ObjectKind::ConfigMap, CONFIG_MAP);
    let operation = || WriteOperation::CreateObject(Box::new(draft.clone()));
    let target = |kind, name: &str| {
        ObjectRef::new(kind, Some("payments".to_owned()), name.to_owned()).expect("namespaced")
    };
    assert!(WriteRequest::new(target(ObjectKind::ConfigMap, "new-config"), operation()).is_some());
    assert!(WriteRequest::new(target(ObjectKind::ConfigMap, "other"), operation()).is_none());
    assert!(WriteRequest::new(target(ObjectKind::Secret, "new-config"), operation()).is_none());
    assert!(WriteRequest::new(target(ObjectKind::Deployment, "new-config"), operation()).is_none());
}

#[test]
fn access_check_is_create_of_kind() {
    let request = config_map_request();
    assert_eq!(
        request.access_check(),
        AccessCheck::Create(ObjectKind::ConfigMap)
    );
    assert_eq!(request.access_check().to_string(), "create configmaps");
    assert!(request.supports_dry_run());
    assert!(
        !AccessCheck::ALL
            .iter()
            .any(|check| matches!(check, AccessCheck::Create(_)))
    );
    let text = "apiVersion: v1\nkind: Namespace\nmetadata:\n  name: team-a\n";
    let namespace = request_of(ObjectKind::Namespace, text);
    assert_eq!(
        namespace.access_check(),
        AccessCheck::Create(ObjectKind::Namespace)
    );
    assert_eq!(namespace.access_check().to_string(), "create namespaces");
}

#[tokio::test]
async fn already_exists_reads_as_invalid_name() {
    let body = status(
        409,
        "AlreadyExists",
        "configmaps \"new-config\" already exists",
    );
    let (connection, _api) = answering(409, body);
    for mode in [WriteMode::DryRun, WriteMode::Commit] {
        let error = connection
            .write(&config_map_request(), mode)
            .await
            .expect_err("a conflict");
        assert!(
            matches!(&error, WriteError::Invalid { message, fields }
                if message == "ConfigMap new-config already exists"
                    && fields == &["metadata.name"]),
            "{error:?}"
        );
    }
}

#[tokio::test]
async fn other_conflicts_keep_their_generic_mapping() {
    let (connection, _api) =
        answering(409, status(409, "Conflict", "the object has been modified"));
    let error = connection
        .write(&config_map_request(), WriteMode::Commit)
        .await
        .expect_err("a conflict");
    assert!(matches!(error, WriteError::Conflict { .. }), "{error:?}");
}

#[tokio::test]
async fn missing_namespace_reads_as_namespace_text() {
    let (connection, _api) = answering(
        404,
        status(404, "NotFound", "namespaces \"payments\" not found"),
    );
    for mode in [WriteMode::DryRun, WriteMode::Commit] {
        let error = connection
            .write(&config_map_request(), mode)
            .await
            .expect_err("a missing namespace");
        assert!(
            matches!(&error, WriteError::Invalid { message, fields }
                if message == "The namespace payments does not exist"
                    && fields == &["metadata.namespace"]),
            "{error:?}"
        );
    }
}

#[tokio::test]
async fn dry_run_reports_dropped_fields() {
    let text = "apiVersion: policy/v1\nkind: PodDisruptionBudget\nmetadata:\n  name: b\n  namespace: payments\nspec:\n  minAvailble: 1\n  selector:\n    matchLabels:\n      app: x\n";
    let request = request_of(ObjectKind::PodDisruptionBudget, text);
    // The server drops the misspelled field and adds a uid.
    let answer = json!({
        "apiVersion": "policy/v1", "kind": "PodDisruptionBudget",
        "metadata": {"name": "b", "namespace": "payments", "uid": "u"},
        "spec": {"selector": {"matchLabels": {"app": "x"}}},
    });
    let (connection, _api) = answering(201, answer);
    let outcome = connection
        .write(&request, WriteMode::DryRun)
        .await
        .expect("accepted");
    assert_eq!(outcome.dropped_fields, ["spec.minAvailble"]);
    // Nothing dropped, nothing reported; another operation never reports any.
    let (connection, _api) = echoing();
    let outcome = connection
        .write(&config_map_request(), WriteMode::DryRun)
        .await
        .expect("accepted");
    assert!(outcome.dropped_fields.is_empty());
}

#[tokio::test]
async fn changed_fields_never_hold_config_map_values() {
    let fields = config_map_request().changed_fields();
    let paths: Vec<_> = fields.iter().map(|field| field.path.as_ref()).collect();
    assert_eq!(
        paths,
        [
            "metadata.name",
            "metadata.namespace",
            "data[KEY]",
            "data[OTHER]"
        ]
    );
    assert_eq!(fields[0].value.as_deref(), Some("new-config"));
    assert!(fields[2..].iter().all(|field| field.value.is_none()));
    let text = format!("{fields:?}");
    assert!(!text.contains("more"), "{text}");
    let binary = format!("{CONFIG_MAP}binaryData:\n  BIN: aGk=\n");
    let paths: Vec<_> = request_of(ObjectKind::ConfigMap, &binary)
        .changed_fields()
        .iter()
        .map(|field| field.path.to_string())
        .collect();
    assert!(paths.contains(&"binaryData[BIN]".to_owned()), "{paths:?}");

    let binding = "apiVersion: rbac.authorization.k8s.io/v1\nkind: RoleBinding\nmetadata:\n  name: r\n  namespace: payments\nroleRef:\n  apiGroup: rbac.authorization.k8s.io\n  kind: ClusterRole\n  name: view\nsubjects:\n  - kind: ServiceAccount\n    name: default\n    namespace: payments\n  - kind: Group\n    name: devs\n";
    let fields: Vec<_> = request_of(ObjectKind::RoleBinding, binding)
        .changed_fields()
        .into_iter()
        .map(|field| (field.path.to_string(), field.value))
        .collect();
    assert_eq!(
        fields[2..],
        [
            ("roleRef".to_owned(), Some("ClusterRole/view".to_owned())),
            (
                "subjects[0]".to_owned(),
                Some("ServiceAccount payments/default".to_owned())
            ),
            ("subjects[1]".to_owned(), Some("Group devs".to_owned())),
        ]
    );
}

#[test]
fn quota_and_budget_fields_carry_their_values() {
    let quota = "apiVersion: v1\nkind: ResourceQuota\nmetadata:\n  name: q\n  namespace: payments\nspec:\n  hard:\n    pods: \"20\"\n    requests.cpu: \"4\"\n";
    let fields = request_of(ObjectKind::ResourceQuota, quota).changed_fields();
    let pairs: Vec<_> = fields[2..]
        .iter()
        .map(|field| (field.path.as_ref(), field.value.as_deref()))
        .collect();
    assert_eq!(
        pairs,
        [
            ("spec.hard[pods]", Some("20")),
            ("spec.hard[requests.cpu]", Some("4"))
        ]
    );
    let budget = "apiVersion: policy/v1\nkind: PodDisruptionBudget\nmetadata:\n  name: b\n  namespace: payments\nspec:\n  maxUnavailable: 25%\n  selector:\n    matchLabels:\n      app: x\n";
    let fields = request_of(ObjectKind::PodDisruptionBudget, budget).changed_fields();
    assert_eq!(fields[2].path, "spec.maxUnavailable");
    assert_eq!(fields[2].value.as_deref(), Some("25%"));
}

#[test]
fn changed_fields_cap_long_lists() {
    let keys: String = (0..25).map(|index| format!("  K{index:02}: v\n")).collect();
    let text = format!(
        "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: big\n  namespace: payments\ndata:\n{keys}"
    );
    let fields = request_of(ObjectKind::ConfigMap, &text).changed_fields();
    // The name and namespace, ten keys, then the remainder.
    assert_eq!(fields.len(), 2 + 10 + 1);
    assert_eq!(fields[12].path, "\u{2026} and 15 more");
    assert!(fields[12].value.is_none());

    let subjects: String = (0..12)
        .map(|index| format!("  - kind: User\n    name: user-{index}\n"))
        .collect();
    let text = format!(
        "apiVersion: rbac.authorization.k8s.io/v1\nkind: RoleBinding\nmetadata:\n  name: r\n  namespace: payments\nroleRef:\n  apiGroup: rbac.authorization.k8s.io\n  kind: ClusterRole\n  name: view\nsubjects:\n{subjects}"
    );
    let fields = request_of(ObjectKind::RoleBinding, &text).changed_fields();
    // The name, namespace, roleRef, ten subjects, then the remainder.
    assert_eq!(fields.len(), 3 + 10 + 1);
    assert_eq!(fields[13].path, "\u{2026} and 2 more");
}

#[tokio::test]
async fn commit_timeout_is_outcome_unknown() {
    let (connection, _api) = FakeApi::failing(WritePolicy::Allowed, Failure::Error);
    let error = connection
        .write(&config_map_request(), WriteMode::Commit)
        .await
        .expect_err("a transport failure");
    assert!(matches!(error, WriteError::OutcomeUnknown), "{error:?}");
}

#[test]
fn request_debug_holds_no_value() {
    let text = format!("{:?}", config_map_request());
    assert!(
        text.contains("CreateObject") && text.contains("new-config"),
        "{text}"
    );
    assert!(!text.contains("value") && !text.contains("more"), "{text}");
}

#[tokio::test]
async fn allow_list_matches_the_operations() {
    // The 0042 row: method, path, query, content type, and body, in both modes.
    let request = config_map_request();
    for (mode, has_dry_run) in [(WriteMode::DryRun, true), (WriteMode::Commit, false)] {
        let (connection, api) = echoing();
        connection.write(&request, mode).await.expect("accepted");
        let sent = &api.requests()[0];
        assert_eq!(sent.method, "POST");
        assert_eq!(sent.path, "/api/v1/namespaces/payments/configmaps");
        assert_eq!(sent.has_query_key("dryRun"), has_dry_run);
        assert!(sent.has_query("fieldManager", "k8sboard"));
        assert_eq!(sent.content_type.as_deref(), Some("application/json"));
        let body = body_of(sent);
        assert!(body.get("status").is_none());
        assert!(body["metadata"].get("uid").is_none());
    }
}

#[tokio::test]
async fn a_missing_namespace_api_reads_as_such_for_a_namespace_create() {
    let text = "apiVersion: v1\nkind: Namespace\nmetadata:\n  name: team-a\n";
    let (connection, _api) = answering(
        404,
        status(
            404,
            "NotFound",
            "the server could not find the requested resource",
        ),
    );
    let error = connection
        .write(&request_of(ObjectKind::Namespace, text), WriteMode::Commit)
        .await
        .expect_err("a missing API");
    assert!(
        matches!(&error, WriteError::Invalid { message, .. }
            if message == "The Namespace API is not available on this cluster"),
        "{error:?}"
    );
}

#[test]
fn finalizers_are_listed_and_capped() {
    let finalizers: String = (0..12)
        .map(|index| format!("    - example.com/f{index}\n"))
        .collect();
    let text = format!(
        "apiVersion: v1\nkind: Namespace\nmetadata:\n  name: team-a\n  finalizers:\n{finalizers}"
    );
    let fields = request_of(ObjectKind::Namespace, &text).changed_fields();
    let paths: Vec<_> = fields.iter().map(|field| field.path.to_string()).collect();
    assert_eq!(paths[1], "metadata.finalizers[0]");
    assert_eq!(fields[1].value.as_deref(), Some("example.com/f0"));
    assert_eq!(paths.len(), 1 + 10 + 1);
    assert_eq!(paths[11], "\u{2026} and 2 more");
}

const SECRET: &str = "apiVersion: v1\nkind: Secret\nmetadata:\n  name: regcred\n  namespace: payments\ntype: kubernetes.io/dockerconfigjson\nstringData:\n  .dockerconfigjson: payload\n";

#[tokio::test]
async fn a_secret_is_posted_with_base64_data_and_no_string_data() {
    let request = request_of(ObjectKind::Secret, SECRET);
    let (connection, api) = echoing();
    connection
        .write(&request, WriteMode::Commit)
        .await
        .expect("accepted");
    let sent = api.requests();
    assert_eq!(sent[0].method, "POST");
    assert_eq!(sent[0].path, "/api/v1/namespaces/payments/secrets");
    assert!(!sent[0].has_query_key("dryRun"));
    assert_eq!(
        body_of(&sent[0]),
        json!({
            "apiVersion": "v1", "kind": "Secret",
            "metadata": {"name": "regcred", "namespace": "payments"},
            "type": "kubernetes.io/dockerconfigjson",
            "data": {".dockerconfigjson": "cGF5bG9hZA=="},
        })
    );
}

#[test]
fn a_secret_create_records_names_and_never_a_value() {
    let request = request_of(ObjectKind::Secret, SECRET);
    let text = format!("{request:?} {:?}", request.changed_fields());
    assert!(!text.contains("payload"));
    assert!(!text.contains("cGF5bG9hZA"));
    assert!(
        request
            .changed_fields()
            .iter()
            .any(|field| field.path == "data[.dockerconfigjson]" && field.value.is_none())
    );
}
