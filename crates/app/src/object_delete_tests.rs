use std::time::Duration;

use cluster::{WriteMode, WriteOutcome};

use super::*;
use crate::workload_actions::workload_actions_tests::test_cluster;
use crate::write_guard::ActionRisk;

fn pod_key(name: &str) -> ResourceKey {
    ResourceKey::Pod {
        namespace: "payments".to_owned(),
        name: name.to_owned(),
    }
}

fn object(name: &str) -> ClusterObject {
    ClusterObject::new(test_cluster(), pod_key(name))
}

fn objects(count: usize) -> Vec<ClusterObject> {
    (0..count).map(|n| object(&format!("api-{n:02}"))).collect()
}

fn identity(uid: &str) -> ObjectIdentity {
    ObjectIdentity {
        uid: uid.to_owned(),
        finalizers: Vec::new(),
        deletion_started: None,
    }
}

fn target_of(kind: ObjectKind, name: &str, facts: TargetFacts) -> DeleteTarget {
    let namespace = kind.is_namespaced().then(|| "payments".to_owned());
    DeleteTarget {
        object: ObjectRef::new(kind, namespace, name.to_owned()).expect("the scope fits the kind"),
        identity: identity(&format!("uid-{name}")),
        facts,
    }
}

fn controller(kind: &str) -> ControllerRef {
    ControllerRef {
        kind: kind.to_owned(),
        name: format!("{}-owner", kind.to_ascii_lowercase()),
    }
}

fn pod(name: &str, has_controller: bool) -> DeleteTarget {
    let controller = has_controller.then(|| controller("ReplicaSet"));
    target_of(ObjectKind::Pod, name, TargetFacts::Pod { controller })
}

fn owned_pod(name: &str, owner: Option<&str>) -> DeleteTarget {
    let controller = owner.map(controller);
    target_of(ObjectKind::Pod, name, TargetFacts::Pod { controller })
}

fn extras(kind: ObjectKind, targets: Vec<DeleteTarget>) -> DeleteExtras {
    DeleteExtras {
        removal: Removal::Delete,
        propagation: DeletePropagation::Background,
        kind,
        targets,
        already_gone: Vec::new(),
    }
}

fn batch_of(kind: ObjectKind, targets: Vec<DeleteTarget>) -> BatchIntent {
    delete_batch(
        &test_cluster(),
        "prod-a",
        extras(kind, targets),
        jiff::Timestamp::UNIX_EPOCH,
    )
}

fn lines(lines: Vec<SharedString>) -> Vec<String> {
    lines.iter().map(ToString::to_string).collect()
}

fn outcome(effect: WriteEffect) -> WriteOutcome {
    WriteOutcome {
        mode: WriteMode::Commit,
        elapsed: Duration::ZERO,
        effect,
        created_name: None,
        dropped_fields: Vec::new(),
        uid: None,
    }
}

// ---- scope ----

#[test]
fn scope_is_the_row_when_unchecked() {
    let subject = object("api-00");
    assert_eq!(delete_scope(&subject, &[]), Ok(vec![subject.clone()]));
    // One ticked row is a single delete, not a bulk of one.
    assert_eq!(
        delete_scope(&subject, std::slice::from_ref(&subject)),
        Ok(vec![subject])
    );
}

#[test]
fn scope_is_the_checked_set_for_a_checked_row() {
    let checked = objects(3);
    assert_eq!(delete_scope(&checked[1], &checked), Ok(checked.clone()));
}

#[test]
fn scope_is_the_row_when_the_cursor_is_not_ticked() {
    let checked = objects(3);
    let elsewhere = object("elsewhere");
    assert_eq!(
        delete_scope(&elsewhere, &checked),
        Ok(vec![elsewhere.clone()])
    );
}

#[test]
fn scope_caps_at_50() {
    let at_cap = objects(MAX_BATCH_ITEMS);
    assert_eq!(delete_scope(&at_cap[0], &at_cap).map(|s| s.len()), Ok(50));
    let over = objects(MAX_BATCH_ITEMS + 1);
    assert_eq!(
        delete_scope(&over[0], &over),
        Err("Select at most 50 rows".into())
    );
}

#[test]
fn scope_refuses_two_clusters() {
    let mut checked = objects(2);
    checked[1].cluster.context = "other".to_owned();
    assert_eq!(
        delete_scope(&checked[0], &checked),
        Err("Select rows of one cluster".into())
    );
}

