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
fn only_six_kinds_are_creatable() {
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
            ObjectKind::Secret,
            ObjectKind::RoleBinding,
        ]
    );
}

#[test]
fn draft_of_other_kind_is_refused() {
    let text =
        "apiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: x\n  namespace: payments\n";
    assert!(matches!(
        error_of(ObjectKind::Deployment, text),
        DraftError::NotCreatable("Deployment")
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
        &status,
        DraftError::ServerFields { fields } if fields == &["status"]
    ));
    assert_eq!(status.to_string(), "status is set by the server; remove it");
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
        let named = format!("metadata.{field}");
        assert!(
            matches!(&error, DraftError::ServerFields { fields } if fields == &[named.as_str()]),
            "{field}: {error}"
        );
    }
}

/// What `kubectl get configmap -o yaml` prints around the ConfigMap of `CONFIG_MAP`.
const KUBECTL_PASTE: &str = "\
apiVersion: v1
kind: ConfigMap
metadata:
  name: new-config
  namespace: payments
  uid: 5b2f
  resourceVersion: \"881\"
  creationTimestamp: \"2026-10-01T08:00:00Z\"
  managedFields:
  - manager: kubectl
data:
  K: v
status: {}
";

#[test]
fn draft_lists_every_server_field_once() {
    let error = error_of(ObjectKind::ConfigMap, KUBECTL_PASTE);
    assert_eq!(
        error.to_string(),
        "5 fields are set by the server: status, metadata.managedFields, \
         metadata.resourceVersion, metadata.uid, metadata.creationTimestamp; remove them"
    );
    assert_eq!(error.fix(), Some(DraftFix::RemoveServerFields));
}

#[test]
fn remove_server_fields_strips_the_whole_list_and_the_text_then_drafts() {
    let fixed = DraftFix::RemoveServerFields
        .apply(KUBECTL_PASTE)
        .expect("a mapping");
    for field in [
        "status",
        "uid",
        "resourceVersion",
        "creationTimestamp",
        "managedFields",
    ] {
        assert!(!fixed.contains(field), "{field} is still in\n{fixed}");
    }
    assert!(fixed.contains("name: new-config"));
    assert!(fixed.contains("K: v"));
    draft(ObjectKind::ConfigMap, &fixed);
}

#[test]
fn several_documents_say_how_many_and_keep_the_first_on_request() {
    let other = CONFIG_MAP.replace("new-config", "other");
    let several = format!("{CONFIG_MAP}---\n{other}---\n{other}");
    let error = error_of(ObjectKind::ConfigMap, &several);
    assert!(matches!(error, DraftError::SeveralDocuments { count: 3 }));
    assert_eq!(error.to_string(), "Found 3 documents; paste one");
    assert_eq!(error.fix(), Some(DraftFix::KeepFirstDocument));
    let kept = DraftFix::KeepFirstDocument.apply(&several).expect("text");
    assert_eq!(kept, CONFIG_MAP);
    draft(ObjectKind::ConfigMap, &kept);
}

