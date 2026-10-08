//! Tests of the 0031 `ReplaceObject` operation over the fake transport: request shape, restored
//! placeholders, preview, and errors.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use super::*;
use crate::edit_preview::EditCheck;
use crate::fake_api::{FakeApi, RecordedRequest};
use crate::object_edit::{EditBase, EditError};
use crate::object_yaml::EnvValues;

const DEPLOYMENT_PATH: &str = "/apis/apps/v1/namespaces/payments/deployments/api";
const ENV_LITERAL: &str = "s3cr3t-env";
const APPLIED: &str = "applied-distinctive";
const SECRET_DATA: &str = "c2VjcmV0";

fn target(kind: ObjectKind, name: &str) -> ObjectRef {
    let namespace = kind.is_namespaced().then(|| "payments".to_owned());
    ObjectRef::new(kind, namespace, name.to_owned()).expect("the namespace fits the kind")
}

fn deployment() -> Value {
    json!({
        "apiVersion": "apps/v1", "kind": "Deployment",
        "metadata": {
            "name": "api", "namespace": "payments", "uid": "uid-1", "resourceVersion": "100",
            "generation": 4, "creationTimestamp": "2026-10-01T08:00:00Z",
            "managedFields": [{"manager": "kubectl"}],
            "annotations": {"kubectl.kubernetes.io/last-applied-configuration": APPLIED},
        },
        "spec": {
            "replicas": 3,
            "template": {
                "metadata": {"annotations": {"kapp.k14s.io/original": APPLIED}},
                "spec": {"containers": [{"name": "api", "image": "api:1", "env": [
                    {"name": "DB_PASS", "value": ENV_LITERAL},
                    {"name": "MODE", "value": "fast"},
                ]}]},
            },
        },
        "status": {"readyReplicas": 3},
    })
}

fn secret() -> Value {
    json!({
        "apiVersion": "v1", "kind": "Secret",
        "metadata": {"name": "db", "namespace": "payments", "uid": "uid-2",
            "resourceVersion": "7", "labels": {"app": "db"}},
        "type": "Opaque",
        "data": {"password": SECRET_DATA},
    })
}

/// A fake API server with one object: GET answers the current object, PUT echoes the body under
/// a bumped `resourceVersion`.
struct Server {
    connection: ClusterConnection,
    api: FakeApi,
    current: Arc<Mutex<Value>>,
    fail_reads: Arc<AtomicBool>,
}

fn server_with(
    policy: WritePolicy,
    object: Value,
    put_status: u16,
    put_body: Option<String>,
) -> Server {
    let current = Arc::new(Mutex::new(object));
    let fail_reads = Arc::new(AtomicBool::new(false));
    let (state, failing) = (Arc::clone(&current), Arc::clone(&fail_reads));
    let (connection, api) = FakeApi::connection(policy, move |request| {
        if request.method == "GET" {
            if failing.load(Ordering::SeqCst) {
                return (
                    500,
                    status_json(500, "etcd is unavailable", &[], "InternalError"),
                );
            }
            let current = state.lock().expect("the state is not poisoned");
            return (200, current.to_string());
        }
        if let Some(body) = &put_body {
            return (put_status, body.clone());
        }
        let mut echoed: Value = serde_json::from_str(&request.body).expect("a JSON body");
        echoed["metadata"]["resourceVersion"] = json!("101");
        (put_status, echoed.to_string())
    });
    Server {
        connection,
        api,
        current,
        fail_reads,
    }
}

fn server(object: Value) -> Server {
    server_with(WritePolicy::Allowed, object, 200, None)
}

fn status_json(code: u16, message: &str, fields: &[&str], reason: &str) -> String {
    let causes: Vec<Value> = fields
        .iter()
        .map(|field| json!({ "reason": "FieldValueInvalid", "message": message, "field": field }))
        .collect();
    json!({
        "kind": "Status", "apiVersion": "v1", "status": "Failure", "message": message,
        "reason": reason, "details": { "causes": causes }, "code": code,
    })
    .to_string()
}

