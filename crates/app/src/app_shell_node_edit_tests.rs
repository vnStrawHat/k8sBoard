//! Bulk Cordon and Uncordon, Edit taints, and Edit labels in a headless window over two loaded
//! clusters, one active at a time (`prod-a`, locked at open, and `stg-b`, unlocked), each session
//! with its own fake API server: a test sees which cluster a request reached, and nothing leaves
//! the machine.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use cluster::fake_api::{FakeApi, RecordedRequest};
use cluster::{NodeReadiness, NodeScheduling, NodeStatus, NodeSummary, NodeSystemInfo};
use gpui_kit::TestAppContext;
use serde_json::{Value, json};

use super::app_shell_switch_tests::open_switch_fixture;
use super::app_shell_write_tests::{Clusters, go_live_answering, switch_to, writes};
use super::batch_write::BATCH_RUNNING_REASON;
use super::node_editor::{BulkLabelEditor, LabelTarget, NodeEditKind, NodeEditor};
use super::write_flow::DryRunState;
use super::*;
use crate::resource_actions::ResourceAction;
use crate::row_selection::BulkState;
use crate::write_guard::WriteLock;

const NODE_JSON: &str = r#"{"apiVersion":"v1","kind":"Node","metadata":{"name":"n"}}"#;

fn conflict() -> (u16, String) {
    let body = json!({
        "kind": "Status", "apiVersion": "v1", "status": "Failure",
        "message": "Operation cannot be fulfilled on nodes: the object has been modified",
        "reason": "Conflict", "code": 409,
    });
    (409, body.to_string())
}

/// The node as the server has it. After a 409 someone else has changed it: `dedicated` is `egress`
/// and `maintenance` is new, at a newer version.
fn node_json(name: &str, is_changed: bool) -> String {
    let (version, taints) = if is_changed {
        (
            "8",
            json!([
                {"key": "dedicated", "value": "egress", "effect": "NoSchedule"},
                {"key": "maintenance", "value": "true", "effect": "NoSchedule"},
            ]),
        )
    } else {
        (
            "7",
            json!([{"key": "dedicated", "value": "ingress", "effect": "NoSchedule"}]),
        )
    };
    json!({
        "apiVersion": "v1", "kind": "Node",
        "metadata": {
            "name": name, "resourceVersion": version,
            "labels": {"kubernetes.io/hostname": name, "team": "infra"},
        },
        "spec": {"taints": taints},
    })
    .to_string()
}

fn summary(name: &str, scheduling: NodeScheduling) -> NodeSummary {
    NodeSummary {
        name: name.to_owned(),
        status: NodeStatus {
            readiness: NodeReadiness::Ready,
            scheduling,
        },
        roles: Vec::new(),
        taints: Vec::new(),
        kubelet_version: "v1.29.5".to_owned(),
        internal_ip: None,
        created_at: None,
        conditions: Vec::new(),
        addresses: Vec::new(),
        system: NodeSystemInfo::default(),
        resources: Vec::new(),
        labels: Vec::new(),
    }
}

/// The server of one cluster: it reads any node, answers every patch with `patch`, and finds
/// nothing else.
fn server(
    patch: Arc<Mutex<(u16, String)>>,
) -> impl Fn(&RecordedRequest) -> (u16, String) + Send + Sync + 'static {
    let is_changed = AtomicBool::new(false);
    move |request| {
        if request.method == "GET"
            && let Some(name) = request.path.strip_prefix("/api/v1/nodes/")
        {
            // `ghost` is listed by the watch but gone when the editor reads it.
            if name == "ghost" {
                return (
                    404,
                    r#"{"kind":"Status","status":"Failure","code":404}"#.to_owned(),
                );
            }
            return (200, node_json(name, is_changed.load(Ordering::SeqCst)));
        }
        if request.method == "PATCH" {
            let answer = patch.lock().expect("the answer").clone();
            // A conflict means someone else changed the node first.
            if answer.0 == 409 {
                is_changed.store(true, Ordering::SeqCst);
            }
            return answer;
        }
        (
            404,
            r#"{"kind":"Status","status":"Failure","code":404}"#.to_owned(),
        )
    }
}

struct NodeTest {
    t: Clusters,
    /// What the next patch of either cluster is answered with.
    patch: Arc<Mutex<(u16, String)>>,
}

fn node_test(name: &str, cx: &mut TestAppContext) -> NodeTest {
    let fixture = open_switch_fixture(name, cx);
    switch_to(&fixture, "stg-b", cx);
    let (prod, stg) = (fixture.cluster("prod-a", cx), fixture.cluster("stg-b", cx));
    let patch = Arc::new(Mutex::new((200, NODE_JSON.to_owned())));
    let stg_api = go_live_answering(&fixture, &stg, "node-b", server(Arc::clone(&patch)), cx);
    fixture
        .shell
        .update(cx, |shell, cx| shell.show_screen(Screen::Nodes, cx));
    cx.run_until_parked();
    fixture.draw_twice(cx);
    NodeTest {
        t: Clusters {
            fixture,
            stg_api,
            prod,
            stg,
        },
        patch,
    }
}

impl NodeTest {
    /// Switches to `prod-a` and makes it live over a new fake server on the Nodes screen. The old
    /// session is gone, so a test never has both clusters live at once.
    fn activate_prod(&self, cx: &mut TestAppContext) -> FakeApi {
        let api =
            self.t
                .activate_answering(&self.t.prod, "node-a", server(Arc::clone(&self.patch)), cx);
        self.t
            .fixture
            .shell
            .update(cx, |shell, cx| shell.show_screen(Screen::Nodes, cx));
        cx.run_until_parked();
        self.t.fixture.draw_twice(cx);
        api
    }

    fn set_nodes(&self, cluster: &ClusterRef, nodes: Vec<NodeSummary>, cx: &mut TestAppContext) {
        let session = self
            .t
            .fixture
            .shell
            .read_with(cx, |shell, _| shell.session_of(cluster).cloned())
            .expect("an open cluster");
        session.update(cx, |session, cx| session.set_nodes_for_test(nodes, cx));
        cx.run_until_parked();
        self.t.fixture.draw_twice(cx);
    }

    fn tick(&self, rows: &[usize], cx: &mut TestAppContext) {
        for &row in rows {
            self.t.fixture.shell.update(cx, |shell, cx| {
                shell.check_rows(crate::table_view::RowCheck::Toggle(row), cx);
            });
        }
        cx.run_until_parked();
        self.t.fixture.draw_twice(cx);
    }

