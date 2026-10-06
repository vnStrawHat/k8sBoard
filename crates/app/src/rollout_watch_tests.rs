use cluster::{DeploymentSummary, WorkloadCondition};

use super::*;

fn deployment(ready: u32, desired: u32) -> DeploymentSummary {
    DeploymentSummary {
        namespace: "lab-shop".to_owned(),
        name: "web".to_owned(),
        created_at: None,
        labels: Vec::new(),
        desired,
        ready,
        up_to_date: ready,
        available: ready,
        strategy: "RollingUpdate".to_owned(),
        max_surge: None,
        max_unavailable: None,
        progress_deadline_seconds: 600,
        is_paused: false,
        generation: 5,
        observed_generation: 5,
        revision: Some("4".to_owned()),
        selector: Vec::new(),
        containers: Vec::new(),
        conditions: Vec::new(),
        template_change: None,
    }
}

fn condition(name: &str, is_true: bool, reason: &str) -> WorkloadCondition {
    WorkloadCondition {
        name: name.to_owned(),
        is_true,
        reason: Some(reason.to_owned()),
        message: None,
        last_transition: None,
    }
}

const SECOND: Duration = Duration::from_secs(1);

#[test]
fn a_status_from_before_the_patch_is_not_the_end() {
    // The watch still shows the old generation and the old, fully ready counts.
    let before = deployment(3, 3);
    assert_eq!(rollout_outcome(&before, 5, SECOND), None);
}

#[test]
fn a_new_generation_is_not_the_end_until_the_controller_observed_it() {
    let mut unseen = deployment(3, 3);
    unseen.generation = 6;
    assert_eq!(rollout_outcome(&unseen, 5, SECOND), None);
}

#[test]
fn a_rollout_with_every_replica_updated_and_ready_is_complete() {
    let mut done = deployment(3, 3);
    done.generation = 6;
    done.observed_generation = 6;
    assert_eq!(
        rollout_outcome(&done, 5, 5 * SECOND),
        Some(RolloutOutcome::Complete {
            ready: 3,
            desired: 3
        })
    );
}

#[test]
fn a_rollout_with_replicas_still_starting_goes_on() {
    let mut rolling = deployment(2, 3);
    rolling.generation = 6;
    rolling.observed_generation = 6;
    assert_eq!(rollout_outcome(&rolling, 5, 10 * SECOND), None);
}

#[test]
fn a_patch_that_changed_nothing_ends_after_the_grace() {
    // A Roll back to the running template leaves the generation as it was.
    let same = deployment(3, 3);
    assert_eq!(rollout_outcome(&same, 5, 2 * SECOND), None);
    assert_eq!(
        rollout_outcome(&same, 5, BEGIN_GRACE),
        Some(RolloutOutcome::Complete {
            ready: 3,
            desired: 3
        })
    );
}

#[test]
fn the_progress_deadline_ends_it_with_the_controllers_reason() {
    let mut stuck = deployment(2, 3);
    stuck.generation = 6;
    stuck.observed_generation = 6;
    stuck.conditions = vec![condition("Progressing", false, DEADLINE_EXCEEDED)];
    assert_eq!(
        rollout_outcome(&stuck, 5, 20 * SECOND),
        Some(RolloutOutcome::Stalled(
            "ProgressDeadlineExceeded".to_owned()
        ))
    );
}

#[test]
fn a_stale_deadline_before_the_new_generation_is_observed_is_not_read() {
    let mut stale = deployment(2, 3);
    stale.generation = 6;
    stale.observed_generation = 5;
    stale.conditions = vec![condition("Progressing", false, DEADLINE_EXCEEDED)];
    assert_eq!(rollout_outcome(&stale, 5, 2 * SECOND), None);
}

#[test]
fn a_replica_failure_ends_it_with_its_reason() {
    let mut failing = deployment(2, 3);
    failing.generation = 6;
    failing.observed_generation = 6;
    failing.conditions = vec![condition("ReplicaFailure", true, "FailedCreate")];
    assert_eq!(
        rollout_outcome(&failing, 5, 8 * SECOND),
        Some(RolloutOutcome::Stalled("FailedCreate".to_owned()))
    );
}

#[test]
fn the_limit_ends_a_slow_rollout_with_its_counts_and_the_unavailable_reason() {
    let mut slow = deployment(1, 3);
    slow.generation = 6;
    slow.observed_generation = 6;
    slow.conditions = vec![condition("Available", false, "MinimumReplicasUnavailable")];
    assert_eq!(rollout_outcome(&slow, 5, WATCH_LIMIT - SECOND), None);
    assert_eq!(
        rollout_outcome(&slow, 5, WATCH_LIMIT),
        Some(RolloutOutcome::Stalled(
            "1/3 ready after 2 min, MinimumReplicasUnavailable".to_owned()
        ))
    );
}

#[test]
fn the_summary_names_one_rollout_and_its_counts() {
    let complete = RolloutOutcome::Complete {
        ready: 3,
        desired: 3,
    };
    assert_eq!(
        rollout_summary(&[("web".to_owned(), complete)]),
        ("Rollout complete: web (3/3 ready)".to_owned(), true)
    );
    let stalled = RolloutOutcome::Stalled("ProgressDeadlineExceeded".to_owned());
    assert_eq!(
        rollout_summary(&[("web".to_owned(), stalled)]),
        (
            "Rollout not progressing: web (ProgressDeadlineExceeded)".to_owned(),
            false
        )
    );
}

#[test]
fn the_summary_of_a_batch_lists_each_object() {
    let outcomes = [
        (
            "web".to_owned(),
            RolloutOutcome::Complete {
                ready: 3,
                desired: 3,
            },
        ),
        (
            "api".to_owned(),
            RolloutOutcome::Stalled("ProgressDeadlineExceeded".to_owned()),
        ),
    ];
    assert_eq!(
        rollout_summary(&outcomes),
        (
            "Rollouts: web complete (3/3), api not progressing (ProgressDeadlineExceeded)"
                .to_owned(),
            false
        )
    );
}

#[test]
fn a_toast_id_names_every_workload_of_the_watch() {
    let workloads = [
        ("lab-shop".to_owned(), "web".to_owned()),
        ("lab-shop".to_owned(), "api".to_owned()),
    ];
    assert_eq!(
        rollout_toast_id(&workloads).as_ref(),
        "lab-shop/web,lab-shop/api"
    );
}
