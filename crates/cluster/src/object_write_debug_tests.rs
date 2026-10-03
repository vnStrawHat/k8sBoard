//! Tests of the 0037 operations: the debug container patch, the node shell pod, and its delete.

use serde_json::{Value, json};

use super::*;
use crate::debug_pod_bodies::DEFAULT_DEBUG_IMAGE;
use crate::fake_api::{FakeApi, RecordedRequest};
use crate::pod_shell::AUTO_SCRIPT;

const NODE_SHELL_NAME: &str = "k8sboard-node-shell-wk-03-x7k2q";
const PODS_PATH: &str = "/api/v1/namespaces/kube-system/pods";
const STRATEGIC_PATCH: &str = "application/strategic-merge-patch+json";
const UID: &str = "5f1c3a52-0f0b-4c1e-9b0e-2a3f6d6c9e11";

fn pod_ref(namespace: &str, name: &str) -> ObjectRef {
    ObjectRef::new(ObjectKind::Pod, Some(namespace.to_owned()), name.to_owned())
        .expect("a namespaced kind")
}

fn debug_request() -> WriteRequest {
    WriteRequest::new(
        pod_ref("payments", "api-0"),
        WriteOperation::AddDebugContainer {
            name: "k8sboard-debug-x7k2q".to_owned(),
            image: DEFAULT_DEBUG_IMAGE.to_owned(),
            target_container: "api".to_owned(),
        },
    )
    .expect("a pod fits the debug container")
}

fn create_request() -> WriteRequest {
    WriteRequest::new(
        pod_ref("kube-system", NODE_SHELL_NAME),
        WriteOperation::CreateNodeShellPod {
            node: "wk-03".to_owned(),
            image: DEFAULT_DEBUG_IMAGE.to_owned(),
            user: Some("readonly".to_owned()),
            instance: "q4m7x2k9pa".to_owned(),
        },
    )
    .expect("a node shell pod name fits the create")
}

fn delete_request() -> WriteRequest {
    WriteRequest::new(
        pod_ref("kube-system", NODE_SHELL_NAME),
        WriteOperation::DeleteNodeShellPod {
            uid: UID.to_owned(),
        },
    )
    .expect("a node shell pod name fits the delete")
}

fn pod_json(name: &str) -> String {
    json!({"apiVersion": "v1", "kind": "Pod", "metadata": {"name": name, "uid": UID}}).to_string()
}

fn answered() -> (ClusterConnection, FakeApi) {
    FakeApi::connection(
        WritePolicy::Allowed,
        |request: &RecordedRequest| match request.method.as_str() {
            "POST" => (201, pod_json(NODE_SHELL_NAME)),
            "DELETE" => (
                200,
                json!({"kind": "Status", "apiVersion": "v1", "status": "Success"}).to_string(),
            ),
            _ => (200, pod_json("api-0")),
        },
    )
}

fn body_of(request: &RecordedRequest) -> Value {
    serde_json::from_str(&request.body).expect("the body is JSON")
}

fn only(api: &FakeApi) -> RecordedRequest {
    let requests = api.requests();
    let [request] = requests.as_slice() else {
        panic!("expected one request, got {requests:?}");
    };
    request.clone()
}

#[tokio::test]
async fn debug_patch_is_strategic_merge() {
    let (connection, api) = answered();
    let outcome = connection
        .write(&debug_request(), WriteMode::Commit)
        .await
        .expect("the patch goes through");
    assert_eq!(outcome.effect, WriteEffect::Patched);
    let sent = only(&api);
    assert_eq!(sent.method, "PATCH");
    assert_eq!(
        sent.path,
        "/api/v1/namespaces/payments/pods/api-0/ephemeralcontainers"
    );
    assert_eq!(sent.content_type.as_deref(), Some(STRATEGIC_PATCH));
    assert!(sent.has_query("fieldManager", "k8sboard"));
    assert!(!sent.has_query_key("dryRun"));
    assert_eq!(
        body_of(&sent),
        json!({"spec": {"ephemeralContainers": [{
            "name": "k8sboard-debug-x7k2q",
            "image": DEFAULT_DEBUG_IMAGE,
            "command": ["sh"],
            "stdin": true, "stdinOnce": true, "tty": true,
            "targetContainerName": "api",
            "imagePullPolicy": "IfNotPresent",
            "terminationMessagePolicy": "File",
        }]}})
    );
}