// ---- the batch ----

#[test]
fn delete_label_names_kind_and_count() {
    assert_eq!(
        batch_of(ObjectKind::Pod, vec![pod("a", true)]).label,
        "Delete pod"
    );
    let twelve: Vec<DeleteTarget> = (0..12).map(|n| pod(&format!("p-{n}"), true)).collect();
    assert_eq!(batch_of(ObjectKind::Pod, twelve).label, "Delete 12 pods");
    let two = vec![
        target_of(ObjectKind::Ingress, "a", TargetFacts::Plain),
        target_of(ObjectKind::Ingress, "b", TargetFacts::Plain),
    ];
    assert_eq!(
        batch_of(ObjectKind::Ingress, two).label,
        "Delete 2 ingresses"
    );
}

#[test]
fn items_carry_the_uid_and_the_propagation() {
    let targets = vec![pod("a", true), pod("b", true)];
    let items = delete_items(&targets, Removal::Delete, DeletePropagation::Orphan);
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].object, "payments/a");
    assert_eq!(items[0].label, "Delete pod payments/a");
    let fields = items[0].request.changed_fields();
    assert_eq!(fields[0].value.as_deref(), Some("Orphan"));
    assert_eq!(
        items[1].request.operation(),
        &WriteOperation::DeleteObject {
            uid: "uid-b".to_owned(),
            propagation: DeletePropagation::Orphan
        }
    );
}

#[test]
fn single_type_name_expects_the_object_name() {
    let single = batch_of(ObjectKind::Pod, vec![pod("api-x", true)]);
    assert_eq!(single.expected(), "api-x");
    assert_eq!(single.typed_hint(), "the pod name");
    let bulk = batch_of(ObjectKind::Pod, vec![pod("a", true), pod("b", true)]);
    assert_eq!(bulk.expected(), "prod-a");
    assert_eq!(bulk.typed_hint(), "the cluster name");
}

#[test]
fn delete_is_destructive_and_audited_as_delete() {
    let batch = batch_of(ObjectKind::Pod, vec![pod("a", true)]);
    assert_eq!(batch.risk, crate::write_guard::ActionRisk::Destructive);
    assert_eq!(batch.button, "Delete");
    assert_eq!(batch.action, ResourceAction::Delete(ObjectKind::Pod));
    assert!(batch.is_delete());
}

#[test]
fn confirm_label_counts_what_is_still_to_delete() {
    let single = batch_of(ObjectKind::Pod, vec![pod("a", true)]);
    assert_eq!(single.confirm_label(0), "Delete");
    let twelve: Vec<DeleteTarget> = (0..12).map(|n| pod(&format!("p-{n}"), true)).collect();
    let bulk = batch_of(ObjectKind::Pod, twelve);
    assert_eq!(bulk.confirm_label(0), "Delete 12 pods");
    assert_eq!(bulk.confirm_label(2), "Delete 10 of 12");
}

#[test]
fn a_propagation_change_rebuilds_the_items() {
    let batch = batch_of(
        ObjectKind::Deployment,
        vec![target_of(ObjectKind::Deployment, "api", TargetFacts::Plain)],
    );
    let rebuilt = with_propagation(
        &batch,
        DeletePropagation::Foreground,
        jiff::Timestamp::UNIX_EPOCH,
    )
    .expect("a delete rebuilds");
    let BatchExtras::Delete(extras) = &rebuilt.plan.extras else {
        panic!("a delete batch");
    };
    assert_eq!(extras.propagation, DeletePropagation::Foreground);
    assert_eq!(
        rebuilt.plan.items[0].request.changed_fields()[0]
            .value
            .as_deref(),
        Some("Foreground")
    );
    // The old batch is untouched.
    assert_eq!(
        batch.plan.items[0].request.changed_fields()[0]
            .value
            .as_deref(),
        Some("Background")
    );
}

#[test]
fn only_a_delete_rebuilds_with_a_propagation() {
    let mut batch = batch_of(ObjectKind::Pod, vec![pod("a", true)]);
    batch.plan.extras = BatchExtras::None;
    assert!(
        with_propagation(
            &batch,
            DeletePropagation::Orphan,
            jiff::Timestamp::UNIX_EPOCH
        )
        .is_none()
    );
}

// ---- warnings ----

