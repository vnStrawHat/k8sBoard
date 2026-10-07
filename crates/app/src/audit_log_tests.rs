use serde_json::Value;

use super::*;
use crate::app_shell::object_delete::Removal;
use crate::cluster_session::AccessState;
use crate::drain_plan::{BudgetPolicy, DrainOptions};
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
        from: None,
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
        fields: vec![AuditField {
            from: Some("false".to_owned()),
            ..field("spec.unschedulable", Some("true"))
        }],
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
            "fields.from",
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
        (AuditOutcome::Abandoned, "abandoned"),
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
        Environment::PRODUCTION,
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
        Environment::DEVELOPMENT,
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
        warnings: Vec::new(),
        change_lines: Vec::new(),
        audit_fields: Vec::new(),
    };
    let access = AccessState::Unknown;
    // The guard is the intent's cluster, whatever else is viewed.
    let guard = test_guard(&access, WriteLock::Unlocked, "stg-b", Environment::STAGING);
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

#[test]
fn audit_records_paths_only() {
    use crate::app_shell::write_flow::WriteIntent;
    use crate::yaml_edit::edit_intent;
    use crate::yaml_edit::yaml_edit_tests::sample_edit;
    use cluster::{ObjectKind, ObjectRef, WriteOperation, WriteRequest};

    let target = ObjectRef::new(
        ObjectKind::Deployment,
        Some("payments".to_owned()),
        "api".to_owned(),
    )
    .expect("a deployment");
    let request = WriteRequest::new(
        target,
        WriteOperation::ReplaceObject(Box::new(sample_edit())),
    )
    .expect("an editable kind");
    let cluster = crate::cluster_registry::ClusterRef {
        kubeconfig: PathBuf::from("test.yaml"),
        context: "stg-b".to_owned(),
    };
    let intent: WriteIntent = edit_intent(
        &cluster,
        &"stg-b".into(),
        ObjectKind::Deployment,
        request,
        Vec::new(),
    );
    let access = AccessState::Unknown;
    let guard = test_guard(&access, WriteLock::Unlocked, "stg-b", Environment::STAGING);
    let entry = audit_entry(&intent, &guard, AuditOutcome::Applied, None, None);
    // The dialog says `Apply changes`; the line says what was done.
    assert_eq!(entry.action, "Edit YAML");
    assert_eq!(entry.fields.len(), 1);
    assert_eq!(entry.fields[0].path, "spec.replicas");
    assert_eq!(entry.fields[0].value, None);
    let json = serde_json::to_string(&entry).expect("a line");
    assert!(!json.contains("\"value\""), "{json}");
}

/// The audit entry a delete of `kind` named `name` would write for `item` of its batch.
fn delete_entry(kind: cluster::ObjectKind, name: &str) -> AuditEntry {
    removal_entry(Removal::Delete, kind, name)
}

/// The same for a removal of any kind of the 0033 start (a restart or an eviction too).
fn removal_entry(removal: Removal, kind: cluster::ObjectKind, name: &str) -> AuditEntry {
    use crate::app_shell::object_delete::{DeleteExtras, DeleteTarget, TargetFacts, delete_batch};
    use cluster::{DeletePropagation, ObjectIdentity, ObjectRef};

    let namespace = kind.is_namespaced().then(|| "team-a".to_owned());
    let target = DeleteTarget {
        object: ObjectRef::new(kind, namespace, name.to_owned()).expect("a valid object"),
        identity: ObjectIdentity {
            uid: "u-1".to_owned(),
            finalizers: Vec::new(),
            deletion_started: None,
        },
        facts: TargetFacts::Plain,
    };
    let cluster = crate::cluster_registry::ClusterRef {
        kubeconfig: PathBuf::from("test.yaml"),
        context: "stg-b".to_owned(),
    };
    let extras = DeleteExtras {
        removal,
        propagation: DeletePropagation::Foreground,
        kind,
        targets: vec![target],
        already_gone: Vec::new(),
    };
    let batch = delete_batch(&cluster, "stg-b", extras, jiff::Timestamp::UNIX_EPOCH);
    let intent = batch.item_intent(&batch.plan.items[0]);
    let access = AccessState::Unknown;
    let guard = test_guard(&access, WriteLock::Unlocked, "stg-b", Environment::STAGING);
    audit_entry(&intent, &guard, AuditOutcome::Applied, None, None)
}