async fn base_of(server: &Server, target: &ObjectRef) -> EditBase {
    server
        .connection
        .edit_base(target, EnvValues::Hidden)
        .await
        .expect("the base is read")
}

fn edited(base: &EditBase, from: &str, to: &str) -> ObjectEdit {
    assert!(base.text().contains(from), "{from:?} not in the text");
    ObjectEdit::new(base, &base.text().replacen(from, to, 1)).expect("an applicable edit")
}

fn replace(target: ObjectRef, edit: ObjectEdit) -> WriteRequest {
    WriteRequest::new(target, WriteOperation::ReplaceObject(Box::new(edit)))
        .expect("an editable kind")
}

/// A server and the request that scales its Deployment to 5. The base GET is already recorded.
async fn scale_request() -> (Server, WriteRequest) {
    let server = server(deployment());
    let target = target(ObjectKind::Deployment, "api");
    let base = base_of(&server, &target).await;
    let request = replace(target, edited(&base, "replicas: 3", "replicas: 5"));
    (server, request)
}

/// The requests after the base read.
fn sent(server: &Server) -> Vec<RecordedRequest> {
    server.api.requests().into_iter().skip(1).collect()
}

fn put_body(server: &Server) -> Value {
    let requests = sent(server);
    let put = requests.last().expect("a request");
    assert_eq!(put.method, "PUT");
    serde_json::from_str(&put.body).expect("a JSON body")
}

#[tokio::test]
async fn replace_dry_run_shape() {
    let (server, request) = scale_request().await;
    server
        .connection
        .write(&request, WriteMode::DryRun)
        .await
        .expect("accepted");
    let requests = sent(&server);
    let methods: Vec<&str> = requests
        .iter()
        .map(|request| request.method.as_str())
        .collect();
    assert_eq!(methods, ["GET", "PUT"]);
    assert_eq!(requests[0].path, DEPLOYMENT_PATH);
    assert_eq!(requests[1].path, DEPLOYMENT_PATH);
    assert_eq!(
        query_pairs(&requests[1]),
        ["dryRun=All", "fieldManager=k8sboard"]
    );
    assert_eq!(
        requests[1].content_type.as_deref(),
        Some("application/json")
    );
}

#[tokio::test]
async fn replace_commit_has_no_dry_run() {
    let (server, request) = scale_request().await;
    server
        .connection
        .write(&request, WriteMode::Commit)
        .await
        .expect("accepted");
    let requests = sent(&server);
    assert_eq!(query_pairs(&requests[1]), ["fieldManager=k8sboard"]);
    assert!(!requests[1].has_query_key("dryRun"));
}

#[tokio::test]
async fn body_carries_base_resource_version_and_uid() {
    let server = server(deployment());
    let target = target(ObjectKind::Deployment, "api");
    let base = base_of(&server, &target).await;
    let text = base
        .text()
        .replacen("replicas: 3", "replicas: 5", 1)
        .replacen(
            "  name: api\n",
            "  name: api\n  resourceVersion: \"999\"\n  uid: typed\n",
            1,
        );
    let request = replace(
        target,
        ObjectEdit::new(&base, &text).expect("an applicable edit"),
    );
    server
        .connection
        .write(&request, WriteMode::Commit)
        .await
        .expect("accepted");
    let body = put_body(&server);
    assert_eq!(body["metadata"]["resourceVersion"], "100");
    assert_eq!(body["metadata"]["uid"], "uid-1");
    assert_eq!(body["spec"]["replicas"], 5);
}

#[tokio::test]
async fn body_has_no_status_or_server_metadata() {
    let (server, request) = scale_request().await;
    server
        .connection
        .write(&request, WriteMode::Commit)
        .await
        .expect("accepted");
    let body = put_body(&server);
    assert!(body.get("status").is_none());
    for field in [
        "managedFields",
        "generation",
        "creationTimestamp",
        "selfLink",
    ] {
        assert!(body["metadata"].get(field).is_none(), "{field}");
    }
    assert_eq!(body["apiVersion"], "apps/v1");
    assert_eq!(body["kind"], "Deployment");
}