#[test]
fn warnings_follow_the_kind_table() {
    let namespace = vec![target_of(
        ObjectKind::Namespace,
        "payments",
        TargetFacts::Plain,
    )];
    assert_eq!(
        lines(kind_warnings(ObjectKind::Namespace, &namespace)),
        ["Deletes every object in payments"]
    );
    let node = vec![target_of(ObjectKind::Node, "wk-04", TargetFacts::Plain)];
    assert_eq!(
        lines(kind_warnings(ObjectKind::Node, &node)),
        ["Removes the node object; its pods are not drained first"]
    );
    let set = vec![target_of(ObjectKind::StatefulSet, "db", TargetFacts::Plain)];
    assert_eq!(
        lines(kind_warnings(ObjectKind::StatefulSet, &set)),
        ["Volume claims stay unless the retention policy deletes them"]
    );
    let config = vec![target_of(ObjectKind::ConfigMap, "cfg", TargetFacts::Plain)];
    assert!(kind_warnings(ObjectKind::ConfigMap, &config).is_empty());
}

#[test]
fn pv_warning_follows_reclaim_policy() {
    let volume = |deletes_asset| {
        vec![target_of(
            ObjectKind::PersistentVolume,
            "pv-1",
            TargetFacts::Volume { deletes_asset },
        )]
    };
    assert_eq!(
        lines(kind_warnings(ObjectKind::PersistentVolume, &volume(true))),
        ["Reclaim policy Delete: the storage asset is deleted too"]
    );
    assert!(kind_warnings(ObjectKind::PersistentVolume, &volume(false)).is_empty());
}

#[test]
fn pvc_warning_uses_the_bound_volume() {
    let claim = |volume| {
        vec![target_of(
            ObjectKind::PersistentVolumeClaim,
            "data",
            TargetFacts::Claim(volume),
        )]
    };
    assert_eq!(
        lines(kind_warnings(
            ObjectKind::PersistentVolumeClaim,
            &claim(ClaimVolume::Deletes("pv-9".to_owned()))
        )),
        ["The PersistentVolume pv-9 is deleted too (reclaim policy Delete)"]
    );
    assert_eq!(
        lines(kind_warnings(
            ObjectKind::PersistentVolumeClaim,
            &claim(ClaimVolume::NotLoaded("pv-9".to_owned()))
        )),
        ["If the bound volume's reclaim policy is Delete, its data is deleted too"]
    );
    assert_eq!(
        lines(kind_warnings(
            ObjectKind::PersistentVolumeClaim,
            &claim(ClaimVolume::Keeps {
                volume: "pv-9".to_owned(),
                policy: "Retain".to_owned()
            })
        )),
        ["The PersistentVolume pv-9 is kept (reclaim policy Retain)"]
    );
    assert!(
        kind_warnings(
            ObjectKind::PersistentVolumeClaim,
            &claim(ClaimVolume::Unbound)
        )
        .is_empty()
    );
}

#[test]
fn uncontrolled_pod_warns() {
    assert_eq!(
        lines(kind_warnings(ObjectKind::Pod, &[pod("a", false)])),
        ["Not managed by a controller; it will not come back"]
    );
    assert!(kind_warnings(ObjectKind::Pod, &[pod("a", true)]).is_empty());
    let mixed = [pod("a", false), pod("b", true), pod("c", false)];
    assert_eq!(
        lines(kind_warnings(ObjectKind::Pod, &mixed)),
        ["2 pods are not managed by a controller and will not come back: payments/a, payments/c"]
    );
}

#[test]
fn finalizer_lines_before_the_delete() {
    let now = jiff::Timestamp::from_second(10_000).expect("a timestamp");
    let with = |finalizers: &[&str], started: Option<i64>| {
        let mut target = pod("a", true);
        target.identity.finalizers = finalizers.iter().map(ToString::to_string).collect();
        target.identity.deletion_started =
            started.map(|at| jiff::Timestamp::from_second(at).expect("a timestamp"));
        target
    };
    assert!(finalizer_lines(&[with(&[], None)], now).is_empty());
    assert_eq!(
        lines(finalizer_lines(&[with(&["foregroundDeletion"], None)], now)),
        ["Has finalizers: foregroundDeletion. Deletion waits until their controllers remove them"]
    );
    assert_eq!(
        lines(finalizer_lines(
            &[with(&["a", "b", "c", "d", "e"], None)],
            now
        )),
        ["Has finalizers: a, b, c +2. Deletion waits until their controllers remove them"]
    );
    assert_eq!(
        lines(finalizer_lines(&[with(&["x"], Some(9_000))], now)),
        [
            "Already being deleted for 16m; waiting for finalizers: x. Deleting again does not remove them"
        ]
    );
    assert_eq!(
        lines(finalizer_lines(&[with(&[], Some(9_940))], now)),
        ["Already terminating for 1m"]
    );
    // Bulk: counts only.
    let many = [
        with(&["x"], None),
        with(&["y"], None),
        with(&[], Some(9_000)),
    ];
    assert_eq!(
        lines(finalizer_lines(&many, now)),
        ["2 objects have finalizers", "1 are already being deleted"]
    );
}