#[tokio::test]
async fn debug_dry_run_adds_the_dry_run_query() {
    let (connection, api) = answered();
    connection
        .write(&debug_request(), WriteMode::DryRun)
        .await
        .expect("the dry-run goes through");
    let sent = only(&api);
    assert!(sent.has_query("dryRun", "All"));
    assert!(sent.has_query("fieldManager", "k8sboard"));
}

#[tokio::test]
async fn node_shell_create_posts_the_pod_to_the_namespace() {
    let (connection, api) = answered();
    connection
        .write(&create_request(), WriteMode::DryRun)
        .await
        .expect("the dry-run goes through");
    connection
        .write(&create_request(), WriteMode::Commit)
        .await
        .expect("the create goes through");
    let requests = api.requests();
    let [dry, commit] = requests.as_slice() else {
        panic!("expected two requests, got {requests:?}");
    };
    for sent in [dry, commit] {
        assert_eq!(sent.method, "POST");
        assert_eq!(sent.path, PODS_PATH);
        assert_eq!(sent.content_type.as_deref(), Some("application/json"));
        assert!(sent.has_query("fieldManager", "k8sboard"));
    }
    assert!(dry.has_query("dryRun", "All"));
    assert!(!commit.has_query_key("dryRun"));
    assert_eq!(body_of(dry), body_of(commit));
}

#[tokio::test]
async fn node_shell_pod_is_privileged_on_the_node() {
    let (connection, api) = answered();
    connection
        .write(&create_request(), WriteMode::Commit)
        .await
        .expect("the create goes through");
    let body = body_of(&only(&api));
    assert_eq!(body["metadata"]["name"], json!(NODE_SHELL_NAME));
    assert_eq!(body["metadata"]["namespace"], json!("kube-system"));
    let labels = &body["metadata"]["labels"];
    assert_eq!(labels["app.kubernetes.io/managed-by"], json!("k8sboard"));
    assert_eq!(labels["k8sboard.io/purpose"], json!("node-shell"));
    assert_eq!(labels["k8sboard.io/node"], json!("wk-03"));
    assert_eq!(labels["k8sboard.io/instance"], json!("q4m7x2k9pa"));
    assert_eq!(
        body["metadata"]["annotations"]["k8sboard.io/user"],
        json!("readonly")
    );
    let spec = &body["spec"];
    assert_eq!(spec["nodeName"], json!("wk-03"));
    assert_eq!(spec["hostPID"], json!(true));
    assert_eq!(spec["activeDeadlineSeconds"], json!(14400));
    assert_eq!(spec["automountServiceAccountToken"], json!(false));
    assert_eq!(spec["tolerations"], json!([{"operator": "Exists"}]));
    let container = &spec["containers"][0];
    assert_eq!(container["securityContext"]["privileged"], json!(true));
    assert_eq!(container["stdinOnce"], json!(true));
    assert_eq!(container["image"], json!(DEFAULT_DEBUG_IMAGE));
    assert_eq!(container["command"][11], json!(AUTO_SCRIPT));
}

#[tokio::test]
async fn commit_returns_the_created_uid() {
    let (connection, _api) = answered();
    let dry = connection
        .write(&create_request(), WriteMode::DryRun)
        .await
        .expect("the dry-run goes through");
    assert_eq!(dry.uid, None);
    assert_eq!(dry.created_name, None);
    let commit = connection
        .write(&create_request(), WriteMode::Commit)
        .await
        .expect("the create goes through");
    assert_eq!(commit.effect, WriteEffect::Created);
    assert_eq!(commit.uid.as_deref(), Some(UID));
    assert_eq!(commit.created_name.as_deref(), Some(NODE_SHELL_NAME));
}

#[tokio::test]
async fn delete_carries_uid_and_zero_grace_without_dry_run() {
    let request = delete_request();
    assert!(!request.supports_dry_run());
    let (connection, api) = answered();
    let outcome = connection
        .write(&request, WriteMode::Commit)
        .await
        .expect("the delete goes through");
    assert_eq!(outcome.effect, WriteEffect::Deleted);
    let sent = only(&api);
    assert_eq!(sent.method, "DELETE");
    assert_eq!(sent.path, format!("{PODS_PATH}/{NODE_SHELL_NAME}"));
    assert!(!sent.has_query_key("dryRun"));
    let body = body_of(&sent);
    assert_eq!(body["gracePeriodSeconds"], json!(0));
    assert_eq!(body["preconditions"]["uid"], json!(UID));
    assert!(body.get("dryRun").is_none());
}

