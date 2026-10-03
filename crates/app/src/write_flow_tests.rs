use std::path::PathBuf;

use cluster::NodeScheduling;

use super::*;
use crate::cluster_session::AccessState;
use crate::environment::Environment;
use crate::write_guard::test_guard;

fn cluster() -> ClusterRef {
    ClusterRef {
        kubeconfig: PathBuf::from("test.yaml"),
        context: "stg-b".to_owned(),
    }
}

fn guard(access: &AccessState, lock: WriteLock) -> ClusterGuard<'_> {
    test_guard(access, lock, "stg-b", Environment::Staging)
}

const PASSED: DryRunState = DryRunState::Passed {
    elapsed: Duration::from_millis(412),
};

fn block(
    guard: Option<&ClusterGuard<'_>>,
    generation: u64,
    dry_run: &DryRunState,
    typed: TypedMatch,
) -> Option<String> {
    commit_block(guard, "stg-b", generation, dry_run, typed, "stg-b")
        .map(|reason| reason.to_string())
}

#[test]
fn commit_block_table() {
    let access = AccessState::Unknown;
    let open = guard(&access, WriteLock::Unlocked);
    let locked = guard(&access, WriteLock::Locked);
    let gone = Some("stg-b is no longer open; nothing was changed");
    // The guard of the cluster is gone, or its connection changed since the dialog opened.
    assert_eq!(
        block(None, 0, &PASSED, TypedMatch::NotNeeded).as_deref(),
        gone
    );
    assert_eq!(
        block(Some(&open), 7, &PASSED, TypedMatch::NotNeeded).as_deref(),
        gone
    );
    assert_eq!(
        block(Some(&locked), 0, &PASSED, TypedMatch::NotNeeded).as_deref(),
        Some("stg-b was locked; nothing was changed")
    );
    assert_eq!(
        block(Some(&open), 0, &DryRunState::Running, TypedMatch::NotNeeded).as_deref(),
        Some("Waiting for the dry-run…")
    );
    let failed = DryRunState::Failed("Dry-run failed: no".into());
    assert_eq!(
        block(Some(&open), 0, &failed, TypedMatch::NotNeeded).as_deref(),
        Some("Dry-run failed: no")
    );
    let rejected = DryRunState::Rejected("policy.example.com".into());
    assert_eq!(
        block(Some(&open), 0, &rejected, TypedMatch::NotNeeded).as_deref(),
        Some(
            "An admission webhook does not support dry-run, so this change cannot be checked: policy.example.com. Nothing was changed."
        )
    );
    assert_eq!(
        block(Some(&open), 0, &PASSED, TypedMatch::Differs).as_deref(),
        Some("Type stg-b to confirm")
    );
    assert_eq!(block(Some(&open), 0, &PASSED, TypedMatch::NotNeeded), None);
    assert_eq!(block(Some(&open), 0, &PASSED, TypedMatch::Matches), None);
}

#[test]
fn the_first_matching_block_wins() {
    let access = AccessState::Unknown;
    let locked = guard(&access, WriteLock::Locked);
    // Locked and still running: the lock is the reason, not the wait.
    assert_eq!(
        block(Some(&locked), 0, &DryRunState::Running, TypedMatch::Differs).as_deref(),
        Some("stg-b was locked; nothing was changed")
    );
}

#[test]
fn an_unlock_needs_only_the_connection_and_the_name() {
    let access = AccessState::Unknown;
    let locked = guard(&access, WriteLock::Locked);
    let unlock = |guard: Option<&ClusterGuard<'_>>, generation, typed| {
        unlock_block(guard, "stg-b", generation, typed, "stg-b").map(|reason| reason.to_string())
    };
    // A locked cluster is exactly what an unlock starts from.
    assert_eq!(unlock(Some(&locked), 0, TypedMatch::NotNeeded), None);
    let gone = Some("stg-b is no longer open; nothing was changed");
    assert_eq!(unlock(None, 0, TypedMatch::NotNeeded).as_deref(), gone);
    assert_eq!(
        unlock(Some(&locked), 3, TypedMatch::Matches).as_deref(),
        gone
    );
    assert_eq!(
        unlock(Some(&locked), 0, TypedMatch::Differs).as_deref(),
        Some("Type stg-b to confirm")
    );
}

#[test]
fn typed_name_must_match_exactly() {
    let confirm = DialogConfirm::TypeName {
        expected: "prod-eu-1".to_owned(),
    };
    assert_eq!(typed_match(&confirm, "prod-eu-1"), TypedMatch::Matches);
    assert_eq!(typed_match(&confirm, "  prod-eu-1 \t"), TypedMatch::Matches);
    for text in ["Prod-eu-1", "prod-eu", "prod-eu-1x", "", "prod eu 1"] {
        assert_eq!(typed_match(&confirm, text), TypedMatch::Differs, "{text:?}");
    }
    assert_eq!(
        typed_match(&DialogConfirm::Click, ""),
        TypedMatch::NotNeeded
    );
}

