use std::path::PathBuf;

use cluster::WriteOperation;

use super::*;
use crate::app_shell::batch_write::MAX_BATCH_ITEMS;

fn cluster() -> ClusterRef {
    ClusterRef {
        kubeconfig: PathBuf::from("/home/me/.kube/config"),
        context: "prod-ctx".to_owned(),
    }
}

fn scope(cluster: &ClusterRef) -> NodeScope<'_> {
    NodeScope {
        cluster,
        cluster_name: "prod-a",
    }
}

fn taint(key: &str, value: Option<&str>, effect: &str) -> NodeTaint {
    NodeTaint {
        key: key.to_owned(),
        value: value.map(str::to_owned),
        effect: effect.to_owned(),
        time_added: None,
    }
}

fn unreachable_taint() -> NodeTaint {
    NodeTaint {
        time_added: Some("2026-10-02T08:00:00Z".parse().expect("a timestamp")),
        ..taint("node.kubernetes.io/unreachable", None, "NoExecute")
    }
}

fn edit_with(taints: Vec<NodeTaint>, labels: &[(&str, &str)]) -> NodeEdit {
    NodeEdit {
        taints,
        labels: labels
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect(),
        resource_version: "42".to_owned(),
    }
}

fn row(key: &str, value: &str, effect: &str) -> TaintRow {
    TaintRow {
        key: key.to_owned(),
        value: value.to_owned(),
        effect: effect.to_owned(),
        time_added: None,
    }
}

fn label(key: &str, value: &str) -> LabelRow {
    LabelRow {
        key: key.to_owned(),
        value: value.to_owned(),
    }
}

fn node_edit() -> NodeEdit {
    edit_with(
        vec![
            taint("dedicated", Some("ingress"), "NoSchedule"),
            unreachable_taint(),
        ],
        &[
            ("kubernetes.io/hostname", "wk-04"),
            ("node-role.kubernetes.io/worker", ""),
            ("team", "infra"),
        ],
    )
}

fn taint_result(rows: &[TaintRow]) -> Result<WriteIntent, SharedString> {
    let cluster = cluster();
    taint_intent(&scope(&cluster), "wk-04", &node_edit(), rows)
}

fn label_result(rows: &[LabelRow]) -> Result<WriteIntent, SharedString> {
    let cluster = cluster();
    label_intent(&scope(&cluster), "wk-04", &node_edit(), rows)
}

fn error_text<T>(result: Result<T, SharedString>) -> String {
    match result {
        Ok(_) => panic!("expected an error"),
        Err(text) => text.to_string(),
    }
}

#[test]
fn system_taints_are_read_only() {
    assert!(is_system_taint("node.kubernetes.io/unschedulable"));
    assert!(is_system_taint(
        "node.cloudprovider.kubernetes.io/uninitialized"
    ));
    assert!(!is_system_taint("dedicated"));
    assert!(!is_system_taint("example.com/node.kubernetes.io/x"));
    assert!(row("node.kubernetes.io/unreachable", "", "NoExecute").is_read_only());
}

#[test]
fn kubelet_labels_are_read_only_but_node_role_is_editable() {
    for key in [
        "kubernetes.io/hostname",
        "beta.kubernetes.io/os",
        "topology.kubernetes.io/zone",
        "node.kubernetes.io/instance-type",
    ] {
        assert!(is_kubelet_label(key), "{key}");
    }
    assert!(!is_kubelet_label("node-role.kubernetes.io/worker"));
    assert!(!is_kubelet_label("team"));
    assert!(!label("team", "x").is_read_only());
}

#[test]
fn rows_start_from_the_node() {
    let edit = node_edit();
    let taints = taint_rows(&edit);
    assert_eq!(taints.len(), 2);
    assert_eq!(taints[0].value, "ingress");
    assert!(taints[1].time_added.is_some());
    assert!(taints[1].is_read_only());
    let labels = label_rows(&edit);
    assert_eq!(labels.len(), 3);
    assert_eq!(labels[0].key, "kubernetes.io/hostname");
}