#[test]
fn dependents_text_per_owner_kind() {
    let text = |kind, single| propagation_choices(kind, single)[0].2.clone();
    assert_eq!(
        text(ObjectKind::Deployment, true),
        "Its ReplicaSets and pods are deleted after the deployment"
    );
    assert_eq!(
        text(ObjectKind::StatefulSet, true),
        "Its pods are deleted after the statefulset"
    );
    assert_eq!(
        text(ObjectKind::CronJob, true),
        "Its Jobs and their pods are deleted after the cronjob"
    );
    assert_eq!(
        text(ObjectKind::Job, false),
        "Their pods are deleted after the jobs"
    );
    let choices = propagation_choices(ObjectKind::Deployment, true);
    assert_eq!(choices[0].0, DeletePropagation::Background);
    assert_eq!(choices[0].1, "Delete in the background (default)");
    assert_eq!(choices[1].0, DeletePropagation::Foreground);
    assert_eq!(
        choices[1].2,
        "The deployment stays until its ReplicaSets and pods are gone"
    );
    assert_eq!(choices[2].0, DeletePropagation::Orphan);
    assert_eq!(
        choices[2].2,
        "Its ReplicaSets and pods keep running without an owner"
    );
}

// ---- progress and notices ----

/// The one item of a single-object removal, as the notice reads it.
fn notice_item(removal: Removal, kind: ObjectKind, name: &str) -> BatchItem {
    let extras = DeleteExtras {
        removal,
        ..extras(kind, vec![target_of(kind, name, TargetFacts::Plain)])
    };
    let mut items = delete_batch(
        &test_cluster(),
        "prod-a",
        extras,
        jiff::Timestamp::UNIX_EPOCH,
    )
    .plan
    .items;
    items.remove(0)
}

fn notice(removal: Removal, kind: ObjectKind, progress: &ItemProgress) -> String {
    single_notice(&notice_item(removal, kind, "a"), progress, removal)
}

#[test]
fn pod_pending_without_finalizers_reads_grace_period() {
    let pending = ItemProgress::Pending(Vec::new());
    assert_eq!(
        notice(Removal::Delete, ObjectKind::Pod, &pending),
        "Delete pod payments/a: terminating (grace period)"
    );
    assert_eq!(
        notice(Removal::Delete, ObjectKind::Deployment, &pending),
        "Delete deployment payments/a: terminating"
    );
}

#[test]
fn single_notices() {
    let pod = |progress: &ItemProgress| notice(Removal::Delete, ObjectKind::Pod, progress);
    assert_eq!(pod(&ItemProgress::Done), "Delete pod payments/a: done");
    assert_eq!(
        pod(&ItemProgress::Pending(vec!["f1".to_owned()])),
        "Delete pod payments/a: marked for deletion; waiting for finalizers: f1"
    );
    assert_eq!(pod(&ItemProgress::Gone), "payments/a was already deleted");
    let conflict = ItemProgress::Failed(RECREATED_TEXT.into());
    assert_eq!(
        pod(&conflict),
        "Delete pod payments/a failed: A new object with this name exists; nothing was deleted"
    );
}

#[test]
fn commit_results_map_to_progress() {
    let mut stopped = None;
    let ok = |effect| Ok(outcome(effect));
    assert_eq!(
        delete_commit_progress(ok(WriteEffect::Deleted), &mut stopped),
        ItemProgress::Done
    );
    assert_eq!(
        delete_commit_progress(
            ok(WriteEffect::DeletionPending {
                finalizers: vec!["f".to_owned()]
            }),
            &mut stopped
        ),
        ItemProgress::Pending(vec!["f".to_owned()])
    );
    assert_eq!(
        delete_commit_progress(
            Err(CheckedWriteError::Write(WriteError::NotFound)),
            &mut stopped
        ),
        ItemProgress::Gone
    );
    let conflict = WriteError::Conflict {
        message: "x".to_owned(),
        managers: Vec::new(),
    };
    assert_eq!(
        delete_commit_progress(Err(CheckedWriteError::Write(conflict)), &mut stopped),
        ItemProgress::Failed(RECREATED_TEXT.into())
    );
    assert!(stopped.is_none());
    let blocked = CheckedWriteError::Blocked("stg-b was locked; nothing was changed".into());
    assert!(matches!(
        delete_commit_progress(Err(blocked), &mut stopped),
        ItemProgress::NotSent(_)
    ));
    assert!(stopped.is_some(), "a blocked commit stops the rest");
}

