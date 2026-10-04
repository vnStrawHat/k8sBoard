use serde_json::json;

use super::*;

const CONFIG_MAP: &str = "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: new-config\n  namespace: payments\ndata:\n  KEY: value\n";

fn binding(role: &str, role_kind: &str, subjects: &str) -> String {
    format!(
        "apiVersion: rbac.authorization.k8s.io/v1\nkind: RoleBinding\nmetadata:\n  name: new-binding\n  namespace: payments\nroleRef:\n  apiGroup: rbac.authorization.k8s.io\n  kind: {role_kind}\n  name: {role}\nsubjects:\n{subjects}"
    )
}

const SERVICE_ACCOUNT: &str =
    "  - kind: ServiceAccount\n    name: default\n    namespace: payments\n";

fn draft(kind: ObjectKind, text: &str) -> ObjectDraft {
    ObjectDraft::new(kind, text).expect("a valid draft")
}

fn error_of(kind: ObjectKind, text: &str) -> DraftError {
    ObjectDraft::new(kind, text).expect_err("a refused draft")
}

#[test]
fn only_five_kinds_are_creatable() {
    let creatable: Vec<_> = ObjectKind::ALL
        .into_iter()
        .filter(|kind| kind.is_creatable())
        .collect();
    assert_eq!(
        creatable,
        [
            ObjectKind::Namespace,
            ObjectKind::ConfigMap,
            ObjectKind::ResourceQuota,
            ObjectKind::PodDisruptionBudget,
            ObjectKind::RoleBinding,
        ]
    );
}

#[test]
fn draft_of_other_kind_is_refused() {
    let text = "apiVersion: v1\nkind: Secret\nmetadata:\n  name: x\n  namespace: payments\n";
    assert!(matches!(
        error_of(ObjectKind::Secret, text),
        DraftError::NotCreatable("Secret")
    ));
}

#[test]
fn draft_refuses_wrong_kind_and_api_version() {
    let wrong_kind = CONFIG_MAP.replace("kind: ConfigMap", "kind: Secret");
    assert!(matches!(
        error_of(ObjectKind::ConfigMap, &wrong_kind),
        DraftError::WrongKind {
            expected: "ConfigMap"
        }
    ));
    let budget = "apiVersion: policy/v1beta1\nkind: PodDisruptionBudget\nmetadata:\n  name: p\n  namespace: payments\n";
    let error = error_of(ObjectKind::PodDisruptionBudget, budget);
    assert!(
        matches!(&error, DraftError::WrongApiVersion { expected } if expected == "policy/v1"),
        "{error}"
    );
}

#[test]
fn draft_refuses_server_fields() {
    let with = |extra: &str| format!("{CONFIG_MAP}{extra}");
    let status = error_of(ObjectKind::ConfigMap, &with("status: {}\n"));
    assert!(matches!(
        status,
        DraftError::ServerField { field: "status" }
    ));
    for field in [
        "uid",
        "resourceVersion",
        "creationTimestamp",
        "managedFields",
        "ownerReferences",
    ] {
        let text = CONFIG_MAP.replace(
            "  namespace: payments\n",
            &format!("  namespace: payments\n  {field}: x\n"),
        );
        let error = error_of(ObjectKind::ConfigMap, &text);
        assert!(
            matches!(&error, DraftError::ServerField { field: named } if *named == field),
            "{field}: {error}"
        );
    }
}

#[test]
fn draft_refuses_generate_name_and_missing_name() {
    let generated = CONFIG_MAP.replace("name: new-config", "generateName: new-");
    assert!(matches!(
        error_of(ObjectKind::ConfigMap, &generated),
        DraftError::GenerateName
    ));
    let nameless = CONFIG_MAP.replace("  name: new-config\n", "");
    assert!(matches!(
        error_of(ObjectKind::ConfigMap, &nameless),
        DraftError::MissingName
    ));
}