#[tokio::test]
async fn request_body_never_holds_placeholders() {
    let fixtures = [
        (
            ObjectKind::Deployment,
            "api",
            deployment(),
            "replicas: 3",
            "replicas: 5",
        ),
        (ObjectKind::Secret, "db", secret(), "app: db", "app: db2"),
    ];
    for (kind, name, object, from, to) in fixtures {
        let server = server(object);
        let target = target(kind, name);
        let base = base_of(&server, &target).await;
        let request = replace(target, edited(&base, from, to));
        for mode in [WriteMode::DryRun, WriteMode::Commit] {
            server
                .connection
                .write(&request, mode)
                .await
                .expect("accepted");
        }
        for put in sent(&server)
            .iter()
            .filter(|request| request.method == "PUT")
        {
            for forbidden in ["<hidden", "# Values shown", "# Secret data"] {
                assert!(!put.body.contains(forbidden), "{name}: {forbidden}");
            }
        }
    }

    // A typed diff marker never becomes a request: the edit is refused before one can be built.
    let server = server(deployment());
    let base = base_of(&server, &target(ObjectKind::Deployment, "api")).await;
    for marker in ["<hidden, changed>", "<hidden, moved>"] {
        let text = base
            .text()
            .replacen("value: <hidden>", &format!("value: {marker}"), 1);
        let error = ObjectEdit::new(&base, &text).expect_err("a marker");
        assert!(
            matches!(error, EditError::MarkerText { .. }),
            "{marker}: {error:?}"
        );
    }
    assert!(sent(&server).is_empty());
}

#[tokio::test]
async fn placeholders_restore_server_values() {
    let (server, request) = scale_request().await;
    server
        .connection
        .write(&request, WriteMode::Commit)
        .await
        .expect("accepted");
    let body = put_body(&server);
    let env = &body["spec"]["template"]["spec"]["containers"][0]["env"];
    assert_eq!(env[0]["value"], ENV_LITERAL);
    assert_eq!(
        body["metadata"]["annotations"]["kubectl.kubernetes.io/last-applied-configuration"],
        APPLIED
    );
    assert_eq!(
        body["spec"]["template"]["metadata"]["annotations"]["kapp.k14s.io/original"],
        APPLIED
    );
}

#[tokio::test]
async fn secret_put_keeps_its_data() {
    let server = server(secret());
    let target = target(ObjectKind::Secret, "db");
    let base = base_of(&server, &target).await;
    let request = replace(target, edited(&base, "app: db", "app: db2"));
    server
        .connection
        .write(&request, WriteMode::Commit)
        .await
        .expect("accepted");
    assert_eq!(put_body(&server)["data"], json!({"password": SECRET_DATA}));
    assert_eq!(put_body(&server)["metadata"]["labels"]["app"], "db2");
}

#[tokio::test]
async fn index_matched_placeholder_is_marked_moved() {
    let mut object = deployment();
    object["spec"]["template"]["spec"]["containers"][0]["env"] = json!([
        {"name": "X", "value": "raw-1", "tag": "one"},
        {"name": "X", "value": "raw-2", "tag": "two"},
    ]);
    let server = server(object);
    let target = target(ObjectKind::Deployment, "api");
    let base = base_of(&server, &target).await;
    let mut text: Value = serde_saphyr::from_str(base.text()).expect("parses");
    text["spec"]["template"]["spec"]["containers"][0]["env"]
        .as_array_mut()
        .expect("a list")
        .reverse();
    let edit = ObjectEdit::new(&base, &serde_saphyr::to_string(&text).expect("serializes"))
        .expect("an applicable edit");
    let outcome = server
        .connection
        .write(&replace(target, edit), WriteMode::DryRun)
        .await
        .expect("accepted");
    let WriteEffect::Replaced(preview) = outcome.effect else {
        panic!("{outcome:?}");
    };
    assert!(
        preview
            .changes
            .iter()
            .any(|change| change.new.as_deref() == Some("<hidden, moved>")),
        "{:?}",
        preview.changes
    );
    assert!(
        preview
            .checks
            .iter()
            .any(|check| matches!(check, EditCheck::Moved { .. }))
    );
}