#[test]
fn dry_run_not_found_is_already_gone_and_a_uid_mismatch_says_so() {
    assert_eq!(
        delete_dry_run_progress(&Err(CheckedWriteError::Write(WriteError::NotFound))),
        ItemProgress::Gone
    );
    let conflict = WriteError::Conflict {
        message: "Precondition failed".to_owned(),
        managers: Vec::new(),
    };
    assert_eq!(
        delete_dry_run_progress(&Err(CheckedWriteError::Write(conflict))),
        ItemProgress::Rejected(RECREATED_TEXT.into())
    );
    assert_eq!(
        delete_dry_run_progress(&Ok(outcome(WriteEffect::Deleted))),
        ItemProgress::Passed
    );
}

#[test]
fn bulk_notice_counts() {
    let results = vec![
        ItemProgress::Done,
        ItemProgress::Done,
        ItemProgress::Pending(vec!["f".to_owned()]),
        ItemProgress::Pending(Vec::new()),
        ItemProgress::Gone,
        ItemProgress::Failed("boom".into()),
        ItemProgress::NotSent("stg-b was locked; nothing was changed".into()),
    ];
    assert_eq!(
        bulk_notice(&results),
        "Deleted 4 of 7, 1 waiting for finalizers, 1 already gone, 1 failed (boom), stopped: stg-b was locked; nothing was changed"
    );
    assert_eq!(
        bulk_notice(&[ItemProgress::Done, ItemProgress::Done]),
        "Deleted 2 of 2"
    );
}

#[test]
fn gone_notices() {
    assert_eq!(
        gone_notice(&["payments/a".into()]),
        "payments/a not found (already deleted or not served)"
    );
    assert_eq!(
        gone_notice(&["a".into(), "b".into(), "c".into()]),
        "None of the 3 objects was found (already deleted or not served)"
    );
}

#[test]
fn reading_notice_counts_the_objects() {
    assert_eq!(reading_text(1), "Reading 1 object…");
    assert_eq!(reading_text(12), "Reading 12 objects…");
}

#[test]
fn helm_release_rows_cannot_be_deleted() {
    let release_key = ResourceKey::Kind {
        kind: crate::resource_kind::ResourceKind::HelmReleases,
        namespace: Some("payments".to_owned()),
        name: "shop".to_owned(),
    };
    assert_eq!(delete_kind(&release_key), None);
    let secret_key = ResourceKey::Kind {
        kind: crate::resource_kind::ResourceKind::Secrets,
        namespace: Some("payments".to_owned()),
        name: "db".to_owned(),
    };
    assert_eq!(delete_kind(&secret_key), Some(ObjectKind::Secret));
}

#[test]
fn identity_failure_names_the_object() {
    let error = ClusterError::Forbidden {
        context: "prod".to_owned(),
        action: "reading the object before deleting it",
        message: "pods \"a\" is forbidden".to_owned(),
    };
    let text = identity_failure(&pod("a", true).object, &error);
    assert!(
        text.starts_with("Could not read a to pin its uid ("),
        "{text}"
    );
    assert!(text.ends_with("); nothing was deleted"), "{text}");
}

// ---- restart pod and evict (spec 0040) ----

fn removal_batch(removal: Removal, targets: Vec<DeleteTarget>) -> BatchIntent {
    let extras = DeleteExtras {
        removal,
        ..extras(ObjectKind::Pod, targets)
    };
    delete_batch(
        &test_cluster(),
        "prod-a",
        extras,
        jiff::Timestamp::UNIX_EPOCH,
    )
}