#[test]
fn taint_intent_keeps_system_rows_and_time_added() {
    let mut rows = taint_rows(&node_edit());
    rows.push(row("gpu", "true", "NoSchedule"));
    let intent = taint_result(&rows).expect("a valid edit");
    let WriteOperation::SetNodeTaints {
        taints,
        resource_version,
        previous,
    } = intent.request.operation()
    else {
        panic!("expected SetNodeTaints");
    };
    assert_eq!(resource_version, "42");
    // The summary and the audit line are made against the taints the editor read.
    assert_eq!(*previous, node_edit().taints);
    assert_eq!(taints.len(), 3);
    assert_eq!(taints[1], unreachable_taint());
    assert_eq!(taints[2], taint("gpu", Some("true"), "NoSchedule"));
    assert_eq!(intent.action, ResourceAction::EditTaints);
    assert_eq!(intent.label, "Edit taints of node wk-04");
    assert_eq!(intent.expected(), "wk-04");
    assert_eq!(intent.cluster, cluster());
}

#[test]
fn adding_no_execute_is_destructive() {
    let mut rows = taint_rows(&node_edit());
    rows.push(row("maintenance", "", "NoExecute"));
    let intent = taint_result(&rows).expect("a valid edit");
    assert_eq!(intent.risk, ActionRisk::Destructive);
    assert_eq!(
        intent.warnings,
        [SharedString::from(
            "NoExecute evicts pods that do not tolerate it"
        )]
    );
}

#[test]
fn changing_the_value_of_an_existing_no_execute_taint_is_destructive() {
    let edit = edit_with(vec![taint("maintenance", Some("a"), "NoExecute")], &[]);
    let cluster = cluster();
    let intent = taint_intent(
        &scope(&cluster),
        "wk-04",
        &edit,
        &[row("maintenance", "b", "NoExecute")],
    )
    .expect("a valid edit");
    // The pods that tolerated the old value are evicted at once.
    assert_eq!(intent.risk, ActionRisk::Destructive);
    assert_eq!(intent.warnings.len(), 1);
}

#[test]
fn a_no_execute_taint_kept_as_it_is_does_not_make_the_edit_destructive() {
    let edit = edit_with(vec![taint("maintenance", Some("a"), "NoExecute")], &[]);
    let cluster = cluster();
    let intent = taint_intent(
        &scope(&cluster),
        "wk-04",
        &edit,
        &[
            row("maintenance", "a", "NoExecute"),
            row("gpu", "", "NoSchedule"),
        ],
    )
    .expect("a valid edit");
    assert_eq!(intent.risk, ActionRisk::Change);
    assert!(intent.warnings.is_empty());
}

#[test]
fn removing_no_execute_is_change() {
    let edit = edit_with(vec![taint("maintenance", None, "NoExecute")], &[]);
    let cluster = cluster();
    let intent = taint_intent(&scope(&cluster), "wk-04", &edit, &[]).expect("a valid edit");
    assert_eq!(intent.risk, ActionRisk::Change);
    let WriteOperation::SetNodeTaints { taints, .. } = intent.request.operation() else {
        panic!("expected SetNodeTaints");
    };
    assert!(taints.is_empty());
}

#[test]
fn a_managed_taint_cannot_be_removed_changed_or_added() {
    let mut without = taint_rows(&node_edit());
    without.remove(1);
    assert_eq!(
        error_text(taint_result(&without)),
        "node.kubernetes.io/unreachable is managed by Kubernetes"
    );
    let mut changed = taint_rows(&node_edit());
    changed[1].value = "x".to_owned();
    assert!(error_text(taint_result(&changed)).contains("managed by Kubernetes"));
    let mut added = taint_rows(&node_edit());
    added.push(row("node.kubernetes.io/not-ready", "", "NoExecute"));
    assert!(error_text(taint_result(&added)).contains("managed by Kubernetes"));
}

#[test]
fn duplicate_keys_are_rejected() {
    let mut rows = taint_rows(&node_edit());
    rows.push(row("dedicated", "other", "NoSchedule"));
    assert_eq!(
        error_text(taint_result(&rows)),
        "dedicated:NoSchedule is listed twice"
    );
    // The same key with another effect is a different taint.
    let mut rows = taint_rows(&node_edit());
    rows.push(row("dedicated", "ingress", "NoExecute"));
    assert!(taint_result(&rows).is_ok());
    let labels = [label("team", "a"), label("team", "b")];
    assert_eq!(error_text(label_result(&labels)), "team is listed twice");
}