#[test]
fn confirmed_needs_a_passed_dry_run_and_match() {
    for typed in [TypedMatch::NotNeeded, TypedMatch::Matches] {
        assert!(confirmed(&PASSED, typed, 4).is_some());
    }
    assert!(confirmed(&PASSED, TypedMatch::Differs, 4).is_none());
    for state in [
        DryRunState::Running,
        DryRunState::Failed("x".into()),
        DryRunState::Rejected("y".into()),
    ] {
        assert!(
            confirmed(&state, TypedMatch::Matches, 4).is_none(),
            "{state:?}"
        );
    }
}

#[test]
fn cordon_label_follows_scheduling() {
    assert_eq!(cordon_label(&NodeScheduling::Enabled), "Cordon");
    assert_eq!(cordon_label(&NodeScheduling::Disabled), "Uncordon");
}

#[test]
fn cordon_intent_targets_the_node() {
    let intent = cordon_intent(&cluster(), "uat-monitor", "wk-04", &NodeScheduling::Enabled)
        .expect("a valid node name");
    assert_eq!(intent.cluster, cluster());
    assert_eq!(intent.label, "Cordon node wk-04");
    assert_eq!(intent.button, "Cordon");
    assert_eq!(intent.risk, ActionRisk::Change);
    assert_eq!(intent.request.target().name(), "wk-04");
    assert_eq!(intent.request.target().kind_name(), "Node");
    assert_eq!(
        intent.request.changed_fields()[0].value.as_deref(),
        Some("true")
    );
    // The `TypeName` tier types the cluster, not the node.
    assert_eq!(intent.expected(), "uat-monitor");
    assert_eq!(intent.typed_hint(), "the cluster name");
}

#[test]
fn a_cordoned_node_gets_an_uncordon_intent() {
    let intent = cordon_intent(&cluster(), "stg-b", "wk-04", &NodeScheduling::Disabled)
        .expect("a valid node name");
    assert_eq!(intent.button, "Uncordon");
    assert_eq!(intent.label, "Uncordon node wk-04");
    assert_eq!(
        intent.request.changed_fields()[0].value.as_deref(),
        Some("false")
    );
}

#[test]
fn a_name_that_changes_the_path_makes_no_intent() {
    assert!(cordon_intent(&cluster(), "stg-b", "wk/04", &NodeScheduling::Enabled).is_none());
}

#[test]
fn a_named_object_asks_for_its_own_name() {
    let mut intent = cordon_intent(&cluster(), "stg-b", "wk-04", &NodeScheduling::Enabled)
        .expect("a valid node name");
    intent.expected_name = Some("wk-04".to_owned());
    assert_eq!(intent.expected(), "wk-04");
    assert_eq!(intent.typed_hint(), "the node name");
}

#[test]
fn dry_run_results_become_dialog_states() {
    let outcome = WriteOutcome {
        mode: WriteMode::DryRun,
        elapsed: Duration::from_millis(412),
        effect: cluster::WriteEffect::Patched,
        created_name: None,
        uid: None,
    };
    assert_eq!(dry_run_state_of(Ok(outcome)), PASSED);
    let state = |error| dry_run_state_of(Err(CheckedWriteError::Write(error)));
    assert_eq!(
        state(WriteError::DryRunRejected {
            reason: "hook".to_owned()
        }),
        DryRunState::Rejected("hook".into())
    );
    assert_eq!(
        state(WriteError::WritesBlocked),
        DryRunState::Failed(
            "Dry-run failed: writes are blocked in this debug build (set K8SBOARD_ALLOW_WRITES=1)"
                .into()
        )
    );
    assert_eq!(
        dry_run_state_of(Err(CheckedWriteError::Blocked("locked".into()))),
        DryRunState::Failed("locked".into())
    );
}

#[test]
fn an_invalid_change_lists_its_field_paths() {
    let error = WriteError::Invalid {
        message: "no".to_owned(),
        fields: vec!["spec.unschedulable".to_owned(), "spec.taints[0]".to_owned()],
    };
    assert_eq!(
        write_error_text(&error),
        "the change is invalid: no (spec.unschedulable, spec.taints[0])"
    );
}

