use cluster::{ControllerRef, WriteEffect, WriteMode};

use super::*;
use crate::app_shell::write_flow::{DryRunState, TypedMatch, confirmed};

fn secs(seconds: u64) -> Duration {
    Duration::from_secs(seconds)
}

fn pod(name: &str) -> DrainPod {
    DrainPod {
        namespace: "payments".to_owned(),
        name: name.to_owned(),
        uid: format!("uid-{name}"),
        labels: Vec::new(),
        controller: Some(ControllerRef {
            kind: "ReplicaSet".to_owned(),
            name: "api".to_owned(),
        }),
        is_mirror: false,
        has_empty_dir: false,
        is_finished: false,
        is_pending: false,
        is_terminating: false,
    }
}

fn key(name: &str) -> PodKey {
    PodKey::of(&pod(name))
}

fn input(nodes: &[&str], to_cordon: &[&str]) -> RunInput {
    RunInput {
        nodes: nodes.iter().map(|node| (*node).to_owned()).collect(),
        to_cordon: to_cordon.iter().map(|node| (*node).to_owned()).collect(),
        options: DrainOptions::default(),
        confirmed: confirmed(&DryRunState::NotSupported, TypedMatch::NotNeeded, 1)
            .expect("a satisfied confirm step"),
        generation: 1,
        checked: HashSet::new(),
    }
}

/// A run over one node already cordoned whose pods were dry-run by the dialog.
fn run_over(pods: &[&str]) -> (DrainRun, Duration) {
    let mut run_input = input(&["wk-04"], &[]);
    run_input.checked = pods.iter().map(|name| format!("uid-{name}")).collect();
    let mut run = DrainRun::new(run_input);
    assert_eq!(run.next_step(secs(0)), NextStep::Read("wk-04".to_owned()));
    run.on_read(Ok(pods.iter().map(|name| pod(name)).collect()), secs(0));
    (run, secs(0))
}

fn ok() -> Result<WriteOutcome, CheckedWriteError> {
    Ok(WriteOutcome {
        mode: WriteMode::Commit,
        elapsed: Duration::from_millis(5),
        effect: WriteEffect::Created,
        created_name: None,
        uid: None,
    })
}

fn refused(after: Option<Duration>) -> Result<WriteOutcome, CheckedWriteError> {
    Err(CheckedWriteError::Write(WriteError::TooManyRequests {
        message: "needs 2 healthy pods".to_owned(),
        retry_after: after,
    }))
}

fn failed(error: WriteError) -> Result<WriteOutcome, CheckedWriteError> {
    Err(CheckedWriteError::Write(error))
}

fn blocked(text: &str) -> Result<WriteOutcome, CheckedWriteError> {
    Err(CheckedWriteError::Blocked(text.into()))
}

fn progress_of(run: &DrainRun, name: &str) -> PodProgress {
    run.nodes[run.current]
        .pods
        .iter()
        .find(|pod| pod.key.name == name)
        .map(|pod| pod.progress.clone())
        .expect("the pod is in the run")
}

#[test]
fn retry_delay_table() {
    let none: Vec<u64> = (1..=6).map(|n| retry_delay(n, None).as_secs()).collect();
    assert_eq!(none, [5, 10, 20, 30, 30, 30]);
    let hinted: Vec<u64> = (1..=4)
        .map(|n| retry_delay(n, Some(secs(10))).as_secs())
        .collect();
    assert_eq!(hinted, [10, 10, 20, 30]);
    // A hint above the cap is capped too, and a huge attempt count does not overflow.
    assert_eq!(retry_delay(1, Some(secs(120))), secs(30));
    assert_eq!(retry_delay(u32::MAX, None), secs(30));
}

#[test]
fn refused_pod_retries_after_backoff() {
    let (mut run, now) = run_over(&["api-1"]);
    let evict = NextStep::Evict(key("api-1"));
    assert_eq!(run.next_step(now), evict);
    run.on_write(&evict, refused(Some(secs(10))), now);
    // Nothing to do until the retry is due: sleep exactly that long.
    assert_eq!(run.next_step(now), NextStep::Sleep(secs(10)));
    assert_eq!(run.next_step(secs(9)), NextStep::Sleep(secs(1)));
    assert_eq!(run.next_step(secs(10)), evict);
    run.on_write(&evict, ok(), secs(10));
    assert_eq!(progress_of(&run, "api-1"), PodProgress::Evicted);
}