#[test]
fn delete_records_propagation_only() {
    let entry = delete_entry(cluster::ObjectKind::Deployment, "api");
    assert_eq!(entry.action, "Delete");
    assert_eq!(entry.fields.len(), 1);
    assert_eq!(entry.fields[0].path, "deleteOptions.propagationPolicy");
    assert_eq!(entry.fields[0].value.as_deref(), Some("Foreground"));
    let json = serde_json::to_string(&entry).expect("a line");
    // The uid is a pin for the request, not something the log keeps.
    assert!(!json.contains("u-1"), "{json}");
}

#[test]
fn delete_of_a_secret_or_config_map_keeps_the_path_and_drops_the_value() {
    for (kind, name) in [
        (cluster::ObjectKind::Secret, "db"),
        (cluster::ObjectKind::ConfigMap, "settings"),
    ] {
        let entry = delete_entry(kind, name);
        assert_eq!(entry.fields[0].path, "deleteOptions.propagationPolicy");
        assert_eq!(entry.fields[0].value, None, "{kind:?}");
    }
}

fn summary(outcome: SummaryOutcome) -> NodeSummary {
    NodeSummary {
        node: "wk-04".to_owned(),
        evicted: 21,
        refused: 2,
        failed: 0,
        skipped: 6,
        unknown: 0,
        outcome,
        reason: None,
    }
}

#[test]
fn a_drain_summary_is_one_line_of_counts_and_an_outcome() {
    let access = AccessState::Unknown;
    let guard = test_guard(&access, WriteLock::Unlocked, "stg-b", Environment::STAGING);
    let identity = AuditIdentity::of(&guard);
    let entry = drain_summary_entry(
        &identity,
        &summary(SummaryOutcome::Stuck),
        BudgetPolicy::Respect,
        Some("night shift"),
    );
    let value = serde_json::to_value(&entry).expect("serializes");
    assert_eq!(value["action"], "Drain");
    assert_eq!(value["cluster"], "stg-b");
    assert_eq!(value["object"]["kind"], "Node");
    assert_eq!(value["object"]["name"], "wk-04");
    assert!(value["object"].get("namespace").is_none());
    assert_eq!(value["outcome"], "stuck");
    assert_eq!(value["note"], "night shift");
    let fields: Vec<(String, String)> = value["fields"]
        .as_array()
        .expect("fields")
        .iter()
        .map(|field| {
            (
                field["path"].as_str().unwrap_or_default().to_owned(),
                field["value"].as_str().unwrap_or_default().to_owned(),
            )
        })
        .collect();
    assert_eq!(
        fields,
        [
            ("evicted".to_owned(), "21".to_owned()),
            ("refused".to_owned(), "2".to_owned()),
            ("failed".to_owned(), "0".to_owned()),
            ("skipped".to_owned(), "6".to_owned()),
        ]
    );
}

#[test]
fn every_drain_outcome_has_its_own_word() {
    let access = AccessState::Unknown;
    let guard = test_guard(&access, WriteLock::Unlocked, "stg-b", Environment::STAGING);
    let identity = AuditIdentity::of(&guard);
    for (outcome, word) in [
        (SummaryOutcome::Drained, "drained"),
        (SummaryOutcome::Stuck, "stuck"),
        (SummaryOutcome::Cancelled, "cancelled"),
        (SummaryOutcome::Stopped, "stopped"),
        (SummaryOutcome::Abandoned, "abandoned"),
    ] {
        let entry = drain_summary_entry(&identity, &summary(outcome), BudgetPolicy::Respect, None);
        let value = serde_json::to_value(&entry).expect("serializes");
        assert_eq!(value["outcome"], word);
        assert!(value.get("note").is_none());
    }
}