    fn open_editor(
        &self,
        kind: NodeEditKind,
        cluster: &ClusterRef,
        node: &str,
        cx: &mut TestAppContext,
    ) {
        self.t.fixture.with_window(cx, |window, cx| {
            self.t.fixture.shell.update(cx, |shell, cx| {
                shell.open_node_editor(kind, cluster, node, None, window, cx);
            });
        });
    }

    fn editor(&self, cx: &mut TestAppContext) -> Option<Entity<NodeEditor>> {
        self.t
            .fixture
            .shell
            .read_with(cx, |shell, _| shell.last_node_editor.clone())
            .and_then(|editor| editor.upgrade())
    }

    fn wait_for_editor(&self, cx: &mut TestAppContext) -> Entity<NodeEditor> {
        self.t.wait_for("the editor to load", cx, |cx| {
            self.editor(cx)
                .is_some_and(|editor| editor.read_with(cx, |editor, _| editor.is_loaded()))
        });
        self.editor(cx).expect("an editor")
    }

    fn add_row(
        &self,
        editor: &Entity<NodeEditor>,
        row: (&str, &str, &str),
        cx: &mut TestAppContext,
    ) {
        self.t.fixture.with_window(cx, |window, cx| {
            editor.update(cx, |editor, cx| {
                editor.add_row_with(row.0, row.1, row.2, window, cx);
            });
        });
    }

    fn review(&self, editor: &Entity<NodeEditor>, cx: &mut TestAppContext) {
        self.t.fixture.with_window(cx, |window, cx| {
            editor.update(cx, |editor, cx| editor.press_review(window, cx));
        });
        cx.run_until_parked();
    }

    fn bulk_states(&self, cx: &mut TestAppContext) -> Vec<(String, BulkState)> {
        self.t.fixture.shell.read_with(cx, |shell, cx| {
            shell
                .bulk_buttons(cx)
                .into_iter()
                .map(|button| (button.label.to_string(), button.state))
                .collect()
        })
    }
}

fn reads_of(api: &FakeApi, path: &str) -> usize {
    api.requests()
        .iter()
        .filter(|request| request.method == "GET" && request.path == path)
        .count()
}

fn body_of(request: &RecordedRequest) -> Value {
    serde_json::from_str(&request.body).expect("the body is JSON")
}

#[gpui_kit::test]
fn the_editor_reads_the_node_from_its_own_cluster(cx: &mut TestAppContext) {
    let t = node_test("node-edit-reads", cx);
    t.open_editor(NodeEditKind::Taints, &t.t.stg, "node-b", cx);
    let editor = t.wait_for_editor(cx);
    assert_eq!(reads_of(&t.t.stg_api, "/api/v1/nodes/node-b"), 1);
    // The node has one taint, so the editor starts with one row and nothing to review.
    editor.read_with(cx, |editor, cx| {
        assert_eq!(editor.row_count(), 1);
        assert_eq!(
            editor.current_intent(cx).err().map(|text| text.to_string()),
            Some("No changes".to_owned())
        );
    });
}

#[gpui_kit::test]
fn a_locked_cluster_opens_no_editor(cx: &mut TestAppContext) {
    let t = node_test("node-edit-locked", cx);
    let prod_api = t.activate_prod(cx);
    t.open_editor(NodeEditKind::Labels, &t.t.prod, "node-a", cx);
    assert!(t.editor(cx).is_none());
    assert_eq!(reads_of(&prod_api, "/api/v1/nodes/node-a"), 0);
}

#[gpui_kit::test]
fn a_node_that_is_not_listed_opens_no_editor(cx: &mut TestAppContext) {
    let t = node_test("node-edit-unlisted", cx);
    t.open_editor(NodeEditKind::Taints, &t.t.stg, "nowhere", cx);
    assert!(t.editor(cx).is_none());
    assert_eq!(reads_of(&t.t.stg_api, "/api/v1/nodes/nowhere"), 0);
}

#[gpui_kit::test]
fn an_unreadable_node_shows_the_error_and_no_rows(cx: &mut TestAppContext) {
    let t = node_test("node-edit-unreadable", cx);
    t.set_nodes(
        &t.t.stg,
        vec![summary("ghost", NodeScheduling::Enabled)],
        cx,
    );
    t.open_editor(NodeEditKind::Taints, &t.t.stg, "ghost", cx);
    t.t.wait_for("the failure", cx, |cx| {
        t.editor(cx)
            .is_some_and(|editor| editor.read_with(cx, |editor, _| editor.failure().is_some()))
    });
    let editor = t.editor(cx).expect("an editor");
    let text = editor.read_with(cx, |editor, _| editor.failure());
    assert!(
        text.is_some_and(|text| text.starts_with("Could not read node ghost")),
        "the error names the node"
    );
    assert_eq!(editor.read_with(cx, |editor, _| editor.row_count()), 0);
}

#[gpui_kit::test]
fn editor_review_opens_the_confirm_dialog(cx: &mut TestAppContext) {
    let t = node_test("node-edit-review", cx);
    t.open_editor(NodeEditKind::Taints, &t.t.stg, "node-b", cx);
    let editor = t.wait_for_editor(cx);
    t.add_row(&editor, ("gpu", "true", "NoSchedule"), cx);
    assert_eq!(editor.read_with(cx, |editor, _| editor.row_count()), 2);
    t.review(&editor, cx);
    t.t.wait_for_dry_run(cx);
    let dialog = t.t.dialog(cx);
    assert_eq!(
        dialog.read_with(cx, |dialog, _| dialog.label()).as_deref(),
        Some("Edit taints of node node-b")
    );
    // The dry-run is a patch of the full list with the version the editor read, on stg only.
    let sent = writes(&t.t.stg_api);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].method, "PATCH");
    assert_eq!(sent[0].path, "/api/v1/nodes/node-b");
    assert!(sent[0].has_query("dryRun", "All"));
    let body = body_of(&sent[0]);
    assert_eq!(body["metadata"]["resourceVersion"], "7");
    assert_eq!(body["spec"]["taints"].as_array().map(Vec::len), Some(2));
}

#[gpui_kit::test]
fn adding_no_execute_makes_the_dialog_destructive(cx: &mut TestAppContext) {
    let t = node_test("node-edit-no-execute", cx);
    t.open_editor(NodeEditKind::Taints, &t.t.stg, "node-b", cx);
    let editor = t.wait_for_editor(cx);
    t.add_row(&editor, ("maintenance", "", "NoExecute"), cx);
    t.review(&editor, cx);
    t.t.wait_for_dry_run(cx);
    let dialog = t.t.dialog(cx);
    let warnings = dialog.read_with(cx, |dialog, _| dialog.warning_lines());
    assert_eq!(
        warnings,
        [SharedString::from(
            "NoExecute evicts pods that do not tolerate it"
        )]
    );
}

