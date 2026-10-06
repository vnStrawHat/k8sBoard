use std::path::PathBuf;

use cluster::{ObjectKind, ObjectRef, WriteError, WriteOperation, WriteOutcome};

use super::*;
use crate::workload_actions::workload_actions_tests::{deployment, test_cluster};

fn other_cluster() -> ClusterRef {
    ClusterRef {
        kubeconfig: PathBuf::from("/home/me/.kube/config"),
        context: "prod-ctx".to_owned(),
    }
}

fn objects(count: usize) -> Vec<KindObject> {
    (0..count)
        .map(|index| KindObject::Deployment(deployment(&format!("api-{index}"))))
        .collect()
}

/// Every row becomes a pause item: the plan, not the action, is under test.
fn item_of(row: &CheckedRow<'_>) -> Result<BatchItem, SkippedItem> {
    let KindObject::Deployment(deployment) = row.object else {
        return Err(SkippedItem {
            object: "?".into(),
            reason: "not a deployment".into(),
        });
    };
    let target = ObjectRef::new(
        ObjectKind::Deployment,
        Some(deployment.namespace.clone()),
        deployment.name.clone(),
    )
    .expect("a valid name");
    let request = WriteRequest::new(target, WriteOperation::SetRolloutPaused { paused: true })
        .expect("a deployment can pause");
    Ok(BatchItem {
        object: format!("{}/{}", deployment.namespace, deployment.name).into(),
        label: format!("Pause rollout of deployment {}", deployment.name).into(),
        request,
    })
}

fn rows<'a>(cluster: &'a ClusterRef, objects: &'a [KindObject]) -> Vec<CheckedRow<'a>> {
    objects
        .iter()
        .map(|object| CheckedRow { cluster, object })
        .collect()
}

#[test]
fn batch_requires_one_cluster() {
    let (stg, prod) = (test_cluster(), other_cluster());
    let objects = objects(2);
    let mixed = vec![
        CheckedRow {
            cluster: &stg,
            object: &objects[0],
        },
        CheckedRow {
            cluster: &prod,
            object: &objects[1],
        },
    ];
    assert_eq!(
        batch_plan(&mixed, item_of).err().as_deref(),
        Some("Select rows of one cluster")
    );
}

#[test]
fn batch_caps_at_fifty() {
    let cluster = test_cluster();
    let at_cap = objects(MAX_BATCH_ITEMS);
    let plan = batch_plan(&rows(&cluster, &at_cap), item_of).expect("fifty is allowed");
    assert_eq!(plan.items.len(), 50);
    assert_eq!(plan.cluster, cluster);
    let over = objects(MAX_BATCH_ITEMS + 1);
    assert_eq!(
        batch_plan(&rows(&cluster, &over), item_of).err().as_deref(),
        Some("Select at most 50 rows")
    );
}

#[test]
fn an_empty_selection_has_no_plan() {
    assert_eq!(
        batch_plan(&[], item_of).err().as_deref(),
        Some("Select rows first")
    );
}

#[test]
fn skipped_rows_are_listed_not_sent() {
    let cluster = test_cluster();
    let objects = objects(3);
    let checked = rows(&cluster, &objects);
    // The middle row is refused (a paused rollout, say): it is a line of the dialog, not a request.
    let plan = batch_plan(&checked, |row| {
        let item = item_of(row)?;
        if item.object.ends_with("api-1") {
            return Err(SkippedItem {
                object: item.object,
                reason: "Resume the rollout first".into(),
            });
        }
        Ok(item)
    })
    .expect("two items remain");
    let names: Vec<&str> = plan.items.iter().map(|item| item.object.as_ref()).collect();
    assert_eq!(names, ["team-a/api-0", "team-a/api-2"]);
    assert_eq!(plan.skipped.len(), 1);
    assert_eq!(plan.skipped[0].reason, "Resume the rollout first");
}

#[test]
fn every_row_skipped_gives_the_first_reason() {
    let cluster = test_cluster();
    let objects = objects(2);
    let reasons = ["already suspended", "something else"];
    let counter = std::cell::Cell::new(0);
    let plan = batch_plan(&rows(&cluster, &objects), |row| {
        let index = counter.replace(counter.get() + 1);
        let item = item_of(row)?;
        Err(SkippedItem {
            object: item.object,
            reason: reasons[index].into(),
        })
    });
    assert_eq!(plan.err().as_deref(), Some("already suspended"));
}