#[test]
fn a_refusal_without_a_hint_uses_the_backoff_alone_and_counts_attempts() {
    let (mut run, _) = run_over(&["api-1"]);
    let evict = NextStep::Evict(key("api-1"));
    run.on_write(&evict, refused(None), secs(0));
    assert!(matches!(
        progress_of(&run, "api-1"),
        PodProgress::Refused { attempt: 1, retry_at, .. } if retry_at == secs(5)
    ));
    run.on_write(&evict, refused(None), secs(5));
    assert!(matches!(
        progress_of(&run, "api-1"),
        PodProgress::Refused { attempt: 2, retry_at, .. } if retry_at == secs(15)
    ));
}

#[test]
fn poll_marks_gone_by_uid() {
    let (mut run, _) = run_over(&["api-1", "api-2"]);
    let first = NextStep::Evict(key("api-1"));
    run.on_write(&first, ok(), secs(0));
    run.on_write(&NextStep::Evict(key("api-2")), ok(), secs(0));
    // Still listed: not gone.
    run.on_poll(Ok(vec![pod("api-1"), pod("api-2")]), secs(3));
    assert_eq!(progress_of(&run, "api-1"), PodProgress::Evicted);
    // api-1 is absent; api-2 came back under the same name with another uid.
    let mut replacement = pod("api-2");
    replacement.uid = "uid-new".to_owned();
    run.on_poll(Ok(vec![replacement]), secs(6));
    assert_eq!(progress_of(&run, "api-1"), PodProgress::Gone);
    assert_eq!(progress_of(&run, "api-2"), PodProgress::Gone);
}

#[test]
fn not_found_and_uid_conflict_are_gone() {
    let (mut run, _) = run_over(&["api-1", "api-2"]);
    run.on_write(
        &NextStep::Evict(key("api-1")),
        failed(WriteError::NotFound),
        secs(0),
    );
    let conflict = WriteError::Conflict {
        message: "uid".to_owned(),
        managers: Vec::new(),
    };
    run.on_write(&NextStep::Evict(key("api-2")), failed(conflict), secs(0));
    assert_eq!(progress_of(&run, "api-1"), PodProgress::Gone);
    assert_eq!(progress_of(&run, "api-2"), PodProgress::Gone);
    assert_eq!(
        run.next_step(secs(0)),
        NextStep::NodeDone(NodeOutcome::Drained)
    );
}

#[test]
fn unknown_outcome_is_retried() {
    let (mut run, _) = run_over(&["api-1"]);
    let evict = NextStep::Evict(key("api-1"));
    run.on_write(&evict, failed(WriteError::OutcomeUnknown), secs(0));
    assert!(matches!(
        progress_of(&run, "api-1"),
        PodProgress::Refused { message, retry_at, .. }
            if message == "No answer; retrying" && retry_at == secs(5)
    ));
    assert_eq!(run.next_step(secs(5)), evict);
}

#[test]
fn timeout_makes_the_node_stuck() {
    let (mut run, _) = run_over(&["api-1", "api-2"]);
    run.on_write(&NextStep::Evict(key("api-1")), refused(None), secs(0));
    run.on_write(&NextStep::Evict(key("api-2")), refused(None), secs(0));
    // The default timeout is 5 minutes.
    let step = run.next_step(secs(300));
    assert_eq!(
        step,
        NextStep::NodeDone(NodeOutcome::Stuck {
            reason: "Timed out after 5m: 2 pods left".into()
        })
    );
    // No further Evict is ever asked of a timed out node, however due the retries are.
    assert!(!matches!(run.next_step(secs(400)), NextStep::Evict(_)));
}