#[test]
fn a_taint_needs_a_key_a_known_effect_and_kubernetes_text() {
    let mut rows = taint_rows(&node_edit());
    rows.push(row("  ", "", "NoSchedule"));
    assert_eq!(
        error_text(taint_result(&rows)),
        "Enter a key for every taint"
    );
    let mut rows = taint_rows(&node_edit());
    rows.push(row("gpu", "", "Evict"));
    assert_eq!(
        error_text(taint_result(&rows)),
        "Evict is not a taint effect"
    );
    let mut rows = taint_rows(&node_edit());
    rows.push(row("bad key", "", "NoSchedule"));
    assert_eq!(
        error_text(taint_result(&rows)),
        format!(
            "Row {}: key 'bad key' is not a valid Kubernetes key",
            rows.len()
        )
    );
    let mut rows = taint_rows(&node_edit());
    rows.push(row("gpu", "has space", "NoSchedule"));
    assert_eq!(
        error_text(taint_result(&rows)),
        format!(
            "Row {}: value 'has space' is not valid (letters, digits, - _ ., max 63)",
            rows.len()
        )
    );
}

#[test]
fn no_changes_disables_review() {
    assert_eq!(
        error_text(taint_result(&taint_rows(&node_edit()))),
        "No changes"
    );
    assert_eq!(
        error_text(label_result(&label_rows(&node_edit()))),
        "No changes"
    );
}

#[test]
fn surrounding_spaces_do_not_count_as_a_change() {
    let mut rows = taint_rows(&node_edit());
    rows[0].value = " ingress ".to_owned();
    rows[0].key = " dedicated".to_owned();
    assert_eq!(error_text(taint_result(&rows)), "No changes");
}

#[test]
fn label_intent_sends_only_changes() {
    let rows = [
        label("kubernetes.io/hostname", "wk-04"),
        label("node-role.kubernetes.io/worker", ""),
        label("team", "platform"),
        label("zone-pref", "a"),
    ];
    let intent = label_result(&rows).expect("a valid edit");
    let WriteOperation::SetNodeLabels { changes } = intent.request.operation() else {
        panic!("expected SetNodeLabels");
    };
    let sent: Vec<(&str, Option<&str>)> = changes
        .iter()
        .map(|change| (change.key.as_str(), change.value.as_deref()))
        .collect();
    assert_eq!(sent, [("team", Some("platform")), ("zone-pref", Some("a"))]);
    assert_eq!(intent.risk, ActionRisk::Change);
    assert_eq!(intent.label, "Edit labels of node wk-04");
}

#[test]
fn a_removed_row_removes_its_label() {
    let rows = [
        label("kubernetes.io/hostname", "wk-04"),
        label("node-role.kubernetes.io/worker", ""),
    ];
    let intent = label_result(&rows).expect("a valid edit");
    let WriteOperation::SetNodeLabels { changes } = intent.request.operation() else {
        panic!("expected SetNodeLabels");
    };
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].key, "team");
    assert_eq!(changes[0].value, None);
}

#[test]
fn a_kubelet_label_cannot_change_or_go() {
    let rows = [
        label("kubernetes.io/hostname", "other"),
        label("node-role.kubernetes.io/worker", ""),
        label("team", "infra"),
    ];
    assert_eq!(
        error_text(label_result(&rows)),
        "kubernetes.io/hostname is set by the kubelet"
    );
    let removed = [label("team", "infra")];
    assert!(error_text(label_result(&removed)).contains("set by the kubelet"));
}

#[test]
fn a_label_needs_a_key_and_a_kubernetes_value() {
    let mut rows = label_rows(&node_edit());
    rows.push(label("", "x"));
    assert_eq!(
        error_text(label_result(&rows)),
        "Enter a key for every label"
    );
    let mut rows = label_rows(&node_edit());
    rows.push(label("fine", "not valid!"));
    assert_eq!(
        error_text(label_result(&rows)),
        format!(
            "Row {}: value 'not valid!' is not valid (letters, digits, - _ ., max 63)",
            rows.len()
        )
    );
}

fn ticked(name: &str, scheduling: NodeScheduling) -> TickedNode {
    TickedNode {
        name: name.to_owned(),
        scheduling,
        labels: Vec::new(),
    }
}

