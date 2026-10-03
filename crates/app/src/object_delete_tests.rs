use std::time::Duration;

use cluster::{WriteMode, WriteOutcome};

use super::*;
use crate::workload_actions::workload_actions_tests::test_cluster;

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

fn pod(name: &str, has_controller: bool) -> DeleteTarget {
    target_of(ObjectKind::Pod, name, TargetFacts::Pod { has_controller })
}

fn extras(kind: ObjectKind, targets: Vec<DeleteTarget>) -> DeleteExtras {
    DeleteExtras {
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
    let items = delete_items(&targets, DeletePropagation::Orphan);
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
    assert_eq!(bulk.confirm_label(0), "Delete 12 of 12");
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
        ["The bound volume pv-9 has reclaim policy Delete: its data is deleted too"]
    );
    assert_eq!(
        lines(kind_warnings(
            ObjectKind::PersistentVolumeClaim,
            &claim(ClaimVolume::NotLoaded)
        )),
        ["If the bound volume's reclaim policy is Delete, its data is deleted too"]
    );
    assert!(
        kind_warnings(
            ObjectKind::PersistentVolumeClaim,
            &claim(ClaimVolume::Keeps)
        )
        .is_empty()
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
        ["2 pods are not managed by a controller"]
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

#[test]
fn pod_pending_without_finalizers_reads_grace_period() {
    let pending = ItemProgress::Pending(Vec::new());
    assert_eq!(
        single_notice("Delete pod payments/a", &pending, "Pod"),
        "Delete pod payments/a: terminating (grace period)"
    );
    assert_eq!(
        single_notice("Delete deployment payments/a", &pending, "Deployment"),
        "Delete deployment payments/a: terminating"
    );
}

#[test]
fn single_notices() {
    let label = "Delete pod payments/a";
    assert_eq!(
        single_notice(label, &ItemProgress::Done, "Pod"),
        "Delete pod payments/a: done"
    );
    assert_eq!(
        single_notice(label, &ItemProgress::Pending(vec!["f1".to_owned()]), "Pod"),
        "Delete pod payments/a: marked for deletion; waiting for finalizers: f1"
    );
    assert_eq!(
        single_notice(label, &ItemProgress::Gone, "Pod"),
        "pod payments/a was already deleted"
    );
    let conflict = ItemProgress::Failed(RECREATED_TEXT.into());
    assert_eq!(
        single_notice(label, &conflict, "Pod"),
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
        "payments/a was already deleted"
    );
    assert_eq!(
        gone_notice(&["a".into(), "b".into(), "c".into()]),
        "All 3 objects were already deleted"
    );
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