#[test]
fn draft_name_rules_per_kind() {
    let namespace =
        |name: &str| format!("apiVersion: v1\nkind: Namespace\nmetadata:\n  name: {name}\n");
    assert!(matches!(
        error_of(ObjectKind::Namespace, &namespace("a.b")),
        DraftError::InvalidName { kind: "Namespace" }
    ));
    draft(ObjectKind::Namespace, &namespace("team-a"));
    draft(
        ObjectKind::ConfigMap,
        &CONFIG_MAP.replace("new-config", "a.b"),
    );
    let named =
        |name: &str| binding("view", "ClusterRole", SERVICE_ACCOUNT).replace("new-binding", name);
    draft(ObjectKind::RoleBinding, &named("system:x"));
    assert!(matches!(
        error_of(ObjectKind::RoleBinding, &named("a/b")),
        DraftError::InvalidName { .. }
    ));
}

#[test]
fn draft_namespace_rules() {
    let no_namespace = CONFIG_MAP.replace("  namespace: payments\n", "");
    assert!(matches!(
        error_of(ObjectKind::ConfigMap, &no_namespace),
        DraftError::MissingNamespace
    ));
    let namespace = "apiVersion: v1\nkind: Namespace\nmetadata:\n  name: a\n  namespace: b\n";
    assert!(matches!(
        error_of(ObjectKind::Namespace, namespace),
        DraftError::UnexpectedNamespace { kind: "Namespace" }
    ));
}

#[test]
fn draft_refuses_hidden_placeholder() {
    let text = CONFIG_MAP.replace("KEY: value", "K: <hidden>");
    let error = error_of(ObjectKind::ConfigMap, &text);
    assert!(
        matches!(&error, DraftError::Placeholder { path } if path == "data.K"),
        "{error}"
    );
    let marker = CONFIG_MAP.replace("KEY: value", "K: <hidden, changed>");
    assert!(matches!(
        error_of(ObjectKind::ConfigMap, &marker),
        DraftError::Placeholder { .. }
    ));
}

#[test]
fn draft_reports_syntax_and_size() {
    assert!(matches!(
        error_of(ObjectKind::ConfigMap, "a: [1, 2\nb: {"),
        DraftError::Text(EditError::Syntax { .. })
    ));
    let large = format!("{CONFIG_MAP}# {}\n", "x".repeat(2 * 1024 * 1024));
    assert!(matches!(
        error_of(ObjectKind::ConfigMap, &large),
        DraftError::Text(EditError::TooLarge)
    ));
    let zero = CONFIG_MAP.replace("KEY: value", "MODE: 0755");
    assert!(matches!(
        error_of(ObjectKind::ConfigMap, &zero),
        DraftError::Text(EditError::LeadingZero { .. })
    ));
}

#[test]
fn draft_refuses_multi_document_yaml() {
    let several = format!(
        "{CONFIG_MAP}---\n{}",
        CONFIG_MAP.replace("new-config", "other")
    );
    assert!(matches!(
        error_of(ObjectKind::ConfigMap, &several),
        DraftError::Text(EditError::NotAnObject)
    ));
    // A leading marker is the start of a single document.
    draft(ObjectKind::ConfigMap, &format!("---\n{CONFIG_MAP}"));
}

#[test]
fn draft_with_a_badly_typed_metadata_field_is_refused() {
    let text = CONFIG_MAP.replace(
        "  namespace: payments\n",
        "  namespace: payments\n  labels: 5\n",
    );
    assert!(matches!(
        error_of(ObjectKind::ConfigMap, &text),
        DraftError::InvalidMetadata
    ));
}

