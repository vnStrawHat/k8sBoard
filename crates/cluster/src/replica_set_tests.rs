use serde_json::{Value, json};

use super::*;
use crate::fake_api::FakeApi;
use crate::object_write::WritePolicy;

fn replica_set(name: &str, owner_kind: &str, owner_name: &str, controller: bool) -> Value {
    json!({
        "apiVersion": "apps/v1", "kind": "ReplicaSet",
        "metadata": {
            "name": name, "namespace": "shop",
            "annotations": {"deployment.kubernetes.io/revision": "3"},
            "ownerReferences": [{
                "apiVersion": "apps/v1", "kind": owner_kind, "name": owner_name,
                "uid": "u1", "controller": controller,
            }],
        },
        "spec": {"selector": {"matchLabels": {"app": "api"}}},
    })
}

fn orphan(name: &str) -> Value {
    json!({
        "apiVersion": "apps/v1", "kind": "ReplicaSet",
        "metadata": {"name": name, "namespace": "shop"},
    })
}

fn connection(items: Vec<Value>) -> (ClusterConnection, FakeApi) {
    FakeApi::connection(WritePolicy::Allowed, move |_| {
        let list = json!({"apiVersion": "apps/v1", "kind": "ReplicaSetList", "metadata": {}, "items": items});
        (200, list.to_string())
    })
}

fn deployment() -> ObjectRef {
    ObjectRef::new(
        ObjectKind::Deployment,
        Some("shop".to_owned()),
        "api".to_owned(),
    )
    .expect("a deployment is namespaced")
}

#[tokio::test]
async fn deployment_revisions_lists_the_namespace_once() {
    let (connection, api) = connection(Vec::new());
    connection
        .deployment_revisions(&deployment(), "app=api")
        .await
        .expect("the list goes through");
    let requests = api.requests();
    let [request] = requests.as_slice() else {
        panic!("expected one list, got {requests:?}");
    };
    assert_eq!(request.method, "GET");
    assert_eq!(request.path, "/apis/apps/v1/namespaces/shop/replicasets");
    assert!(request.has_query("labelSelector", "app%3Dapi"));
}

#[tokio::test]
async fn deployment_revisions_empty_selector_sends_nothing() {
    let (connection, api) = connection(vec![replica_set("api-1", "Deployment", "api", true)]);
    for selector in ["", "app=api,<invalid>"] {
        let sets = connection
            .deployment_revisions(&deployment(), selector)
            .await
            .expect("no request is made");
        assert!(sets.is_empty());
    }
    assert!(api.requests().is_empty());
}

#[tokio::test]
async fn deployment_revisions_keeps_only_controlled_sets() {
    let (connection, _api) = connection(vec![
        replica_set("api-1", "Deployment", "api", true),
        replica_set("other-1", "Deployment", "other", true),
        orphan("orphan-1"),
        replica_set("api-2", "Deployment", "api", false),
        replica_set("job-1", "Job", "api", true),
    ]);
    let sets = connection
        .deployment_revisions(&deployment(), "app=api")
        .await
        .expect("the list goes through");
    let names: Vec<&str> = sets.iter().map(|set| set.name.as_str()).collect();
    assert_eq!(names, ["api-1"]);
}

#[tokio::test]
async fn deployment_revisions_refuses_other_kinds() {
    let (connection, api) = connection(Vec::new());
    let service = ObjectRef::new(
        ObjectKind::Service,
        Some("shop".to_owned()),
        "api".to_owned(),
    )
    .expect("a service is namespaced");
    let error = connection
        .deployment_revisions(&service, "app=api")
        .await
        .expect_err("a service has no revisions");
    assert!(matches!(error, ClusterError::UnexpectedResponse { .. }));
    assert!(api.requests().is_empty());
}