#[test]
fn bulk_cordon_is_a_batch_and_skips_already_cordoned() {
    let cluster = cluster();
    let nodes = [
        ticked("wk-01", NodeScheduling::Enabled),
        ticked("wk-02", NodeScheduling::Disabled),
        ticked("wk-03", NodeScheduling::Enabled),
    ];
    let batch = cordon_batch(&scope(&cluster), CordonMode::Cordon, &nodes).expect("a batch");
    assert_eq!(batch.label, "Cordon 2 nodes");
    assert_eq!(batch.action, ResourceAction::Cordon);
    assert_eq!(batch.risk, ActionRisk::Change);
    assert_eq!(batch.plan.items.len(), 2);
    assert_eq!(batch.plan.items[0].label, "Cordon node wk-01");
    assert!(matches!(
        batch.plan.items[0].request.operation(),
        WriteOperation::SetNodeSchedulable { schedulable: false }
    ));
    assert_eq!(batch.plan.skipped.len(), 1);
    assert_eq!(batch.plan.skipped[0].object, "wk-02");
    assert_eq!(batch.plan.skipped[0].reason, "already cordoned");
    assert_eq!(batch.plan.cluster, cluster);
    assert!(matches!(batch.plan.extras, BatchExtras::None));
    // The batch names no single object, so the tier types the cluster name.
    assert_eq!(batch.expected(), "prod-a");
}

#[test]
fn bulk_uncordon_sends_schedulable_and_skips_schedulable_nodes() {
    let cluster = cluster();
    let nodes = [
        ticked("wk-01", NodeScheduling::Disabled),
        ticked("wk-02", NodeScheduling::Enabled),
    ];
    let batch = cordon_batch(&scope(&cluster), CordonMode::Uncordon, &nodes).expect("a batch");
    assert_eq!(batch.label, "Uncordon 1 node");
    assert_eq!(batch.action, ResourceAction::Uncordon);
    assert!(matches!(
        batch.plan.items[0].request.operation(),
        WriteOperation::SetNodeSchedulable { schedulable: true }
    ));
    assert_eq!(batch.plan.skipped[0].reason, "already schedulable");
}

#[test]
fn bulk_cordon_with_every_node_cordoned_says_why() {
    let cluster = cluster();
    let nodes = [ticked("wk-01", NodeScheduling::Disabled)];
    let error = cordon_batch(&scope(&cluster), CordonMode::Cordon, &nodes);
    assert_eq!(error_text(error), "All selected nodes are already cordoned");
    let nodes = [ticked("wk-01", NodeScheduling::Enabled)];
    let error = cordon_batch(&scope(&cluster), CordonMode::Uncordon, &nodes);
    assert_eq!(
        error_text(error),
        "All selected nodes are already schedulable"
    );
}

#[test]
fn bulk_cordon_refuses_an_unsafe_node_name() {
    let cluster = cluster();
    let nodes = [ticked("a/../b", NodeScheduling::Enabled)];
    assert!(cordon_batch(&scope(&cluster), CordonMode::Cordon, &nodes).is_err());
    assert_eq!(MAX_BATCH_ITEMS, 50);
}

// ---- bulk Edit labels (spec 0040) ----

fn labelled(name: &str, labels: &[&str]) -> TickedNode {
    TickedNode {
        labels: labels.iter().map(|term| (*term).to_owned()).collect(),
        ..ticked(name, NodeScheduling::Enabled)
    }
}

fn set(key: &str, value: &str) -> LabelChange {
    LabelChange {
        key: key.to_owned(),
        value: Some(value.to_owned()),
    }
}

fn remove(key: &str) -> LabelChange {
    LabelChange {
        key: key.to_owned(),
        value: None,
    }
}

/// The per-node change lists of a batch, by node.
fn node_changes(batch: &BatchIntent) -> Vec<(String, Vec<LabelChange>)> {
    batch
        .plan
        .items
        .iter()
        .map(|item| {
            let WriteOperation::SetNodeLabels { changes } = item.request.operation() else {
                panic!("a label write");
            };
            (item.object.to_string(), changes.clone())
        })
        .collect()
}