#[test]
fn removal_requests_are_uid_pinned() {
    let targets = vec![owned_pod("api-0", Some("ReplicaSet"))];
    let restart = removal_batch(Removal::Restart, targets.clone());
    assert_eq!(
        restart.plan.items[0].request.operation(),
        &WriteOperation::DeleteObject {
            uid: "uid-api-0".to_owned(),
            propagation: DeletePropagation::Background,
        }
    );
    let evict = removal_batch(Removal::Evict, targets);
    assert_eq!(
        evict.plan.items[0].request.operation(),
        &WriteOperation::EvictPod {
            uid: "uid-api-0".to_owned(),
            grace: GracePeriod::PodDefault,
        }
    );
}

#[test]
fn removal_batch_texts() {
    let targets = vec![owned_pod("api-0", Some("ReplicaSet"))];
    for (removal, action, title, verb, audit, item) in [
        (
            Removal::Restart,
            ResourceAction::RestartPod,
            "Restart pod",
            "Restart",
            "Restart pod",
            "Restart pod payments/api-0",
        ),
        (
            Removal::Evict,
            ResourceAction::EvictPod,
            "Evict pod",
            "Evict",
            "Evict",
            "Evict pod payments/api-0",
        ),
        (
            Removal::Delete,
            ResourceAction::Delete(ObjectKind::Pod),
            "Delete pod",
            "Delete",
            "Delete",
            "Delete pod payments/api-0",
        ),
    ] {
        let batch = removal_batch(removal, targets.clone());
        assert_eq!(batch.action, action);
        assert_eq!(batch.label, title);
        assert_eq!(batch.verb, verb);
        assert_eq!(batch.button, audit);
        assert_eq!(batch.plan.items[0].label, item);
        // The pod name is what a TypeName tier asks for.
        assert_eq!(batch.expected(), "api-0");
        assert_eq!(batch.risk, ActionRisk::Destructive);
    }
}

#[test]
fn removal_warnings_by_owner() {
    let warnings = |removal: Removal, owner: Option<&str>| {
        lines(removal_batch(removal, vec![owned_pod("web-0", owner)]).warnings)
    };
    let pdb = "Restart deletes the pod without checking PodDisruptionBudgets; Evict checks them";
    assert_eq!(warnings(Removal::Restart, Some("ReplicaSet")), [pdb]);
    assert_eq!(
        warnings(Removal::Restart, Some("StatefulSet")),
        [
            pdb,
            "The replacement keeps the name web-0 and its volume claims"
        ]
    );
    assert_eq!(
        warnings(Removal::Restart, Some("Job")),
        [
            pdb,
            "A Job may count the deleted pod as failed toward its backoffLimit"
        ]
    );
    // The bare-pod line of an eviction shows exactly once: `kind_warnings` runs for Delete only.
    assert_eq!(
        warnings(Removal::Evict, None),
        ["Not managed by a controller; it will not come back"]
    );
    assert_eq!(
        warnings(Removal::Evict, Some("DaemonSet")),
        ["A DaemonSet pod is recreated on the same node at once"]
    );
    assert!(warnings(Removal::Evict, Some("ReplicaSet")).is_empty());
    // A delete keeps its own table and gains none of these.
    assert_eq!(
        warnings(Removal::Delete, None),
        ["Not managed by a controller; it will not come back"]
    );
}

#[test]
fn removal_warnings_come_after_the_finalizer_lines() {
    let mut target = owned_pod("kafka-1", Some("StatefulSet"));
    target.identity.finalizers = vec!["f1".to_owned()];
    let batch = removal_batch(Removal::Restart, vec![target]);
    let text = lines(batch.warnings);
    assert!(text[0].starts_with("Has finalizers: f1"), "{text:?}");
    assert!(text[1].starts_with("Restart deletes the pod"), "{text:?}");
}

#[test]
fn removal_notices() {
    let restart = |progress: &ItemProgress| notice(Removal::Restart, ObjectKind::Pod, progress);
    let evict = |progress: &ItemProgress| notice(Removal::Evict, ObjectKind::Pod, progress);
    assert_eq!(
        restart(&ItemProgress::Done),
        "Restart pod payments/a: terminating; its controller creates a replacement"
    );
    assert_eq!(
        restart(&ItemProgress::Pending(Vec::new())),
        "Restart pod payments/a: terminating; its controller creates a replacement"
    );
    assert_eq!(
        evict(&ItemProgress::Done),
        "Evict pod payments/a: accepted; the pod is terminating"
    );
    // A gone pod reads from the item's object, whatever the label says.
    assert_eq!(
        restart(&ItemProgress::Gone),
        "payments/a was already deleted"
    );
    assert_eq!(evict(&ItemProgress::Gone), "payments/a was already deleted");
    let conflict = ItemProgress::Failed(RECREATED_TEXT.into());
    assert_eq!(
        evict(&conflict),
        "Evict pod payments/a failed: A new object with this name exists; nothing was deleted"
    );
    let refused = ItemProgress::Failed(
        "refused for now: The disruption budget api-pdb needs 2 healthy pods and has 2 currently"
            .into(),
    );
    assert_eq!(
        evict(&refused),
        "Evict pod payments/a failed: refused for now: The disruption budget api-pdb needs 2 healthy pods and has 2 currently"
    );
}

