use serde_json::json;

use super::*;

fn deployment() -> Value {
    json!({
        "metadata": {"uid": "dep-uid"},
        "spec": {"template": {
            "metadata": {"labels": {"app": "api", "pod-template-hash": "now"}},
            "spec": {"containers": [{"name": "api", "image": "api:2"}]},
        }},
    })
}

fn replica_set(owner_uid: &str, revision: &str, image: &str) -> Value {
    json!({
        "metadata": {
            "annotations": {"deployment.kubernetes.io/revision": revision},
            "ownerReferences": [{"uid": owner_uid, "controller": true}],
        },
        "spec": {"template": {
            "metadata": {"labels": {"app": "api", "pod-template-hash": "then"}},
            "spec": {"containers": [{"name": "api", "image": image}]},
        }},
    })
}

#[test]
fn rollback_tests_the_uid_and_replaces_the_template_without_the_hash() {
    let operations = rollback_operations(&deployment(), &replica_set("dep-uid", "37", "api:1"), 37)
        .unwrap_or_else(|_| panic!("a valid roll back"));
    assert_eq!(
        operations,
        json!([
            {"op": "test", "path": "/metadata/uid", "value": "dep-uid"},
            {"op": "replace", "path": "/spec/template", "value": {
                "metadata": {"labels": {"app": "api"}},
                "spec": {"containers": [{"name": "api", "image": "api:1"}]},
            }},
        ])
    );
}

#[test]
fn rollback_refuses_a_replica_set_of_another_owner_or_revision() {
    let refusals = [
        replica_set("other-uid", "37", "api:1"),
        replica_set("dep-uid", "38", "api:1"),
    ];
    for replica_set in refusals {
        let refusal = rollback_operations(&deployment(), &replica_set, 37).expect_err("refused");
        assert!(matches!(refusal, RollBackRefusal::ForeignReplicaSet));
    }
    let mut not_controller = replica_set("dep-uid", "37", "api:1");
    not_controller["metadata"]["ownerReferences"][0]["controller"] = json!(false);
    let refusal = rollback_operations(&deployment(), &not_controller, 37).expect_err("refused");
    assert!(matches!(refusal, RollBackRefusal::ForeignReplicaSet));
}

#[test]
fn rollback_refuses_the_current_template_and_unreadable_objects() {
    let same = rollback_operations(&deployment(), &replica_set("dep-uid", "37", "api:2"), 37);
    assert!(matches!(same, Err(RollBackRefusal::SameTemplate)));
    let no_uid = rollback_operations(&json!({}), &replica_set("dep-uid", "37", "api:1"), 37);
    assert!(matches!(no_uid, Err(RollBackRefusal::Unreadable)));
    let no_template = json!({"metadata": replica_set("dep-uid", "37", "x")["metadata"]});
    assert!(matches!(
        rollback_operations(&deployment(), &no_template, 37),
        Err(RollBackRefusal::Unreadable)
    ));
}

fn cron_job() -> Value {
    json!({
        "metadata": {"uid": "cron-uid"},
        "spec": {"jobTemplate": {
            "metadata": {"labels": {"app": "n"}, "annotations": {"note": "keep"}},
            "spec": {"template": {"spec": {"restartPolicy": "Never"}}},
        }},
    })
}

#[test]
fn trigger_copies_the_template_and_owns_the_job_without_block_owner_deletion() {
    let body = trigger_job_body(&cron_job(), Some("shop"), "nightly").expect("a body");
    assert_eq!(body["metadata"]["generateName"], "nightly-manual-");
    assert_eq!(body["metadata"]["namespace"], "shop");
    assert_eq!(body["metadata"]["labels"], json!({"app": "n"}));
    assert_eq!(
        body["metadata"]["annotations"],
        json!({"note": "keep", "cronjob.kubernetes.io/instantiate": "manual"})
    );
    assert_eq!(body["metadata"]["ownerReferences"][0]["controller"], true);
    assert!(
        body["metadata"]["ownerReferences"][0]
            .get("blockOwnerDeletion")
            .is_none()
    );
    assert_eq!(body["spec"], cron_job()["spec"]["jobTemplate"]["spec"]);
}

#[test]
fn trigger_needs_a_uid_a_template_and_a_namespace() {
    let mut no_uid = cron_job();
    no_uid["metadata"] = json!({});
    assert!(trigger_job_body(&no_uid, Some("shop"), "nightly").is_none());
    assert!(trigger_job_body(&json!({"metadata": {"uid": "u"}}), Some("shop"), "n").is_none());
    assert!(trigger_job_body(&cron_job(), None, "nightly").is_none());
}

fn job() -> Value {
    let labels = json!({
        "app": "etl", "controller-uid": "c", "job-name": "etl-1",
        "batch.kubernetes.io/controller-uid": "c", "batch.kubernetes.io/job-name": "etl-1",
    });
    json!({
        "metadata": {"labels": labels, "annotations": {"a": "1"}, "ownerReferences": [{"uid": "x"}]},
        "spec": {
            "selector": {"matchLabels": {"controller-uid": "c"}}, "manualSelector": true,
            "suspend": true, "completions": 2,
            "template": {"metadata": {"labels": labels}},
        },
    })
}

#[test]
fn rerun_is_a_standalone_unsuspended_copy() {
    let body = rerun_job_body(&job(), Some("shop"), "etl-1").expect("a body");
    assert_eq!(body["metadata"]["generateName"], "etl-1-rerun-");
    assert_eq!(body["metadata"]["labels"], json!({"app": "etl"}));
    assert!(body["metadata"].get("ownerReferences").is_none());
    assert!(body["metadata"].get("annotations").is_none());
    assert_eq!(
        body["spec"]["template"]["metadata"]["labels"],
        json!({"app": "etl"})
    );
    assert_eq!(body["spec"]["suspend"], false);
    assert_eq!(body["spec"]["completions"], 2);
    assert!(body["spec"].get("selector").is_none());
    assert!(body["spec"].get("manualSelector").is_none());
}

#[test]
fn rerun_needs_a_spec_and_a_namespace() {
    assert!(rerun_job_body(&json!({"metadata": {}}), Some("shop"), "j").is_none());
    assert!(rerun_job_body(&job(), None, "j").is_none());
}

#[test]
fn set_image_patch_merges_the_container_by_name_and_writes_the_cause() {
    assert_eq!(
        set_image_patch("web", "nginx:1.26-alpine", Some("release test")),
        json!({
            "metadata": {"annotations": {"kubernetes.io/change-cause": "release test"}},
            "spec": {"template": {"spec": {"containers": [
                {"name": "web", "image": "nginx:1.26-alpine"},
            ]}}},
        })
    );
}

#[test]
fn set_image_patch_without_a_cause_removes_the_old_one() {
    let patch = set_image_patch("web", "nginx:1.26-alpine", None);
    assert_eq!(
        patch["metadata"]["annotations"]["kubernetes.io/change-cause"],
        Value::Null
    );
}

#[test]
fn a_change_cause_is_one_line_of_bounded_length() {
    assert!(is_valid_change_cause("release test"));
    assert!(!is_valid_change_cause(""));
    assert!(!is_valid_change_cause("two\nlines"));
    assert!(is_valid_change_cause(&"x".repeat(MAX_CHANGE_CAUSE_CHARS)));
    assert!(!is_valid_change_cause(
        &"x".repeat(MAX_CHANGE_CAUSE_CHARS + 1)
    ));
}