#[gpui_kit::test]
fn a_taint_conflict_goes_straight_back_to_the_editor_and_untouched_rows_follow_the_node(
    cx: &mut TestAppContext,
) {
    let t = node_test("node-edit-conflict", cx);
    t.open_editor(NodeEditKind::Taints, &t.t.stg, "node-b", cx);
    let editor = t.wait_for_editor(cx);
    t.add_row(&editor, ("gpu", "true", "NoSchedule"), cx);
    *t.patch.lock().expect("the answer") = conflict();
    let first = editor.entity_id();
    t.review(&editor, cx);
    // The 409 of the dry-run shows no step with the server's words: the confirm closes and the
    // editor reopens on the node as it is now.
    t.t.wait_for("the editor to reopen", cx, |cx| {
        t.editor(cx).is_some_and(|editor| {
            editor.entity_id() != first && editor.read_with(cx, |editor, _| editor.is_loaded())
        })
    });
    assert_eq!(reads_of(&t.t.stg_api, "/api/v1/nodes/node-b"), 2);
    let reopened = t.editor(cx).expect("an editor");
    reopened.read_with(cx, |editor, cx| {
        assert_eq!(
            editor.notice().as_deref(),
            Some(
                "The node changed (by someone else): dedicated=ingress:NoSchedule became \
                 dedicated=egress:NoSchedule, added maintenance=true:NoSchedule. Rows you did \
                 not touch follow the node; your edits are kept. Review before applying."
            )
        );
        // The row nobody touched took the node's value, the node's new taint joined, and the
        // user's added row is kept.
        assert_eq!(editor.row_count(), 3);
        let intent = editor
            .current_intent(cx)
            .expect("the user's row is a change");
        let lines: Vec<String> = intent
            .request
            .changed_fields()
            .into_iter()
            .map(|field| field.path.into_owned())
            .collect();
        assert_eq!(lines, ["+ gpu=true:NoSchedule"]);
    });
}

#[gpui_kit::test]
fn edit_labels_commit_sends_a_minimal_patch(cx: &mut TestAppContext) {
    let t = node_test("node-edit-labels-commit", cx);
    t.open_editor(NodeEditKind::Labels, &t.t.stg, "node-b", cx);
    let editor = t.wait_for_editor(cx);
    t.add_row(&editor, ("env", "staging", ""), cx);
    t.review(&editor, cx);
    t.t.wait_for_dry_run(cx);
    t.t.confirm(cx);
    t.t.wait_for("the commit", cx, |_| writes(&t.t.stg_api).len() == 2);
    let commit = &writes(&t.t.stg_api)[1];
    assert!(!commit.has_query_key("dryRun"));
    assert_eq!(
        commit.content_type.as_deref(),
        Some("application/merge-patch+json")
    );
    // Only the added key goes out: the other labels are left alone.
    assert_eq!(
        body_of(commit),
        json!({"metadata": {"labels": {"env": "staging"}}})
    );
}

#[gpui_kit::test]
fn header_edit_labels_by_tick_count(cx: &mut TestAppContext) {
    let t = node_test("node-edit-header", cx);
    let names: Vec<String> = (0..52).map(|index| format!("n{index:02}")).collect();
    t.set_nodes(
        &t.t.stg,
        names
            .iter()
            .map(|name| summary(name, NodeScheduling::Enabled))
            .collect(),
        cx,
    );
    let target = |cx: &mut TestAppContext| {
        t.t.fixture
            .shell
            .read_with(cx, |shell, cx| shell.edit_labels_target(cx))
    };
    // None ticked.
    assert_eq!(target(cx).err().as_deref(), Some("Tick nodes first"));
    // One ticked: the 0034 editor, on its own node.
    t.tick(&[1], cx);
    let Ok(LabelTarget::One { cluster, node }) = target(cx) else {
        panic!("one node is ticked");
    };
    assert_eq!((cluster, node.as_str()), (t.t.stg.clone(), "n01"));
    // Two ticked: the bulk editor over both.
    t.tick(&[0], cx);
    let Ok(LabelTarget::Several { cluster, nodes }) = target(cx) else {
        panic!("two nodes are ticked");
    };
    assert_eq!(cluster, t.t.stg);
    assert_eq!(nodes.len(), 2);
    // Fifty ticked is the most; fifty-one is refused with the text of the other bulk buttons.
    t.tick(&(2..50).collect::<Vec<_>>(), cx);
    assert!(matches!(target(cx), Ok(LabelTarget::Several { nodes, .. }) if nodes.len() == 50));
    t.tick(&[50], cx);
    assert_eq!(target(cx).err().as_deref(), Some("Select at most 50 rows"));
}

#[gpui_kit::test]
fn the_header_edit_labels_is_off_while_a_batch_runs(cx: &mut TestAppContext) {
    let t = node_test("node-edit-header-batch", cx);
    three_nodes(&t, cx);
    t.tick(&[0, 1], cx);
    let stg = t.t.stg.clone();
    t.t.fixture.shell.update(cx, |shell, _| {
        shell.running_batches.insert(stg);
    });
    let target =
        t.t.fixture
            .shell
            .read_with(cx, |shell, cx| shell.edit_labels_target(cx));
    assert_eq!(target.err().as_deref(), Some(BATCH_RUNNING_REASON));
}

#[gpui_kit::test]
fn the_header_edit_labels_follows_the_lock_of_the_nodes_cluster(cx: &mut TestAppContext) {
    let t = node_test("node-edit-header-locked", cx);
    t.activate_prod(cx);
    t.tick(&[0], cx);
    let target =
        t.t.fixture
            .shell
            .read_with(cx, |shell, cx| shell.edit_labels_target(cx));
    assert_eq!(target.err().as_deref(), Some("prod-a is read-only"));
    // Two ticked nodes of a locked cluster read the same.
    t.set_nodes(
        &t.t.prod,
        vec![
            summary("p1", NodeScheduling::Enabled),
            summary("p2", NodeScheduling::Enabled),
        ],
        cx,
    );
    t.tick(&[1], cx);
    let target =
        t.t.fixture
            .shell
            .read_with(cx, |shell, cx| shell.edit_labels_target(cx));
    assert_eq!(target.err().as_deref(), Some("prod-a is read-only"));
}

fn state_of(states: &[(String, BulkState)], label: &str) -> BulkState {
    states
        .iter()
        .find(|(name, _)| name == label)
        .map(|(_, state)| state.clone())
        .unwrap_or_else(|| panic!("no {label} button in {states:?}"))
}