#[test]
fn timeout_is_per_node() {
    let mut run = DrainRun::new(input(&["a", "b"], &[]));
    run.on_read(Ok(vec![pod("one")]), secs(0));
    run.on_write(&NextStep::DryRun(key("one")), ok(), secs(0));
    run.on_write(&NextStep::Evict(key("one")), ok(), secs(0));
    // Node a takes 4 of its 5 minutes.
    run.on_poll(Ok(Vec::new()), secs(240));
    assert_eq!(
        run.next_step(secs(240)),
        NextStep::NodeDone(NodeOutcome::Drained)
    );
    run.on_node_done(NodeOutcome::Drained);
    assert_eq!(run.next_step(secs(240)), NextStep::Read("b".to_owned()));
    run.on_read(Ok(vec![pod("two")]), secs(240));
    run.on_write(&NextStep::DryRun(key("two")), ok(), secs(240));
    run.on_write(&NextStep::Evict(key("two")), refused(None), secs(240));
    // Node b still has its own 5 minutes: 4 minutes in, it is not stuck.
    assert!(!matches!(run.next_step(secs(480)), NextStep::NodeDone(_)));
    assert!(matches!(
        run.next_step(secs(540)),
        NextStep::NodeDone(NodeOutcome::Stuck { .. })
    ));
}

#[test]
fn poll_runs_every_three_seconds() {
    let (mut run, _) = run_over(&["api-1"]);
    run.on_write(&NextStep::Evict(key("api-1")), ok(), secs(0));
    assert_eq!(run.next_step(secs(0)), NextStep::Poll);
    run.on_poll(Ok(vec![pod("api-1")]), secs(0));
    assert_eq!(run.next_step(secs(1)), NextStep::Sleep(secs(2)));
    assert_eq!(run.next_step(secs(2)), NextStep::Sleep(secs(1)));
    assert_eq!(run.next_step(secs(3)), NextStep::Poll);
    run.on_poll(Ok(vec![pod("api-1")]), secs(3));
    assert_eq!(run.next_step(secs(5)), NextStep::Sleep(secs(1)));
}

#[test]
fn a_failed_poll_is_shown_and_retried_at_the_next_poll() {
    let (mut run, _) = run_over(&["api-1"]);
    run.on_write(&NextStep::Evict(key("api-1")), ok(), secs(0));
    run.on_poll(Err("connection reset".into()), secs(0));
    assert_eq!(
        run.poll_error().map(|text| text.as_ref()),
        Some("connection reset")
    );
    assert!(run.is_running());
    assert_eq!(run.next_step(secs(3)), NextStep::Poll);
    run.on_poll(Ok(Vec::new()), secs(3));
    assert_eq!(run.poll_error(), None);
}

#[test]
fn failed_pod_does_not_stop_the_others() {
    let (mut run, _) = run_over(&["api-1", "api-2"]);
    let denied = failed(WriteError::Denied {
        message: "pods/eviction is forbidden".to_owned(),
    });
    run.on_write(&NextStep::Evict(key("api-1")), denied, secs(0));
    assert!(matches!(progress_of(&run, "api-1"), PodProgress::Failed(_)));
    // The next pod is still evicted.
    assert_eq!(run.next_step(secs(0)), NextStep::Evict(key("api-2")));
    run.on_write(&NextStep::Evict(key("api-2")), ok(), secs(0));
    run.on_poll(Ok(Vec::new()), secs(3));
    // Everything else is done, so the node ends stuck with the first failure.
    match run.next_step(secs(3)) {
        NextStep::NodeDone(NodeOutcome::Stuck { reason }) => {
            assert!(reason.contains("forbidden"), "{reason}");
        }
        other => panic!("expected a stuck node, got {other:?}"),
    }
}

#[test]
fn cancel_sends_nothing_more() {
    let mut run = DrainRun::new(input(&["a", "b"], &["a", "b"]));
    assert_eq!(run.next_step(secs(0)), NextStep::Cordon("a".to_owned()));
    run.on_write(&NextStep::Cordon("a".to_owned()), ok(), secs(0));
    run.cancel();
    for now in [0, 1, 60, 600] {
        assert_eq!(run.next_step(secs(now)), NextStep::Finished);
    }
    assert!(!run.is_running());
    // Nothing was uncordoned: the node a cordoned stays in the list the tab offers.
    assert_eq!(run.cordoned(), ["a".to_owned()]);
    assert_eq!(run.end(), Some(&RunEnd::Cancelled));
}

#[test]
fn in_flight_result_is_applied_after_cancel() {
    let (mut run, _) = run_over(&["api-1"]);
    let evict = NextStep::Evict(key("api-1"));
    run.cancel();
    run.on_write(&evict, ok(), secs(1));
    assert_eq!(progress_of(&run, "api-1"), PodProgress::Evicted);
    assert_eq!(run.next_step(secs(2)), NextStep::Finished);
    // And the first Cancel's reason is not replaced by a later stop.
    run.stop("later");
    assert_eq!(run.end(), Some(&RunEnd::Cancelled));
}