#[tokio::test]
async fn the_outcome_carries_a_masked_preview() {
    let (server, request) = scale_request().await;
    let outcome = server
        .connection
        .write(&request, WriteMode::DryRun)
        .await
        .expect("accepted");
    assert_eq!(outcome.created_name, None);
    let WriteEffect::Replaced(preview) = outcome.effect else {
        panic!("{outcome:?}");
    };
    assert_eq!(preview.changes.len(), 1);
    assert_eq!(preview.changes[0].path.to_string(), "spec.replicas");
    for side in preview
        .changes
        .iter()
        .flat_map(|change| [&change.old, &change.new])
    {
        let text = side.as_deref().unwrap_or_default();
        assert!(!text.contains(ENV_LITERAL), "{text}");
        assert!(!text.contains(APPLIED), "{text}");
    }
}

#[tokio::test]
async fn stale_base_is_a_conflict_without_put() {
    let (server, request) = scale_request().await;
    server.current.lock().expect("the state is not poisoned")["metadata"]["resourceVersion"] =
        json!("101");
    for mode in [WriteMode::DryRun, WriteMode::Commit] {
        let error = server
            .connection
            .write(&request, mode)
            .await
            .expect_err("stale");
        assert!(
            matches!(&error, WriteError::Conflict { message, managers }
                if message.contains("resourceVersion 100") && message.contains("101") && managers.is_empty()),
            "{error:?}"
        );
    }
    assert!(sent(&server).iter().all(|request| request.method == "GET"));
}

#[tokio::test]
async fn server_409_is_a_conflict() {
    let body = status_json(409, "the object has been modified", &[], "Conflict");
    let server = server_with(WritePolicy::Allowed, deployment(), 409, Some(body));
    let target = target(ObjectKind::Deployment, "api");
    let base = base_of(&server, &target).await;
    let request = replace(target, edited(&base, "replicas: 3", "replicas: 5"));
    let error = server
        .connection
        .write(&request, WriteMode::Commit)
        .await
        .expect_err("409");
    assert!(
        matches!(&error, WriteError::Conflict { message, managers }
            if message == "the object has been modified" && managers.is_empty()),
        "{error:?}"
    );
}

#[tokio::test]
async fn server_422_lists_fields_verbatim() {
    let body = status_json(
        422,
        "Deployment is invalid",
        &["spec.replicas", "spec.template.spec.containers[0].image"],
        "Invalid",
    );
    let server = server_with(WritePolicy::Allowed, deployment(), 422, Some(body));
    let target = target(ObjectKind::Deployment, "api");
    let base = base_of(&server, &target).await;
    let request = replace(target, edited(&base, "replicas: 3", "replicas: -1"));
    let error = server
        .connection
        .write(&request, WriteMode::DryRun)
        .await
        .expect_err("422");
    assert!(
        matches!(&error, WriteError::Invalid { fields, .. }
            if fields == &["spec.replicas".to_owned(), "spec.template.spec.containers[0].image".to_owned()]),
        "{error:?}"
    );
}

#[tokio::test]
async fn secret_errors_are_redacted() {
    let message = format!("Secret \"db\" is invalid: data[password]: \"{SECRET_DATA}\"");
    let body = status_json(422, &message, &["metadata.labels"], "Invalid");
    let server = server_with(WritePolicy::Allowed, secret(), 422, Some(body));
    let target = target(ObjectKind::Secret, "db");
    let base = base_of(&server, &target).await;
    let request = replace(target, edited(&base, "app: db", "app: db2"));
    let error = server
        .connection
        .write(&request, WriteMode::DryRun)
        .await
        .expect_err("422");
    let shown = format!("{error} {error:?}");
    assert!(!shown.contains(SECRET_DATA), "{shown}");
    assert!(shown.contains("metadata.labels"), "{shown}");
}