#[test]
fn label_batch_skips_nodes_that_already_match() {
    let cluster = cluster();
    let nodes = [
        labelled("wk-01", &["team=dev", "old-key=x"]),
        labelled("wk-02", &["team=infra"]),
        labelled("wk-03", &["role=worker"]),
    ];
    let changes = [set("team", "infra"), remove("old-key")];
    let batch = label_batch(&scope(&cluster), &nodes, &changes, None).expect("a batch");
    // wk-01 gets both; wk-02 has the Set already and no old key; wk-03 only lacks the Set.
    assert_eq!(
        node_changes(&batch),
        [
            (
                "wk-01".to_owned(),
                vec![remove("old-key"), set("team", "infra")]
            ),
            ("wk-03".to_owned(), vec![set("team", "infra")]),
        ]
    );
    assert_eq!(batch.plan.skipped.len(), 1);
    assert_eq!(batch.plan.skipped[0].object, "wk-02");
    assert_eq!(batch.plan.skipped[0].reason, "already labelled");
    assert_eq!(batch.label, "Edit labels of 2 nodes");
    assert_eq!(batch.verb, "Edit labels");
    assert_eq!(batch.button, "Edit labels");
    assert_eq!(batch.action, ResourceAction::EditLabels);
    assert_eq!(batch.risk, ActionRisk::Change);
    assert_eq!(batch.plan.on_failure, BatchFailure::Continue);
    assert!(matches!(batch.plan.extras, BatchExtras::Labels(_)));
    assert_eq!(batch.plan.items[0].label, "Edit labels of node wk-01");
    // A bulk names no single object, so the tier types the cluster name.
    assert_eq!(batch.expected(), "prod-a");
    // A Remove adds the DaemonSet warning; a Set alone does not.
    assert_eq!(
        batch.warnings,
        [SharedString::from(
            "Removing a label can make DaemonSets that select nodes by it delete their pods on these nodes"
        )]
    );
    let only_set =
        label_batch(&scope(&cluster), &nodes, &[set("team", "infra")], None).expect("a batch");
    assert!(only_set.warnings.is_empty());
}

#[test]
fn label_batch_keeps_a_changed_value_and_treats_an_empty_value_as_a_value() {
    let cluster = cluster();
    // `team=` is a label with an empty value, not a missing one.
    let nodes = [
        labelled("wk-01", &["team="]),
        labelled("wk-02", &["team=infra"]),
    ];
    let batch = label_batch(&scope(&cluster), &nodes, &[set("team", "")], None).expect("a batch");
    assert_eq!(
        node_changes(&batch),
        [("wk-02".to_owned(), vec![set("team", "")])]
    );
    // A Remove of a key a node lacks is dropped for that node.
    let batch = label_batch(&scope(&cluster), &nodes, &[remove("team")], None).expect("a batch");
    assert_eq!(batch.plan.items.len(), 2);
    let none = [labelled("wk-01", &[])];
    assert_eq!(
        error_text(label_batch(
            &scope(&cluster),
            &none,
            &[remove("team")],
            None
        )),
        "All selected nodes already have these labels"
    );
}

#[test]
fn label_batch_checks_in_order() {
    let cluster = cluster();
    let nodes = [labelled("wk-01", &["team=infra"])];
    let try_with =
        |changes: &[LabelChange]| error_text(label_batch(&scope(&cluster), &nodes, changes, None));
    assert_eq!(try_with(&[]), "No changes");
    assert_eq!(try_with(&[set("", "x")]), "Enter a key for every label");
    assert_eq!(try_with(&[set("a", "1"), remove("a")]), "a is listed twice");
    assert_eq!(
        try_with(&[set("kubernetes.io/os", "linux")]),
        "kubernetes.io/os is set by the kubelet"
    );
    assert_eq!(
        try_with(&[set("fine", "not valid!")]),
        "A key or value is not valid for Kubernetes (letters, digits, - _ .)"
    );
    assert_eq!(
        try_with(&[set("team", "infra")]),
        "All selected nodes already have these labels"
    );
    // The first failing check wins: an empty key before a kubelet key and a bad value.
    assert_eq!(
        try_with(&[set("kubernetes.io/os", "bad value!"), set(" ", "x")]),
        "Enter a key for every label"
    );
    assert_eq!(
        try_with(&[set("kubernetes.io/os", "bad value!")]),
        "kubernetes.io/os is set by the kubelet"
    );
}