fn three_nodes(t: &NodeTest, cx: &mut TestAppContext) {
    t.set_nodes(
        &t.t.stg,
        vec![
            summary("n1", NodeScheduling::Enabled),
            summary("n2", NodeScheduling::Disabled),
            summary("n3", NodeScheduling::Enabled),
        ],
        cx,
    );
}

#[gpui_kit::test]
fn bulk_cordon_is_a_batch_and_skips_already_cordoned(cx: &mut TestAppContext) {
    let t = node_test("node-bulk-cordon", cx);
    three_nodes(&t, cx);
    t.tick(&[0, 1, 2], cx);
    let states = t.bulk_states(cx);
    assert_eq!(
        state_of(&states, "Cordon"),
        BulkState::Ready(ResourceAction::Cordon)
    );
    assert_eq!(
        state_of(&states, "Uncordon"),
        BulkState::Ready(ResourceAction::Uncordon)
    );
    t.t.fixture.with_window(cx, |window, cx| {
        t.t.fixture.shell.update(cx, |shell, cx| {
            shell.run_bulk(ResourceAction::Cordon, window, cx);
        });
    });
    t.t.wait_for_dry_run(cx);
    // Two dry-runs (n1, n3), one per node that is not cordoned yet, on the nodes' own cluster.
    let dry_runs = writes(&t.t.stg_api);
    assert_eq!(dry_runs.len(), 2);
    assert!(
        dry_runs
            .iter()
            .all(|request| request.has_query("dryRun", "All"))
    );
    assert_eq!(dry_runs[0].path, "/api/v1/nodes/n1");
    assert_eq!(dry_runs[1].path, "/api/v1/nodes/n3");
    t.t.confirm(cx);
    t.t.wait_for("the commits", cx, |_| writes(&t.t.stg_api).len() == 4);
    let commits = &writes(&t.t.stg_api)[2..];
    assert!(
        commits
            .iter()
            .all(|request| !request.has_query_key("dryRun"))
    );
    assert_eq!(
        body_of(&commits[0]),
        json!({"spec": {"unschedulable": true}})
    );
}

#[gpui_kit::test]
fn bulk_cordon_needs_every_dry_run(cx: &mut TestAppContext) {
    let t = node_test("node-bulk-dry-run", cx);
    three_nodes(&t, cx);
    t.tick(&[0, 2], cx);
    *t.patch.lock().expect("the answer") = conflict();
    t.t.fixture.with_window(cx, |window, cx| {
        t.t.fixture.shell.update(cx, |shell, cx| {
            shell.run_bulk(ResourceAction::Cordon, window, cx);
        });
    });
    t.t.wait_for_dry_run(cx);
    assert!(t.t.block(cx).is_some(), "the confirm stays off");
    t.t.confirm(cx);
    cx.run_until_parked();
    // Only the two dry-runs ever left.
    assert_eq!(writes(&t.t.stg_api).len(), 2);
}

#[gpui_kit::test]
fn bulk_uncordon_of_schedulable_nodes_says_why(cx: &mut TestAppContext) {
    let t = node_test("node-bulk-uncordon", cx);
    three_nodes(&t, cx);
    t.tick(&[0, 2], cx);
    let states = t.bulk_states(cx);
    assert_eq!(
        state_of(&states, "Uncordon"),
        BulkState::Off("All selected nodes are already schedulable".into())
    );
    t.tick(&[1], cx);
    let states = t.bulk_states(cx);
    assert_eq!(
        state_of(&states, "Uncordon"),
        BulkState::Ready(ResourceAction::Uncordon)
    );
}

#[gpui_kit::test]
fn bulk_buttons_are_off_while_a_batch_runs_on_the_cluster(cx: &mut TestAppContext) {
    let t = node_test("node-bulk-running", cx);
    three_nodes(&t, cx);
    t.tick(&[0], cx);
    let states = t.bulk_states(cx);
    assert!(matches!(state_of(&states, "Cordon"), BulkState::Ready(_)));
    let stg = t.t.stg.clone();
    t.t.fixture.shell.update(cx, |shell, _| {
        shell.running_batches.insert(stg);
    });
    let states = t.bulk_states(cx);
    assert_eq!(
        state_of(&states, "Cordon"),
        BulkState::Off(BATCH_RUNNING_REASON.into())
    );
}

#[gpui_kit::test]
fn bulk_cordon_on_a_locked_cluster_is_off(cx: &mut TestAppContext) {
    let t = node_test("node-bulk-locked", cx);
    t.activate_prod(cx);
    t.set_nodes(&t.t.prod, vec![summary("p1", NodeScheduling::Enabled)], cx);
    t.tick(&[0], cx);
    let states = t.bulk_states(cx);
    assert_eq!(
        state_of(&states, "Cordon"),
        BulkState::Off("prod-a is read-only".into())
    );
}

#[gpui_kit::test]
fn taint_retry_after_another_error_checks_again_and_keeps_the_rows(cx: &mut TestAppContext) {
    let t = node_test("node-edit-server-error", cx);
    t.open_editor(NodeEditKind::Taints, &t.t.stg, "node-b", cx);
    let editor = t.wait_for_editor(cx);
    t.add_row(&editor, ("gpu", "true", "NoSchedule"), cx);
    let body = json!({
        "kind": "Status", "apiVersion": "v1", "status": "Failure",
        "message": "etcd is busy", "reason": "InternalError", "code": 500,
    });
    *t.patch.lock().expect("the answer") = (500, body.to_string());
    t.review(&editor, cx);
    t.t.wait_for_dry_run(cx);
    let dialog = t.t.dialog(cx);
    assert!(matches!(
        dialog.read_with(cx, |dialog, _| dialog.dry_run_state()),
        Some(DryRunState::Failed(_))
    ));
    // Not a 409: Retry runs the same check again and does not read the node or reopen the editor.
    t.t.fixture.with_window(cx, |window, cx| {
        dialog.update(cx, |dialog, cx| dialog.press_retry(window, cx));
    });
    t.t.wait_for("the second dry-run", cx, |_| {
        writes(&t.t.stg_api).len() == 2
    });
    assert_eq!(reads_of(&t.t.stg_api, "/api/v1/nodes/node-b"), 1);
    let body_of_both: Vec<Value> = writes(&t.t.stg_api).iter().map(body_of).collect();
    assert_eq!(
        body_of_both[0], body_of_both[1],
        "the same rows are checked again"
    );
}

// ---- Edit labels of several nodes (spec 0040) ----

fn labelled(name: &str, labels: &[&str]) -> NodeSummary {
    NodeSummary {
        labels: labels.iter().map(|term| (*term).to_owned()).collect(),
        ..summary(name, NodeScheduling::Enabled)
    }
}