#[test]
fn a_propagation_change_keeps_the_removal() {
    let batch = removal_batch(Removal::Evict, vec![owned_pod("a", Some("ReplicaSet"))]);
    let rebuilt = with_propagation(
        &batch,
        DeletePropagation::Foreground,
        jiff::Timestamp::UNIX_EPOCH,
    )
    .expect("a delete batch");
    assert_eq!(rebuilt.action, ResourceAction::EvictPod);
    assert!(matches!(
        rebuilt.plan.items[0].request.operation(),
        WriteOperation::EvictPod { .. }
    ));
}

#[test]
fn the_warning_of_a_bulk_delete_names_the_pods_that_will_not_come_back() {
    let names = ["a", "b", "c", "d", "e", "f", "g"];
    let targets: Vec<DeleteTarget> = names.iter().map(|name| pod(name, false)).collect();
    assert_eq!(
        lines(kind_warnings(ObjectKind::Pod, &targets)),
        [
            "7 pods are not managed by a controller and will not come back: \
          payments/a, payments/b, payments/c, payments/d, payments/e, +2"
        ]
    );
}

#[test]
fn only_the_pods_of_a_bulk_delete_without_a_controller_get_the_tag() {
    let mixed = vec![pod("a", false), pod("b", true), pod("c", false)];
    let batch = batch_of(ObjectKind::Pod, mixed);
    assert_eq!(
        pods_without_controller(&batch),
        ["payments/a", "payments/c"]
    );
    // A lone pod is told by its warning line, a controller-owned set has nothing to tag.
    let single = batch_of(ObjectKind::Pod, vec![pod("a", false)]);
    assert!(pods_without_controller(&single).is_empty());
    let owned = batch_of(ObjectKind::Pod, vec![pod("a", true), pod("b", true)]);
    assert!(pods_without_controller(&owned).is_empty());
}

#[test]
fn a_volume_read_settles_a_claim_whose_list_was_not_loaded() {
    let not_loaded = || TargetFacts::Claim(ClaimVolume::NotLoaded("pv-9".to_owned()));
    assert_eq!(
        with_reclaim_policy(not_loaded(), Some("Delete".to_owned())),
        TargetFacts::Claim(ClaimVolume::Deletes("pv-9".to_owned()))
    );
    assert_eq!(
        with_reclaim_policy(not_loaded(), Some("Retain".to_owned())),
        TargetFacts::Claim(ClaimVolume::Keeps {
            volume: "pv-9".to_owned(),
            policy: "Retain".to_owned()
        })
    );
    // A failed read keeps the conditional line; any other fact is left alone.
    assert_eq!(with_reclaim_policy(not_loaded(), None), not_loaded());
    assert_eq!(
        with_reclaim_policy(TargetFacts::Plain, Some("Delete".to_owned())),
        TargetFacts::Plain
    );
}

fn binding_target(kind: ObjectKind, name: &str, subjects: &[&str]) -> DeleteTarget {
    target_of(
        kind,
        name,
        TargetFacts::Binding(BindingFacts {
            role_kind: "ClusterRole".to_owned(),
            role: "cluster-admin".to_owned(),
            subjects: subjects.iter().map(|text| (*text).to_owned()).collect(),
        }),
    )
}

#[test]
fn a_binding_delete_says_what_it_takes_away() {
    let target = binding_target(
        ObjectKind::ClusterRoleBinding,
        "lab-admin",
        &["ServiceAccount lab-batch/default"],
    );
    assert_eq!(
        lines(kind_warnings(ObjectKind::ClusterRoleBinding, &[target])),
        ["Removes cluster-admin from ServiceAccount lab-batch/default"]
    );
    let many = binding_target(
        ObjectKind::RoleBinding,
        "readers",
        &["User a", "User b", "Group c", "User d", "User e"],
    );
    assert_eq!(
        lines(kind_warnings(ObjectKind::RoleBinding, &[many])),
        ["Removes cluster-admin from User a, User b, Group c and 2 more"]
    );
    let none = binding_target(ObjectKind::RoleBinding, "empty", &[]);
    assert_eq!(
        lines(kind_warnings(ObjectKind::RoleBinding, &[none])),
        ["Removes the binding of cluster-admin; it has no subjects"]
    );
}

