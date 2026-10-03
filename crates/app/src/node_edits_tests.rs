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
    } = intent.request.operation()
    else {
        panic!("expected SetNodeTaints");
    };
    assert_eq!(resource_version, "42");
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
    assert!(error_text(taint_result(&rows)).contains("not valid for Kubernetes"));
    let mut rows = taint_rows(&node_edit());
    rows.push(row("gpu", "has space", "NoSchedule"));
    assert!(error_text(taint_result(&rows)).contains("not valid for Kubernetes"));
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
    assert!(error_text(label_result(&rows)).contains("not valid for Kubernetes"));
}

fn ticked(name: &str, scheduling: NodeScheduling) -> TickedNode {
    TickedNode {
        name: name.to_owned(),
        scheduling,
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
