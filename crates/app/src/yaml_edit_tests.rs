use cluster::WriteError;

use super::*;

fn conflict() -> CheckedWriteError {
    CheckedWriteError::Write(WriteError::Conflict {
        message: "the object changed".to_owned(),
        managers: Vec::new(),
    })
}

#[test]
fn rollout_check_names_the_strategy() {
    let rolling = EditCheck::Rollout {
        strategy: "RollingUpdate".to_owned(),
    };
    assert_eq!(
        check_text(&rolling, ""),
        "Pods will be replaced (RollingUpdate)"
    );
}

#[test]
fn on_delete_rollout_says_pods_change_only_when_deleted() {
    let on_delete = EditCheck::Rollout {
        strategy: "OnDelete".to_owned(),
    };
    assert_eq!(
        check_text(&on_delete, ""),
        "Pods change only when they are deleted (OnDelete)"
    );
}

#[test]
fn stale_last_applied_warns_kubectl_apply_users() {
    let text = check_text(&EditCheck::StaleLastApplied, "");
    assert!(text.starts_with("kubectl apply users:"), "{text}");
    assert!(text.contains("can revert this change"), "{text}");
}

#[test]
fn leading_zero_check_reads_the_number_of_its_line() {
    let text = "spec:\n  defaultMode: 0644\n";
    assert_eq!(
        check_text(&EditCheck::LeadingZero { line: 2 }, text),
        "Line 2: 0644 is read as 644 (YAML 1.2); write 420 or 0o644"
    );
}

#[test]
fn leading_zero_check_without_a_number_on_the_line_stays_general() {
    let text = check_text(&EditCheck::LeadingZero { line: 9 }, "a: 1\n");
    assert!(
        text.starts_with("Line 9: a number with a leading zero"),
        "{text}"
    );
}

#[test]
fn leading_zero_with_an_octal_digit_out_of_range_has_no_octal_hint() {
    let text = check_text(&EditCheck::LeadingZero { line: 1 }, "mode: 089\n");
    assert_eq!(text, "Line 1: 089 is read as 89 (YAML 1.2)");
}

#[test]
fn footer_follows_the_preview_and_the_text() {
    assert_eq!(
        footer_text(&PreviewState::NotChecked, "a"),
        "Not checked yet"
    );
    let passed = PreviewState::Passed(Box::new(PassedPreview {
        for_text: "a".into(),
        request: None,
        changes: Vec::new(),
        more_changes: 0,
        checks: Vec::new(),
        quota: QuotaLine::None,
        rows: Vec::new(),
        elapsed: Duration::from_millis(412),
    }));
    assert_eq!(
        footer_text(&passed, "a"),
        "Dry-run OK · 412 ms · unchanged since you opened it"
    );
    assert_eq!(footer_text(&passed, "b"), "Changed since the last check");
}

#[test]
fn footer_shows_the_failure() {
    let local = PreviewState::Failed(PreviewFailure::Local(EditError::NoChanges));
    assert_eq!(footer_text(&local, ""), "nothing changed");
    let server = PreviewState::Failed(PreviewFailure::Server("boom".into()));
    assert_eq!(footer_text(&server, ""), "boom");
    let invalid = PreviewState::Failed(PreviewFailure::Invalid {
        message: "the change is invalid".into(),
        fields: vec!["spec.replicas".into()],
    });
    assert_eq!(footer_text(&invalid, ""), "the change is invalid");
}

#[test]
fn a_409_is_a_conflict_and_a_404_a_deleted_object() {
    assert_eq!(edit_failure_of(&conflict()), EditFailure::Conflict);
    let gone = CheckedWriteError::Write(WriteError::NotFound);
    assert_eq!(edit_failure_of(&gone), EditFailure::Deleted);
}

#[test]
fn an_unknown_outcome_is_told_apart() {
    let unknown = CheckedWriteError::Write(WriteError::OutcomeUnknown);
    assert_eq!(edit_failure_of(&unknown), EditFailure::OutcomeUnknown);
}

#[test]
fn a_422_keeps_its_field_paths_verbatim() {
    let invalid = CheckedWriteError::Write(WriteError::Invalid {
        message: "Deployment.apps \"api\" is invalid".to_owned(),
        fields: vec![
            "spec.template.spec.containers[0].image".to_owned(),
            "spec.replicas".to_owned(),
        ],
    });
    assert_eq!(
        edit_failure_of(&invalid),
        EditFailure::Invalid {
            message: "Deployment.apps \"api\" is invalid".into(),
            fields: vec![
                "spec.template.spec.containers[0].image".into(),
                "spec.replicas".into()
            ],
        }
    );
}