#[test]
fn blocked_write_stops_the_run() {
    let (mut run, _) = run_over(&["api-1"]);
    run.on_write(
        &NextStep::Evict(key("api-1")),
        blocked("prod-a was locked; nothing was changed"),
        secs(1),
    );
    assert_eq!(
        run.end(),
        Some(&RunEnd::Stopped(
            "prod-a was locked; nothing was changed; drain stopped".into()
        ))
    );
    assert_eq!(run.next_step(secs(2)), NextStep::Finished);
}

#[test]
fn multi_node_cordons_all_first() {
    let mut run = DrainRun::new(input(&["a", "b"], &["a", "b"]));
    assert_eq!(run.next_step(secs(0)), NextStep::Cordon("a".to_owned()));
    run.on_write(&NextStep::Cordon("a".to_owned()), ok(), secs(0));
    assert_eq!(run.next_step(secs(0)), NextStep::Cordon("b".to_owned()));
    run.on_write(&NextStep::Cordon("b".to_owned()), ok(), secs(0));
    // Only then does the first node start, so evicted pods never land on the next node.
    assert_eq!(run.next_step(secs(0)), NextStep::Read("a".to_owned()));
    assert_eq!(run.cordoned(), ["a".to_owned(), "b".to_owned()]);
}

#[test]
fn an_already_cordoned_node_is_not_cordoned_again() {
    let mut run = DrainRun::new(input(&["a", "b"], &["b"]));
    assert_eq!(run.next_step(secs(0)), NextStep::Cordon("b".to_owned()));
    run.on_write(&NextStep::Cordon("b".to_owned()), ok(), secs(0));
    assert_eq!(run.next_step(secs(0)), NextStep::Read("a".to_owned()));
}

#[test]
fn cordon_failure_stops_the_run() {
    let mut run = DrainRun::new(input(&["a", "b"], &["a", "b"]));
    run.on_write(&NextStep::Cordon("a".to_owned()), ok(), secs(0));
    let denied = failed(WriteError::Denied {
        message: "nodes is forbidden".to_owned(),
    });
    run.on_write(&NextStep::Cordon("b".to_owned()), denied, secs(0));
    match run.end() {
        Some(RunEnd::Stopped(text)) => {
            assert!(text.starts_with("Could not cordon b: "), "{text}");
        }
        other => panic!("expected a stopped run, got {other:?}"),
    }
    // a stays cordoned; nothing is evicted.
    assert_eq!(run.cordoned(), ["a".to_owned()]);
    assert_eq!(run.next_step(secs(1)), NextStep::Finished);
    assert!(run.take_summaries().is_empty(), "no node was reached");
}

#[test]
fn stuck_node_stops_later_nodes() {
    let mut run = DrainRun::new(input(&["a", "b"], &[]));
    run.on_read(Ok(vec![pod("one")]), secs(0));
    run.on_write(&NextStep::DryRun(key("one")), ok(), secs(0));
    run.on_write(&NextStep::Evict(key("one")), refused(None), secs(0));
    let stuck = NodeOutcome::Stuck {
        reason: "Timed out after 5m: 1 pod left".into(),
    };
    run.on_node_done(stuck);
    assert_eq!(run.next_step(secs(301)), NextStep::Finished);
    assert_eq!(
        run.end_notice().as_deref(),
        Some("Drain stopped: a stuck (Timed out after 5m: 1 pod left)")
    );
    // Node b was never read, so it was never evicted from.
    assert!(run.nodes[1].pods.is_empty());
}

#[test]
fn new_pod_gets_a_dry_run_first() {
    // The dialog dry-ran api-1 only; api-2 arrived since.
    let mut run_input = input(&["wk-04"], &[]);
    run_input.checked = ["uid-api-1".to_owned()].into_iter().collect();
    let mut run = DrainRun::new(run_input);
    run.on_read(Ok(vec![pod("api-1"), pod("api-2")]), secs(0));
    assert_eq!(run.next_step(secs(0)), NextStep::DryRun(key("api-2")));
    // A 429 on the dry-run is recorded and the pod stays pending, to be evicted and retried.
    run.on_write(&NextStep::DryRun(key("api-2")), refused(None), secs(0));
    assert_eq!(progress_of(&run, "api-2"), PodProgress::Pending);
    assert_eq!(run.next_step(secs(0)), NextStep::Evict(key("api-1")));
    run.on_write(&NextStep::Evict(key("api-1")), ok(), secs(0));
    assert_eq!(run.next_step(secs(0)), NextStep::Evict(key("api-2")));
}