#[test]
fn only_server_fields_and_documents_have_a_fix() {
    assert_eq!(error_of(ObjectKind::ConfigMap, "kind: [").fix(), None);
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
fn a_leading_marker_is_the_start_of_one_document() {
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

#[test]
fn missing_paths_skips_empty_strings_like_null() {
    // A ServiceAccount subject with `apiGroup: ""` comes back without the field.
    let draft =
        json!({"subjects": [{"kind": "ServiceAccount", "apiGroup": "", "name": "default"}]});
    let answer = json!({"subjects": [{"kind": "ServiceAccount", "name": "default"}]});
    assert!(missing_paths(&draft, &answer).is_empty());
}

#[test]
fn missing_paths_reports_every_leaf_of_a_large_draft() {
    let data: serde_json::Map<String, serde_json::Value> = (0..1000)
        .map(|index| (format!("K{index}"), json!("v")))
        .collect();
    let draft = json!({"data": data});
    assert_eq!(missing_paths(&draft, &json!({})).len(), 1000);
}

#[test]
fn kube_system_accounts_and_system_roles_warn_without_typing_the_name() {
    let subjects = "  - kind: ServiceAccount\n    name: coredns\n    namespace: kube-system\n  - kind: ServiceAccount\n    name: default\n    namespace: payments\n";
    let warnings = draft(
        ObjectKind::RoleBinding,
        &binding("view", "ClusterRole", subjects),
    )
    .warnings()
    .to_vec();
    assert_eq!(
        warnings,
        [DraftWarning::SystemNamespaceAccount {
            name: "coredns".to_owned()
        }]
    );
    assert!(!warnings[0].needs_typed_name());
    let role = draft(
        ObjectKind::RoleBinding,
        &binding("system:node-proxier", "ClusterRole", SERVICE_ACCOUNT),
    );
    assert_eq!(
        role.warnings(),
        [DraftWarning::SystemRole {
            role: "system:node-proxier".to_owned()
        }]
    );
    // A namespaced Role of that name is not a built-in ClusterRole.
    let role = binding("system:x", "Role", SERVICE_ACCOUNT);
    assert!(draft(ObjectKind::RoleBinding, &role).warnings().is_empty());
}

const SECRET: &str = "apiVersion: v1\nkind: Secret\nmetadata:\n  name: regcred\n  namespace: payments\ntype: Opaque\nstringData:\n  PASSWORD: hunter2\ndata:\n  OLD: b2xk\n";

#[test]
fn a_secret_folds_string_data_into_base64_data() {
    let secret = draft(ObjectKind::Secret, SECRET);
    let body = secret.body();
    assert!(body.get("stringData").is_none());
    assert_eq!(body.pointer("/data/PASSWORD"), Some(&json!("aHVudGVyMg==")));
    assert_eq!(body.pointer("/data/OLD"), Some(&json!("b2xk")));
}

#[test]
fn string_data_wins_over_data_of_the_same_key() {
    let text = SECRET.replace("OLD: b2xk", "PASSWORD: b2xk");
    let secret = draft(ObjectKind::Secret, &text);
    assert_eq!(
        secret.body().pointer("/data/PASSWORD"),
        Some(&json!("aHVudGVyMg=="))
    );
}

#[test]
fn a_secret_with_a_value_that_is_not_text_is_refused() {
    let text = SECRET.replace("PASSWORD: hunter2", "PASSWORD: 5");
    assert!(matches!(
        error_of(ObjectKind::Secret, &text),
        DraftError::InvalidSecretData
    ));
    let text = SECRET.replace("stringData:\n  PASSWORD: hunter2\n", "stringData: nope\n");
    assert!(matches!(
        error_of(ObjectKind::Secret, &text),
        DraftError::InvalidSecretData
    ));
}

#[test]
fn a_secret_lists_its_type_and_key_names_and_never_a_value() {
    let secret = draft(ObjectKind::Secret, SECRET);
    let fields: Vec<(String, Option<String>)> = secret
        .changed_fields()
        .into_iter()
        .map(|field| (field.path.into_owned(), field.value))
        .collect();
    assert_eq!(
        fields,
        [
            ("metadata.name".to_owned(), Some("regcred".to_owned())),
            ("metadata.namespace".to_owned(), Some("payments".to_owned())),
            ("type".to_owned(), Some("Opaque".to_owned())),
            ("data[OLD]".to_owned(), None),
            ("data[PASSWORD]".to_owned(), None),
        ]
    );
    let shown = format!("{:?} {fields:?}", secret);
    assert!(!shown.contains("hunter2"));
    assert!(!shown.contains("aHVudGVyMg"));
}

#[test]
fn a_secret_needs_a_namespace() {
    let text = SECRET.replace("  namespace: payments\n", "");
    assert!(matches!(
        error_of(ObjectKind::Secret, &text),
        DraftError::MissingNamespace
    ));
}
