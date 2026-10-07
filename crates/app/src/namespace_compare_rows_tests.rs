use cluster::fake_api::FakeApi;
use cluster::{EnvValues, WritePolicy};
use serde_json::{Value, json};

use super::*;
use crate::yaml_diff::DiffRowKind;

fn deployment(replicas: u32, image: &str) -> Value {
    json!({
        "metadata": {"name": "web"},
        "spec": {
            "replicas": replicas,
            "template": {"spec": {"containers": [{"name": "web", "image": image}]}},
        },
    })
}

fn config_map(name: &str, data: Value) -> Value {
    json!({"metadata": {"name": name}, "data": data})
}

fn secret(password: &str) -> Value {
    json!({
        "metadata": {"name": "web-secret"},
        "type": "Opaque",
        "data": {"password": password},
    })
}

/// What a fake server lists for `kind` in `namespace`.
fn listed(namespace: &str, resource: &str) -> Vec<Value> {
    match (namespace, resource) {
        ("lab-shop", "deployments") => vec![deployment(1, "web:1")],
        ("lab-shop-stg", "deployments") => vec![deployment(2, "web:2")],
        ("lab-shop", "configmaps") => vec![config_map("web-config", json!({"LOG_LEVEL": "info"}))],
        ("lab-shop-stg", "configmaps") => vec![
            config_map("web-config", json!({"LOG_LEVEL": "debug"})),
            config_map("stg-only", json!({"A": "1"})),
        ],
        ("lab-shop", "secrets") => vec![secret("c2VjcmV0LW9uZQ==")],
        ("lab-shop-stg", "secrets") => vec![secret("c2VjcmV0LXR3bw==")],
        _ => Vec::new(),
    }
}

fn compared() -> NamespaceComparison {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime");
    runtime.block_on(async {
        let (connection, _api) = FakeApi::connection(WritePolicy::Blocked, |request| {
            let mut parts = request.path.rsplit('/');
            let resource = parts.next().unwrap_or_default();
            let namespace = parts.next().unwrap_or_default();
            let items = listed(namespace, resource);
            let list = json!({"apiVersion": "v1", "kind": "List", "metadata": {}, "items": items});
            (200, list.to_string())
        });
        connection
            .compare_namespaces("lab-shop", "lab-shop-stg", EnvValues::Hidden)
            .await
    })
}

fn texts(lines: &[CompareLine]) -> Vec<String> {
    lines
        .iter()
        .map(|line| match line {
            CompareLine::Kind { title, summary } => format!("kind {title} | {summary}"),
            CompareLine::Group(label) => format!("group {label}"),
            CompareLine::Name(name) => format!("name {name}"),
            CompareLine::Object { name, is_open, .. } => format!("object {name} open={is_open}"),
            CompareLine::Change(text) => format!("change {text}"),
            CompareLine::Diff(_) => "diff".to_owned(),
            CompareLine::Note(text) => format!("note {text}"),
        })
        .collect()
}

#[test]
fn the_summary_counts_every_group() {
    assert_eq!(
        summary_text(&compared()),
        "lab-shop ↔ lab-shop-stg · 3 differ · 0 only in lab-shop · 1 only in lab-shop-stg · 0 same"
    );
}

#[test]
fn each_kind_lists_only_in_groups_then_the_differing_objects_with_their_changes() {
    let lines = texts(&compare_lines(&compared(), &OpenDiffs::new()));
    let deployment_at = lines
        .iter()
        .position(|line| line.starts_with("kind Deployment"))
        .expect("Deployment differs");
    assert_eq!(
        &lines[deployment_at..deployment_at + 6],
        [
            "kind Deployment | 1 differ · 0 only in lab-shop · 0 only in lab-shop-stg",
            "group Differs (1)",
            "object web open=false",
            "change spec.replicas: 1 → 2",
            "change spec.template.spec.containers[web].image: web:1 → web:2",
            "kind ConfigMap | 1 differ · 0 only in lab-shop · 1 only in lab-shop-stg",
        ]
    );
    let config_map_at = deployment_at + 5;
    assert_eq!(
        &lines[config_map_at + 1..config_map_at + 7],
        [
            "group Only in lab-shop-stg (1)",
            "name stg-only",
            "group Differs (1)",
            "object web-config open=false",
            "change data.LOG_LEVEL: info → debug",
            "kind Secret | 1 differ · 0 only in lab-shop · 0 only in lab-shop-stg",
        ]
    );
}