fn rejected(text: &str) -> ItemProgress {
    ItemProgress::Rejected(text.to_owned().into())
}

#[test]
fn the_dry_run_line_needs_every_item_to_pass() {
    let elapsed = Duration::from_millis(412);
    let passed = ItemProgress::Passed;
    assert_eq!(
        summarize_dry_runs(&[passed.clone(), passed.clone(), passed.clone()], elapsed),
        DryRunState::Passed { elapsed }
    );
    // One failure keeps Apply off and names the first reason.
    let states = [
        passed.clone(),
        rejected("Forbidden"),
        passed.clone(),
        rejected("later"),
    ];
    assert_eq!(
        summarize_dry_runs(&states, elapsed),
        DryRunState::Failed("Dry-run failed for 2 of 4: Forbidden".into())
    );
    // A lone object says it once: its row reads `failed`, the line carries the cause.
    assert_eq!(
        summarize_dry_runs(&[rejected("refused for now")], elapsed),
        DryRunState::Failed("Dry-run failed: refused for now".into())
    );
    // Items still waiting or checking keep the line running, even after a failure.
    assert_eq!(
        summarize_dry_runs(&[rejected("Forbidden"), ItemProgress::Checking], elapsed),
        DryRunState::Running
    );
    assert_eq!(
        summarize_dry_runs(&[passed, ItemProgress::Waiting], elapsed),
        DryRunState::Running
    );
}

#[test]
fn a_dry_run_that_stopped_says_why() {
    let blocked: Result<WriteOutcome, CheckedWriteError> =
        Err(CheckedWriteError::Blocked("stg-b is no longer open".into()));
    assert_eq!(
        dry_run_progress(&blocked),
        ItemProgress::Rejected("stg-b is no longer open".into())
    );
    let denied: Result<WriteOutcome, CheckedWriteError> =
        Err(CheckedWriteError::Write(WriteError::WritesBlocked));
    assert!(matches!(
        dry_run_progress(&denied),
        ItemProgress::Rejected(_)
    ));
}

fn done() -> Result<WriteOutcome, CheckedWriteError> {
    Ok(WriteOutcome {
        mode: cluster::WriteMode::Commit,
        elapsed: Duration::ZERO,
        effect: cluster::WriteEffect::Patched,
        created_name: None,
        uid: None,
        dropped_fields: Vec::new(),
    })
}

#[test]
fn batch_continues_after_a_failed_commit() {
    let mut stopped = None;
    let first = commit_progress(done(), &mut stopped);
    let second = commit_progress(
        Err(CheckedWriteError::Write(WriteError::OutcomeUnknown)),
        &mut stopped,
    );
    assert_eq!(first, ItemProgress::Done);
    assert_eq!(second, ItemProgress::Unknown);
    assert_eq!(stopped, None);
    let failed = commit_progress(
        Err(CheckedWriteError::Write(WriteError::WritesBlocked)),
        &mut stopped,
    );
    assert!(matches!(failed, ItemProgress::Failed(_)));
    // A failure is the object's: the next item is still free to go.
    assert_eq!(stopped, None);
}

#[test]
fn batch_stops_when_blocked() {
    let mut stopped = None;
    let blocked = commit_progress(
        Err(CheckedWriteError::Blocked(
            "stg-b was locked; nothing was changed".into(),
        )),
        &mut stopped,
    );
    assert_eq!(
        blocked,
        ItemProgress::NotSent("stg-b was locked; nothing was changed".into())
    );
    // The rest read the same reason: the loop sends nothing more.
    assert_eq!(
        stopped.as_deref(),
        Some("stg-b was locked; nothing was changed")
    );
}

#[test]
fn the_notice_counts_what_went_through() {
    let done = ItemProgress::Done;
    assert_eq!(
        batch_notice(
            "Restart",
            &[done.clone(), done.clone(), done.clone(), done.clone()]
        ),
        "Restart: 4 done"
    );
    let failed = ItemProgress::Failed("Forbidden".into());
    assert_eq!(
        batch_notice(
            "Restart",
            &[done.clone(), failed, done.clone(), done.clone()]
        ),
        "Restart: 3 done, 1 failed (Forbidden)"
    );
    let not_sent = ItemProgress::NotSent("stg-b was locked; nothing was changed".into());
    assert_eq!(
        batch_notice(
            "Scale",
            &[done, ItemProgress::Unknown, not_sent.clone(), not_sent]
        ),
        "Scale: 1 done, 1 unknown, 2 not sent (stg-b was locked; nothing was changed)"
    );
}

