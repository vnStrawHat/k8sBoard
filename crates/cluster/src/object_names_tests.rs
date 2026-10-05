use serde_json::{Value, json};

use super::*;
use crate::fake_api::{FakeApi, RecordedRequest};
use crate::object_write::WritePolicy;

fn item(namespace: Option<&str>, name: &str) -> Value {
    let mut metadata = json!({"name": name, "labels": {"owner": "never-kept"}});
    if let Some(namespace) = namespace {
        metadata["namespace"] = json!(namespace);
    }
    json!({"apiVersion": "v1", "kind": "PartialObjectMetadata", "metadata": metadata})
}

fn page(items: Vec<Value>, continue_token: Option<&str>) -> String {
    let mut metadata = json!({"resourceVersion": "1"});
    if let Some(token) = continue_token {
        metadata["continue"] = json!(token);
    }
    json!({
        "apiVersion": "v1",
        "kind": "PartialObjectMetadataList",
        "metadata": metadata,
        "items": items,
    })
    .to_string()
}

fn full_page(count: usize, offset: usize, continue_token: Option<&str>) -> String {
    let items = (0..count)
        .map(|index| item(Some("shop"), &format!("svc-{}", offset + index)))
        .collect();
    page(items, continue_token)
}

fn name(namespace: Option<&str>, name: &str) -> ObjectName {
    ObjectName {
        namespace: namespace.map(str::to_owned),
        name: name.to_owned(),
    }
}

#[tokio::test]
async fn two_pages_are_joined_through_the_continue_token() {
    let respond = |request: &RecordedRequest| {
        let body = if request.has_query("continue", "tok") {
            page(vec![item(Some("shop"), "web")], None)
        } else {
            page(vec![item(Some("shop"), "api")], Some("tok"))
        };
        (200, body)
    };
    let (connection, api) = FakeApi::connection(WritePolicy::Blocked, respond);
    let list = connection
        .list_object_names(ObjectKind::Service, &NamespaceScope::All)
        .await
        .expect("the names are listed");
    assert_eq!(
        list,
        NameList {
            names: vec![name(Some("shop"), "api"), name(Some("shop"), "web")],
            is_truncated: false,
        }
    );
    assert_eq!(api.requests().len(), 2);
}

#[tokio::test]
async fn the_list_is_metadata_only_and_never_asks_for_the_watch_cache() {
    let (connection, api) =
        FakeApi::connection(WritePolicy::Blocked, |_| (200, page(vec![], None)));
    connection
        .list_object_names(ObjectKind::Ingress, &NamespaceScope::All)
        .await
        .expect("the names are listed");
    let requests = api.requests();
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.method, "GET");
    assert_eq!(request.path, "/apis/networking.k8s.io/v1/ingresses");
    assert!(request.has_query("limit", "500"), "{}", request.query);
    assert!(
        !request.has_query_key("resourceVersion"),
        "{}",
        request.query
    );
    let accept = request.accept.as_deref().unwrap_or_default();
    assert!(accept.contains("PartialObjectMetadataList"), "{accept}");
}

#[tokio::test]
async fn only_namespace_and_name_are_kept() {
    let respond = |_: &RecordedRequest| (200, page(vec![item(Some("shop"), "api")], None));
    let (connection, _) = FakeApi::connection(WritePolicy::Blocked, respond);
    let list = connection
        .list_object_names(ObjectKind::CronJob, &NamespaceScope::All)
        .await
        .expect("the names are listed");
    // `ObjectName` has no other field to hold a label, so the shape is the proof.
    assert_eq!(list.names, [name(Some("shop"), "api")]);
}

#[tokio::test]
async fn a_cluster_scoped_kind_has_no_namespace() {
    let respond = |_: &RecordedRequest| (200, page(vec![item(None, "wk-01")], None));
    let (connection, api) = FakeApi::connection(WritePolicy::Blocked, respond);
    let list = connection
        .list_object_names(ObjectKind::Node, &NamespaceScope::Named("shop".to_owned()))
        .await
        .expect("the names are listed");
    assert_eq!(list.names, [name(None, "wk-01")]);
    assert_eq!(api.requests()[0].path, "/api/v1/nodes");
}

#[tokio::test]
async fn the_limit_truncates_and_stops_paging() {
    let respond = |request: &RecordedRequest| {
        let offset = request
            .query
            .split('&')
            .find_map(|pair| pair.strip_prefix("continue="))
            .map_or(0, |token| token.parse().expect("a numeric token"));
        (
            200,
            full_page(500, offset, Some(&(offset + 500).to_string())),
        )
    };
    let (connection, api) = FakeApi::connection(WritePolicy::Blocked, respond);
    let list = connection
        .list_object_names(ObjectKind::Service, &NamespaceScope::All)
        .await
        .expect("the names are listed");
    assert_eq!(list.names.len(), 5_000);
    assert!(list.is_truncated);
    assert_eq!(api.requests().len(), 10, "no page past the limit");
}

#[tokio::test]
async fn a_kind_that_ends_exactly_at_the_limit_is_not_truncated() {
    let respond = |request: &RecordedRequest| {
        let offset: usize = request
            .query
            .split('&')
            .find_map(|pair| pair.strip_prefix("continue="))
            .map_or(0, |token| token.parse().expect("a numeric token"));
        let next = offset + 500;
        // The tenth page has no continue token: nothing is left.
        let token = (next < 5_000).then(|| next.to_string());
        (200, full_page(500, offset, token.as_deref()))
    };
    let (connection, _) = FakeApi::connection(WritePolicy::Blocked, respond);
    let list = connection
        .list_object_names(ObjectKind::Service, &NamespaceScope::All)
        .await
        .expect("the names are listed");
    assert_eq!(list.names.len(), 5_000);
    assert!(!list.is_truncated);
}

#[tokio::test]
async fn several_namespaces_are_listed_one_by_one() {
    let respond = |request: &RecordedRequest| {
        let namespace = request
            .path
            .split('/')
            .nth(5)
            .unwrap_or_default()
            .to_owned();
        (200, page(vec![item(Some(&namespace), "api")], None))
    };
    let (connection, api) = FakeApi::connection(WritePolicy::Blocked, respond);
    let scope = NamespaceScope::Several(vec!["shop".to_owned(), "billing".to_owned()]);
    let list = connection
        .list_object_names(ObjectKind::StatefulSet, &scope)
        .await
        .expect("the names are listed");
    assert_eq!(
        list.names,
        [name(Some("shop"), "api"), name(Some("billing"), "api")]
    );
    let paths: Vec<String> = api
        .requests()
        .into_iter()
        .map(|request| request.path)
        .collect();
    assert_eq!(
        paths,
        [
            "/apis/apps/v1/namespaces/shop/statefulsets",
            "/apis/apps/v1/namespaces/billing/statefulsets",
        ]
    );
}

#[tokio::test]
async fn a_refused_list_is_an_error() {
    let respond = |_: &RecordedRequest| {
        let status = json!({
            "kind": "Status", "apiVersion": "v1", "status": "Failure",
            "message": "forbidden", "reason": "Forbidden", "code": 403,
        });
        (403, status.to_string())
    };
    let (connection, _) = FakeApi::connection(WritePolicy::Blocked, respond);
    let error = connection
        .list_object_names(ObjectKind::NetworkPolicy, &NamespaceScope::All)
        .await
        .expect_err("a 403 is an error");
    assert!(matches!(error, ClusterError::Forbidden { .. }), "{error:?}");
}
