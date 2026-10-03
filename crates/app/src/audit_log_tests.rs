use serde_json::Value;

use super::*;
use crate::cluster_session::AccessState;
use crate::environment::Environment;
use crate::write_guard::test_guard;

fn temp_dir(name: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("k8sboard-0030-audit-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn field(path: &str, value: Option<&str>) -> AuditField {
    AuditField {
        path: path.to_owned(),
        value: value.map(str::to_owned),
    }
}

/// Every key set, so the serialized value shows the whole allow-list.
fn full_entry() -> AuditEntry {
    AuditEntry {
        at: "2026-10-02T09:12:03Z".to_owned(),
        cluster: "uat-monitor".to_owned(),
        context: "readonly@Monitor".to_owned(),
        user: Some("readonly".to_owned()),
        action: "Cordon".to_owned(),
        object: Some(AuditObject {
            kind: "Pod".to_owned(),
            namespace: Some("team-a".to_owned()),
            name: "api-0".to_owned(),
        }),
        fields: vec![field("spec.unschedulable", Some("true"))],
        outcome: AuditOutcome::Failed,
        error: Some("refused".to_owned()),
        note: Some("maintenance".to_owned()),
    }
}

fn collect_keys(prefix: &str, value: &Value, keys: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                collect_keys(&path, child, keys);
                keys.push(path);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_keys(prefix, item, keys);
            }
        }
        _ => {}
    }
}

fn keys_of(entry: &AuditEntry) -> Vec<String> {
    let value = serde_json::to_value(entry).expect("serializes");
    let mut keys = Vec::new();
    collect_keys("", &value, &mut keys);
    keys.sort();
    keys.dedup();
    keys
}

#[test]
fn audit_keys_are_the_allow_list() {
    assert_eq!(
        keys_of(&full_entry()),
        [
            "action",
            "at",
            "cluster",
            "context",
            "error",
            "fields",
            "fields.path",
            "fields.value",
            "note",
            "object",
            "object.kind",
            "object.name",
            "object.namespace",
            "outcome",
            "user",
        ]
    );
}

#[test]
fn outcomes_serialize_lowercase() {
    for (outcome, text) in [
        (AuditOutcome::Applied, "applied"),
        (AuditOutcome::Failed, "failed"),
        (AuditOutcome::Unknown, "unknown"),
    ] {
        let mut entry = full_entry();
        entry.outcome = outcome;
        let value = serde_json::to_value(&entry).expect("serializes");
        assert_eq!(value["outcome"], text);
    }
}

#[test]
fn lock_entry_names_the_guard_cluster() {
    let access = AccessState::Unknown;
    let guard = test_guard(
        &access,
        WriteLock::Unlocked,
        "prod-eu-1",
        Environment::Production,
    );
    let lock = lock_entry(&guard, WriteLock::Locked);
    assert_eq!(lock.action, "Lock");
    assert_eq!(lock.cluster, "prod-eu-1");
    assert_eq!(lock.context, "prod-eu-1");
    assert_eq!(lock.user.as_deref(), Some("tester"));
    assert!(lock.object.is_none() && lock.fields.is_empty());
    assert_eq!(lock.outcome, AuditOutcome::Applied);
    // `2026-10-02T09:12:03Z`: whole seconds, UTC.
    assert!(lock.at.len() == 20 && lock.at.ends_with('Z'), "{}", lock.at);
    let unlock = lock_entry(&guard, WriteLock::Unlocked);
    assert_eq!(unlock.action, "Unlock");
}

#[test]
fn a_lock_line_has_no_object_error_or_note_key() {
    let access = AccessState::Unknown;
    let guard = test_guard(
        &access,
        WriteLock::Locked,
        "dev-1",
        Environment::Development,
    );
    assert_eq!(
        keys_of(&lock_entry(&guard, WriteLock::Unlocked)),
        [
            "action", "at", "cluster", "context", "fields", "outcome", "user"
        ]
    );
}

#[test]
fn secret_kind_records_no_values() {
    let fields = vec![
        field("data.password", Some("hunter2")),
        field("type", Some("Opaque")),
    ];
    let recorded = recordable_fields("Secret", fields);
    assert_eq!(recorded.len(), 2);
    assert!(recorded.iter().all(|field| field.value.is_none()));
    assert_eq!(recorded[0].path, "data.password");
}

#[test]
fn config_map_records_paths_only() {
    let recorded = recordable_fields("ConfigMap", vec![field("data.url", Some("https://x"))]);
    assert_eq!(recorded[0].path, "data.url");
    assert!(recorded[0].value.is_none());
}

