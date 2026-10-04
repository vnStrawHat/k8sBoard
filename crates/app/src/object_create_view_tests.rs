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
    let (risk, name) = confirm_risk(&request(ObjectKind::RoleBinding, &binding("view")));
    assert_eq!(risk, ActionRisk::Change);
    assert_eq!(name, None);
}

#[test]
fn a_powerful_role_types_the_binding_name() {
    for role in ["cluster-admin", "admin", "edit"] {
        let (risk, name) = confirm_risk(&request(ObjectKind::RoleBinding, &binding(role)));
        assert_eq!(risk, ActionRisk::Privileged, "{role}");
        assert_eq!(name.as_deref(), Some("new-binding"));
    }
}

#[test]
fn a_broad_subject_types_the_binding_name() {
    let text = binding("view").replace(
        "  - kind: ServiceAccount\n    name: default\n    namespace: payments\n",
        "  - kind: Group\n    name: system:authenticated\n    apiGroup: rbac.authorization.k8s.io\n",
    );
    let (risk, name) = confirm_risk(&request(ObjectKind::RoleBinding, &text));
    assert_eq!(risk, ActionRisk::Privileged);
    assert_eq!(name.as_deref(), Some("new-binding"));
}

#[test]
fn a_privileged_pod_security_warning_alone_keeps_the_tier() {
    let text = "apiVersion: v1\nkind: Namespace\nmetadata:\n  name: team\n  labels:\n    pod-security.kubernetes.io/enforce: privileged\n";
    let (risk, name) = confirm_risk(&request(ObjectKind::Namespace, text));
    assert_eq!((risk, name), (ActionRisk::Change, None));
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