impl NodeTest {
    fn bulk_editor(&self, cx: &mut TestAppContext) -> Option<Entity<BulkLabelEditor>> {
        self.t
            .fixture
            .shell
            .read_with(cx, |shell, _| shell.last_bulk_label_editor.clone())
            .and_then(|editor| editor.upgrade())
    }

    /// Opens the bulk editor of the ticked nodes of `cluster`, as the header button does.
    fn open_bulk(&self, cluster: &ClusterRef, names: &[String], cx: &mut TestAppContext) {
        self.t.fixture.with_window(cx, |window, cx| {
            self.t.fixture.shell.update(cx, |shell, cx| {
                shell.open_bulk_label_editor(cluster, names, window, cx);
            });
        });
    }

    fn fill(
        &self,
        editor: &Entity<BulkLabelEditor>,
        row: (&str, &str, bool),
        cx: &mut TestAppContext,
    ) {
        self.t.fixture.with_window(cx, |window, cx| {
            editor.update(cx, |editor, cx| {
                editor.fill_last_row(row.0, row.1, row.2, window, cx);
            });
        });
    }

    fn review_bulk(&self, editor: &Entity<BulkLabelEditor>, cx: &mut TestAppContext) {
        self.t.fixture.with_window(cx, |window, cx| {
            editor.update(cx, |editor, cx| editor.press_review(window, cx));
        });
        cx.run_until_parked();
    }

    /// Ticks the three nodes `n1`..`n3`, one of them labelled already, and opens the editor with a
    /// Set typed.
    fn bulk_over_three(&self, cx: &mut TestAppContext) -> Entity<BulkLabelEditor> {
        self.set_nodes(
            &self.t.stg,
            vec![
                labelled("n1", &["team=infra"]),
                labelled("n2", &["team=dev", "old-key=x"]),
                labelled("n3", &[]),
            ],
            cx,
        );
        self.tick(&[0, 1, 2], cx);
        self.open_bulk(&self.t.stg, &["n1", "n2", "n3"].map(str::to_owned), cx);
        let editor = self.bulk_editor(cx).expect("the bulk editor opened");
        self.fill(&editor, ("team", "infra", false), cx);
        editor
    }
}

#[gpui_kit::test]
fn the_bulk_editor_opens_with_one_empty_row_and_no_review(cx: &mut TestAppContext) {
    let t = node_test("bulk-labels-open", cx);
    let editor = t.bulk_over_three(cx);
    editor.read_with(cx, |editor, _| assert_eq!(editor.row_count(), 1));
    // An empty row is not a change, so there is nothing to review yet.
    t.t.fixture.with_window(cx, |window, cx| {
        editor.update(cx, |editor, cx| editor.push_row(window, cx));
    });
    t.fill(&editor, ("", "", false), cx);
    editor.read_with(cx, |editor, cx| {
        assert_eq!(editor.row_count(), 2);
        assert_eq!(editor.current_changes(cx).len(), 1);
    });
    // Nothing is sent by opening it, and no node is read.
    assert!(writes(&t.t.stg_api).is_empty());
    assert_eq!(reads_of(&t.t.stg_api, "/api/v1/nodes/n1"), 0);
}

#[gpui_kit::test]
fn the_bulk_editor_names_a_problem_before_review(cx: &mut TestAppContext) {
    let t = node_test("bulk-labels-problem", cx);
    let editor = t.bulk_over_three(cx);
    editor.read_with(cx, |editor, _| assert_eq!(editor.current_problem(), None));
    t.t.fixture.with_window(cx, |window, cx| {
        editor.update(cx, |editor, cx| editor.push_row(window, cx));
    });
    t.fill(&editor, ("kubernetes.io/os", "linux", false), cx);
    editor.read_with(cx, |editor, _| {
        assert_eq!(
            editor.current_problem().as_deref(),
            Some("kubernetes.io/os is set by the kubelet")
        );
    });
}

#[gpui_kit::test]
fn bulk_labels_dry_run_every_node_then_commit(cx: &mut TestAppContext) {
    let t = node_test("bulk-labels-run", cx);
    let dir = t.t.enable_audit_folder("bulk-labels-run", cx);
    let editor = t.bulk_over_three(cx);
    // A second row removes a key.
    t.t.fixture.with_window(cx, |window, cx| {
        editor.update(cx, |editor, cx| editor.push_row(window, cx));
    });
    t.fill(&editor, ("old-key", "", true), cx);
    t.review_bulk(&editor, cx);
    t.t.wait_for_dry_run(cx);
    // n1 has team=infra already and no old key: skipped. n2 and n3 are dry-run, one PATCH each.
    let dry_runs = writes(&t.t.stg_api);
    let paths: Vec<&str> = dry_runs
        .iter()
        .map(|request| request.path.as_str())
        .collect();
    assert_eq!(paths, ["/api/v1/nodes/n2", "/api/v1/nodes/n3"]);
    assert!(
        dry_runs
            .iter()
            .all(|request| request.method == "PATCH" && request.has_query("dryRun", "All"))
    );
    assert_eq!(
        body_of(&dry_runs[0]),
        json!({"metadata": {"labels": {"old-key": null, "team": "infra"}}})
    );
    assert_eq!(
        body_of(&dry_runs[1]),
        json!({"metadata": {"labels": {"team": "infra"}}})
    );
    let dialog = t.t.dialog(cx);
    dialog.read_with(cx, |dialog, _| {
        assert_eq!(dialog.label().as_deref(), Some("Edit labels of 2 nodes"));
        // The removal carries the DaemonSet warning.
        assert!(
            dialog
                .warning_lines()
                .iter()
                .any(|line| line.contains("DaemonSets that select nodes by it"))
        );
    });
    // The cluster tier of staging is a click; nothing is committed before the confirm.
    assert_eq!(writes(&t.t.stg_api).len(), 2);
    t.t.confirm(cx);
    t.t.wait_for("the commits", cx, |_| writes(&t.t.stg_api).len() == 4);
    let commits = &writes(&t.t.stg_api)[2..];
    assert!(
        commits
            .iter()
            .all(|request| !request.has_query_key("dryRun"))
    );
    // One audit line per committed node, with the changed keys.
    t.t.wait_for("the audit lines", cx, |_| {
        super::app_shell_write_tests::audit_lines(&dir).len() == 2
    });
    let lines = super::app_shell_write_tests::audit_lines(&dir);
    for (line, node) in lines.iter().zip(["n2", "n3"]) {
        assert_eq!(line["action"], "Edit labels");
        assert_eq!(line["object"]["name"], node);
        assert_eq!(line["fields"][0]["path"], "metadata.labels");
    }
    assert_eq!(lines[0]["fields"][0]["value"], "-old-key; team=infra");
}