#[test]
fn other_kinds_keep_their_values() {
    let recorded = recordable_fields("Node", vec![field("spec.unschedulable", Some("true"))]);
    assert_eq!(recorded[0].value.as_deref(), Some("true"));
}

#[test]
fn note_is_trimmed_and_capped() {
    assert_eq!(
        clean_note("  rolling the pool \n"),
        Some("rolling the pool".to_owned())
    );
    assert_eq!(
        clean_note("line one\r\nline\ttwo"),
        Some("line one  line two".to_owned())
    );
    assert_eq!(clean_note(" \n\t "), None);
    assert_eq!(clean_note(""), None);
    let long = "x".repeat(900);
    assert_eq!(
        clean_note(&long).map(|note| note.chars().count()),
        Some(500)
    );
}

#[test]
fn append_adds_one_line_per_entry() {
    let dir = temp_dir("append");
    append_audit(&dir, &full_entry()).expect("first line");
    let mut second = full_entry();
    second.action = "Uncordon".to_owned();
    second.note = None;
    append_audit(&dir, &second).expect("second line");
    let text = std::fs::read_to_string(audit_path(&dir)).expect("the log exists");
    assert!(text.ends_with('\n'));
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 2);
    let parsed: Vec<Value> = lines
        .iter()
        .map(|line| serde_json::from_str(line).expect("each line is JSON"))
        .collect();
    assert_eq!(parsed[0]["action"], "Cordon");
    assert_eq!(parsed[1]["action"], "Uncordon");
    assert!(parsed[1].get("note").is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn append_fails_without_a_folder() {
    let dir = temp_dir("missing").join("not-there");
    assert!(append_audit(&dir, &full_entry()).is_err());
    let _ = std::fs::remove_dir_all(dir.parent().expect("a parent"));
}

#[cfg(unix)]
#[test]
fn append_creates_owner_only_file() {
    use std::os::unix::fs::PermissionsExt as _;

    let dir = temp_dir("mode");
    append_audit(&dir, &full_entry()).expect("a line");
    let mode = std::fs::metadata(audit_path(&dir))
        .expect("the log exists")
        .permissions()
        .mode();
    assert_eq!(mode & 0o077, 0, "{mode:o}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_log_is_audit_jsonl_in_the_settings_folder() {
    let dir = PathBuf::from("settings-folder");
    assert_eq!(audit_path(&dir), dir.join("audit.jsonl"));
}

#[test]
fn write_entry_uses_the_intent_cluster() {
    use crate::app_shell::write_flow::WriteIntent;
    use crate::cluster_registry::ClusterRef;
    use crate::resource_actions::ResourceAction;
    use crate::write_guard::ActionRisk;
    use cluster::{ObjectKind, ObjectRef, WriteOperation, WriteRequest};

    let target = ObjectRef::new(ObjectKind::Node, None, "wk-04".to_owned()).expect("a node");
    let operation = WriteOperation::SetNodeSchedulable { schedulable: false };
    let request = WriteRequest::new(target, operation).expect("a node fits");
    let intent = WriteIntent {
        cluster: ClusterRef {
            kubeconfig: PathBuf::from("test.yaml"),
            context: "stg-b".to_owned(),
        },
        cluster_name: "stg-b".into(),
        action: ResourceAction::Cordon,
        label: "Cordon node wk-04".into(),
        button: "Cordon".into(),
        request,
        risk: ActionRisk::Change,
        expected_name: None,
        warnings: Vec::new(),
    };
    let access = AccessState::Unknown;
    // The guard is the intent's cluster, whatever else is viewed.
    let guard = test_guard(&access, WriteLock::Unlocked, "stg-b", Environment::Staging);
    let entry = audit_entry(
        &intent,
        &guard,
        AuditOutcome::Unknown,
        Some("no answer".to_owned()),
        Some("  maintenance\n"),
    );
    assert_eq!(entry.cluster, "stg-b");
    assert_eq!(entry.context, "stg-b");
    assert_eq!(entry.action, "Cordon");
    assert_eq!(entry.outcome, AuditOutcome::Unknown);
    assert_eq!(entry.error.as_deref(), Some("no answer"));
    assert_eq!(entry.note.as_deref(), Some("maintenance"));
    let object = entry.object.expect("an object");
    assert_eq!(
        (object.kind.as_str(), object.name.as_str()),
        ("Node", "wk-04")
    );
    assert_eq!(object.namespace, None);
    assert_eq!(entry.fields[0].path, "spec.unschedulable");
    assert_eq!(entry.fields[0].value.as_deref(), Some("true"));
}