#[tokio::test]
async fn a_dry_run_of_the_delete_sends_nothing() {
    let (connection, api) = answered();
    let error = connection
        .write(&delete_request(), WriteMode::DryRun)
        .await
        .expect_err("the delete has no dry-run");
    assert!(matches!(error, WriteError::Invalid { .. }), "{error:?}");
    assert!(api.requests().is_empty());
}

#[tokio::test]
async fn a_delete_of_a_gone_pod_is_not_found_and_a_changed_uid_is_a_conflict() {
    let status = |code: u16| {
        FakeApi::connection(WritePolicy::Allowed, move |_| {
            (
                code,
                json!({"kind": "Status", "apiVersion": "v1", "status": "Failure",
                    "message": "m", "reason": "r", "code": code})
                .to_string(),
            )
        })
    };
    let (gone, _) = status(404);
    let error = gone
        .write(&delete_request(), WriteMode::Commit)
        .await
        .expect_err("the pod is gone");
    assert!(matches!(error, WriteError::NotFound), "{error:?}");
    let (taken, _) = status(409);
    let error = taken
        .write(&delete_request(), WriteMode::Commit)
        .await
        .expect_err("another pod took the name");
    assert!(matches!(error, WriteError::Conflict { .. }), "{error:?}");
}

#[test]
fn node_shell_requests_reject_foreign_pods() {
    let foreign = pod_ref("kube-system", "coredns-5d78c9869d-abcde");
    for operation in [
        WriteOperation::DeleteNodeShellPod {
            uid: UID.to_owned(),
        },
        WriteOperation::CreateNodeShellPod {
            node: "wk-03".to_owned(),
            image: DEFAULT_DEBUG_IMAGE.to_owned(),
            user: None,
            instance: "q4m7x2k9pa".to_owned(),
        },
    ] {
        assert!(WriteRequest::new(foreign.clone(), operation).is_none());
    }
}

#[test]
fn debug_operations_reject_other_kinds() {
    let node = ObjectRef::new(ObjectKind::Node, None, "wk-03".to_owned()).expect("a node");
    let service = ObjectRef::new(
        ObjectKind::Service,
        Some("payments".to_owned()),
        "api".to_owned(),
    )
    .expect("a service");
    for target in [node, service] {
        assert!(
            WriteRequest::new(
                target.clone(),
                WriteOperation::AddDebugContainer {
                    name: "k8sboard-debug-x7k2q".to_owned(),
                    image: DEFAULT_DEBUG_IMAGE.to_owned(),
                    target_container: "api".to_owned(),
                },
            )
            .is_none()
        );
        assert!(
            WriteRequest::new(
                target,
                WriteOperation::DeleteNodeShellPod {
                    uid: UID.to_owned()
                },
            )
            .is_none()
        );
    }
}