#[test]
fn a_stuck_summary_records_why_in_the_error() {
    let access = AccessState::Unknown;
    let guard = test_guard(&access, WriteLock::Unlocked, "stg-b", Environment::STAGING);
    let identity = AuditIdentity::of(&guard);
    let stuck = NodeSummary {
        reason: Some("Timed out after 5m: 2 pods left".into()),
        ..summary(SummaryOutcome::Stuck)
    };
    let value = serde_json::to_value(drain_summary_entry(
        &identity,
        &stuck,
        BudgetPolicy::Respect,
        None,
    ))
    .expect("JSON");
    assert_eq!(value["error"], "Timed out after 5m: 2 pods left");
    let drained = serde_json::to_value(drain_summary_entry(
        &identity,
        &summary(SummaryOutcome::Drained),
        BudgetPolicy::Respect,
        None,
    ))
    .expect("JSON");
    assert!(drained.get("error").is_none());
}

#[test]
fn the_unknown_count_is_a_field_only_when_there_is_one() {
    let access = AccessState::Unknown;
    let guard = test_guard(&access, WriteLock::Unlocked, "stg-b", Environment::STAGING);
    let identity = AuditIdentity::of(&guard);
    let value = |unknown| {
        let line = NodeSummary {
            unknown,
            ..summary(SummaryOutcome::Stopped)
        };
        serde_json::to_value(drain_summary_entry(
            &identity,
            &line,
            BudgetPolicy::Respect,
            None,
        ))
        .expect("JSON")
    };
    assert_eq!(value(0)["fields"].as_array().map(Vec::len), Some(4));
    let with = value(1);
    assert_eq!(
        with["fields"][4],
        serde_json::json!({"path": "unknown", "value": "1"})
    );
}

#[test]
fn a_commit_in_the_air_at_quit_is_an_unknown_line() {
    use crate::drain_plan::PodKey;
    let access = AccessState::Unknown;
    let guard = test_guard(&access, WriteLock::Unlocked, "stg-b", Environment::STAGING);
    let identity = AuditIdentity::of(&guard);
    let evict = NextStep::Evict(PodKey {
        namespace: "payments".to_owned(),
        name: "api-1".to_owned(),
        uid: "u-1".to_owned(),
    });
    let options = DrainOptions {
        grace: GracePeriod::Seconds(30),
        ..DrainOptions::default()
    };
    let entry = drain_in_flight_entry(&identity, &evict, &options, Some("note")).expect("a line");
    let value = serde_json::to_value(entry).expect("JSON");
    assert_eq!(value["action"], "Evict");
    assert_eq!(value["outcome"], "unknown");
    assert_eq!(value["object"]["kind"], "Pod");
    assert_eq!(value["object"]["namespace"], "payments");
    assert_eq!(
        value["fields"][0],
        serde_json::json!({"path": "pods/eviction", "value": "grace 30s"})
    );
    assert_eq!(value["note"], "note");
    assert!(
        value["error"]
            .as_str()
            .is_some_and(|text| text.contains("in flight"))
    );
    let cordon = drain_in_flight_entry(
        &identity,
        &NextStep::Cordon("wk-04".to_owned()),
        &DrainOptions::default(),
        None,
    )
    .expect("a line");
    let value = serde_json::to_value(cordon).expect("JSON");
    assert_eq!(
        (value["action"].as_str(), value["object"]["name"].as_str()),
        (Some("Cordon"), Some("wk-04"))
    );
    // A read or a dry-run has nothing to record.
    assert!(
        drain_in_flight_entry(&identity, &NextStep::Poll, &DrainOptions::default(), None).is_none()
    );
}

fn entry_named(action: &str) -> AuditEntry {
    let mut entry = full_entry();
    action.clone_into(&mut entry.action);
    entry
}

fn actions_in(dir: &Path) -> Vec<String> {
    std::fs::read_to_string(audit_path(dir))
        .expect("the log exists")
        .lines()
        .map(|line| {
            let value: Value = serde_json::from_str(line).expect("each line is JSON");
            value["action"].as_str().unwrap_or_default().to_owned()
        })
        .collect()
}