#[tokio::test]
async fn debug_policy_blocks_before_the_get() {
    let server = server_with(WritePolicy::Blocked, deployment(), 200, None);
    let target = target(ObjectKind::Deployment, "api");
    let base = base_of(&server, &target).await;
    let request = replace(target, edited(&base, "replicas: 3", "replicas: 5"));
    for mode in [WriteMode::DryRun, WriteMode::Commit] {
        let error = server
            .connection
            .write(&request, mode)
            .await
            .expect_err("blocked");
        assert!(matches!(error, WriteError::WritesBlocked), "{error:?}");
    }
    assert!(sent(&server).is_empty());
}

#[tokio::test]
async fn failed_fresh_read_is_never_outcome_unknown() {
    let (server, request) = scale_request().await;
    server.fail_reads.store(true, Ordering::SeqCst);
    let error = server
        .connection
        .write(&request, WriteMode::Commit)
        .await
        .expect_err("500");
    assert!(matches!(error, WriteError::Cluster(_)), "{error:?}");
    assert!(sent(&server).iter().all(|request| request.method == "GET"));
}

#[tokio::test]
async fn request_rejects_a_foreign_edit() {
    let server = server(deployment());
    let base = base_of(&server, &target(ObjectKind::Deployment, "api")).await;
    let edit = edited(&base, "replicas: 3", "replicas: 5");
    let other = target(ObjectKind::Deployment, "other");
    assert!(
        WriteRequest::new(other, WriteOperation::ReplaceObject(Box::new(edit.clone()))).is_none()
    );
    let node = ObjectRef::new(ObjectKind::Node, None, "wk-04".to_owned()).expect("a node");
    assert!(WriteRequest::new(node, WriteOperation::ReplaceObject(Box::new(edit))).is_none());
}

#[tokio::test]
async fn only_editable_kinds_can_be_replaced() {
    let server = server(json!({
        "apiVersion": "v1", "kind": "Namespace",
        "metadata": {"name": "team", "uid": "u", "resourceVersion": "1", "labels": {"a": "x"}},
    }));
    let team = target(ObjectKind::Namespace, "team");
    let base = base_of(&server, &team).await;
    let edit = edited(&base, "a: x", "a: y");
    assert!(WriteRequest::new(team, WriteOperation::ReplaceObject(Box::new(edit))).is_none());
}

#[tokio::test]
async fn replace_access_check_is_update() {
    let (_server, request) = scale_request().await;
    assert_eq!(
        request.access_check(),
        AccessCheck::Update(ObjectKind::Deployment)
    );
    assert_eq!(request.access_check().to_string(), "update deployments");
    assert!(request.supports_dry_run());
}

#[tokio::test]
async fn changed_fields_are_paths_without_values() {
    let (_server, request) = scale_request().await;
    let fields = request.changed_fields();
    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0].path, "spec.replicas");
    assert_eq!(fields[0].value, None);
}

#[tokio::test]
async fn manual_debug_shows_names_only() {
    let (_server, request) = scale_request().await;
    assert_eq!(
        format!("{request:?}"),
        r#"WriteRequest { operation: ReplaceObject, kind: Deployment, namespace: Some("payments"), name: "api" }"#
    );
    assert_eq!(format!("{:?}", request.operation()), "ReplaceObject");
}

fn cluster_role(name: &str) -> Value {
    json!({
        "apiVersion": "rbac.authorization.k8s.io/v1", "kind": "ClusterRole",
        "metadata": {"name": name, "uid": "u", "resourceVersion": "1", "labels": {"a": "x"}},
    })
}

/// Whether an edit of a role named `name` can become a write request.
async fn role_is_accepted(name: &str) -> bool {
    let server = server(cluster_role(name));
    let role = target(ObjectKind::ClusterRole, name);
    let base = base_of(&server, &role).await;
    let edit = edited(&base, "a: x", "a: y");
    WriteRequest::new(role, WriteOperation::ReplaceObject(Box::new(edit))).is_some()
}