#[gpui_kit::test]
fn a_review_that_finds_nothing_to_do_says_why_and_sends_nothing(cx: &mut TestAppContext) {
    let t = node_test("bulk-labels-noop", cx);
    t.set_nodes(
        &t.t.stg,
        vec![
            labelled("n1", &["team=infra"]),
            labelled("n2", &["team=infra"]),
        ],
        cx,
    );
    t.tick(&[0, 1], cx);
    t.open_bulk(&t.t.stg, &["n1", "n2"].map(str::to_owned), cx);
    let editor = t.bulk_editor(cx).expect("the bulk editor opened");
    t.fill(&editor, ("team", "infra", false), cx);
    editor.read_with(cx, |editor, _| {
        assert_eq!(
            editor.current_problem().as_deref(),
            Some("All selected nodes already have these labels")
        );
    });
    let before =
        t.t.fixture
            .with_window(cx, |window, cx| window.notifications(cx).len());
    t.review_bulk(&editor, cx);
    assert!(!t.t.has_dialog(cx), "no batch dialog opens");
    let after =
        t.t.fixture
            .with_window(cx, |window, cx| window.notifications(cx).len());
    assert!(after > before, "Review says why");
    assert!(writes(&t.t.stg_api).is_empty());
}

#[gpui_kit::test]
fn bulk_labels_batch_of_a_sends_nothing_to_b(cx: &mut TestAppContext) {
    let t = node_test("bulk-labels-switch", cx);
    let dir = t.t.enable_audit_folder("bulk-labels-switch", cx);
    let editor = t.bulk_over_three(cx);
    // The selection leaves stg-b while the editor stands: Review finds nothing ticked there.
    let prod_api = t.activate_prod(cx);
    t.review_bulk(&editor, cx);
    assert!(!t.t.has_dialog(cx), "no batch of a cluster that left");
    assert!(writes(&t.t.stg_api).is_empty());
    assert!(writes(&prod_api).is_empty());
    assert!(super::app_shell_write_tests::audit_lines(&dir).is_empty());
}

#[gpui_kit::test]
fn bulk_labels_stop_when_the_cluster_locks_mid_batch(cx: &mut TestAppContext) {
    use gpui_kit::component::dialog::Confirm;
    let t = node_test("bulk-labels-lock", cx);
    let editor = t.bulk_over_three(cx);
    t.review_bulk(&editor, cx);
    t.t.wait_for_dry_run(cx);
    assert_eq!(
        writes(&t.t.stg_api).len(),
        2,
        "two dry-runs: n1 has the label already"
    );
    // Locked between the dry-runs and the confirm: every commit is blocked.
    t.t.set_lock(&t.t.stg, WriteLock::Locked, cx);
    t.t.fixture.with_window(cx, |window, cx| {
        window.dispatch_action(Box::new(Confirm { secondary: false }), cx);
    });
    cx.run_until_parked();
    assert_eq!(writes(&t.t.stg_api).len(), 2, "nothing was committed");
}

#[gpui_kit::test]
fn a_held_enter_never_confirms_the_bulk_labels(cx: &mut TestAppContext) {
    use gpui_kit::InputEvent as _;
    use gpui_kit::test::TestWindowExt as _;
    let t = node_test("bulk-labels-held-enter", cx);
    let editor = t.bulk_over_three(cx);
    t.review_bulk(&editor, cx);
    t.t.wait_for_dry_run(cx);
    t.t.fixture
        .with_window(cx, |window, cx| window.render_frame(cx));
    let held = gpui_kit::KeyDownEvent {
        keystroke: gpui_kit::Keystroke::parse("enter").expect("a valid keystroke"),
        is_held: true,
        prefer_character_input: false,
    };
    for _ in 0..3 {
        t.t.fixture.with_window(cx, |window, cx| {
            window.dispatch_event(held.clone().to_platform_input(), cx);
        });
    }
    cx.run_until_parked();
    assert_eq!(writes(&t.t.stg_api).len(), 2, "only the dry-runs were sent");
    let fresh = gpui_kit::KeyDownEvent {
        is_held: false,
        ..held
    };
    t.t.fixture.with_window(cx, |window, cx| {
        window.dispatch_event(fresh.to_platform_input(), cx);
    });
    t.t.wait_for("the commits", cx, |_| writes(&t.t.stg_api).len() == 4);
}

#[gpui_kit::test]
fn removing_a_bulk_row_drops_its_subscriptions_and_refreshes_the_problem(cx: &mut TestAppContext) {
    let t = node_test("bulk-labels-remove-row", cx);
    let editor = t.bulk_over_three(cx);
    t.t.fixture.with_window(cx, |window, cx| {
        editor.update(cx, |editor, cx| editor.push_row(window, cx));
    });
    t.fill(&editor, ("kubernetes.io/os", "linux", false), cx);
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.row_subscription_count(), 6, "three per row");
        assert!(editor.current_problem().is_some());
    });
    // The row with the kubelet key goes: its subscriptions go with it and the problem clears.
    t.t.fixture.with_window(cx, |_, cx| {
        editor.update(cx, |editor, cx| editor.drop_row(1, cx));
    });
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.row_count(), 1);
        assert_eq!(editor.row_subscription_count(), 3);
        assert_eq!(editor.current_problem(), None);
    });
}

fn enter(is_held: bool) -> gpui_kit::KeyDownEvent {
    gpui_kit::KeyDownEvent {
        keystroke: gpui_kit::Keystroke::parse("enter").expect("a valid keystroke"),
        is_held,
        prefer_character_input: false,
    }
}

#[gpui_kit::test]
fn a_fresh_enter_in_a_taint_field_presses_review_and_a_held_one_does_not(cx: &mut TestAppContext) {
    use gpui_kit::InputEvent as _;
    let t = node_test("node-edit-enter", cx);
    t.open_editor(NodeEditKind::Taints, &t.t.stg, "node-b", cx);
    let editor = t.wait_for_editor(cx);
    t.add_row(&editor, ("gpu", "true", "NoSchedule"), cx);
    t.t.fixture.with_window(cx, |window, cx| {
        editor.update(cx, |editor, cx| editor.focus_added_key(window, cx));
    });
    t.t.fixture.draw_twice(cx);
    t.t.fixture.with_window(cx, |window, cx| {
        window.dispatch_event(enter(true).to_platform_input(), cx);
    });
    cx.run_until_parked();
    assert!(
        writes(&t.t.stg_api).is_empty(),
        "a held Enter reviews nothing"
    );
    t.t.fixture.with_window(cx, |window, cx| {
        window.dispatch_event(enter(false).to_platform_input(), cx);
    });
    t.t.wait_for_dry_run(cx);
    assert_eq!(
        writes(&t.t.stg_api).len(),
        1,
        "the review dry-ran the patch"
    );
}