#[test]
fn label_batch_refuses_an_unsafe_node_name() {
    let cluster = cluster();
    let nodes = [labelled("a/../b", &[])];
    assert!(label_batch(&scope(&cluster), &nodes, &[set("team", "infra")], None).is_err());
}

#[test]
fn label_batch_trims_keys_and_values_like_the_single_editor() {
    let cluster = cluster();
    let nodes = [labelled("wk-01", &[])];
    let batch =
        label_batch(&scope(&cluster), &nodes, &[set(" team ", " infra ")], None).expect("a batch");
    assert_eq!(
        node_changes(&batch),
        [("wk-01".to_owned(), vec![set("team", "infra")])]
    );
}

#[test]
fn the_target_names_stop_after_five_with_the_rest_counted() {
    let names = |count: usize| -> Vec<String> { (1..=count).map(|n| format!("n{n}")).collect() };
    assert_eq!(node_names_text(&names(2)), "n1, n2");
    assert_eq!(node_names_text(&names(5)), "n1, n2, n3, n4, n5");
    assert_eq!(node_names_text(&names(8)), "n1, n2, n3, n4, n5 +3");
}

#[test]
fn a_row_problem_names_the_row_and_the_input_to_mark() {
    let mut rows = taint_rows(&node_edit());
    let before = rows.len();
    rows.push(row("bad key!", "", "NoSchedule"));
    let problem = taint_row_problem(&rows).expect("a problem");
    assert_eq!(
        (problem.index, problem.field),
        (before, RowField::Key),
        "the mark follows the row position"
    );
    assert_eq!(
        problem.text.as_ref(),
        format!(
            "Row {}: key 'bad key!' is not a valid Kubernetes key",
            before + 1
        )
    );
    // A prefix with a slash is fine; a bad value marks the value input of that row.
    let rows = [label("example.com/team", "ok"), label("team", "no good")];
    let problem = label_row_problem(&rows).expect("a problem");
    assert_eq!((problem.index, problem.field), (1, RowField::Value));
    assert!(label_row_problem(&[label("example.com/team", "ok")]).is_none());
}

#[test]
fn a_long_key_is_cut_in_the_row_message() {
    let rows = [label(&format!("{} !", "k".repeat(60)), "x")];
    let text = label_row_problem(&rows).expect("a problem").text;
    assert!(
        text.starts_with("Row 1: key '") && text.contains("…'"),
        "{text}"
    );
}

#[test]
fn conflict_notice_lists_what_changed_on_the_server() {
    let base = vec![
        taint("dedicated", Some("ingress"), "NoSchedule"),
        taint("maintenance", None, "NoExecute"),
        taint("tier", Some("a"), "NoSchedule"),
    ];
    let now = edit_with(
        vec![
            taint("dedicated", Some("ingress"), "NoSchedule"),
            taint("tier", Some("b"), "NoSchedule"),
            taint("gpu", Some("true"), "NoSchedule"),
        ],
        &[],
    );
    assert_eq!(
        conflict_notice(Some(&base), &now),
        "The node changed (by someone else): tier=a:NoSchedule became tier=b:NoSchedule, \
         added gpu=true:NoSchedule, removed maintenance:NoExecute. \
         Rows you did not touch follow the node; your edits are kept. Review before applying."
    );
    // Without the editor's first read, the current taints are the least it can say.
    assert_eq!(
        conflict_notice(None, &now),
        "The node changed; your rows are kept. Taints on the node now: \
         dedicated=ingress:NoSchedule, tier=b:NoSchedule, gpu=true:NoSchedule."
    );
    assert!(conflict_notice(Some(&now.taints), &now).starts_with("The node changed; your rows"));
}