#[test]
fn lines_land_in_submission_order_whoever_waits() {
    let dir = temp_dir("order");
    // Queued without waiting, as the main thread does for an Open line...
    let receipts: Vec<AuditReceipt> = (0..100)
        .map(|index| submit_audit(&dir, &entry_named(&format!("line-{index:03}"))))
        .collect();
    // ...and a blocking append from another thread, as the tokio runtime does for a Delete line,
    // which lands behind everything queued before it.
    let last = std::thread::scope(|scope| {
        scope
            .spawn(|| append_audit(&dir, &entry_named("line-100")))
            .join()
            .expect("the thread finished")
    });
    last.expect("the last line");
    for receipt in receipts {
        futures::executor::block_on(receipt).expect("a queued line");
    }
    let expected: Vec<String> = (0..=100).map(|index| format!("line-{index:03}")).collect();
    assert_eq!(actions_in(&dir), expected);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_line_that_cannot_be_written_fails_its_own_receipt_only() {
    let dir = temp_dir("receipt");
    let missing = dir.join("not-there");
    let failed = submit_audit(&missing, &entry_named("lost"));
    let kept = submit_audit(&dir, &entry_named("kept"));
    assert!(futures::executor::block_on(failed).is_err());
    futures::executor::block_on(kept).expect("the next line still lands");
    assert_eq!(actions_in(&dir), ["kept"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn restart_pod_records_its_own_action_and_the_propagation() {
    let entry = removal_entry(Removal::Restart, cluster::ObjectKind::Pod, "api-0");
    assert_eq!(entry.action, "Restart pod");
    assert_eq!(entry.fields.len(), 1);
    assert_eq!(entry.fields[0].path, "deleteOptions.propagationPolicy");
    assert_eq!(entry.fields[0].value.as_deref(), Some("Foreground"));
}

#[test]
fn evict_records_the_grace_period_and_no_body() {
    let entry = removal_entry(Removal::Evict, cluster::ObjectKind::Pod, "api-0");
    assert_eq!(entry.action, "Evict");
    assert_eq!(
        entry.object.as_ref().map(|object| object.name.as_str()),
        Some("api-0")
    );
    let fields: Vec<(&str, Option<&str>)> = entry
        .fields
        .iter()
        .map(|field| (field.path.as_str(), field.value.as_deref()))
        .collect();
    assert_eq!(fields, [("pods/eviction", Some("grace pod default"))]);
}

#[test]
fn skip_pdbs_summary_records_disable_eviction() {
    let access = AccessState::Unknown;
    let guard = test_guard(&access, WriteLock::Unlocked, "stg-b", Environment::STAGING);
    let identity = AuditIdentity::of(&guard);
    let line = summary(SummaryOutcome::Drained);
    let fields = |budgets| {
        let entry = drain_summary_entry(&identity, &line, budgets, None);
        serde_json::to_value(entry).expect("JSON")["fields"].clone()
    };
    // A drain that respects the budgets evicted its pods and says so.
    let respect = fields(BudgetPolicy::Respect);
    assert!(respect.as_array().is_some_and(|list| {
        list.iter().all(|field| field["path"] != "disable_eviction")
            && list.iter().any(|field| field["path"] == "evicted")
    }));
    let skip = fields(BudgetPolicy::Skip);
    let paths: Vec<&str> = skip
        .as_array()
        .expect("fields")
        .iter()
        .filter_map(|field| field["path"].as_str())
        .collect();
    assert_eq!(
        paths,
        [
            "deleted",
            "refused",
            "failed",
            "skipped",
            "disable_eviction"
        ]
    );
    // The count of the removed pods is the same one under its other name.
    assert_eq!(skip[0]["value"], respect[0]["value"]);
    assert_eq!(skip[4]["value"], "true");
}

#[test]
fn a_skip_pdbs_commit_in_the_air_is_a_delete_line() {
    use crate::drain_plan::PodKey;
    let access = AccessState::Unknown;
    let guard = test_guard(&access, WriteLock::Unlocked, "stg-b", Environment::STAGING);
    let identity = AuditIdentity::of(&guard);
    let step = NextStep::Evict(PodKey {
        namespace: "payments".to_owned(),
        name: "api-1".to_owned(),
        uid: "u-1".to_owned(),
    });
    let options = DrainOptions {
        budgets: BudgetPolicy::Skip,
        ..DrainOptions::default()
    };
    let entry = drain_in_flight_entry(&identity, &step, &options, None).expect("a line");
    let value = serde_json::to_value(entry).expect("JSON");
    assert_eq!(value["action"], "Delete");
    assert_eq!(
        value["fields"][0],
        serde_json::json!({"path": "deleteOptions.propagationPolicy", "value": "Background"})
    );
}

#[test]
fn audit_line_for_create() {
    use crate::object_create_view::create_intent;
    use cluster::{ObjectDraft, ObjectKind, WriteOperation, WriteRequest};

    let text = "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: new-config\n  namespace: payments\ndata:\n  PASSWORD: S3cr3t-0042\n";
    let draft = ObjectDraft::new(ObjectKind::ConfigMap, text).expect("a valid draft");
    let request = WriteRequest::new(
        draft.target().clone(),
        WriteOperation::CreateObject(Box::new(draft)),
    )
    .expect("a creatable kind");
    let cluster = crate::cluster_registry::ClusterRef {
        kubeconfig: PathBuf::from("test.yaml"),
        context: "stg-b".to_owned(),
    };
    let intent = create_intent(
        &cluster,
        &"stg-b".into(),
        ObjectKind::ConfigMap,
        request,
        Vec::new(),
    );
    let access = AccessState::Unknown;
    let guard = test_guard(&access, WriteLock::Unlocked, "stg-b", Environment::STAGING);
    let entry = audit_entry(&intent, &guard, AuditOutcome::Applied, None, None);
    assert_eq!(entry.action, "Create");
    let paths: Vec<_> = entry
        .fields
        .iter()
        .map(|field| field.path.as_str())
        .collect();
    assert_eq!(
        paths,
        ["metadata.name", "metadata.namespace", "data[PASSWORD]"]
    );
    // A ConfigMap records paths only: no field value, and no name either.
    assert!(entry.fields.iter().all(|field| field.value.is_none()));
    let json = serde_json::to_string(&entry).expect("a line");
    assert!(!json.contains("S3cr3t-0042"), "{json}");
}

#[test]
fn a_taint_edit_records_each_taint_that_changed() {
    use crate::node_edits::{NodeScope, TaintRow, taint_intent};
    use cluster::{NodeEdit, NodeTaint};

    let taint = |key: &str, value: &str| NodeTaint {
        key: key.to_owned(),
        value: Some(value.to_owned()),
        effect: "NoSchedule".to_owned(),
        time_added: None,
    };
    let edit = NodeEdit {
        taints: vec![taint("conflict", "4"), taint("workload", "data")],
        labels: std::collections::BTreeMap::new(),
        resource_version: "7".to_owned(),
    };
    let row = |key: &str, value: &str| TaintRow {
        key: key.to_owned(),
        value: value.to_owned(),
        effect: "NoSchedule".to_owned(),
        time_added: None,
    };
    let cluster = crate::cluster_registry::ClusterRef {
        kubeconfig: PathBuf::from("test.yaml"),
        context: "stg-b".to_owned(),
    };
    let scope = NodeScope {
        cluster: &cluster,
        cluster_name: "stg-b",
    };
    let rows = [row("conflict", "3"), row("maintenance", "true")];
    let intent = taint_intent(&scope, "wk-04", &edit, &rows).expect("a valid edit");
    let access = AccessState::Unknown;
    let guard = test_guard(&access, WriteLock::Unlocked, "stg-b", Environment::STAGING);
    let entry = audit_entry(&intent, &guard, AuditOutcome::Applied, None, None);
    let paths: Vec<&str> = entry
        .fields
        .iter()
        .map(|field| field.path.as_str())
        .collect();
    assert_eq!(
        paths,
        [
            "conflict 4 → 3",
            "− workload=data:NoSchedule",
            "+ maintenance=true:NoSchedule",
        ]
    );
    assert!(entry.fields.iter().all(|field| field.value.is_none()));
}

#[test]
fn a_scale_line_keeps_the_old_and_the_new_count() {
    use cluster::{ObjectKind, ObjectRef, WriteOperation, WriteRequest};

    let cluster = crate::cluster_registry::ClusterRef {
        kubeconfig: PathBuf::from("test.yaml"),
        context: "stg-b".to_owned(),
    };
    let target = ObjectRef::new(
        ObjectKind::Deployment,
        Some("shop".to_owned()),
        "web".to_owned(),
    )
    .expect("a deployment");
    let operation = WriteOperation::ScaleWorkload {
        replicas: 0,
        previous: 1,
    };
    let request = WriteRequest::new(target, operation).expect("a deployment scales");
    let intent = WriteIntent {
        cluster,
        cluster_name: "stg-b".into(),
        action: crate::resource_actions::ResourceAction::Scale(ObjectKind::Deployment),
        label: "Scale deployment web from 1 to 0".into(),
        button: "Scale".into(),
        request,
        risk: crate::write_guard::ActionRisk::Destructive,
        warnings: Vec::new(),
        change_lines: Vec::new(),
        audit_fields: Vec::new(),
    };
    let access = AccessState::Unknown;
    let guard = test_guard(&access, WriteLock::Unlocked, "stg-b", Environment::STAGING);
    let entry = audit_entry(&intent, &guard, AuditOutcome::Applied, None, None);
    let json = serde_json::to_value(&entry.fields).expect("fields serialize");
    assert_eq!(
        json,
        serde_json::json!([{ "path": "spec.replicas", "value": "0", "from": "1" }])
    );
}

#[test]
fn a_path_only_kind_drops_the_old_value_too() {
    let fields = vec![AuditField {
        from: Some("hunter2".to_owned()),
        ..field("data.password", Some("hunter3"))
    }];
    let recorded = recordable_fields("Secret", fields);
    assert!(recorded[0].value.is_none() && recorded[0].from.is_none());
}

#[test]
fn an_edit_line_refines_its_path_with_the_values_the_intent_knows() {
    use crate::app_shell::write_flow::WriteIntent;
    use crate::yaml_edit::edit_intent;
    use crate::yaml_edit::yaml_edit_tests::sample_edit;
    use cluster::{ObjectKind, ObjectRef, WriteOperation, WriteRequest};

    let target = ObjectRef::new(
        ObjectKind::Deployment,
        Some("payments".to_owned()),
        "api".to_owned(),
    )
    .expect("a deployment");
    let request = WriteRequest::new(
        target,
        WriteOperation::ReplaceObject(Box::new(sample_edit())),
    )
    .expect("an editable kind");
    let cluster = crate::cluster_registry::ClusterRef {
        kubeconfig: PathBuf::from("test.yaml"),
        context: "stg-b".to_owned(),
    };
    let mut intent: WriteIntent = edit_intent(
        &cluster,
        &"stg-b".into(),
        ObjectKind::Deployment,
        request,
        Vec::new(),
    );
    intent.audit_fields = vec![
        AuditField {
            from: Some("3".to_owned()),
            ..field("spec.replicas", Some("4"))
        },
        AuditField {
            from: None,
            ..field("metadata.labels.team", Some("platform"))
        },
    ];
    let access = AccessState::Unknown;
    let guard = test_guard(&access, WriteLock::Unlocked, "stg-b", Environment::STAGING);
    let entry = audit_entry(&intent, &guard, AuditOutcome::Applied, None, None);
    let json = serde_json::to_value(&entry.fields).expect("fields serialize");
    assert_eq!(
        json,
        serde_json::json!([
            { "path": "spec.replicas", "value": "4", "from": "3" },
            { "path": "metadata.labels.team", "value": "platform" },
        ])
    );
}

#[test]
fn a_secret_edit_keeps_no_values_even_when_the_intent_knows_them() {
    let fields = vec![AuditField {
        from: Some("old".to_owned()),
        ..field("metadata.labels.team", Some("new"))
    }];
    let recorded = recordable_fields("Secret", fields);
    assert!(recorded[0].value.is_none() && recorded[0].from.is_none());
}