#[test]
fn failure_notices_name_the_label_and_the_outcome() {
    assert_eq!(
        failure_notice(
            "Cordon node wk-04",
            &CheckedWriteError::Write(WriteError::NotFound)
        ),
        "Cordon node wk-04 failed: the object no longer exists"
    );
    assert_eq!(
        failure_notice(
            "Cordon node wk-04",
            &CheckedWriteError::Write(WriteError::OutcomeUnknown)
        ),
        "Cordon node wk-04: the outcome is unknown; the change may have been applied. Refresh to check."
    );
    assert_eq!(
        failure_notice(
            "Cordon node wk-04",
            &CheckedWriteError::Blocked("stg-b was locked; nothing was changed".into())
        ),
        "Cordon node wk-04: stg-b was locked; nothing was changed"
    );
}

#[test]
fn only_a_conflict_or_a_refusal_can_be_retried() {
    let conflict = CheckedWriteError::Write(WriteError::Conflict {
        message: "moved".to_owned(),
        managers: Vec::new(),
    });
    assert_eq!(
        retryable_text(&conflict).as_deref(),
        Some("The object changed since the check: moved")
    );
    let refused = CheckedWriteError::Write(WriteError::TooManyRequests {
        message: "slow".to_owned(),
        retry_after: None,
    });
    assert_eq!(
        retryable_text(&refused).as_deref(),
        Some("The server refused for now: slow")
    );
    for error in [
        CheckedWriteError::Write(WriteError::NotFound),
        CheckedWriteError::Write(WriteError::OutcomeUnknown),
        CheckedWriteError::Blocked("x".into()),
    ] {
        assert!(retryable_text(&error).is_none());
    }
}

#[test]
fn write_entry_records_unknown_outcome() {
    assert_eq!(
        audit_outcome(&Err(WriteError::OutcomeUnknown)),
        Some(AuditOutcome::Unknown)
    );
    assert_eq!(
        audit_outcome(&Err(WriteError::NotFound)),
        Some(AuditOutcome::Failed)
    );
}

#[test]
fn blocked_is_never_audited() {
    assert_eq!(audit_outcome(&Err(WriteError::WritesBlocked)), None);
    let refused = WriteError::TooManyRequests {
        message: "slow".to_owned(),
        retry_after: None,
    };
    assert_eq!(audit_outcome(&Err(refused)), None);
}

#[test]
fn a_stream_start_has_no_dry_run_to_wait_for_but_still_checks_lock_and_name() {
    let access = AccessState::Unknown;
    let open = guard(&access, WriteLock::Unlocked);
    let locked = guard(&access, WriteLock::Locked);
    let not_supported = DryRunState::NotSupported;
    assert_eq!(
        block(Some(&open), 0, &not_supported, TypedMatch::NotNeeded),
        None
    );
    assert_eq!(
        block(Some(&locked), 0, &not_supported, TypedMatch::NotNeeded).as_deref(),
        Some("stg-b was locked; nothing was changed")
    );
    assert_eq!(
        block(Some(&open), 0, &not_supported, TypedMatch::Differs).as_deref(),
        Some("Type stg-b to confirm")
    );
    assert_eq!(
        block(None, 0, &not_supported, TypedMatch::NotNeeded).as_deref(),
        Some("stg-b is no longer open; nothing was changed")
    );
}

#[test]
fn confirmed_accepts_a_start_with_no_dry_run_once_the_name_matches() {
    let not_supported = DryRunState::NotSupported;
    assert!(confirmed(&not_supported, TypedMatch::NotNeeded, 3).is_some());
    assert!(confirmed(&not_supported, TypedMatch::Matches, 3).is_some());
    assert!(confirmed(&not_supported, TypedMatch::Differs, 3).is_none());
    // A change that is still being checked, or failed its check, is not confirmed.
    assert!(confirmed(&DryRunState::Running, TypedMatch::NotNeeded, 3).is_none());
}

#[test]
fn only_both_exec_verbs_give_a_permit() {
    use cluster::{AccessCheck, AccessDecision, AccessReport, AccessReview};
    let report = |denied: &[AccessCheck]| {
        AccessState::Known(AccessReport {
            reviews: AccessCheck::ALL
                .into_iter()
                .map(|check| AccessReview {
                    check,
                    decision: if denied.contains(&check) {
                        AccessDecision::Denied { reason: None }
                    } else {
                        AccessDecision::Allowed
                    },
                })
                .collect(),
        })
    };
    assert!(exec_permit_of(&report(&[])).is_some());
    assert!(exec_permit_of(&report(&[AccessCheck::GetPodExec])).is_none());
    assert!(exec_permit_of(&report(&[AccessCheck::CreatePodExec])).is_none());
    assert!(exec_permit_of(&AccessState::Unknown).is_none());
}