#[test]
fn a_secret_line_names_the_key_and_never_a_value() {
    let lines = texts(&compare_lines(&compared(), &OpenDiffs::new()));
    let change = lines
        .iter()
        .find(|line| line.starts_with("change data.password"))
        .expect("the password differs");
    assert!(change.contains("<hash:"), "{change}");
    assert!(lines.iter().all(|line| !line.contains("c2VjcmV0")));
    let open = OpenDiffs::from([(ObjectKind::Secret, "web-secret".to_owned())]);
    let with_diff = compare_lines(&compared(), &open);
    for line in &with_diff {
        if let CompareLine::Diff(row) = line {
            assert!(!row.text.contains("c2VjcmV0"), "{}", row.text);
        }
    }
}

#[test]
fn open_diff_adds_the_line_diff_of_that_object_only() {
    let open = OpenDiffs::from([(ObjectKind::Deployment, "web".to_owned())]);
    let lines = compare_lines(&compared(), &open);
    let diffs: Vec<&DiffRow> = lines
        .iter()
        .filter_map(|line| match line {
            CompareLine::Diff(row) => Some(row),
            _ => None,
        })
        .collect();
    assert!(
        diffs
            .iter()
            .any(|row| row.kind == DiffRowKind::Removed && row.text.contains("web:1"))
    );
    assert!(
        diffs
            .iter()
            .any(|row| row.kind == DiffRowKind::Added && row.text.contains("web:2"))
    );
    let opened = texts(&lines);
    assert!(opened.contains(&"object web open=true".to_owned()));
    assert!(opened.contains(&"object web-config open=false".to_owned()));
}

#[test]
fn an_unreadable_kind_is_one_note_and_the_rest_still_shows() {
    let mut comparison = compared();
    comparison.kinds.push(KindComparison {
        kind: ObjectKind::Ingress,
        outcome: KindOutcome::Unreadable("forbidden".to_owned()),
    });
    let lines = texts(&compare_lines(&comparison, &OpenDiffs::new()));
    let at = lines
        .iter()
        .position(|line| line.starts_with("kind Ingress"))
        .expect("the kind is named");
    assert_eq!(lines[at + 1], "note Could not be read: forbidden");
    assert!(lines.iter().any(|line| line.starts_with("kind Deployment")));
}

#[test]
fn equal_namespaces_say_so_with_the_object_count() {
    let comparison = NamespaceComparison {
        left: "a".to_owned(),
        right: "b".to_owned(),
        hidden_env_values: 0,
        kinds: vec![KindComparison {
            kind: ObjectKind::ConfigMap,
            outcome: KindOutcome::Compared {
                only_left: Vec::new(),
                only_right: Vec::new(),
                differs: Vec::new(),
                same: 3,
            },
        }],
    };
    assert_eq!(
        compare_lines(&comparison, &OpenDiffs::new()),
        [CompareLine::Note(
            "The two namespaces are the same (3 objects).".to_owned()
        )]
    );
}

#[test]
fn hidden_env_values_get_a_note_only_when_there_are_some() {
    let mut comparison = compared();
    assert_eq!(hidden_env_note(&comparison), None);
    comparison.hidden_env_values = 2;
    assert!(
        hidden_env_note(&comparison)
            .is_some_and(|note| note.starts_with("2 env values are hidden"))
    );
}