#[tokio::test]
async fn rbac_names_may_use_a_colon() {
    assert!(role_is_accepted("system:aggregate-to-admin").await);
}

#[tokio::test]
async fn rbac_names_that_change_the_path_are_refused() {
    for name in ["a/b", "..", "a%2F", "a?b", "a#b", r"a\b", "caf\u{e9}"] {
        assert!(!role_is_accepted(name).await, "{name}");
    }
}

#[tokio::test]
async fn other_kinds_keep_the_dns_rule() {
    let server = server(cluster_role("x"));
    let colon = target(ObjectKind::Deployment, "a:b");
    let base = base_of(&server, &colon).await;
    let edit = edited(&base, "a: x", "a: y");
    assert!(WriteRequest::new(colon, WriteOperation::ReplaceObject(Box::new(edit))).is_none());
}

#[tokio::test]
async fn a_commit_reports_the_uid_of_the_replaced_object_and_a_dry_run_does_not() {
    let (server, request) = scale_request().await;
    let dry_run = server
        .connection
        .write(&request, WriteMode::DryRun)
        .await
        .expect("accepted");
    assert_eq!(dry_run.uid, None);
    let committed = server
        .connection
        .write(&request, WriteMode::Commit)
        .await
        .expect("accepted");
    assert_eq!(committed.uid.as_deref(), Some("uid-1"));
}

#[tokio::test]
async fn a_secret_is_masked_by_its_target_even_when_the_answer_names_no_kind() {
    let answer = json!({
        "apiVersion": "v1",
        "metadata": {"name": "db", "namespace": "payments", "uid": "uid-2", "resourceVersion": "8",
            "labels": {"app": "db2"}},
        "type": "Opaque",
        "data": {"password": SECRET_DATA},
    })
    .to_string();
    let server = server_with(WritePolicy::Allowed, secret(), 200, Some(answer));
    let target = target(ObjectKind::Secret, "db");
    let base = base_of(&server, &target).await;
    let request = replace(target, edited(&base, "app: db", "app: db2"));
    let outcome = server
        .connection
        .write(&request, WriteMode::DryRun)
        .await
        .expect("accepted");
    let WriteEffect::Replaced(preview) = outcome.effect else {
        panic!("{outcome:?}");
    };
    assert!(
        preview
            .changes
            .iter()
            .any(|change| change.path.to_string() == "metadata.labels.app"),
        "{:?}",
        preview.changes
    );
    for side in preview
        .changes
        .iter()
        .flat_map(|change| [&change.old, &change.new])
    {
        let text = side.as_deref().unwrap_or_default();
        assert!(!text.contains(SECRET_DATA), "{text}");
    }
}

#[tokio::test]
async fn a_helm_release_record_is_refused_even_if_the_type_changed_after_the_read() {
    let (server, request) = scale_request().await;
    server.current.lock().expect("the state is not poisoned")["type"] = json!("helm.sh/release.v1");
    let error = server
        .connection
        .write(&request, WriteMode::Commit)
        .await
        .expect_err("a release record");
    assert!(matches!(error, WriteError::Cluster(_)), "{error:?}");
    assert!(sent(&server).iter().all(|request| request.method == "GET"));
}

#[tokio::test]
async fn the_raw_query_carries_no_other_parameter() {
    let (server, request) = scale_request().await;
    server
        .connection
        .write(&request, WriteMode::DryRun)
        .await
        .expect("accepted");
    let requests = sent(&server);
    let put = requests.last().expect("a request");
    // kube starts its query with an empty pair, which `query_pairs` drops.
    assert_eq!(
        put.query.trim_start_matches('&'),
        "dryRun=All&fieldManager=k8sboard"
    );
}

/// The query pairs in order; kube starts its query with an empty one.
fn query_pairs(request: &RecordedRequest) -> Vec<&str> {
    request
        .query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .collect()
}