#[test]
fn an_item_intent_carries_the_batch_cluster_action_and_risk() {
    let cluster = test_cluster();
    let objects = objects(2);
    let plan = batch_plan(&rows(&cluster, &objects), item_of).expect("a plan");
    let batch = BatchIntent {
        cluster: cluster.clone(),
        cluster_name: "stg-b".into(),
        action: ResourceAction::PauseRollout,
        label: "Pause 2 deployments".into(),
        verb: "Pause".into(),
        button: "Pause".into(),
        risk: ActionRisk::Change,
        warnings: vec!["a line".into()],
        plan,
    };
    assert_eq!(batch.confirm_label(0), "Pause 2");
    assert_eq!(batch.expected(), "stg-b");
    let item = batch.item_intent(&batch.plan.items[1]);
    assert_eq!(item.cluster, cluster);
    assert_eq!(item.action, ResourceAction::PauseRollout);
    assert_eq!(item.label, "Pause rollout of deployment api-1");
    assert_eq!(item.button, "Pause");
    assert_eq!(item.request.target().name(), "api-1");
    assert_eq!(item.warnings.len(), 1);
}

#[test]
fn a_batch_of_one_types_its_object_and_a_larger_one_the_cluster() {
    let mut batch = ordered_batch();
    assert_eq!(batch.expected(), "stg-b");
    assert_eq!(batch.typed_hint(), "the cluster name");
    batch.plan.items.truncate(1);
    assert_eq!(batch.expected(), "api-0");
    assert_eq!(batch.typed_hint(), "the deployment name");
}

#[test]
fn item_states_read_as_the_list_shows_them() {
    assert_eq!(ItemProgress::Waiting.text(), "waiting");
    assert_eq!(ItemProgress::Checking.text(), "…");
    assert_eq!(ItemProgress::Passed.text(), "passed");
    assert_eq!(ItemProgress::Applying.text(), "applying…");
    assert_eq!(ItemProgress::Done.text(), "done");
    assert_eq!(ItemProgress::Unknown.text(), "outcome unknown");
    assert_eq!(
        ItemProgress::NotSent("an earlier step failed".into()).text(),
        "Not sent: an earlier step failed"
    );
}

// ---- BatchFailure (spec 0032b) ----

fn ordered_batch() -> BatchIntent {
    let cluster = test_cluster();
    let objects = objects(2);
    let mut plan = batch_plan(&rows(&cluster, &objects), item_of).expect("a plan");
    plan.on_failure = BatchFailure::Stop;
    BatchIntent {
        cluster,
        cluster_name: "stg-b".into(),
        action: ResourceAction::PauseRollout,
        label: "Make gp3 the default storage class".into(),
        verb: "Set default".into(),
        button: "Set default".into(),
        risk: ActionRisk::Change,
        warnings: Vec::new(),
        plan,
    }
}

fn refused() -> Result<WriteOutcome, CheckedWriteError> {
    Err(CheckedWriteError::Write(WriteError::Invalid {
        message: "boom".to_owned(),
        fields: Vec::new(),
    }))
}

#[test]
fn a_plan_that_continues_lets_the_next_item_go_after_a_failure() {
    let mut batch = ordered_batch();
    batch.plan.on_failure = BatchFailure::Continue;
    let mut stopped = None;
    let progress = batch.commit_progress(refused(), &mut stopped);
    assert!(matches!(progress, ItemProgress::Failed(_)));
    assert_eq!(stopped, None);
}

#[test]
fn an_ordered_plan_stops_after_a_failed_item() {
    let batch = ordered_batch();
    let mut stopped = None;
    let progress = batch.commit_progress(refused(), &mut stopped);
    assert!(matches!(progress, ItemProgress::Failed(_)));
    assert_eq!(stopped.as_deref(), Some("an earlier step failed"));
}