#[test]
fn a_dry_run_that_fails_otherwise_fails_the_pod() {
    let mut run = DrainRun::new(input(&["wk-04"], &[]));
    run.on_read(Ok(vec![pod("api-2")]), secs(0));
    let invalid = failed(WriteError::Invalid {
        message: "bad".to_owned(),
        fields: Vec::new(),
    });
    run.on_write(&NextStep::DryRun(key("api-2")), invalid, secs(0));
    assert!(matches!(progress_of(&run, "api-2"), PodProgress::Failed(_)));
}

#[test]
fn classification_at_node_start_follows_the_dialog_options() {
    let mut daemon = pod("agent");
    daemon.controller = Some(ControllerRef {
        kind: "DaemonSet".to_owned(),
        name: "agent".to_owned(),
    });
    let mut mirror = pod("etcd");
    mirror.is_mirror = true;
    let mut going = pod("going");
    going.is_terminating = true;
    let mut scratch = pod("scratch");
    scratch.has_empty_dir = true;
    let mut run = DrainRun::new(input(&["wk-04"], &[]));
    run.on_read(
        Ok(vec![daemon, mirror, going, scratch, pod("api-1")]),
        secs(0),
    );
    assert_eq!(
        progress_of(&run, "agent"),
        PodProgress::Skipped(SkipReason::DaemonSet)
    );
    assert_eq!(
        progress_of(&run, "etcd"),
        PodProgress::Skipped(SkipReason::Mirror)
    );
    assert_eq!(progress_of(&run, "going"), PodProgress::Awaited);
    // Needs an option the user did not tick: failed up front.
    assert_eq!(
        progress_of(&run, "scratch"),
        PodProgress::Failed("Not evicted: needs Delete emptyDir data".into())
    );
    assert_eq!(progress_of(&run, "api-1"), PodProgress::Pending);
}

#[test]
fn an_unreadable_node_ends_stuck_with_the_error() {
    let mut run = DrainRun::new(input(&["a", "b"], &[]));
    run.on_read(Err("Could not list pods on a: boom".into()), secs(0));
    assert_eq!(run.next_step(secs(1)), NextStep::Finished);
    let lines = run.take_summaries();
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].outcome, SummaryOutcome::Stuck);
}

#[test]
fn node_end_writes_one_summary_line() {
    // Drained: one line with the counts; a second call writes nothing more.
    let mut run = DrainRun::new(input(&["a", "b"], &[]));
    let mut daemon = pod("agent");
    daemon.controller = Some(ControllerRef {
        kind: "DaemonSet".to_owned(),
        name: "agent".to_owned(),
    });
    run.on_read(Ok(vec![pod("one"), pod("two"), daemon]), secs(0));
    for name in ["one", "two"] {
        run.on_write(&NextStep::DryRun(key(name)), ok(), secs(0));
        run.on_write(&NextStep::Evict(key(name)), ok(), secs(0));
    }
    run.on_poll(Ok(Vec::new()), secs(3));
    assert_eq!(
        run.next_step(secs(3)),
        NextStep::NodeDone(NodeOutcome::Drained)
    );
    run.on_node_done(NodeOutcome::Drained);
    let lines = run.take_summaries();
    assert_eq!(
        lines,
        [NodeSummary {
            node: "a".to_owned(),
            evicted: 2,
            refused: 0,
            failed: 0,
            skipped: 1,
            unknown: 0,
            outcome: SummaryOutcome::Drained,
            reason: None,
        }]
    );
    assert!(run.take_summaries().is_empty());
    // Node b starts and is then cancelled: one cancelled line; a is not repeated.
    run.on_read(Ok(vec![pod("three")]), secs(10));
    run.cancel();
    let lines = run.take_summaries();
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].node, "b");
    assert_eq!(lines[0].outcome, SummaryOutcome::Cancelled);
}