#[test]
fn a_reopened_editor_keeps_the_users_rows_and_the_nodes_managed_taints() {
    let kept = vec![
        TaintRow {
            time_added: None,
            ..row("node.kubernetes.io/unreachable", "", "NoExecute")
        },
        row("dedicated", "ingress", "NoSchedule"),
        row("gpu", "true", "NoSchedule"),
    ];
    // The node lost the managed taint and gained another meanwhile.
    let now = edit_with(
        vec![
            taint("node.kubernetes.io/unschedulable", None, "NoSchedule"),
            taint("dedicated", Some("ingress"), "NoSchedule"),
        ],
        &[],
    );
    let keys: Vec<String> = rows_after_conflict(&now, None, &kept)
        .into_iter()
        .map(|row| row.key)
        .collect();
    assert_eq!(
        keys,
        ["node.kubernetes.io/unschedulable", "dedicated", "gpu"]
    );
}

/// The taints the editor read, the user's rows over them, and the node as it is after the conflict.
fn conflict_of(
    base: &[NodeTaint],
    mine: &[TaintRow],
    now: Vec<NodeTaint>,
) -> Vec<(String, String, String)> {
    rows_after_conflict(&edit_with(now, &[]), Some(base), mine)
        .into_iter()
        .map(|row| (row.key, row.value, row.effect))
        .collect()
}

fn text_row(key: &str, value: &str, effect: &str) -> (String, String, String) {
    (key.to_owned(), value.to_owned(), effect.to_owned())
}

#[test]
fn an_untouched_row_follows_the_nodes_new_value() {
    // The user only added `gpu`; someone else changed `conflict` from 4 to 3 meanwhile.
    let base = [taint("conflict", Some("4"), "NoSchedule")];
    let mine = [
        row("conflict", "4", "NoSchedule"),
        row("gpu", "true", "NoSchedule"),
    ];
    let now = vec![taint("conflict", Some("3"), "NoSchedule")];
    assert_eq!(
        conflict_of(&base, &mine, now),
        [
            text_row("conflict", "3", "NoSchedule"),
            text_row("gpu", "true", "NoSchedule"),
        ]
    );
}

#[test]
fn an_edited_row_is_kept_as_typed_over_the_nodes_value() {
    let base = [taint("conflict", Some("4"), "NoSchedule")];
    let mine = [row("conflict", "9", "NoSchedule")];
    let now = vec![taint("conflict", Some("3"), "NoSchedule")];
    assert_eq!(
        conflict_of(&base, &mine, now),
        [text_row("conflict", "9", "NoSchedule")]
    );
}

#[test]
fn a_row_the_user_removed_stays_removed_and_a_new_taint_of_the_node_joins() {
    let base = [
        taint("workload", Some("data"), "NoSchedule"),
        taint("tier", Some("a"), "NoSchedule"),
    ];
    // The user removed `workload`; the node dropped `tier` and gained `maintenance`.
    let mine = [row("tier", "a", "NoSchedule")];
    let now = vec![
        taint("workload", Some("data"), "NoSchedule"),
        taint("maintenance", Some("true"), "NoSchedule"),
    ];
    assert_eq!(
        conflict_of(&base, &mine, now),
        [text_row("maintenance", "true", "NoSchedule")]
    );
}

#[test]
fn a_row_the_user_added_beats_a_taint_the_node_gained_under_the_same_key_and_effect() {
    let base = [];
    let mine = [row("gpu", "mine", "NoSchedule")];
    let now = vec![taint("gpu", Some("theirs"), "NoSchedule")];
    assert_eq!(
        conflict_of(&base, &mine, now),
        [text_row("gpu", "mine", "NoSchedule")]
    );
}

#[test]
fn a_changed_effect_is_an_edit_of_a_new_row_and_the_old_one_is_removed() {
    let base = [taint("gpu", None, "NoSchedule")];
    let mine = [row("gpu", "", "NoExecute")];
    let now = vec![taint("gpu", Some("x"), "NoSchedule")];
    assert_eq!(
        conflict_of(&base, &mine, now),
        [text_row("gpu", "", "NoExecute")]
    );
}

// ---- L17: the words and the DaemonSet warning of a bulk label edit ----

fn daemon_set_selecting(
    name: &str,
    node_selector: &[&str],
    affinity_keys: &[&str],
) -> DaemonSetSummary {
    DaemonSetSummary {
        node_selector: node_selector
            .iter()
            .map(|term| (*term).to_owned())
            .collect(),
        node_affinity_keys: affinity_keys.iter().map(|key| (*key).to_owned()).collect(),
        ..crate::topology_fixtures::daemon_set(name, 2, 2)
    }
}

