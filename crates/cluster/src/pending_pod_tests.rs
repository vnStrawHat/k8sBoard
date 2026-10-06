use serde_json::{Value, json};

use super::*;
use crate::fake_api::FakeApi;
use crate::object_write::WritePolicy;

const UNSCHEDULABLE: &str = "0/3 nodes are available: 1 node(s) had volume node affinity conflict, 1 node(s) had untolerated taint {workload: data}.";

fn pod(value: Value) -> Pod {
    serde_json::from_value(value).expect("a pod")
}

#[test]
fn an_unscheduled_pod_carries_the_scheduler_message() {
    let pending = pending_pod(&pod(json!({
        "metadata": {
            "name": "db-0", "namespace": "shop", "uid": "u-1",
            "ownerReferences": [{
                "apiVersion": "apps/v1", "kind": "StatefulSet", "name": "db",
                "uid": "o-1", "controller": true,
            }],
        },
        "status": {"phase": "Pending", "conditions": [{
            "type": "PodScheduled", "status": "False", "reason": "Unschedulable",
            "message": UNSCHEDULABLE,
        }]},
    })));
    assert_eq!(pending.reason.as_deref(), Some(UNSCHEDULABLE));
    assert_eq!(pending.uid, "u-1");
    let controller = pending.controller.expect("a controller");
    assert_eq!(
        (controller.kind.as_str(), controller.name.as_str()),
        ("StatefulSet", "db")
    );
}

#[test]
fn a_scheduled_or_unjudged_pod_has_no_reason() {
    let scheduled = pending_pod(&pod(json!({
        "metadata": {"name": "a", "namespace": "n", "uid": "u"},
        "status": {"phase": "Pending", "conditions": [{
            "type": "PodScheduled", "status": "True",
        }]},
    })));
    let unjudged = pending_pod(&pod(json!({
        "metadata": {"name": "b", "namespace": "n", "uid": "u"},
        "status": {"phase": "Pending"},
    })));
    assert_eq!(scheduled.reason, None);
    assert_eq!(unjudged.reason, None);
}

#[tokio::test]
async fn pending_pods_are_listed_by_phase_and_sorted() {
    let (connection, api) = FakeApi::connection(WritePolicy::Blocked, |_| {
        let list = json!({
            "apiVersion": "v1", "kind": "PodList", "metadata": {},
            "items": [
                {"metadata": {"name": "b", "namespace": "z", "uid": "u-b"}},
                {"metadata": {"name": "a", "namespace": "a", "uid": "u-a"}},
            ],
        });
        (200, list.to_string())
    });
    let pods = connection.pending_pods().await.expect("the list");
    let names: Vec<_> = pods.iter().map(|pod| pod.name.as_str()).collect();
    assert_eq!(names, ["a", "b"]);
    let requests = api.requests();
    let [request] = requests.as_slice() else {
        panic!("expected one list, got {requests:?}");
    };
    assert_eq!(request.path, "/api/v1/pods");
    assert!(
        request.has_query("fieldSelector", "status.phase%3DPending"),
        "{}",
        request.query
    );
}
