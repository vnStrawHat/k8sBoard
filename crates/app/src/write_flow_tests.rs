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
    test_guard(access, lock, "stg-b", Environment::STAGING)
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
    // The `TypeName` tier types the node the change is on.
    assert_eq!(intent.expected(), "wk-04");
    assert_eq!(intent.typed_hint(), "the node name");
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
fn dry_run_results_become_dialog_states() {
    let outcome = WriteOutcome {
        mode: WriteMode::DryRun,
        elapsed: Duration::from_millis(412),
        effect: cluster::WriteEffect::Patched,
        created_name: None,
        uid: None,
        dropped_fields: Vec::new(),
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
        DryRunState::Failed("Dry-run failed: writes are blocked in this screenshot build".into())
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
fn a_create_names_what_it_made_in_the_notice() {
    assert_eq!(
        success_notice(
            "Trigger cronjob reconcile now",
            Some("reconcile-manual-x7k2p"),
            false
        ),
        "Trigger cronjob reconcile now: created reconcile-manual-x7k2p"
    );
    assert_eq!(
        success_notice("Cordon node wk-04", None, false),
        "Cordoned node wk-04."
    );
}

#[test]
fn a_recreate_and_a_reclaim_policy_read_in_the_past_tense() {
    assert_eq!(
        success_notice("Recreate claim data with class standard", None, false),
        "Recreated claim data with class standard."
    );
    assert_eq!(
        success_notice("Set reclaim policy of volume pv-1 to Retain", None, false),
        "Set reclaim policy of volume pv-1 to Retain."
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

#[test]
fn the_audit_records_the_name_the_server_picked() {
    let field = created_name_field("reconcile-manual-x7k2p");
    assert_eq!(field.path, "metadata.name");
    assert_eq!(field.value.as_deref(), Some("reconcile-manual-x7k2p"));
}

fn allowing_only(allowed: &[cluster::AccessCheck]) -> AccessState {
    use cluster::{AccessCheck, AccessDecision, AccessReport, AccessReview};
    AccessState::Known(AccessReport {
        reviews: AccessCheck::ALL
            .into_iter()
            .map(|check| AccessReview {
                check,
                decision: if allowed.contains(&check) {
                    AccessDecision::Allowed
                } else {
                    AccessDecision::Denied { reason: None }
                },
            })
            .collect(),
    })
}

#[test]
fn only_both_port_forward_verbs_give_a_permit() {
    use cluster::AccessCheck;
    let both = [
        AccessCheck::GetPodPortForward,
        AccessCheck::CreatePodPortForward,
    ];
    assert!(port_forward_permit_of(&allowing_only(&both)).is_some());
    assert!(port_forward_permit_of(&allowing_only(&both[..1])).is_none());
    assert!(port_forward_permit_of(&allowing_only(&both[1..])).is_none());
    assert!(port_forward_permit_of(&AccessState::Unknown).is_none());
}

#[test]
fn run_guarded_picks_the_port_forward_permit() {
    use cluster::AccessCheck;
    let forward = ConnectOpen::PortForward(
        Rc::new(|_, _, _, _, _| {}),
        LocalPortChoice {
            initial: LocalPortSpec::Auto,
            automatic: 15432,
            chosen: Rc::new(std::cell::Cell::new(None)),
        },
    );
    let shell = ConnectOpen::Exec(Rc::new(|_, _, _, _, _| {}));
    let forward_rights = allowing_only(&[
        AccessCheck::GetPodPortForward,
        AccessCheck::CreatePodPortForward,
    ]);
    let exec_rights = allowing_only(&[AccessCheck::GetPodExec, AccessCheck::CreatePodExec]);
    // Each open takes its own permit, and exec rights never open a forward (or the reverse).
    assert!(forward.granted(&forward_rights).is_some());
    assert!(forward.granted(&exec_rights).is_none());
    assert!(shell.granted(&exec_rights).is_some());
    assert!(shell.granted(&forward_rights).is_none());
    // No permit: the open call is never reached.
    assert!(forward.granted(&AccessState::Unknown).is_none());
}

// ---- 0037: create, then attach ----

#[test]
fn only_both_attach_verbs_give_a_permit() {
    use cluster::AccessCheck;
    let both = [AccessCheck::GetPodAttach, AccessCheck::CreatePodAttach];
    assert!(attach_permit_of(&allowing_only(&both)).is_some());
    assert!(attach_permit_of(&allowing_only(&both[..1])).is_none());
    assert!(attach_permit_of(&allowing_only(&both[1..])).is_none());
    assert!(attach_permit_of(&AccessState::Unknown).is_none());
}

fn debug_write_intent() -> Rc<WriteIntent> {
    let target = ObjectRef::new(
        ObjectKind::Pod,
        Some("shop".to_owned()),
        "multi-0".to_owned(),
    )
    .expect("a pod");
    let request = WriteRequest::new(
        target,
        WriteOperation::AddDebugContainer {
            name: "k8sboard-debug-x7k2q".to_owned(),
            image: cluster::DEFAULT_DEBUG_IMAGE.to_owned(),
            target_container: "web".to_owned(),
        },
    )
    .expect("a fitting request");
    Rc::new(WriteIntent {
        cluster: cluster(),
        cluster_name: "stg-b".into(),
        action: ResourceAction::DebugContainer,
        label: "Add debug container to multi-0".into(),
        button: "Add debug container".into(),
        request,
        risk: ActionRisk::Change,
        warnings: Vec::new(),
        change_lines: Vec::new(),
        audit_fields: Vec::new(),
    })
}

fn connect_intent(open: ConnectOpen) -> ConnectIntent {
    ConnectIntent {
        cluster: cluster(),
        cluster_name: "stg-b".into(),
        action: ResourceAction::DebugContainer,
        label: "Add debug container to multi-0".into(),
        button: "Add debug container".into(),
        risk: ActionRisk::Change,
        warnings: Vec::new(),
        object: AuditObject {
            kind: "Pod".to_owned(),
            namespace: Some("shop".to_owned()),
            name: "multi-0".to_owned(),
        },
        fields: Vec::new(),
        open,
    }
}

fn attach_open() -> ConnectOpen {
    ConnectOpen::CreateThenAttach(CreateThenAttach {
        create: debug_write_intent(),
        open: Rc::new(|_, _, _, _, _, _| {}),
        discard: Rc::new(|_, _, _, _| {}),
    })
}

#[test]
fn create_then_attach_picks_the_attach_permit() {
    use cluster::AccessCheck;
    let open = attach_open();
    let attach_rights = allowing_only(&[AccessCheck::GetPodAttach, AccessCheck::CreatePodAttach]);
    let exec_rights = allowing_only(&[AccessCheck::GetPodExec, AccessCheck::CreatePodExec]);
    assert!(open.granted(&attach_rights).is_some());
    // Exec rights never open an attach, so nothing is created.
    assert!(open.granted(&exec_rights).is_none());
    assert!(open.granted(&AccessState::Unknown).is_none());
}

#[test]
fn attach_open_takes_the_attach_permit() {
    use cluster::AccessCheck;
    let open = ConnectOpen::Attach(Rc::new(|_, _, _, _, _| {}));
    let attach_rights = allowing_only(&[AccessCheck::GetPodAttach, AccessCheck::CreatePodAttach]);
    let exec_rights = allowing_only(&[AccessCheck::GetPodExec, AccessCheck::CreatePodExec]);
    assert!(open.granted(&attach_rights).is_some());
    // One verb of the pair, or exec rights, never open an attach.
    assert!(
        open.granted(&allowing_only(&[AccessCheck::GetPodAttach]))
            .is_none()
    );
    assert!(open.granted(&exec_rights).is_none());
    assert!(open.granted(&AccessState::Unknown).is_none());
    // An attach writes nothing first, so its dialog has no dry-run.
    assert!(connect_intent(open).create().is_none());
}

#[test]
fn a_start_that_writes_first_exposes_its_write() {
    let writes = connect_intent(attach_open());
    let create = writes.create().expect("a write comes first");
    assert_eq!(create.button, "Add debug container");
    let plain = connect_intent(ConnectOpen::Exec(Rc::new(|_, _, _, _, _| {})));
    assert!(plain.create().is_none());
}

#[test]
fn a_start_types_the_name_of_the_object_it_opens_on() {
    let debug = connect_intent(attach_open());
    assert_eq!(debug.expected(), "multi-0");
    assert_eq!(debug.typed_hint(), "the pod name");
    let mut node = connect_intent(attach_open());
    node.object = AuditObject {
        kind: "Node".to_owned(),
        namespace: None,
        name: "wk-03".to_owned(),
    };
    assert_eq!(node.expected(), "wk-03");
    assert_eq!(node.typed_hint(), "the node name");
}

fn cleanup_request(name: &str, uid: &str) -> Option<WriteRequest> {
    let target = ObjectRef::new(
        ObjectKind::Pod,
        Some("kube-system".to_owned()),
        name.to_owned(),
    )?;
    WriteRequest::new(
        target,
        WriteOperation::DeleteNodeShellPod {
            uid: uid.to_owned(),
        },
    )
}

#[test]
fn a_cleanup_is_built_only_from_a_node_shell_delete() {
    use cluster::WritePolicy;
    use cluster::fake_api::FakeApi;
    // The fake client's worker is a tokio task, so it is built inside a runtime.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a tokio runtime");
    let _guard = runtime.enter();
    let (connection, _api) = FakeApi::connection(WritePolicy::Allowed, |_| (200, "{}".to_owned()));
    let access = AccessState::Unknown;
    let guard = guard(&access, WriteLock::Unlocked);
    let delete = cleanup_request("k8sboard-node-shell-wk-03-x7k2q", "uid-1").expect("a delete");
    assert!(NodeShellCleanup::new(connection.clone(), delete, CleanupAudit::of(&guard)).is_some());
    // The write path itself refuses a delete of any other pod, so there is nothing to wrap.
    assert!(cleanup_request("coredns-5d78c9869d-abcde", "uid-1").is_none());
    // And any other operation is refused here.
    let cordon = cordon_intent(&cluster(), "stg-b", "wk-03", &NodeScheduling::Enabled)
        .expect("a cordon")
        .request;
    assert!(NodeShellCleanup::new(connection, cordon, CleanupAudit::of(&guard)).is_none());
}

#[test]
fn the_cleanup_audit_line_names_the_delete_and_the_copied_cluster() {
    use cluster::WritePolicy;
    use cluster::fake_api::FakeApi;
    // The fake client's worker is a tokio task, so it is built inside a runtime.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a tokio runtime");
    let _guard = runtime.enter();
    let (connection, _api) = FakeApi::connection(WritePolicy::Allowed, |_| (200, "{}".to_owned()));
    let access = AccessState::Unknown;
    let delete = cleanup_request("k8sboard-node-shell-wk-03-x7k2q", "uid-1").expect("a delete");
    let cleanup = NodeShellCleanup::new(
        connection,
        delete,
        CleanupAudit::of(&guard(&access, WriteLock::Unlocked)),
    )
    .expect("a cleanup");
    let entry = cleanup.audit_entry(AuditOutcome::Abandoned, Some("quit".to_owned()));
    let line = serde_json::to_value(&entry).expect("serializes");
    assert_eq!(line["action"], "Delete node shell pod");
    assert_eq!(line["cluster"], "stg-b");
    assert_eq!(line["outcome"], "abandoned");
    assert_eq!(line["object"]["kind"], "Pod");
    assert_eq!(line["object"]["namespace"], "kube-system");
    assert_eq!(line["object"]["name"], "k8sboard-node-shell-wk-03-x7k2q");
    assert_eq!(line["fields"][0]["path"], "metadata.uid");
    assert_eq!(cleanup.namespace(), "kube-system");
    assert_eq!(cleanup.pod(), "k8sboard-node-shell-wk-03-x7k2q");
}

#[test]
fn a_start_is_refused_by_its_gate_when_the_setting_is_off() {
    let access = AccessState::Known(cluster::AccessReport {
        reviews: cluster::AccessCheck::ALL
            .into_iter()
            .map(|check| cluster::AccessReview {
                check,
                decision: cluster::AccessDecision::Allowed,
            })
            .collect(),
    });
    let mut guard = guard(&access, WriteLock::Unlocked);
    let mut intent = connect_intent(attach_open());
    intent.action = ResourceAction::OpenNodeShell;
    guard.profile.allow_node_shell = true;
    assert_eq!(intent.gate_block(&guard), None);
    guard.profile.allow_node_shell = false;
    assert_eq!(
        intent.gate_block(&guard).as_deref(),
        Some("Node shell is off for stg-b (Settings › Clusters › Safety)")
    );
}

fn create_request(kind: ObjectKind, text: &str) -> WriteRequest {
    let draft = cluster::ObjectDraft::new(kind, text).expect("a valid draft");
    WriteRequest::new(
        draft.target().clone(),
        WriteOperation::CreateObject(Box::new(draft)),
    )
    .expect("a creatable kind fits")
}

#[test]
fn a_create_notice_names_the_kind_and_where_it_is() {
    let config_map =
        "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: new-config\n  namespace: payments\n";
    assert_eq!(
        create_success_notice(&create_request(ObjectKind::ConfigMap, config_map)),
        "Created ConfigMap payments/new-config"
    );
    let namespace = "apiVersion: v1\nkind: Namespace\nmetadata:\n  name: team-a\n";
    assert_eq!(
        create_success_notice(&create_request(ObjectKind::Namespace, namespace)),
        "Created Namespace team-a"
    );
}

fn deployment_intent(action: ResourceAction, operation: WriteOperation) -> WriteIntent {
    job_intent(action, ObjectKind::Deployment, operation)
}

#[test]
fn only_a_deployment_rollout_start_is_followed_to_its_end() {
    let restart = |kind| {
        job_intent(
            ResourceAction::RestartRollout(kind),
            kind,
            WriteOperation::RestartRollout {
                restarted_at: jiff::Timestamp::UNIX_EPOCH,
            },
        )
    };
    assert!(watches_rollout(&restart(ObjectKind::Deployment)));
    assert!(!watches_rollout(&restart(ObjectKind::StatefulSet)));
    let pause = |paused| {
        deployment_intent(
            ResourceAction::PauseRollout,
            WriteOperation::SetRolloutPaused { paused },
        )
    };
    assert!(watches_rollout(&pause(false)), "Resume starts the rollout");
    assert!(!watches_rollout(&pause(true)), "Pause starts nothing");
    let scale = deployment_intent(
        ResourceAction::Scale(ObjectKind::Deployment),
        WriteOperation::ScaleWorkload {
            replicas: 4,
            previous: 3,
        },
    );
    assert!(watches_rollout(&scale));
    let mut edit = deployment_intent(
        ResourceAction::Scale(ObjectKind::Deployment),
        WriteOperation::ScaleWorkload {
            replicas: 4,
            previous: 3,
        },
    );
    let target = ObjectRef::new(
        ObjectKind::Deployment,
        Some("payments".to_owned()),
        "api".to_owned(),
    )
    .expect("a deployment");
    let replace =
        WriteOperation::ReplaceObject(Box::new(crate::yaml_edit::yaml_edit_tests::sample_edit()));
    edit.action = ResourceAction::EditYaml(ObjectKind::Deployment);
    edit.request = WriteRequest::new(target, replace).expect("an editable kind");
    assert!(watches_rollout(&edit), "an edit of spec.replicas rolls");
    let rerun = job_intent(
        ResourceAction::RerunJob,
        ObjectKind::Job,
        WriteOperation::RerunJob,
    );
    assert!(!watches_rollout(&rerun));
}

#[test]
fn a_watched_commit_says_the_rollout_is_under_way() {
    assert_eq!(
        success_notice("Restart rollout of statefulset db", None, false),
        "Restarted rollout of statefulset db."
    );
    assert_eq!(
        success_notice("Restart rollout of deployment web", None, true),
        "Restarted rollout of deployment web. Watching rollout…"
    );
    assert_eq!(
        success_notice("Scale deployment web from 3 to 4", None, true),
        "Scaled deployment web from 3 to 4. Watching rollout…"
    );
}

fn job_intent(action: ResourceAction, kind: ObjectKind, operation: WriteOperation) -> WriteIntent {
    let target = ObjectRef::new(kind, Some("shop".to_owned()), "heartbeat".to_owned())
        .expect("a namespaced object");
    WriteIntent {
        cluster: cluster(),
        cluster_name: "stg-b".into(),
        action,
        label: "Trigger cronjob heartbeat now".into(),
        button: "Trigger now".into(),
        request: WriteRequest::new(target, operation).expect("a fitting request"),
        risk: ActionRisk::Change,
        warnings: Vec::new(),
        change_lines: Vec::new(),
        audit_fields: Vec::new(),
    }
}

#[test]
fn a_trigger_offers_to_view_the_job_it_created() {
    let intent = job_intent(
        ResourceAction::TriggerCronJob,
        ObjectKind::CronJob,
        WriteOperation::TriggerCronJob,
    );
    let subject = created_job_subject(&intent, Some("heartbeat-manual-x7k2p")).expect("a job");
    assert_eq!(
        subject.key,
        ResourceKey::Kind {
            kind: crate::resource_kind::ResourceKind::Jobs,
            namespace: Some("shop".to_owned()),
            name: "heartbeat-manual-x7k2p".to_owned(),
        }
    );
    // Without a name from the server there is nothing to view; other actions never offer it.
    assert!(created_job_subject(&intent, None).is_none());
    let rerun = job_intent(
        ResourceAction::RerunJob,
        ObjectKind::Job,
        WriteOperation::RerunJob,
    );
    assert!(created_job_subject(&rerun, Some("heartbeat-rerun-abc12")).is_some());
    let cordon = job_intent(
        ResourceAction::Cordon,
        ObjectKind::CronJob,
        WriteOperation::TriggerCronJob,
    );
    assert!(created_job_subject(&cordon, Some("x")).is_none());
}

#[test]
fn a_scale_toast_offers_the_old_count_unless_nothing_changed() {
    let scale = |replicas, previous| {
        deployment_intent(
            ResourceAction::Scale(ObjectKind::Deployment),
            WriteOperation::ScaleWorkload { replicas, previous },
        )
    };
    assert_eq!(scale_back_of(&scale(0, 1)), Some(1));
    assert_eq!(scale_back_of(&scale(3, 3)), None);
    assert_eq!(scale_back_label(1), "Scale back to 1");
    let pause = deployment_intent(
        ResourceAction::PauseRollout,
        WriteOperation::SetRolloutPaused { paused: true },
    );
    assert_eq!(scale_back_of(&pause), None);
}

#[test]
fn a_dry_run_the_server_refuses_for_good_has_no_retry_but_a_transient_one_has() {
    let refused = |error: WriteError| dry_run_state_of(Err(CheckedWriteError::Write(error)));
    for error in [
        WriteError::Invalid {
            message: "only dynamically provisioned pvc can be resized".to_owned(),
            fields: Vec::new(),
        },
        WriteError::Denied {
            message: "cannot patch".to_owned(),
        },
        WriteError::NotFound,
    ] {
        assert!(matches!(refused(error), DryRunState::Refused(_)));
    }
    for error in [
        WriteError::TooManyRequests {
            message: "slow down".to_owned(),
            retry_after: None,
        },
        WriteError::OutcomeUnknown,
    ] {
        assert!(matches!(refused(error), DryRunState::Failed(_)));
    }
    let blocked = dry_run_state_of(Err(CheckedWriteError::Blocked("locked".into())));
    assert_eq!(blocked, DryRunState::Failed("locked".into()));
}