#[test]
fn an_ordered_plan_stops_after_an_unknown_outcome() {
    let batch = ordered_batch();
    let mut stopped = None;
    let result = Err(CheckedWriteError::Write(WriteError::OutcomeUnknown));
    assert_eq!(
        batch.commit_progress(result, &mut stopped),
        ItemProgress::Unknown
    );
    assert_eq!(stopped.as_deref(), Some("an earlier step failed"));
}

#[test]
fn a_blocked_item_keeps_its_own_reason_in_an_ordered_plan() {
    let batch = ordered_batch();
    let mut stopped = None;
    let result = Err(CheckedWriteError::Blocked("stg-b was locked".into()));
    batch.commit_progress(result, &mut stopped);
    assert_eq!(stopped.as_deref(), Some("stg-b was locked"));
}

#[test]
fn a_stopped_plan_says_how_far_it_got_and_why() {
    let results = [
        ItemProgress::Failed("the change is invalid: boom".into()),
        ItemProgress::NotSent("an earlier step failed".into()),
    ];
    assert_eq!(
        stop_notice("Make gp3 the default storage class", &results).as_deref(),
        Some(
            "Make gp3 the default storage class: stopped after 0 of 2: the change is invalid: boom"
        )
    );
    let partial = [ItemProgress::Done, ItemProgress::Failed("boom".into())];
    assert_eq!(
        stop_notice("Make gp3 the default storage class", &partial).as_deref(),
        Some("Make gp3 the default storage class: stopped after 1 of 2: boom")
    );
    let unknown = [ItemProgress::Done, ItemProgress::Unknown];
    assert_eq!(
        stop_notice("Label", &unknown).as_deref(),
        Some(
            "Label: stopped after 1 of 2: the outcome is unknown; the change may have been applied"
        )
    );
}

#[test]
fn a_plan_that_went_through_has_no_stop_notice() {
    let results = [ItemProgress::Done, ItemProgress::Done];
    assert_eq!(stop_notice("Label", &results), None);
    let batch = ordered_batch();
    assert_eq!(batch.notice(&results), "Set default: 2 done");
    assert!(batch.retry_subject(&results).is_none());
}

#[test]
fn only_a_plan_that_continues_uses_the_counting_notice() {
    let results = [ItemProgress::Done, ItemProgress::Failed("boom".into())];
    let mut batch = ordered_batch();
    assert_eq!(
        batch.notice(&results),
        "Make gp3 the default storage class: stopped after 1 of 2: boom"
    );
    batch.plan.on_failure = BatchFailure::Continue;
    assert_eq!(
        batch.notice(&results),
        "Set default: 1 done, 1 failed (boom). Not done: team-a/api-1"
    );
}

// ---- The result of a batch that did not go through entirely ----

fn plain_batch(count: usize) -> BatchIntent {
    let cluster = test_cluster();
    let objects = objects(count);
    let plan = batch_plan(&rows(&cluster, &objects), item_of).expect("a plan");
    BatchIntent {
        cluster,
        cluster_name: "stg-b".into(),
        action: ResourceAction::PauseRollout,
        label: format!("Pause {count} deployments").into(),
        verb: "Pause".into(),
        button: "Pause".into(),
        risk: ActionRisk::Change,
        warnings: Vec::new(),
        plan,
    }
}

fn failed() -> ItemProgress {
    ItemProgress::Failed("Forbidden".into())
}

#[test]
fn the_notice_names_up_to_five_objects_that_did_not_go_through() {
    let batch = plain_batch(8);
    let mut results = vec![ItemProgress::Done; 8];
    results[1] = failed();
    results[3] = ItemProgress::Unknown;
    let name = |index: usize| batch.plan.items[index].object.clone();
    assert_eq!(
        batch.notice(&results),
        format!(
            "Pause: 6 done, 1 failed, 1 unknown (Forbidden). Not done: {}, {}",
            name(1),
            name(3)
        )
    );
    let all_failed = vec![failed(); 8];
    let text = batch.notice(&all_failed);
    assert!(text.ends_with(&format!("{}, +3", name(4))), "{text}");
    assert!(!text.contains("api-5"), "{text}");
}

#[test]
fn a_clean_batch_and_a_batch_of_one_name_nothing() {
    let batch = plain_batch(3);
    assert_eq!(batch.notice(&vec![ItemProgress::Done; 3]), "Pause: 3 done");
    let single = plain_batch(1);
    assert_eq!(
        single.notice(&[failed()]),
        "Pause: 0 done, 1 failed (Forbidden)"
    );
}