#[test]
fn a_bulk_binding_delete_names_each_binding_and_counts_the_rest() {
    let targets: Vec<DeleteTarget> = ["a", "b", "c", "d"]
        .iter()
        .map(|name| binding_target(ObjectKind::ClusterRoleBinding, name, &["User x"]))
        .collect();
    assert_eq!(
        lines(kind_warnings(ObjectKind::ClusterRoleBinding, &targets)),
        [
            "a: Removes cluster-admin from User x",
            "b: Removes cluster-admin from User x",
            "c: Removes cluster-admin from User x",
            "and 1 more bindings",
        ]
    );
}

#[test]
fn the_audit_line_of_a_deleted_binding_keeps_its_grant() {
    let target = binding_target(
        ObjectKind::ClusterRoleBinding,
        "lab-admin",
        &["ServiceAccount lab-batch/default", "User alice"],
    );
    let batch = batch_of(ObjectKind::ClusterRoleBinding, vec![target]);
    let BatchExtras::Delete(extras) = &batch.plan.extras else {
        panic!("a delete batch");
    };
    let fields = audit_fields_of(extras, &batch.plan.items[0]);
    let paths: Vec<(&str, Option<&str>)> = fields
        .iter()
        .map(|field| (field.path.as_str(), field.value.as_deref()))
        .collect();
    assert_eq!(
        paths,
        [
            ("roleRef", Some("ClusterRole/cluster-admin")),
            (
                "subjects",
                Some("ServiceAccount lab-batch/default, User alice")
            ),
        ]
    );
    // The item's own intent carries them to the audit line.
    let intent = batch.item_intent(&batch.plan.items[0]);
    assert_eq!(intent.audit_fields.len(), 2);
}

fn replica_set_target(name: &str, pods: u32, rollback: Option<(&str, &str)>) -> DeleteTarget {
    target_of(
        ObjectKind::ReplicaSet,
        name,
        TargetFacts::ReplicaSet(ReplicaSetFacts {
            pods,
            rollback: rollback.map(|(deployment, revision)| RollbackTarget {
                deployment: deployment.to_owned(),
                revision: revision.to_owned(),
            }),
        }),
    )
}

#[test]
fn deleting_a_rollback_target_says_the_deployment_loses_that_revision() {
    let old = replica_set_target("web-6f", 0, Some(("web", "21")));
    assert_eq!(
        lines(kind_warnings(ObjectKind::ReplicaSet, &[old])),
        ["Deployment web can no longer roll back to rev 21"]
    );
    let live = replica_set_target("web-7a", 3, None);
    assert!(kind_warnings(ObjectKind::ReplicaSet, &[live]).is_empty());
    let two = [
        replica_set_target("a", 0, Some(("web", "20"))),
        replica_set_target("b", 0, Some(("web", "21"))),
    ];
    assert_eq!(
        lines(kind_warnings(ObjectKind::ReplicaSet, &two)),
        [
            "2 of these are revisions kept for rollback: their Deployments can no longer roll back to them"
        ]
    );
}

#[test]
fn a_replica_set_without_pods_has_no_propagation_choice() {
    let batch = |targets| extras(ObjectKind::ReplicaSet, targets);
    assert!(!has_dependents(&batch(vec![replica_set_target(
        "old", 0, None
    )])));
    assert!(has_dependents(&batch(vec![replica_set_target(
        "live", 2, None
    )])));
    // One with pods among the rest keeps the choice.
    assert!(has_dependents(&batch(vec![
        replica_set_target("old", 0, None),
        replica_set_target("live", 2, None)
    ])));
    // Other owner kinds are unchanged.
    assert!(has_dependents(&extras(
        ObjectKind::Deployment,
        vec![target_of(ObjectKind::Deployment, "web", TargetFacts::Plain)]
    )));
    assert!(!has_dependents(&extras(
        ObjectKind::ConfigMap,
        vec![target_of(ObjectKind::ConfigMap, "cm", TargetFacts::Plain)]
    )));
}
