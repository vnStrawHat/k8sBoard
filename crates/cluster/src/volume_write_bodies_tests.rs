use serde_json::json;

use super::*;

fn pending_claim() -> Value {
    json!({
        "metadata": {
            "name": "pvc-wrong-class", "namespace": "lab-broken", "uid": "claim-uid",
            "labels": { "app": "db" },
            "annotations": { "volume.kubernetes.io/selected-node": "node-a" },
            "resourceVersion": "9", "creationTimestamp": "2026-10-01T10:00:00Z",
        },
        "spec": {
            "accessModes": ["ReadWriteOnce"],
            "resources": { "requests": { "storage": "100Mi" } },
            "storageClassName": "missing",
            "volumeMode": "Filesystem",
        },
        "status": { "phase": "Pending" },
    })
}

#[test]
fn the_new_claim_keeps_size_modes_and_labels_and_takes_the_new_class() {
    let body = recreated_claim_body(&pending_claim(), "claim-uid", "standard").expect("a body");
    assert_eq!(
        body,
        json!({
            "apiVersion": "v1",
            "kind": "PersistentVolumeClaim",
            "metadata": { "name": "pvc-wrong-class", "namespace": "lab-broken", "labels": { "app": "db" } },
            "spec": {
                "accessModes": ["ReadWriteOnce"],
                "resources": { "requests": { "storage": "100Mi" } },
                "storageClassName": "standard",
                "volumeMode": "Filesystem",
            },
        })
    );
}

#[test]
fn a_claim_that_is_bound_replaced_or_going_is_not_recreated() {
    let refusal = |edit: fn(&mut Value)| {
        let mut claim = pending_claim();
        edit(&mut claim);
        recreated_claim_body(&claim, "claim-uid", "standard").expect_err("a refusal")
    };
    assert_eq!(
        refusal(|claim| claim["status"]["phase"] = json!("Bound")),
        RecreateRefusal::Bound
    );
    assert_eq!(
        refusal(|claim| claim["spec"]["volumeName"] = json!("pv-1")),
        RecreateRefusal::Bound
    );
    assert_eq!(
        refusal(|claim| claim["metadata"]["deletionTimestamp"] = json!("2026-10-02T00:00:00Z")),
        RecreateRefusal::Terminating
    );
    assert_eq!(
        refusal(|claim| claim["metadata"]["uid"] = json!("another")),
        RecreateRefusal::Replaced
    );
    assert_eq!(
        refusal(|claim| claim["metadata"]["namespace"] = Value::Null),
        RecreateRefusal::Unreadable
    );
}

#[test]
fn a_reclaim_policy_is_one_merge_patch_and_only_retain_and_delete_parse() {
    assert_eq!(
        reclaim_policy_patch(ReclaimPolicy::Retain),
        json!({ "spec": { "persistentVolumeReclaimPolicy": "Retain" } })
    );
    assert_eq!(ReclaimPolicy::parse("Delete"), Some(ReclaimPolicy::Delete));
    assert_eq!(ReclaimPolicy::parse("Recycle"), None);
    assert_eq!(ReclaimPolicy::Retain.to_string(), "Retain");
}