#[gpui_kit::test]
fn enter_in_the_taint_editor_does_nothing_while_the_edit_has_no_changes(cx: &mut TestAppContext) {
    use gpui_kit::InputEvent as _;
    let t = node_test("node-edit-enter-empty", cx);
    t.open_editor(NodeEditKind::Taints, &t.t.stg, "node-b", cx);
    let editor = t.wait_for_editor(cx);
    t.add_row(&editor, ("", "", "NoSchedule"), cx);
    t.t.fixture.with_window(cx, |window, cx| {
        editor.update(cx, |editor, cx| editor.focus_added_key(window, cx));
    });
    t.t.fixture.draw_twice(cx);
    t.t.fixture.with_window(cx, |window, cx| {
        window.dispatch_event(enter(false).to_platform_input(), cx);
    });
    cx.run_until_parked();
    assert!(writes(&t.t.stg_api).is_empty());
    assert!(t.editor(cx).is_some(), "the editor stays open");
}

#[gpui_kit::test]
fn a_fresh_enter_in_a_bulk_label_field_presses_review(cx: &mut TestAppContext) {
    use gpui_kit::InputEvent as _;
    let t = node_test("bulk-labels-enter", cx);
    let editor = t.bulk_over_three(cx);
    t.t.fixture.with_window(cx, |window, cx| {
        editor.update(cx, |editor, cx| editor.focus_last_key(window, cx));
    });
    t.t.fixture.draw_twice(cx);
    t.t.fixture.with_window(cx, |window, cx| {
        window.dispatch_event(enter(true).to_platform_input(), cx);
    });
    cx.run_until_parked();
    assert!(
        writes(&t.t.stg_api).is_empty(),
        "a held Enter reviews nothing"
    );
    t.t.fixture.with_window(cx, |window, cx| {
        window.dispatch_event(enter(false).to_platform_input(), cx);
    });
    t.t.wait_for_dry_run(cx);
    assert!(
        !writes(&t.t.stg_api).is_empty(),
        "the review dry-ran the batch"
    );
}

#[gpui_kit::test]
fn the_node_selection_bar_has_edit_labels(cx: &mut TestAppContext) {
    let t = node_test("node-bar-labels", cx);
    three_nodes(&t, cx);
    t.tick(&[0, 1], cx);
    let states = t.bulk_states(cx);
    assert_eq!(
        state_of(&states, "Edit labels…"),
        BulkState::Ready(ResourceAction::EditLabels)
    );
    // Pressed on two nodes it opens the bulk editor, which names them.
    t.t.fixture.with_window(cx, |window, cx| {
        t.t.fixture.shell.update(cx, |shell, cx| {
            shell.run_bulk(ResourceAction::EditLabels, window, cx);
        });
    });
    let editor = t.bulk_editor(cx).expect("the bulk editor opened");
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.current_targets(), "n1, n2")
    });
}

#[gpui_kit::test]
fn edit_labels_on_the_bar_is_off_over_the_cap(cx: &mut TestAppContext) {
    let t = node_test("node-bar-labels-cap", cx);
    let names: Vec<String> = (0..51).map(|index| format!("n{index:02}")).collect();
    t.set_nodes(
        &t.t.stg,
        names
            .iter()
            .map(|name| summary(name, NodeScheduling::Enabled))
            .collect(),
        cx,
    );
    t.tick(&(0..51).collect::<Vec<_>>(), cx);
    assert_eq!(
        state_of(&t.bulk_states(cx), "Edit labels…"),
        BulkState::Off("Select at most 50 rows".into())
    );
}

// ---- L8 and L9: Add, the empty-key line, and closing with changes ----

impl NodeTest {
    fn press_add(&self, editor: &Entity<NodeEditor>, cx: &mut TestAppContext) {
        self.add_row(editor, ("", "", "NoSchedule"), cx);
    }

    fn has_dialog(&self, cx: &mut TestAppContext) -> bool {
        self.t
            .fixture
            .with_window(cx, |window, cx| window.has_active_dialog(cx))
    }

    /// Closes the top dialog, whatever it is, and tells whether another one is left under it.
    fn close_top_dialog(&self, cx: &mut TestAppContext) -> bool {
        self.t.fixture.with_window(cx, |window, cx| {
            window.close_dialog(cx);
            window.has_active_dialog(cx)
        })
    }
}

#[gpui_kit::test]
fn add_puts_the_new_row_first_with_the_cursor_in_its_key(cx: &mut TestAppContext) {
    let t = node_test("node-edit-add-top", cx);
    t.open_editor(NodeEditKind::Taints, &t.t.stg, "node-b", cx);
    let editor = t.wait_for_editor(cx);
    t.press_add(&editor, cx);
    editor.read_with(cx, |editor, cx| {
        assert_eq!(editor.row_keys(cx), ["", "dedicated"]);
    });
    t.t.fixture.draw_twice(cx);
    let is_focused = t.t.fixture.with_window(cx, |window, cx| {
        editor.read_with(cx, |editor, cx| editor.is_added_key_focused(window, cx))
    });
    assert!(is_focused, "typing goes into the new row");
}

#[gpui_kit::test]
fn text_set_right_after_add_stays_in_the_new_taint_row(cx: &mut TestAppContext) {
    let t = node_test("node-edit-add-keeps", cx);
    t.open_editor(NodeEditKind::Taints, &t.t.stg, "node-b", cx);
    let editor = t.wait_for_editor(cx);
    t.press_add(&editor, cx);
    t.t.fixture.with_window(cx, |window, cx| {
        editor.update(cx, |editor, cx| editor.set_first_key("gpu", window, cx));
    });
    t.t.fixture.draw_twice(cx);
    editor.read_with(cx, |editor, cx| {
        assert_eq!(editor.row_keys(cx), ["gpu", "dedicated"]);
        assert_eq!(editor.shown_problem_now(cx), None);
    });
}