fn warnings_with(
    nodes: &[TickedNode],
    changes: &[LabelChange],
    sets: &[DaemonSetSummary],
) -> Vec<String> {
    let cluster = cluster();
    let sets: Vec<&DaemonSetSummary> = sets.iter().collect();
    label_batch(&scope(&cluster), nodes, changes, Some(&sets))
        .expect("a batch")
        .warnings
        .iter()
        .map(ToString::to_string)
        .collect()
}

#[test]
fn a_bulk_label_edit_says_apply_to_n_nodes_and_names_what_it_did() {
    let cluster = cluster();
    let nodes = [labelled("wk-01", &["old=x"]), labelled("wk-02", &["old=x"])];
    let batch = label_batch(
        &scope(&cluster),
        &nodes,
        &[set("lab-batch", "1"), remove("old")],
        None,
    )
    .expect("a batch");
    let BatchExtras::Labels(extras) = &batch.plan.extras else {
        panic!("a label batch carries its words");
    };
    assert_eq!(extras.confirm, "Apply to 2 nodes");
    assert_eq!(
        extras.done,
        "Set lab-batch on 2 nodes. Removed old from 2 nodes"
    );
    assert_eq!(batch.confirm_label(0), "Apply to 2 nodes");
    let set_only = label_batch(
        &scope(&cluster),
        &nodes[..1],
        &[set("lab-batch", "1")],
        None,
    )
    .expect("one");
    assert_eq!(batch_extras(&set_only).confirm, "Apply to 1 node");
    assert_eq!(batch_extras(&set_only).done, "Set lab-batch on 1 node");
    let remove_only = label_batch(&scope(&cluster), &nodes, &[remove("old")], None).expect("two");
    assert_eq!(batch_extras(&remove_only).done, "Removed old from 2 nodes");
}

fn batch_extras(batch: &BatchIntent) -> &LabelExtras {
    match &batch.plan.extras {
        BatchExtras::Labels(extras) => extras,
        _ => panic!("a label batch carries its words"),
    }
}

#[test]
fn the_daemon_set_warning_needs_a_daemon_set_that_selects_by_the_key() {
    let nodes = [labelled("wk-01", &["team=infra"])];
    let unrelated = daemon_set_selecting("agent", &["disk=ssd"], &[]);
    assert!(warnings_with(&nodes, &[remove("team")], &[unrelated]).is_empty());

    let by_selector = daemon_set_selecting("logs", &["team=infra"], &[]);
    let by_affinity = daemon_set_selecting("mon", &[], &["team"]);
    assert_eq!(
        warnings_with(
            &nodes,
            &[remove("team")],
            std::slice::from_ref(&by_selector)
        ),
        ["DaemonSet shop/logs selects nodes by team: its pods on these nodes are deleted"]
    );
    assert_eq!(
        warnings_with(&nodes, &[remove("team")], &[by_selector, by_affinity]),
        [
            "DaemonSets shop/logs, shop/mon select nodes by team: their pods on these nodes are deleted"
        ]
    );
}

#[test]
fn a_set_warns_only_when_it_changes_a_value_a_daemon_set_selects_by() {
    let sets = [daemon_set_selecting("logs", &["team=infra"], &[])];
    // Adding the label to a node that lacks it deletes nothing.
    assert!(warnings_with(&[labelled("wk-01", &[])], &[set("team", "infra")], &sets).is_empty());
    // Changing the value on a node that has it moves the node out of the selector.
    assert_eq!(
        warnings_with(
            &[labelled("wk-01", &["team=infra"])],
            &[set("team", "dev")],
            &sets
        )
        .len(),
        1
    );
}

#[test]
fn without_the_daemon_sets_a_removal_keeps_the_generic_warning() {
    let cluster = cluster();
    let nodes = [labelled("wk-01", &["team=infra"])];
    let batch = label_batch(&scope(&cluster), &nodes, &[remove("team")], None).expect("a batch");
    assert_eq!(batch.warnings.len(), 1);
    assert!(batch.warnings[0].starts_with("Removing a label can make DaemonSets"));
    let set_only =
        label_batch(&scope(&cluster), &nodes, &[set("team", "dev")], None).expect("a batch");
    assert!(set_only.warnings.is_empty());
}