#[test]
fn a_429_says_the_server_refused_for_now() {
    let refused = CheckedWriteError::Write(WriteError::TooManyRequests {
        message: "slow down".to_owned(),
        retry_after: None,
    });
    assert_eq!(
        edit_failure_of(&refused),
        EditFailure::Refused("The server refused for now: slow down".into())
    );
}

#[test]
fn a_blocked_step_and_other_errors_carry_their_text() {
    let blocked = CheckedWriteError::Blocked("prod-a was locked; nothing was changed".into());
    assert_eq!(
        edit_failure_of(&blocked),
        EditFailure::Other("prod-a was locked; nothing was changed".into())
    );
    let denied = CheckedWriteError::Write(WriteError::Denied {
        message: "no".to_owned(),
    });
    assert_eq!(
        edit_failure_of(&denied),
        EditFailure::Other("not permitted: no".into())
    );
}

#[test]
fn the_edit_intent_records_edit_yaml_with_a_paths_only_request() {
    let object = ObjectRef::new(
        ObjectKind::Deployment,
        Some("payments".to_owned()),
        "api".to_owned(),
    )
    .expect("a valid object");
    let cluster = ClusterRef {
        kubeconfig: std::path::PathBuf::from("k.yaml"),
        context: "prod-a".to_owned(),
    };
    let request = WriteRequest::new(
        object,
        WriteOperation::ReplaceObject(Box::new(sample_edit())),
    )
    .expect("an editable kind");
    let intent = edit_intent(
        &cluster,
        &"prod-a".into(),
        ObjectKind::Deployment,
        request,
        vec!["warning".into()],
    );
    assert_eq!(
        intent.action,
        ResourceAction::EditYaml(ObjectKind::Deployment)
    );
    assert_eq!(intent.button.as_ref(), "Apply changes");
    assert_eq!(intent.risk, ActionRisk::Change);
    assert_eq!(intent.expected(), "api");
    assert_eq!(intent.warnings.len(), 1);
    let fields = intent.request.changed_fields();
    assert!(fields.iter().all(|field| field.value.is_none()));
}

/// An edit of a Deployment replica count, built through the cluster crate's public path.
pub(crate) fn sample_edit() -> ObjectEdit {
    let object = serde_json::json!({
        "apiVersion": "apps/v1",
        "kind": "Deployment",
        "metadata": {"name": "api", "namespace": "payments", "uid": "u", "resourceVersion": "1"},
        "spec": {"replicas": 3},
    });
    let target = ObjectRef::new(
        ObjectKind::Deployment,
        Some("payments".to_owned()),
        "api".to_owned(),
    )
    .expect("a valid object");
    let body = object.to_string();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime");
    let _guard = runtime.enter();
    let (connection, _api) =
        cluster::fake_api::FakeApi::connection(cluster::WritePolicy::Blocked, move |_| {
            (200, body.clone())
        });
    let base = runtime
        .block_on(connection.edit_base(&target, EnvValues::Hidden))
        .expect("the base reads");
    let text = base.text().replace("replicas: 3", "replicas: 5");
    ObjectEdit::new(&base, &text).expect("a change")
}

#[test]
fn a_long_path_is_cut_in_the_middle_and_keeps_its_field() {
    let path = "spec.template.spec.containers[api].resources.limits.memory";
    let shown = elide_middle(path, 34);
    assert_eq!(shown.chars().count(), 34);
    assert!(shown.starts_with("spec.templ"), "{shown}");
    assert!(shown.ends_with("limits.memory"), "{shown}");
    assert!(shown.contains('…'));
}

#[test]
fn a_short_path_is_shown_whole() {
    assert_eq!(elide_middle("spec.replicas", 34), "spec.replicas");
    assert_eq!(elide_middle("abcdef", 6), "abcdef");
}

#[test]
fn history_tab_only_for_deployments() {
    use super::yaml_edit_panels::edit_tabs;
    assert_eq!(
        edit_tabs(ObjectKind::Deployment),
        [EditTab::Editor, EditTab::Diff, EditTab::History]
    );
    for kind in [
        ObjectKind::Service,
        ObjectKind::StatefulSet,
        ObjectKind::ConfigMap,
    ] {
        assert_eq!(edit_tabs(kind), [EditTab::Editor, EditTab::Diff]);
    }
}