#[test]
fn role_binding_warnings() {
    let warnings = |role: &str, subjects: &str| {
        draft(
            ObjectKind::RoleBinding,
            &binding(role, "ClusterRole", subjects),
        )
        .warnings()
        .to_vec()
    };
    for role in ["cluster-admin", "admin", "edit"] {
        assert_eq!(
            warnings(role, SERVICE_ACCOUNT),
            [DraftWarning::PowerfulRole {
                role: role.to_owned()
            }]
        );
    }
    let group = "  - kind: Group\n    name: system:authenticated\n    apiGroup: rbac.authorization.k8s.io\n";
    let user =
        "  - kind: User\n    name: system:anonymous\n    apiGroup: rbac.authorization.k8s.io\n";
    let broad = warnings("view", &format!("{group}{user}{SERVICE_ACCOUNT}"));
    assert_eq!(
        broad,
        [
            DraftWarning::BroadSubject {
                kind: "Group".to_owned(),
                name: "system:authenticated".to_owned()
            },
            DraftWarning::BroadSubject {
                kind: "User".to_owned(),
                name: "system:anonymous".to_owned()
            },
        ]
    );
    assert!(warnings("view", SERVICE_ACCOUNT).is_empty());
    assert!(broad.iter().all(DraftWarning::needs_typed_name));
    assert!(!DraftWarning::PrivilegedPodSecurity.needs_typed_name());
    // A namespaced Role called `admin` is not the aggregate ClusterRole.
    let role = binding("admin", "Role", SERVICE_ACCOUNT);
    assert!(draft(ObjectKind::RoleBinding, &role).warnings().is_empty());
}

#[test]
fn namespace_privileged_pod_security_warns() {
    let namespace = |level: &str| {
        format!(
            "apiVersion: v1\nkind: Namespace\nmetadata:\n  name: team\n  labels:\n    pod-security.kubernetes.io/enforce: {level}\n"
        )
    };
    assert_eq!(
        draft(ObjectKind::Namespace, &namespace("privileged")).warnings(),
        [DraftWarning::PrivilegedPodSecurity]
    );
    assert!(
        draft(ObjectKind::Namespace, &namespace("baseline"))
            .warnings()
            .is_empty()
    );
}

#[test]
fn missing_paths_reports_dropped_leaves_only() {
    let draft = json!({
        "metadata": {"name": "p", "labels": {}, "annotations": null},
        "spec": {"minAvailble": 1, "selector": {"matchLabels": {"app": "x"}}, "list": [1, 2], "none": []},
    });
    let answer = json!({
        "metadata": {"name": "p", "uid": "u-1", "labels": {"added": "by-server"}},
        "spec": {"selector": {"matchLabels": {"app": "y"}}, "list": [1], "defaulted": true},
    });
    let mut paths: Vec<_> = missing_paths(&draft, &answer)
        .iter()
        .map(ToString::to_string)
        .collect();
    // The order of the map keys depends on whether serde_json keeps insertion order.
    paths.sort();
    assert_eq!(paths, ["spec.list[1]", "spec.minAvailble"]);
}

#[test]
fn draft_debug_holds_no_content() {
    let text = CONFIG_MAP.replace("value", "S3cr3t-0042");
    let text = format!("{:?}", draft(ObjectKind::ConfigMap, &text));
    assert!(text.contains("ConfigMap") && text.contains("payments") && text.contains("new-config"));
    assert!(!text.contains("S3cr3t-0042"), "{text}");
}

#[test]
fn draft_target_and_body_follow_the_text() {
    let draft = draft(ObjectKind::ConfigMap, CONFIG_MAP);
    assert_eq!(draft.target().name(), "new-config");
    assert_eq!(draft.target().namespace(), Some("payments"));
    assert_eq!(draft.body()["data"], json!({"KEY": "value"}));
    assert!(draft.is_consistent());
}

#[test]
fn checked_operation_refuses_inconsistent_draft() {
    use crate::object_write::{WriteOperation, WriteRequest};

    let good = draft(ObjectKind::ConfigMap, CONFIG_MAP);
    let request = |draft: ObjectDraft| {
        WriteRequest::new(
            draft.target().clone(),
            WriteOperation::CreateObject(Box::new(draft)),
        )
    };
    assert!(request(good.clone()).is_some());
    let mut renamed = good.clone();
    renamed.body["metadata"]["name"] = json!("another");
    let mut with_status = good.clone();
    with_status.body["status"] = json!({});
    let mut with_uid = good.clone();
    with_uid.body["metadata"]["uid"] = json!("u");
    let mut other_kind = good.clone();
    other_kind.body["kind"] = json!("Secret");
    let mut generated = good;
    generated.body["metadata"]["generateName"] = json!("x-");
    for draft in [renamed, with_status, with_uid, other_kind, generated] {
        assert!(!draft.is_consistent());
        assert!(request(draft).is_none());
    }
}