#[test]
fn stuck_and_stopped_nodes_get_their_own_outcomes() {
    let (mut run, _) = run_over(&["api-1"]);
    run.on_write(&NextStep::Evict(key("api-1")), refused(None), secs(0));
    run.on_node_done(NodeOutcome::Stuck {
        reason: "Timed out".into(),
    });
    let lines = run.take_summaries();
    assert_eq!(
        (lines[0].outcome.clone(), lines[0].refused),
        (SummaryOutcome::Stuck, 1)
    );
    let (mut stopped, _) = run_over(&["api-1"]);
    stopped.stop("k8sBoard is closing");
    let lines = stopped.take_summaries();
    assert_eq!(lines[0].outcome, SummaryOutcome::Stopped);
}

#[test]
fn a_node_never_reached_writes_no_line() {
    let mut run = DrainRun::new(input(&["a", "b"], &["a", "b"]));
    run.cancel();
    assert!(run.take_summaries().is_empty());
}

#[test]
fn the_tab_reads_states_progress_and_rows() {
    let mut run = DrainRun::new(input(&["a", "b"], &["a", "b"]));
    let states = run.node_states();
    assert_eq!(states[0].1, NodeState::Cordoning);
    run.on_write(&NextStep::Cordon("a".to_owned()), ok(), secs(0));
    run.on_write(&NextStep::Cordon("b".to_owned()), ok(), secs(0));
    assert_eq!(
        run.node_states()[0].1,
        NodeState::Evicting { gone: 0, total: 0 }
    );
    assert_eq!(run.node_states()[1].1, NodeState::Waiting);
    run.on_read(Ok(vec![pod("one"), pod("two"), pod("three")]), secs(0));
    run.on_write(&NextStep::DryRun(key("one")), ok(), secs(0));
    run.on_write(&NextStep::Evict(key("one")), ok(), secs(0));
    run.on_write(&NextStep::DryRun(key("two")), ok(), secs(0));
    run.on_write(
        &NextStep::Evict(key("two")),
        refused(Some(secs(10))),
        secs(2),
    );
    run.on_poll(Ok(vec![pod("two"), pod("three")]), secs(3));
    assert_eq!(run.progress(), (1, 3));
    assert_eq!(run.node_states()[0].1.text(), "Evicting 1/3");
    assert_eq!(run.timeout_left(secs(60)), Some(secs(240)));
    let rows = run.pod_rows(secs(4));
    // The warning row comes first, with its countdown and attempt.
    assert_eq!(rows[0].pod, "payments/two");
    assert_eq!(
        rows[0].text,
        "Refused by PDB: needs 2 healthy pods · retry in 8 s (attempt 1)"
    );
    assert_eq!(rows[0].tone, StatusTone::Warn);
    let texts: Vec<&str> = rows.iter().map(|row| row.text.as_ref()).collect();
    assert_eq!(texts[1..], ["Gone", "Waiting"]);
    run.cancel();
    assert_eq!(run.node_states()[0].1, NodeState::Cancelled);
    assert_eq!(run.timeout_left(secs(60)), None);
}

#[test]
fn the_end_notice_names_the_outcome() {
    let (mut run, _) = run_over(&[]);
    run.on_node_done(NodeOutcome::Drained);
    assert_eq!(run.end_notice().as_deref(), Some("Drain: wk-04 drained"));
    let mut several = DrainRun::new(input(&["a", "b", "c"], &[]));
    several.cancel();
    assert_eq!(several.end_notice().as_deref(), Some("Drain cancelled"));
    let mut stopped = DrainRun::new(input(&["a"], &[]));
    stopped.stop("prod-a is no longer open");
    assert_eq!(
        stopped.end_notice().as_deref(),
        Some("Drain stopped: prod-a is no longer open")
    );
    assert_eq!(DrainRun::new(input(&["a"], &[])).end_notice(), None);
}

#[test]
fn the_end_notice_is_handed_out_once() {
    let mut run = DrainRun::new(input(&["a"], &[]));
    assert_eq!(run.take_end_notice(), None, "the run is still going");
    run.cancel();
    assert_eq!(run.take_end_notice().as_deref(), Some("Drain cancelled"));
    assert_eq!(run.take_end_notice(), None);
}

