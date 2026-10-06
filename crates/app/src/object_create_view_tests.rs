use cluster::{ObjectDraft, ObjectKind, WriteOperation, WriteRequest};

use super::*;

fn request(kind: ObjectKind, text: &str) -> WriteRequest {
    let draft = ObjectDraft::new(kind, text).expect("a valid draft");
    WriteRequest::new(
        draft.target().clone(),
        WriteOperation::CreateObject(Box::new(draft)),
    )
    .expect("a creatable kind fits")
}

fn binding(role: &str) -> String {
    format!(
        "apiVersion: rbac.authorization.k8s.io/v1\nkind: RoleBinding\nmetadata:\n  name: new-binding\n  namespace: payments\nroleRef:\n  apiGroup: rbac.authorization.k8s.io\n  kind: ClusterRole\n  name: {role}\nsubjects:\n  - kind: ServiceAccount\n    name: default\n    namespace: payments\n"
    )
}

fn cluster_ref() -> ClusterRef {
    ClusterRef {
        kubeconfig: std::path::PathBuf::from("test.yaml"),
        context: "stg-b".to_owned(),
    }
}

#[test]
fn a_plain_draft_keeps_change_and_the_cluster_tier() {
    let risk = confirm_risk(&request(ObjectKind::RoleBinding, &binding("view")));
    assert_eq!(risk, ActionRisk::Change);
}

#[test]
fn a_powerful_role_is_privileged() {
    for role in ["cluster-admin", "admin", "edit"] {
        let risk = confirm_risk(&request(ObjectKind::RoleBinding, &binding(role)));
        assert_eq!(risk, ActionRisk::Privileged, "{role}");
    }
}

#[test]
fn a_broad_subject_is_privileged() {
    let text = binding("view").replace(
        "  - kind: ServiceAccount\n    name: default\n    namespace: payments\n",
        "  - kind: Group\n    name: system:authenticated\n    apiGroup: rbac.authorization.k8s.io\n",
    );
    let risk = confirm_risk(&request(ObjectKind::RoleBinding, &text));
    assert_eq!(risk, ActionRisk::Privileged);
}

#[test]
fn a_privileged_pod_security_warning_alone_keeps_the_tier() {
    let text = "apiVersion: v1\nkind: Namespace\nmetadata:\n  name: team\n  labels:\n    pod-security.kubernetes.io/enforce: privileged\n";
    let risk = confirm_risk(&request(ObjectKind::Namespace, text));
    assert_eq!(risk, ActionRisk::Change);
}

#[test]
fn the_intent_names_the_object_and_the_button() {
    let config_map =
        "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: new-config\n  namespace: payments\n";
    let intent = create_intent(
        &cluster_ref(),
        &"stg".into(),
        ObjectKind::ConfigMap,
        request(ObjectKind::ConfigMap, config_map),
        Vec::new(),
    );
    assert_eq!(intent.label, "Create ConfigMap payments/new-config");
    assert_eq!(intent.button, "Create");
    assert_eq!(
        intent.action,
        ResourceAction::CreateObject(ObjectKind::ConfigMap)
    );
    let namespace = "apiVersion: v1\nkind: Namespace\nmetadata:\n  name: team\n";
    let intent = create_intent(
        &cluster_ref(),
        &"stg".into(),
        ObjectKind::Namespace,
        request(ObjectKind::Namespace, namespace),
        Vec::new(),
    );
    assert_eq!(intent.label, "Create Namespace team");
}

#[test]
fn warnings_and_dropped_fields_have_their_own_text() {
    assert_eq!(
        dropped_text("spec.minAvailble"),
        "spec.minAvailble is not a known field; the server dropped it"
    );
    let role = DraftWarning::PowerfulRole {
        role: "cluster-admin".to_owned(),
    };
    assert!(warning_text(&role).contains("cluster-admin"));
    let subject = DraftWarning::BroadSubject {
        kind: "Group".to_owned(),
        name: "system:authenticated".to_owned(),
    };
    assert!(warning_text(&subject).contains("Group system:authenticated"));
    assert!(warning_text(&DraftWarning::PrivilegedPodSecurity).contains("privileged"));
}

#[test]
fn failures_read_in_the_footer() {
    let footer = |failure| footer_text(&CreateCheck::Failed(failure_of(failure)), "");
    assert_eq!(
        footer(EditFailure::OutcomeUnknown),
        "No answer in time; the object may have been created. Check the list before trying again."
    );
    assert_eq!(
        footer(EditFailure::Invalid {
            message: "ConfigMap a already exists".into(),
            fields: vec!["metadata.name".into()]
        }),
        "ConfigMap a already exists"
    );
    assert_eq!(
        footer(EditFailure::Refused(
            "The server refused for now: slow".into()
        )),
        "The server refused for now: slow"
    );
}

#[test]
fn the_footer_follows_the_check() {
    assert_eq!(footer_text(&CreateCheck::NotChecked, ""), "Not checked yet");
    let passed = CreateCheck::Passed(Box::new(PassedCreate {
        for_text: "a: 1".into(),
        request: None,
        warnings: Vec::new(),
        elapsed: Duration::from_millis(212),
    }));
    assert_eq!(footer_text(&passed, "a: 1"), "Dry-run OK · 212 ms");
    assert_eq!(footer_text(&passed, "a: 2"), "Changed since the last check");
}

#[test]
fn a_thousand_dropped_fields_are_cut_at_ten_lines() {
    let dropped: Vec<String> = (0..1000).map(|index| format!("data.K{index}")).collect();
    let lines = warning_lines(&[], &dropped);
    assert_eq!(lines.len(), 11);
    assert_eq!(
        lines[0],
        "data.K0 is not a known field; the server dropped it"
    );
    assert_eq!(lines[10], "\u{2026} and 990 more");
}

#[test]
fn five_hundred_broad_subjects_are_cut_at_ten_lines() {
    let subjects: String = (0..500)
        .map(|index| format!("  - kind: Group\n    name: system:g{index}\n    apiGroup: rbac.authorization.k8s.io\n"))
        .collect();
    let text = binding("view").replace(
        "  - kind: ServiceAccount\n    name: default\n    namespace: payments\n",
        &subjects,
    );
    let draft = ObjectDraft::new(ObjectKind::RoleBinding, &text).expect("a valid draft");
    assert_eq!(draft.warnings().len(), 500);
    let lines = warning_lines(draft.warnings(), &[]);
    assert_eq!(lines.len(), 11);
    assert_eq!(lines[10], "\u{2026} and 490 more");
    // The warnings still ask for the binding name: only their lines are cut.
    let request = request(ObjectKind::RoleBinding, &text);
    assert_eq!(confirm_risk(&request), ActionRisk::Privileged);
}

#[test]
fn a_short_list_is_shown_whole() {
    let lines = warning_lines(
        &[DraftWarning::PrivilegedPodSecurity],
        &["spec.minAvailble".to_owned()],
    );
    assert_eq!(lines.len(), 2);
}
