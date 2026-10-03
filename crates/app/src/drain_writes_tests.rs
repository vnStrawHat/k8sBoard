use std::path::PathBuf;

use super::*;
use crate::write_guard::ActionRisk;

fn cluster() -> ClusterRef {
    ClusterRef {
        kubeconfig: PathBuf::from("/home/me/.kube/config"),
        context: "prod-ctx".to_owned(),
    }
}

fn pod(uid: &str) -> PodKey {
    PodKey {
        namespace: "payments".to_owned(),
        name: "api-1".to_owned(),
        uid: uid.to_owned(),
    }
}

#[test]
fn a_cordon_write_cordons_the_node_of_its_own_cluster() {
    let cluster = cluster();
    let scope = DrainScope {
        cluster: &cluster,
        cluster_name: "prod-a",
    };
    let intent = cordon_write(scope, "wk-04").expect("a valid node");
    assert_eq!(intent.cluster, cluster);
    assert_eq!(intent.label, "Cordon node wk-04");
    assert!(matches!(
        intent.request.operation(),
        WriteOperation::SetNodeSchedulable { schedulable: false }
    ));
    assert!(cordon_write(scope, "a/../b").is_none());
}

#[test]
fn an_eviction_write_is_pinned_to_the_uid_and_destructive() {
    let cluster = cluster();
    let scope = DrainScope {
        cluster: &cluster,
        cluster_name: "prod-a",
    };
    let intent = evict_write(scope, &pod("u-1"), GracePeriod::Seconds(30)).expect("a valid pod");
    assert_eq!(intent.label, "Evict pod payments/api-1");
    assert_eq!(intent.button, "Evict");
    assert_eq!(intent.risk, ActionRisk::Destructive);
    assert_eq!(intent.cluster, cluster);
    let WriteOperation::EvictPod { uid, grace } = intent.request.operation() else {
        panic!("expected EvictPod");
    };
    assert_eq!(uid, "u-1");
    assert_eq!(*grace, GracePeriod::Seconds(30));
    // Without a uid the eviction could hit a recreated pod.
    assert!(evict_write(scope, &pod(""), GracePeriod::PodDefault).is_none());
}