#[test]
fn the_status_line_follows_the_run() {
    let mut run = DrainRun::new(input(&["wk-04", "wk-05"], &["wk-04", "wk-05"]));
    assert_eq!(run.status_text(secs(0)), "Cordoning wk-04, wk-05…");
    run.on_write(&NextStep::Cordon("wk-04".to_owned()), ok(), secs(0));
    run.on_write(&NextStep::Cordon("wk-05".to_owned()), ok(), secs(0));
    assert_eq!(
        run.status_text(secs(0)),
        "Draining wk-04 (1/2) · Reading pods…"
    );
    run.on_read(Ok(vec![pod("one")]), secs(0));
    assert_eq!(
        run.status_text(secs(108)),
        "Draining wk-04 (1/2) · Timeout in 3:12"
    );
    run.cancel();
    assert_eq!(
        run.status_text(secs(110)),
        "Cancelled · 0 of 1 evicted on wk-04 · cordoned: wk-04, wk-05"
    );
}

#[test]
fn the_status_line_reports_stuck_and_stopped_runs() {
    let mut stuck = DrainRun::new(input(&["wk-04"], &["wk-04"]));
    stuck.on_write(&NextStep::Cordon("wk-04".to_owned()), ok(), secs(0));
    stuck.on_read(Ok(vec![pod("one")]), secs(0));
    stuck.on_node_done(NodeOutcome::Stuck {
        reason: "Timed out after 5m: 1 pod left".into(),
    });
    assert_eq!(
        stuck.status_text(secs(301)),
        "Stuck on wk-04: Timed out after 5m: 1 pod left · cordoned: wk-04"
    );
    let mut stopped = DrainRun::new(input(&["wk-04"], &[]));
    stopped.stop("prod-a was locked; nothing was changed; drain stopped");
    assert_eq!(
        stopped.status_text(secs(1)),
        "Stopped: prod-a was locked; nothing was changed; drain stopped"
    );
    let (mut done, _) = run_over(&[]);
    done.on_node_done(NodeOutcome::Drained);
    assert_eq!(done.status_text(secs(1)), "Drained");
}

#[test]
fn only_a_commit_is_in_flight_until_its_answer() {
    let (mut run, _) = run_over(&["api-1"]);
    assert_eq!(run.in_flight(), None);
    let dry_run = NextStep::DryRun(key("api-1"));
    run.begin_write(&dry_run);
    assert_eq!(run.in_flight(), None, "a dry-run changes nothing");
    let evict = NextStep::Evict(key("api-1"));
    run.begin_write(&evict);
    assert_eq!(run.in_flight(), Some(&evict));
    run.on_write(&evict, ok(), secs(1));
    assert_eq!(run.in_flight(), None);
    let cordon = NextStep::Cordon("wk-04".to_owned());
    run.begin_write(&cordon);
    assert_eq!(run.in_flight(), Some(&cordon));
    // A blocked write clears it too: nothing was sent.
    run.on_write(&cordon, blocked("locked"), secs(2));
    assert_eq!(run.in_flight(), None);
}

#[test]
fn a_stopped_summary_counts_the_eviction_in_the_air_as_unknown() {
    let (mut run, _) = run_over(&["api-1", "api-2"]);
    run.on_write(&NextStep::Evict(key("api-1")), ok(), secs(0));
    let in_the_air = NextStep::Evict(key("api-2"));
    run.begin_write(&in_the_air);
    run.stop("k8sBoard is closing");
    let lines = run.take_summaries();
    assert_eq!(lines.len(), 1);
    assert_eq!((lines[0].evicted, lines[0].unknown), (1, 1));
    assert_eq!(lines[0].outcome, SummaryOutcome::Stopped);
}

#[test]
fn a_stuck_summary_carries_the_reason() {
    let (mut run, _) = run_over(&["api-1"]);
    run.on_write(&NextStep::Evict(key("api-1")), refused(None), secs(0));
    run.on_node_done(NodeOutcome::Stuck {
        reason: "Timed out after 5m: 1 pod left".into(),
    });
    let lines = run.take_summaries();
    assert_eq!(
        lines[0].reason.as_ref().map(|text| text.as_ref()),
        Some("Timed out after 5m: 1 pod left")
    );
    // A drained node has none.
    let (mut done, _) = run_over(&[]);
    done.on_node_done(NodeOutcome::Drained);
    assert_eq!(done.take_summaries()[0].reason, None);
}