#[gpui_kit::test]
fn an_empty_key_is_named_only_after_review_and_the_line_goes_with_the_fix(cx: &mut TestAppContext) {
    let t = node_test("node-edit-empty-key", cx);
    t.open_editor(NodeEditKind::Labels, &t.t.stg, "node-b", cx);
    let editor = t.wait_for_editor(cx);
    t.press_add(&editor, cx);
    t.t.fixture.draw_twice(cx);
    editor.read_with(cx, |editor, cx| {
        // Typing has not named a problem, and Review… stays pressable.
        assert_eq!(editor.shown_problem_now(cx), None);
        assert!(!editor.is_review_off_now(cx));
    });
    t.review(&editor, cx);
    t.t.fixture.draw_twice(cx);
    editor.read_with(cx, |editor, cx| {
        assert_eq!(
            editor.shown_problem_now(cx).as_deref(),
            Some("Enter a key for every label")
        );
    });
    assert!(writes(&t.t.stg_api).is_empty(), "nothing was reviewed");
    t.t.fixture.with_window(cx, |window, cx| {
        editor.update(cx, |editor, cx| editor.set_first_key("gpu", window, cx));
    });
    t.t.fixture.draw_twice(cx);
    editor.read_with(cx, |editor, cx| {
        assert!(!editor.has_tried_review_now());
        assert_eq!(editor.shown_problem_now(cx), None);
    });
}

#[gpui_kit::test]
fn cancel_closes_an_untouched_editor_and_asks_before_losing_changes(cx: &mut TestAppContext) {
    let t = node_test("node-edit-cancel", cx);
    t.open_editor(NodeEditKind::Taints, &t.t.stg, "node-b", cx);
    let editor = t.wait_for_editor(cx);
    t.t.fixture.with_window(cx, |window, cx| {
        editor.update(cx, |editor, cx| editor.press_cancel(window, cx));
    });
    assert!(!t.has_dialog(cx), "nothing changed, so it closes at once");

    t.open_editor(NodeEditKind::Taints, &t.t.stg, "node-b", cx);
    let editor = t.wait_for_editor(cx);
    t.add_row(&editor, ("gpu", "true", "NoSchedule"), cx);
    t.t.fixture.with_window(cx, |window, cx| {
        editor.update(cx, |editor, cx| editor.press_cancel(window, cx));
    });
    // The question sits over the editor: closing it leaves the editor open.
    assert!(t.close_top_dialog(cx), "Keep editing leaves the editor");
    assert!(!t.close_top_dialog(cx));
}

#[gpui_kit::test]
fn escape_and_an_outside_click_never_drop_changed_rows(cx: &mut TestAppContext) {
    let t = node_test("node-edit-escape", cx);
    t.open_editor(NodeEditKind::Labels, &t.t.stg, "node-b", cx);
    let editor = t.wait_for_editor(cx);
    t.add_row(&editor, ("gpu", "true", ""), cx);
    t.t.fixture.draw_twice(cx);
    // A click on the backdrop does nothing.
    let mut visual = gpui_kit::VisualTestContext::from_window(t.t.fixture.window.into(), cx);
    visual.simulate_click(
        gpui_kit::point(gpui_kit::px(3.), gpui_kit::px(450.)),
        Default::default(),
    );
    assert!(t.has_dialog(cx));
    assert!(!t.close_top_dialog(cx), "only the editor was open");

    t.open_editor(NodeEditKind::Labels, &t.t.stg, "node-b", cx);
    let editor = t.wait_for_editor(cx);
    t.add_row(&editor, ("gpu", "true", ""), cx);
    t.t.fixture.draw_twice(cx);
    t.t.fixture.press("escape", cx);
    assert!(
        t.close_top_dialog(cx),
        "Escape asked, and the editor stayed"
    );
    assert!(!t.close_top_dialog(cx));
}

#[gpui_kit::test]
fn escape_closes_an_untouched_editor_at_once(cx: &mut TestAppContext) {
    let t = node_test("node-edit-escape-clean", cx);
    t.open_editor(NodeEditKind::Taints, &t.t.stg, "node-b", cx);
    t.wait_for_editor(cx);
    t.t.fixture.draw_twice(cx);
    t.t.fixture.press("escape", cx);
    assert!(!t.has_dialog(cx));
}

#[gpui_kit::test]
fn the_bulk_editor_asks_before_losing_a_typed_change(cx: &mut TestAppContext) {
    let t = node_test("bulk-labels-escape", cx);
    let editor = t.bulk_over_three(cx);
    editor.read_with(cx, |editor, _| assert_eq!(editor.row_count(), 1));
    t.t.fixture.draw_twice(cx);
    t.t.fixture.press("escape", cx);
    assert!(
        t.close_top_dialog(cx),
        "the typed change kept the editor open"
    );
    assert!(!t.close_top_dialog(cx));
}

// ---- L16: the page keys scroll a drawer that has the keyboard ----

impl NodeTest {
    fn node_cursor(&self, cx: &mut TestAppContext) -> Option<usize> {
        self.t
            .fixture
            .shell
            .read_with(cx, |shell, cx| shell.node_table.read(cx).selected_row())
    }

    fn is_drawer_keyed(&self, cx: &mut TestAppContext) -> bool {
        self.t
            .fixture
            .shell
            .read_with(cx, |shell, _| shell.is_drawer_keyed)
    }
}

#[gpui_kit::test]
fn page_keys_scroll_the_drawer_opened_with_enter_and_not_the_table(cx: &mut TestAppContext) {
    let t = node_test("node-drawer-keys", cx);
    three_nodes(&t, cx);
    t.t.fixture.press("j", cx);
    t.t.fixture.press("enter", cx);
    t.t.fixture.draw_twice(cx);
    assert!(
        t.is_drawer_keyed(cx),
        "Enter hands the keyboard to the drawer"
    );
    let cursor = t.node_cursor(cx);
    assert!(cursor.is_some());
    for key in ["pagedown", "end", "pageup", "home"] {
        t.t.fixture.press(key, cx);
        assert_eq!(
            t.node_cursor(cx),
            cursor,
            "{key} left the table cursor alone"
        );
    }
    // Closing the drawer gives the keys back to the table: End goes to the last row.
    t.t.fixture.press("escape", cx);
    assert!(!t.is_drawer_keyed(cx));
    t.t.fixture.press("end", cx);
    assert_eq!(t.node_cursor(cx), Some(2));
}

#[gpui_kit::test]
fn page_keys_move_the_table_while_the_drawer_was_only_opened_by_a_click(cx: &mut TestAppContext) {
    let t = node_test("node-drawer-keys-click", cx);
    three_nodes(&t, cx);
    t.t.fixture.shell.update(cx, |shell, cx| {
        shell
            .node_table
            .update(cx, |table, cx| table.set_selected_row(0, cx));
    });
    cx.run_until_parked();
    t.t.fixture.draw_twice(cx);
    assert!(!t.is_drawer_keyed(cx));
    t.t.fixture.press("end", cx);
    assert_eq!(t.node_cursor(cx), Some(2));
}