#[test]
fn retry_failed_sends_the_failed_and_unsent_items_only() {
    let batch = plain_batch(5);
    let results = [
        ItemProgress::Done,
        failed(),
        ItemProgress::Unknown,
        ItemProgress::NotSent("stopped by user".into()),
        ItemProgress::NotSent("stopped by user".into()),
    ];
    let retry = batch.retry_batch(&results).expect("something to retry");
    let objects: Vec<&str> = retry.plan.items.iter().map(|i| i.object.as_ref()).collect();
    let expected: Vec<&str> = [1, 3, 4]
        .iter()
        .map(|index| batch.plan.items[*index].object.as_ref())
        .collect();
    assert_eq!(objects, expected);
    assert_eq!(retry.confirm_label(0), "Pause 3");
    assert_eq!(retry.action, batch.action);
    assert!(retry.plan.skipped.is_empty());
}

#[test]
fn nothing_is_retried_when_nothing_failed_or_the_plan_is_ordered() {
    let batch = plain_batch(2);
    assert!(batch.retry_batch(&vec![ItemProgress::Done; 2]).is_none());
    assert!(
        batch
            .retry_batch(&[ItemProgress::Done, ItemProgress::Unknown])
            .is_none(),
        "an unknown outcome may have been applied"
    );
    let ordered = ordered_batch();
    assert!(ordered.retry_batch(&[failed(), failed()]).is_none());
}

#[test]
fn a_restart_batch_says_requested_and_only_the_sent_deployments_are_watched() {
    let mut batch = plain_batch(3);
    batch.action = ResourceAction::RestartRollout(ObjectKind::Deployment);
    batch.verb = "Restart".into();
    assert!(batch.is_restart());
    let results = [ItemProgress::Done, failed(), ItemProgress::Done];
    assert_eq!(
        batch.notice(&[ItemProgress::Done, ItemProgress::Done, ItemProgress::Done]),
        "Restart: 3 requested"
    );
    assert_eq!(
        batch.watched_rollouts(&results),
        [
            ("team-a".to_owned(), "api-0".to_owned()),
            ("team-a".to_owned(), "api-2".to_owned())
        ]
    );
}

#[test]
fn other_batches_say_done_and_watch_nothing() {
    let batch = plain_batch(2);
    assert!(!batch.is_restart());
    assert!(
        batch
            .watched_rollouts(&vec![ItemProgress::Done; 2])
            .is_empty()
    );
    let mut stateful = plain_batch(2);
    stateful.action = ResourceAction::RestartRollout(ObjectKind::StatefulSet);
    assert!(
        stateful
            .watched_rollouts(&vec![ItemProgress::Done; 2])
            .is_empty()
    );
}

#[test]
fn a_bulk_label_edit_that_went_through_reads_as_what_it_did() {
    let cluster = test_cluster();
    let nodes: Vec<crate::node_edits::TickedNode> = ["wk-01", "wk-02"]
        .iter()
        .map(|name| crate::node_edits::TickedNode {
            name: (*name).to_owned(),
            scheduling: cluster::NodeScheduling::Enabled,
            labels: vec!["lab-batch=1".to_owned()],
        })
        .collect();
    let scope = crate::node_edits::NodeScope {
        cluster: &cluster,
        cluster_name: "lab",
    };
    let change = cluster::LabelChange {
        key: "lab-batch".to_owned(),
        value: None,
    };
    let batch = crate::node_edits::label_batch(&scope, &nodes, &[change], None).expect("a batch");
    let done = [ItemProgress::Done, ItemProgress::Done];
    assert_eq!(batch.notice(&done), "Removed lab-batch from 2 nodes");
    // A failure falls back to the counts, which say what went wrong.
    let partial = [ItemProgress::Done, ItemProgress::Failed("boom".into())];
    assert!(
        batch
            .notice(&partial)
            .starts_with("Edit labels: 1 done, 1 failed")
    );
    // The retry of the failed part is a smaller batch with the plain words.
    let retry = batch.retry_batch(&partial).expect("a retry");
    assert!(matches!(retry.plan.extras, BatchExtras::None));
    assert_eq!(retry.confirm_label(0), "Edit labels 1");
}
