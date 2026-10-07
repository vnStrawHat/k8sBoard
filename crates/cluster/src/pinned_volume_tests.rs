use serde_json::{Value, json};

use super::*;
use crate::fake_api::FakeApi;
use crate::object_write::WritePolicy;

fn pod(namespace: &str, name: &str, claims: &[&str]) -> DrainPod {
    DrainPod {
        namespace: namespace.to_owned(),
        name: name.to_owned(),
        uid: format!("uid-{name}"),
        labels: Vec::new(),
        controller: None,
        is_mirror: false,
        has_empty_dir: false,
        is_finished: false,
        is_pending: false,
        is_terminating: false,
        claims: claims.iter().map(|claim| (*claim).to_owned()).collect(),
        pinned_volume: None,
        placement: crate::PodPlacement::default(),
    }
}

fn volume(claim: &str, term: Value) -> Value {
    json!({
        "metadata": {"name": format!("pv-{claim}")},
        "spec": {
            "claimRef": {"namespace": "shop", "name": claim},
            "nodeAffinity": {"required": {"nodeSelectorTerms": [term]}},
        },
    })
}

fn hostname_term(host: &str) -> Value {
    json!({"matchExpressions": [{
        "key": "kubernetes.io/hostname", "operator": "In", "values": [host],
    }]})
}

fn answering(volumes: Vec<Value>) -> impl Fn(&crate::fake_api::RecordedRequest) -> (u16, String) {
    move |request| {
        if request.path == "/api/v1/nodes/wk-04" {
            let node = json!({
                "apiVersion": "v1", "kind": "Node",
                "metadata": {
                    "name": "wk-04", "resourceVersion": "7",
                    "labels": {"kubernetes.io/hostname": "wk-04.lab"},
                },
            });
            return (200, node.to_string());
        }
        if request.path == "/api/v1/persistentvolumes" {
            let list = json!({
                "apiVersion": "v1", "kind": "PersistentVolumeList",
                "metadata": {}, "items": volumes,
            });
            return (200, list.to_string());
        }
        (404, "{}".to_owned())
    }
}

async fn pinned(volumes: Vec<Value>, pods: &mut [DrainPod]) -> Result<(), ClusterError> {
    let (connection, _api) = FakeApi::connection(WritePolicy::Blocked, answering(volumes));
    connection.pin_volumes("wk-04", pods).await
}

#[tokio::test]
async fn a_claim_bound_to_a_volume_of_this_host_pins_its_pod() {
    let volumes = vec![
        volume("data-db-0", hostname_term("wk-04.lab")),
        volume("data-db-1", hostname_term("wk-05.lab")),
    ];
    let mut pods = [
        pod("shop", "db-0", &["data-db-0"]),
        pod("shop", "db-1", &["data-db-1"]),
        pod("shop", "web", &[]),
    ];
    pinned(volumes, &mut pods).await.expect("the reads");
    assert_eq!(pods[0].pinned_volume.as_deref(), Some("data-db-0"));
    assert_eq!(pods[1].pinned_volume, None);
    assert_eq!(pods[2].pinned_volume, None);
}

#[tokio::test]
async fn the_node_name_matches_when_the_host_label_differs() {
    let field = json!({"matchFields": [{
        "key": "metadata.name", "operator": "In", "values": ["wk-04"],
    }]});
    let mut pods = [pod("shop", "db-0", &["data-db-0"])];
    pinned(vec![volume("data-db-0", field)], &mut pods)
        .await
        .expect("the reads");
    assert_eq!(pods[0].pinned_volume.as_deref(), Some("data-db-0"));
}

#[tokio::test]
async fn a_zone_affinity_does_not_pin_a_pod_to_one_node() {
    let zone = json!({"matchExpressions": [{
        "key": "topology.kubernetes.io/zone", "operator": "In", "values": ["wk-04"],
    }]});
    let mut pods = [pod("shop", "db-0", &["data-db-0"])];
    pinned(vec![volume("data-db-0", zone)], &mut pods)
        .await
        .expect("the reads");
    assert_eq!(pods[0].pinned_volume, None);
}

#[tokio::test]
async fn the_same_claim_name_in_another_namespace_is_not_pinned() {
    let mut pods = [pod("other", "db-0", &["data-db-0"])];
    pinned(
        vec![volume("data-db-0", hostname_term("wk-04.lab"))],
        &mut pods,
    )
    .await
    .expect("the reads");
    assert_eq!(pods[0].pinned_volume, None);
}

#[tokio::test]
async fn pods_without_claims_ask_nothing() {
    let (connection, api) = FakeApi::connection(WritePolicy::Blocked, answering(Vec::new()));
    let mut pods = [pod("shop", "web", &[])];
    connection
        .pin_volumes("wk-04", &mut pods)
        .await
        .expect("nothing to read");
    assert!(api.requests().is_empty());
}

#[tokio::test]
async fn a_failed_volume_list_is_an_error_and_pins_nothing() {
    let (connection, _api) = FakeApi::connection(WritePolicy::Blocked, |request| {
        if request.path == "/api/v1/nodes/wk-04" {
            let node = json!({
                "apiVersion": "v1", "kind": "Node",
                "metadata": {"name": "wk-04", "resourceVersion": "7"},
            });
            return (200, node.to_string());
        }
        (403, "{}".to_owned())
    });
    let mut pods = [pod("shop", "db-0", &["data-db-0"])];
    let result = connection.pin_volumes("wk-04", &mut pods).await;
    assert!(result.is_err());
    assert_eq!(pods[0].pinned_volume, None);
}