#[test]
fn debug_values_the_server_would_refuse_are_refused_here() {
    let patch = |name: &str, image: &str, target: &str| {
        WriteRequest::new(
            pod_ref("payments", "api-0"),
            WriteOperation::AddDebugContainer {
                name: name.to_owned(),
                image: image.to_owned(),
                target_container: target.to_owned(),
            },
        )
    };
    assert!(patch("k8sboard-debug-x7k2q", DEFAULT_DEBUG_IMAGE, "api").is_some());
    assert!(patch("other-x7k2q", DEFAULT_DEBUG_IMAGE, "api").is_none());
    assert!(patch("k8sboard-debug-x7k2q", "bus ybox", "api").is_none());
    assert!(patch("k8sboard-debug-x7k2q", "", "api").is_none());
    assert!(patch("k8sboard-debug-x7k2q", DEFAULT_DEBUG_IMAGE, "Api").is_none());
    let delete = |uid: &str| {
        WriteRequest::new(
            pod_ref("kube-system", NODE_SHELL_NAME),
            WriteOperation::DeleteNodeShellPod {
                uid: uid.to_owned(),
            },
        )
    };
    assert!(delete("").is_none());
    assert!(delete(r#"x","extra":"1"#).is_none());
    let create = |node: &str, instance: &str| {
        WriteRequest::new(
            pod_ref("kube-system", NODE_SHELL_NAME),
            WriteOperation::CreateNodeShellPod {
                node: node.to_owned(),
                image: DEFAULT_DEBUG_IMAGE.to_owned(),
                user: None,
                instance: instance.to_owned(),
            },
        )
    };
    assert!(create("wk-03", "q4m7x2k9pa").is_some());
    assert!(create("Wk 03", "q4m7x2k9pa").is_none());
    assert!(create("wk-03", "").is_none());
}

#[tokio::test]
async fn blocked_policy_sends_nothing_for_new_operations() {
    let (connection, api) = FakeApi::connection(WritePolicy::Blocked, |_| (200, "{}".to_owned()));
    for request in [debug_request(), create_request(), delete_request()] {
        for mode in [WriteMode::DryRun, WriteMode::Commit] {
            let error = connection
                .write(&request, mode)
                .await
                .expect_err("writes are blocked");
            assert!(matches!(error, WriteError::WritesBlocked), "{error:?}");
        }
    }
    assert!(api.requests().is_empty());
}

#[test]
fn the_new_operations_name_their_permission() {
    assert_eq!(
        debug_request().access_check(),
        AccessCheck::PatchPodEphemeralContainers
    );
    assert_eq!(create_request().access_check(), AccessCheck::CreatePods);
    assert_eq!(delete_request().access_check(), AccessCheck::DeletePods);
}

fn recorded(fields: Vec<ChangedField>) -> Vec<(String, Option<String>)> {
    fields
        .into_iter()
        .map(|field| (field.path.into_owned(), field.value))
        .collect()
}

#[test]
fn changed_fields_name_privilege_and_image() {
    let field = |path: &str, value: &str| (path.to_owned(), Some(value.to_owned()));
    assert_eq!(
        recorded(create_request().changed_fields()),
        [
            field("spec.nodeName", "wk-03"),
            field("spec.hostPID", "true"),
            field("spec.containers[0].securityContext.privileged", "true"),
            field("spec.containers[0].image", DEFAULT_DEBUG_IMAGE),
        ]
    );
    assert_eq!(
        recorded(debug_request().changed_fields()),
        [
            field("spec.ephemeralContainers[].name", "k8sboard-debug-x7k2q"),
            field("spec.ephemeralContainers[].image", DEFAULT_DEBUG_IMAGE),
            field("spec.ephemeralContainers[].targetContainerName", "api"),
        ]
    );
}

#[test]
fn manual_debug_hides_image_and_command() {
    for request in [debug_request(), create_request(), delete_request()] {
        let text = format!("{request:?}");
        assert!(!text.contains("busybox"), "{text}");
        assert!(!text.contains("nsenter"), "{text}");
        assert!(!text.contains(UID), "{text}");
        assert!(text.contains("Pod"), "{text}");
    }
    assert_eq!(
        format!("{:?}", create_request().operation()),
        "CreateNodeShellPod"
    );
    assert_eq!(
        format!("{:?}", delete_request().operation()),
        "DeleteNodeShellPod"
    );
    assert_eq!(
        format!("{:?}", debug_request().operation()),
        "AddDebugContainer"
    );
}

#[tokio::test]
async fn allow_list_matches_the_operations() {
    // The three rows of spec 0037: method, path, query, content type, and body.
    let cases = [
        (
            debug_request(),
            "PATCH",
            "/api/v1/namespaces/payments/pods/api-0/ephemeralcontainers",
            STRATEGIC_PATCH,
        ),
        (create_request(), "POST", PODS_PATH, "application/json"),
        (
            delete_request(),
            "DELETE",
            "/api/v1/namespaces/kube-system/pods/k8sboard-node-shell-wk-03-x7k2q",
            "application/json",
        ),
    ];
    for (request, method, path, content_type) in cases {
        let (connection, api) = answered();
        let modes: &[WriteMode] = if request.supports_dry_run() {
            &[WriteMode::DryRun, WriteMode::Commit]
        } else {
            &[WriteMode::Commit]
        };
        for mode in modes {
            connection
                .write(&request, *mode)
                .await
                .expect("the write goes through");
        }
        let sent = api.requests();
        assert_eq!(sent.len(), modes.len(), "{request:?}");
        for (sent, mode) in sent.iter().zip(modes) {
            assert_eq!(sent.method, method, "{request:?}");
            assert_eq!(sent.path, path, "{request:?}");
            // A delete carries its options in the body, not in the query.
            if method != "DELETE" {
                assert!(sent.has_query("fieldManager", "k8sboard"), "{request:?}");
                assert_eq!(sent.has_query_key("dryRun"), *mode == WriteMode::DryRun);
            }
            assert!(!sent.has_query_key("dryRun") || *mode == WriteMode::DryRun);
            // The body is JSON of the operation alone; a dry-run and a commit send the same one.
            assert_eq!(
                sent.content_type.as_deref(),
                Some(content_type),
                "{request:?}"
            );
            assert_eq!(body_of(sent), body_of(&sent_first(&api)), "{request:?}");
        }
    }
}

fn sent_first(api: &FakeApi) -> RecordedRequest {
    api.requests().remove(0)
}
