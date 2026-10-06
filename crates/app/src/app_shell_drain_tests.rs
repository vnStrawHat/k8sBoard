//! The drain dialog in a headless window over two loaded clusters, one active at a time (`prod-a`,
//! locked at open, and `stg-b`, unlocked), each session with its own fake API server: a test sees
//! which cluster a request reached, and nothing leaves the machine.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use cluster::fake_api::{FakeApi, RecordedRequest};
use cluster::{NodeReadiness, NodeScheduling, NodeStatus, NodeSummary, NodeSystemInfo};
use gpui_kit::InputEvent as _;
use gpui_kit::{Entity, TestAppContext};
use serde_json::{Value, json};

use super::app_shell_switch_tests::open_switch_fixture;
use super::app_shell_write_tests::{Clusters, audit_lines, go_live_answering, switch_to, writes};
use super::drain_dialog::DrainDialog;
use super::write_flow::DryRunState;
use super::*;
use crate::app_shell::node_editor::NodeEditKind;
use crate::drain_plan::{Budget, DrainOption, PodCheck, PodVerdict, PreviewLine};
use crate::drain_tab::DrainTab;
use crate::environment::Environment;
use crate::table_selection::{ClusterObject, ResourceKey};
use crate::write_guard::{DialogConfirm, WriteLock};

const NODE_JSON: &str = r#"{"apiVersion":"v1","kind":"Node","metadata":{"name":"n"}}"#;
const PDB_REFUSAL: &str = "The disruption budget api-pdb needs 2 healthy pods and has 2 currently";

/// What the fake server of a cluster answers.
#[derive(Default)]
struct DrainServer {
    /// A committed eviction waits for this to be released, once.
    eviction_gate: Gate,
    /// A dry-run eviction waits for this to be released, once: the dialog is mid-check.
    dry_run_gate: Gate,
    pods: Vec<Value>,
    budgets: Vec<Value>,
    /// Pod names the eviction answers 429 for.
    refusing: Vec<String>,
    /// Pod names the eviction answers 403 for.
    forbidden: Vec<String>,
    /// The pod list answers 500.
    is_pod_list_broken: bool,
    /// The cordon patch answers 403.
    is_cordon_forbidden: bool,
    /// The access review of `delete` answers not allowed (the lazy check Skip PDBs reads).
    is_delete_denied: bool,
    /// Pod names a direct delete answers 429 for (the API's own rate limiting, not a budget).
    throttled: Vec<String>,
    /// The delete right holds in a namespace only: a review without one is denied.
    is_delete_namespaced_only: bool,
}

fn status(code: u16, reason: &str, message: &str, details: Value) -> (u16, String) {
    let body = json!({
        "kind": "Status", "apiVersion": "v1", "status": "Failure",
        "message": message, "reason": reason, "details": details, "code": code,
    });
    (code, body.to_string())
}

fn list_of(kind: &str, items: &[Value]) -> String {
    json!({
        "apiVersion": "v1", "kind": format!("{kind}List"), "metadata": {}, "items": items,
    })
    .to_string()
}

fn pod_json(namespace: &str, name: &str, owner: Option<&str>, empty_dir: bool) -> Value {
    let mut metadata = json!({
        "name": name, "namespace": namespace, "uid": format!("uid-{name}"),
        "labels": {"app": "api"},
    });
    if let Some(kind) = owner {
        metadata["ownerReferences"] = json!([{
            "apiVersion": "apps/v1", "kind": kind, "name": "owner", "uid": "o-1", "controller": true,
        }]);
    }
    let volumes = if empty_dir {
        json!([{"name": "scratch", "emptyDir": {}}])
    } else {
        json!([])
    };
    json!({
        "apiVersion": "v1", "kind": "Pod", "metadata": metadata,
        "spec": {"nodeName": "node-b", "containers": [], "volumes": volumes},
        "status": {"phase": "Running"},
    })
}

fn budget_json(name: &str, expected: u32, allowed: u32) -> Value {
    json!({
        "apiVersion": "policy/v1", "kind": "PodDisruptionBudget",
        "metadata": {"name": name, "namespace": "payments", "generation": 1},
        "spec": {"selector": {"matchLabels": {"app": "api"}}, "minAvailable": expected},
        "status": {
            "observedGeneration": 1, "currentHealthy": expected, "desiredHealthy": expected,
            "expectedPods": expected, "disruptionsAllowed": allowed,
        },
    })
}

fn server(
    state: Arc<Mutex<DrainServer>>,
) -> impl Fn(&RecordedRequest) -> (u16, String) + Send + Sync + 'static {
    let (gate, dry_run_gate) = {
        let state = state.lock().expect("the server state");
        (
            Arc::clone(&state.eviction_gate),
            Arc::clone(&state.dry_run_gate),
        )
    };
    move |request| {
        let is_dry_run_eviction = request.method == "POST"
            && request.path.ends_with("/eviction")
            && request.has_query("dryRun", "All");
        if is_dry_run_eviction {
            let held = dry_run_gate.lock().expect("the gate").take();
            if let Some(held) = held {
                let _ = held.recv_timeout(Duration::from_secs(10));
            }
        }
        let is_commit_eviction = request.method == "POST"
            && request.path.ends_with("/eviction")
            && !request.has_query("dryRun", "All");
        // A direct delete of a drain that skips the budgets waits at the same gate.
        let is_commit_delete = request.method == "DELETE" && !request.body.contains("\"dryRun\"");
        if is_commit_eviction || is_commit_delete {
            let held = gate.lock().expect("the gate").take();
            if let Some(held) = held {
                let _ = held.recv_timeout(Duration::from_secs(10));
            }
        }
        let mut state = state.lock().expect("the server state");
        let is_list = request.method == "GET" && !request.query.contains("watch=");
        if is_list && request.path == "/api/v1/pods" && request.query.contains("fieldSelector") {
            if state.is_pod_list_broken {
                return status(500, "InternalError", "boom", Value::Null);
            }
            return (200, list_of("Pod", &state.pods));
        }
        if is_list && request.path == "/apis/policy/v1/poddisruptionbudgets" {
            return (200, list_of("PodDisruptionBudget", &state.budgets));
        }
        if request.method == "POST" && request.path.ends_with("/selfsubjectaccessreviews") {
            let asks_delete = request.body.contains("\"verb\":\"delete\"");
            let is_cluster_wide = !request.body.contains("\"namespace\"");
            let is_denied =
                state.is_delete_denied || (state.is_delete_namespaced_only && is_cluster_wide);
            let is_allowed = !(asks_delete && is_denied);
            let review = json!({
                "apiVersion": "authorization.k8s.io/v1", "kind": "SelfSubjectAccessReview",
                "metadata": {}, "spec": {}, "status": {"allowed": is_allowed},
            });
            return (201, review.to_string());
        }
        if request.method == "DELETE" {
            let name = request
                .path
                .rsplit('/')
                .next()
                .unwrap_or_default()
                .to_owned();
            if state.throttled.contains(&name) {
                return status(429, "TooManyRequests", "Too many requests", Value::Null);
            }
            // A committed delete takes the pod off the node; a dry-run leaves it.
            if !request.body.contains("\"dryRun\"") {
                state
                    .pods
                    .retain(|pod| pod["metadata"]["name"].as_str() != Some(name.as_str()));
            }
            let ok =
                json!({"kind": "Status", "apiVersion": "v1", "status": "Success", "code": 200});
            return (200, ok.to_string());
        }
        if request.method == "PATCH" {
            if state.is_cordon_forbidden {
                return status(
                    403,
                    "Forbidden",
                    "nodes \"n\" is forbidden: User \"u\" cannot patch resource \"nodes\"",
                    Value::Null,
                );
            }
            return (200, NODE_JSON.to_owned());
        }
        if request.method == "POST" && request.path.ends_with("/eviction") {
            let name = request
                .path
                .trim_end_matches("/eviction")
                .rsplit('/')
                .next()
                .unwrap_or_default()
                .to_owned();
            let name = name.as_str();
            if state.refusing.iter().any(|refused| refused == name) {
                return status(
                    429,
                    "TooManyRequests",
                    "Cannot evict pod as it would violate the pod's disruption budget.",
                    json!({"causes": [{"reason": "DisruptionBudget", "message": PDB_REFUSAL}]}),
                );
            }
            if state.forbidden.iter().any(|forbidden| forbidden == name) {
                return status(
                    403,
                    "Forbidden",
                    "pods/eviction \"x\" is forbidden: User \"u\" cannot create resource \"pods/eviction\"",
                    Value::Null,
                );
            }
            // A committed eviction takes the pod off the node; a dry-run leaves it.
            if !request.has_query("dryRun", "All") {
                state
                    .pods
                    .retain(|pod| pod["metadata"]["name"].as_str() != Some(name));
            }
            let ok =
                json!({"kind": "Status", "apiVersion": "v1", "status": "Success", "code": 201});
            return (201, ok.to_string());
        }
        status(404, "NotFound", "no", Value::Null)
    }
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

type Gate = Arc<Mutex<Option<std::sync::mpsc::Receiver<()>>>>;

struct DrainTest {
    t: Clusters,
    prod_state: Arc<Mutex<DrainServer>>,
    /// Holds the next committed eviction of the staging server.
    stg_gate: Gate,
    /// Holds the next dry-run eviction of the staging server.
    stg_dry_run_gate: Gate,
}

fn drain_test(
    name: &str,
    setup: impl FnOnce(&mut DrainServer),
    cx: &mut TestAppContext,
) -> DrainTest {
    let fixture = open_switch_fixture(name, cx);
    switch_to(&fixture, "stg-b", cx);
    let (prod, stg) = (fixture.cluster("prod-a", cx), fixture.cluster("stg-b", cx));
    let mut stg_server = DrainServer::default();
    setup(&mut stg_server);
    let stg_gate = Arc::clone(&stg_server.eviction_gate);
    let stg_dry_run_gate = Arc::clone(&stg_server.dry_run_gate);
    let stg_state = Arc::new(Mutex::new(stg_server));
    let prod_state = Arc::new(Mutex::new(DrainServer::default()));
    let stg_api = go_live_answering(&fixture, &stg, "node-b", server(Arc::clone(&stg_state)), cx);
    fixture
        .shell
        .update(cx, |shell, cx| shell.show_screen(Screen::Nodes, cx));
    cx.run_until_parked();
    fixture.draw_twice(cx);
    DrainTest {
        t: Clusters {
            fixture,
            stg_api,
            prod,
            stg,
        },
        prod_state,
        stg_gate,
        stg_dry_run_gate,
    }
}

impl DrainTest {
    /// Switches to `prod-a` and makes it live over a new fake server on the Nodes screen. The old
    /// session is gone, so a test never has both clusters live at once.
    fn activate_prod(&self, cx: &mut TestAppContext) -> FakeApi {
        let api = self.t.activate_answering(
            &self.t.prod,
            "node-a",
            server(Arc::clone(&self.prod_state)),
            cx,
        );
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

    fn cursor_on_node(&self, cluster: &ClusterRef, name: &str, cx: &mut TestAppContext) {
        let object = ClusterObject::new(
            cluster.clone(),
            ResourceKey::Node {
                name: name.to_owned(),
            },
        );
        self.t.fixture.shell.update(cx, |shell, cx| {
            shell.change_selection(Some(object), cx);
        });
        cx.run_until_parked();
    }

    fn open(&self, cluster: &ClusterRef, nodes: &[&str], cx: &mut TestAppContext) {
        let nodes: Vec<String> = nodes.iter().map(|node| (*node).to_owned()).collect();
        self.t.fixture.with_window(cx, |window, cx| {
            self.t.fixture.shell.update(cx, |shell, cx| {
                shell.start_drain(cluster, &nodes, window, cx);
            });
        });
    }

    fn dialog(&self, cx: &mut TestAppContext) -> Option<Entity<DrainDialog>> {
        self.t
            .fixture
            .shell
            .read_with(cx, |shell, _| shell.last_drain_dialog.clone())
            .and_then(|dialog| dialog.upgrade())
    }

    /// Opens the dialog and waits until the reads are done and every dry-run has answered.
    fn open_and_settle(
        &self,
        cluster: &ClusterRef,
        nodes: &[&str],
        cx: &mut TestAppContext,
    ) -> Entity<DrainDialog> {
        self.open(cluster, nodes, cx);
        let dialog = self.dialog(cx).expect("a drain dialog");
        self.settle(&dialog, cx);
        dialog
    }

    fn settle(&self, dialog: &Entity<DrainDialog>, cx: &mut TestAppContext) {
        self.t.wait_for("the dry-runs", cx, |cx| {
            dialog.read_with(cx, |dialog, _| {
                !dialog.is_busy_loading() && !matches!(dialog.state(), DryRunState::Running)
            })
        });
    }
}

fn eviction_posts(api: &FakeApi) -> Vec<RecordedRequest> {
    writes(api)
        .into_iter()
        .filter(|request| request.method == "POST" && request.path.ends_with("/eviction"))
        .collect()
}

fn pod_lists(api: &FakeApi) -> Vec<RecordedRequest> {
    api.requests()
        .into_iter()
        .filter(|request| {
            request.method == "GET"
                && request.path == "/api/v1/pods"
                && request.query.contains("fieldSelector")
        })
        .collect()
}

fn three_pods(server: &mut DrainServer) {
    server.pods = vec![
        pod_json("payments", "api-1", Some("ReplicaSet"), false),
        pod_json("payments", "api-2", Some("ReplicaSet"), false),
        pod_json("kube-system", "agent-1", Some("DaemonSet"), false),
    ];
}

#[gpui_kit::test]
fn drain_uses_the_active_cluster(cx: &mut TestAppContext) {
    let t = drain_test("drain-slot", three_pods, cx);
    // The cursor is on a node of the open cluster, `stg-b`; `prod-a` is the locked one.
    t.cursor_on_node(&t.t.stg, "node-b", cx);
    t.t.fixture.press("d", cx);
    let dialog = t.dialog(cx).expect("D opens the dialog");
    t.settle(&dialog, cx);
    dialog.read_with(cx, |dialog, _| {
        // The guard and the tier are stg-b's own: a click on STG, never prod-a's typed name.
        assert_eq!(*dialog.tier(), DialogConfirm::Click);
        assert_eq!(dialog.environment(), &Environment::STAGING);
        assert_eq!(dialog.expected_name(), "node-b");
        assert_eq!(dialog.plans().len(), 1);
        assert_eq!(dialog.plans()[0].node, "node-b");
    });
    // Reads and dry-runs went to the node's cluster only.
    assert_eq!(pod_lists(&t.t.stg_api).len(), 1);
    let sent = writes(&t.t.stg_api);
    assert_eq!(sent.len(), 3, "a cordon and two evictions: {sent:?}");
    assert!(
        sent.iter()
            .all(|request| request.has_query("dryRun", "All"))
    );
    assert_eq!(sent[0].method, "PATCH");
    assert_eq!(sent[0].path, "/api/v1/nodes/node-b");
    assert_eq!(
        sent[1].path,
        "/api/v1/namespaces/payments/pods/api-1/eviction"
    );
    assert_eq!(
        sent[2].path,
        "/api/v1/namespaces/payments/pods/api-2/eviction"
    );
}

#[gpui_kit::test]
fn the_d_key_opens_a_dialog_and_nothing_runs_from_it(cx: &mut TestAppContext) {
    let t = drain_test("drain-d-key", three_pods, cx);
    t.cursor_on_node(&t.t.stg, "node-b", cx);
    t.t.fixture.press("d", cx);
    let dialog = t.dialog(cx).expect("a dialog");
    t.settle(&dialog, cx);
    // Only dry-runs left the app: no commit, however long it waits.
    cx.run_until_parked();
    assert!(
        writes(&t.t.stg_api)
            .iter()
            .all(|request| request.has_query("dryRun", "All"))
    );
    // Everything passed, so the button is on; it still waits for the user.
    assert_eq!(
        dialog.read_with(cx, |dialog, cx| dialog.drain_blocked_by(cx)),
        None
    );
    assert!(t.tab(cx).is_none(), "nothing runs until Drain is pressed");
}

#[gpui_kit::test]
fn the_dialog_on_a_locked_cluster_opens_as_a_preview_with_no_confirm(cx: &mut TestAppContext) {
    let t = drain_test("drain-locked", three_pods, cx);
    let prod_api = t.activate_prod(cx);
    t.prod_state.lock().expect("state").pods =
        vec![pod_json("payments", "api-1", Some("ReplicaSet"), false)];
    let dialog = t.open_and_settle(&t.t.prod, &["node-a"], cx);
    let reason = Some("prod-a is read-only".into());
    dialog.read_with(cx, |dialog, cx| {
        assert!(dialog.is_preview());
        // The gate's reason stands where the confirm buttons would be, for both of them.
        assert_eq!(dialog.drain_blocked_by(cx), reason);
        assert_eq!(dialog.cordon_blocked_by(cx), reason);
        // The plan is read as usual, and no pod was asked about.
        assert_eq!(dialog.plans().len(), 1);
        let planned = dialog.plans()[0]
            .evictions()
            .next()
            .expect("a pod to evict");
        assert_eq!(dialog.check_of(&planned.pod.uid), PodCheck::NotChecked);
    });
    assert_eq!(pod_lists(&prod_api).len(), 1);
    // A preview sends nothing at all, not even a dry-run, and nothing the user presses commits.
    t.t.fixture.with_window(cx, |window, cx| {
        dialog.update(cx, |dialog, cx| {
            dialog.press_drain(window, cx);
            dialog.press_cordon_only(window, cx);
        });
    });
    cx.run_until_parked();
    assert!(t.tab(cx).is_none());
    assert!(writes(&prod_api).is_empty());
}

#[gpui_kit::test]
fn the_dialog_of_an_unlocked_cluster_is_not_a_preview(cx: &mut TestAppContext) {
    let t = drain_test("drain-not-preview", three_pods, cx);
    let dialog = t.open_and_settle(&t.t.stg, &["node-b"], cx);
    assert!(!dialog.read_with(cx, |dialog, _| dialog.is_preview()));
}

#[gpui_kit::test]
fn drain_dialog_requires_the_node_name_on_prod(cx: &mut TestAppContext) {
    let t = drain_test("drain-prod", three_pods, cx);
    t.activate_prod(cx);
    t.t.set_lock(&t.t.prod, WriteLock::Unlocked, cx);
    t.prod_state.lock().expect("state").pods =
        vec![pod_json("payments", "api-1", Some("ReplicaSet"), false)];
    let dialog = t.open_and_settle(&t.t.prod, &["node-a"], cx);
    dialog.read_with(cx, |dialog, _| {
        assert_eq!(
            *dialog.tier(),
            DialogConfirm::TypeName {
                expected: "node-a".to_owned()
            }
        );
        assert_eq!(dialog.environment(), &Environment::PRODUCTION);
    });
}

#[gpui_kit::test]
fn drain_of_several_nodes_types_the_cluster_name(cx: &mut TestAppContext) {
    let t = drain_test("drain-several", three_pods, cx);
    let prod_api = t.activate_prod(cx);
    t.t.set_lock(&t.t.prod, WriteLock::Unlocked, cx);
    t.set_nodes(
        &t.t.prod,
        vec![
            summary("node-a", NodeScheduling::Enabled),
            summary("node-c", NodeScheduling::Enabled),
        ],
        cx,
    );
    let dialog = t.open_and_settle(&t.t.prod, &["node-a", "node-c"], cx);
    dialog.read_with(cx, |dialog, _| {
        assert_eq!(
            *dialog.tier(),
            DialogConfirm::TypeName {
                expected: "prod-a".to_owned()
            }
        );
        assert_eq!(dialog.expected_name(), "prod-a");
        assert_eq!(dialog.plans().len(), 2);
    });
    // One pod list per node, one budget list, and a cordon dry-run for each node.
    assert_eq!(pod_lists(&prod_api).len(), 2);
    assert_eq!(
        writes(&prod_api)
            .iter()
            .filter(|request| request.method == "PATCH")
            .count(),
        2
    );
}

#[gpui_kit::test]
fn an_already_cordoned_node_is_not_dry_run_again(cx: &mut TestAppContext) {
    let t = drain_test("drain-cordoned", three_pods, cx);
    t.set_nodes(
        &t.t.stg,
        vec![summary("node-b", NodeScheduling::Disabled)],
        cx,
    );
    let dialog = t.open_and_settle(&t.t.stg, &["node-b"], cx);
    let sent = writes(&t.t.stg_api);
    assert_eq!(sent.len(), 2, "two evictions, no cordon: {sent:?}");
    assert!(sent.iter().all(|request| request.method == "POST"));
    dialog.read_with(cx, |dialog, cx| {
        assert_eq!(
            dialog.cordon_blocked_by(cx).as_deref(),
            Some("The node is already cordoned")
        );
    });
}

#[gpui_kit::test]
fn a_429_is_a_wait_and_never_a_failure(cx: &mut TestAppContext) {
    let t = drain_test(
        "drain-429",
        |server| {
            three_pods(server);
            server.refusing = vec!["api-2".to_owned()];
        },
        cx,
    );
    let dialog = t.open_and_settle(&t.t.stg, &["node-b"], cx);
    dialog.read_with(cx, |dialog, _| {
        assert_eq!(dialog.check_of("uid-api-1"), PodCheck::Accepted);
        assert_eq!(
            dialog.check_of("uid-api-2"),
            PodCheck::Refused(PDB_REFUSAL.into())
        );
        assert!(matches!(dialog.state(), DryRunState::Passed { .. }));
        assert_eq!(
            dialog.dry_run_line(),
            "Server dry-run: cordon passed · 1 of 2 evictions accepted, 1 refused by PDB"
        );
    });
    // The refused pod floats to the top: a short form, with the server's words as the tooltip.
    let first = dialog.read_with(cx, |dialog, _| dialog.preview()[0].clone());
    match first {
        PreviewLine::Pod {
            name,
            result,
            detail,
            ..
        } => {
            assert_eq!(name.as_ref(), "api-2");
            assert_eq!(result.as_ref(), "PDB api-pdb: 0 allowed (2/2 healthy)");
            assert_eq!(detail.as_deref(), Some(PDB_REFUSAL));
        }
        other => panic!("expected a pod line, got {other:?}"),
    }
}

#[gpui_kit::test]
fn an_eviction_dry_run_that_fails_otherwise_fails_the_dry_run(cx: &mut TestAppContext) {
    let t = drain_test(
        "drain-forbidden",
        |server| {
            three_pods(server);
            server.forbidden = vec!["api-1".to_owned()];
        },
        cx,
    );
    let dialog = t.open_and_settle(&t.t.stg, &["node-b"], cx);
    dialog.read_with(cx, |dialog, cx| {
        assert!(matches!(dialog.state(), DryRunState::Failed(_)));
        assert!(matches!(dialog.check_of("uid-api-1"), PodCheck::Failed(_)));
        assert!(dialog.drain_blocked_by(cx).is_some());
        // Cordon only is held by its own check alone.
        assert!(matches!(
            dialog.cordon_dry_run(),
            DryRunState::Passed { .. }
        ));
        assert_eq!(dialog.cordon_blocked_by(cx), None);
    });
}

#[gpui_kit::test]
fn a_failed_cordon_dry_run_holds_both_buttons(cx: &mut TestAppContext) {
    let t = drain_test(
        "drain-cordon-denied",
        |server| {
            three_pods(server);
            server.is_cordon_forbidden = true;
        },
        cx,
    );
    let dialog = t.open_and_settle(&t.t.stg, &["node-b"], cx);
    dialog.read_with(cx, |dialog, cx| {
        assert!(matches!(dialog.state(), DryRunState::Failed(_)));
        assert!(dialog.drain_blocked_by(cx).is_some());
        assert!(dialog.cordon_blocked_by(cx).is_some());
    });
}

#[gpui_kit::test]
fn passing_dry_run_downgrades_blocked_and_waits(cx: &mut TestAppContext) {
    let t = drain_test(
        "drain-downgrade",
        |server| {
            three_pods(server);
            // A budget that allows one eviction: the second pod waits on it locally.
            server.budgets = vec![budget_json("api-pdb", 2, 1)];
        },
        cx,
    );
    let dialog = t.open_and_settle(&t.t.stg, &["node-b"], cx);
    dialog.read_with(cx, |dialog, _| {
        let plan = &dialog.plans()[0];
        let verdicts: Vec<&PodVerdict> = plan
            .pods
            .iter()
            .filter(|planned| planned.pod.namespace == "payments")
            .map(|planned| &planned.verdict)
            .collect();
        assert!(matches!(
            verdicts[0],
            PodVerdict::Evict(Budget::Allows { .. })
        ));
        assert!(matches!(
            verdicts[1],
            PodVerdict::Evict(Budget::Waits { .. })
        ));
        // The server would evict it now, and says so.
        let texts: Vec<String> = dialog
            .preview()
            .iter()
            .filter_map(|line| match line {
                PreviewLine::Pod { name, result, .. } if name.as_ref() == "api-2" => {
                    Some(result.to_string())
                }
                _ => None,
            })
            .collect();
        assert_eq!(texts, ["Dry-run accepted"]);
    });
}

#[gpui_kit::test]
fn ticking_an_option_dry_runs_the_pods_it_adds(cx: &mut TestAppContext) {
    let t = drain_test(
        "drain-option",
        |server| {
            server.pods = vec![
                pod_json("payments", "api-1", Some("ReplicaSet"), false),
                pod_json("data", "cache-0", Some("StatefulSet"), true),
            ];
        },
        cx,
    );
    let dialog = t.open_and_settle(&t.t.stg, &["node-b"], cx);
    assert_eq!(eviction_posts(&t.t.stg_api).len(), 1);
    dialog.read_with(cx, |dialog, cx| {
        let blocker = dialog.drain_blocked_by(cx).expect("a pod needs an option");
        assert!(blocker.starts_with("1 pod needs \"Delete emptyDir data\""));
    });
    dialog.update(cx, |dialog, cx| {
        dialog.tick(DrainOption::DeleteEmptyDir, true, cx)
    });
    t.settle(&dialog, cx);
    t.t.wait_for("the new dry-run", cx, |_| {
        eviction_posts(&t.t.stg_api).len() == 2
    });
    assert_eq!(
        eviction_posts(&t.t.stg_api)[1].path,
        "/api/v1/namespaces/data/pods/cache-0/eviction"
    );
    dialog.read_with(cx, |dialog, cx| {
        assert_eq!(dialog.drain_blocked_by(cx), None);
        assert!(dialog.options().delete_empty_dir);
    });
}

#[gpui_kit::test]
fn grace_and_timeout_choices_reach_the_options(cx: &mut TestAppContext) {
    let t = drain_test("drain-choices", three_pods, cx);
    let dialog = t.open_and_settle(&t.t.stg, &["node-b"], cx);
    t.t.fixture.with_window(cx, |window, cx| {
        dialog.update(cx, |dialog, cx| {
            dialog.pick_grace(2, window, cx);
            dialog.pick_timeout(3, window, cx);
        });
    });
    dialog.read_with(cx, |dialog, _| {
        assert_eq!(dialog.options().grace, cluster::GracePeriod::Seconds(30));
        assert_eq!(dialog.options().timeout.as_secs(), 30 * 60);
    });
}

#[gpui_kit::test]
fn a_pod_list_that_fails_is_shown_and_blocks_drain_but_not_cordon_only(cx: &mut TestAppContext) {
    let t = drain_test(
        "drain-list-fails",
        |server| {
            three_pods(server);
            server.is_pod_list_broken = true;
        },
        cx,
    );
    let dialog = t.open_and_settle(&t.t.stg, &["node-b"], cx);
    // The cordon needs no pod list, so its dry-run still runs.
    t.t.wait_for("the cordon dry-run", cx, |cx| {
        dialog.read_with(cx, |dialog, _| {
            !matches!(dialog.cordon_dry_run(), DryRunState::Running)
        })
    });
    dialog.read_with(cx, |dialog, cx| {
        let reason = dialog.drain_blocked_by(cx).expect("the read failed");
        assert!(
            reason.starts_with("Could not list pods on node-b:"),
            "{reason}"
        );
        assert_eq!(dialog.cordon_blocked_by(cx), None);
    });
}

#[gpui_kit::test]
fn cordon_only_sends_no_eviction(cx: &mut TestAppContext) {
    let t = drain_test("drain-cordon-only", three_pods, cx);
    let dir = t.t.enable_audit_folder("drain-cordon-only", cx);
    let dialog = t.open_and_settle(&t.t.stg, &["node-b"], cx);
    assert_eq!(eviction_posts(&t.t.stg_api).len(), 2, "the dry-runs");
    t.t.fixture.with_window(cx, |window, cx| {
        dialog.update(cx, |dialog, cx| dialog.press_cordon_only(window, cx));
    });
    t.t.wait_for("the commit", cx, |_| {
        writes(&t.t.stg_api)
            .iter()
            .any(|request| request.method == "PATCH" && !request.has_query_key("dryRun"))
    });
    t.t.wait_for("the audit line", cx, |_| audit_lines(&dir).len() == 1);
    // Still the two dry-run evictions, and none without `dryRun`.
    assert!(
        eviction_posts(&t.t.stg_api)
            .iter()
            .all(|request| request.has_query("dryRun", "All"))
    );
    let line = &audit_lines(&dir)[0];
    assert_eq!(line["action"], "Cordon");
    assert_eq!(line["cluster"], "stg-b");
    assert_eq!(line["object"]["name"], "node-b");
    assert_eq!(line["outcome"], "applied");
    t.t.wait_for("the dialog to close", cx, |cx| {
        dialog.read_with(cx, |dialog, _| !dialog.is_open())
    });
}

#[gpui_kit::test]
fn cordon_only_commits_through_checked_write(cx: &mut TestAppContext) {
    // A lock after the dry-runs stops the commit: nothing but dry-runs left.
    let t = drain_test("drain-cordon-lock", three_pods, cx);
    let dialog = t.open_and_settle(&t.t.stg, &["node-b"], cx);
    t.t.set_lock(&t.t.stg, WriteLock::Locked, cx);
    t.t.fixture.with_window(cx, |window, cx| {
        dialog.update(cx, |dialog, cx| dialog.press_cordon_only(window, cx));
    });
    cx.run_until_parked();
    assert!(
        writes(&t.t.stg_api)
            .iter()
            .all(|request| request.has_query("dryRun", "All"))
    );
}

#[gpui_kit::test]
fn cordon_only_needs_the_typed_name_on_prod(cx: &mut TestAppContext) {
    let t = drain_test("drain-cordon-typed", three_pods, cx);
    let prod_api = t.activate_prod(cx);
    t.t.set_lock(&t.t.prod, WriteLock::Unlocked, cx);
    let dialog = t.open_and_settle(&t.t.prod, &["node-a"], cx);
    t.t.fixture.with_window(cx, |window, cx| {
        dialog.update(cx, |dialog, cx| dialog.press_cordon_only(window, cx));
    });
    cx.run_until_parked();
    // The name is not typed: nothing is committed.
    assert!(
        writes(&prod_api)
            .iter()
            .all(|request| request.has_query("dryRun", "All"))
    );
    t.t.fixture.with_window(cx, |window, cx| {
        dialog.update(cx, |dialog, cx| {
            dialog.type_text("node-a", window, cx);
            dialog.press_cordon_only(window, cx);
        });
    });
    t.t.wait_for("the commit", cx, |_| {
        writes(&prod_api)
            .iter()
            .any(|request| request.method == "PATCH" && !request.has_query_key("dryRun"))
    });
}

impl DrainTest {
    fn tick(&self, rows: &[usize], cx: &mut TestAppContext) {
        for &row in rows {
            self.t.fixture.shell.update(cx, |shell, cx| {
                shell.check_rows(crate::table_view::RowCheck::Toggle(row), cx);
            });
        }
        cx.run_until_parked();
        self.t.fixture.draw_twice(cx);
    }

    fn drain_button(&self, cx: &mut TestAppContext) -> crate::row_selection::BulkState {
        self.t.fixture.shell.read_with(cx, |shell, cx| {
            shell
                .bulk_buttons(cx)
                .into_iter()
                .find(|button| button.label.as_ref() == "Drain…")
                .map(|button| button.state)
                .expect("the Nodes bar has a Drain button")
        })
    }
}

#[gpui_kit::test]
fn the_selection_bar_drains_the_ticked_nodes_of_one_cluster(cx: &mut TestAppContext) {
    let t = drain_test("drain-bar", three_pods, cx);
    t.set_nodes(
        &t.t.stg,
        vec![
            summary("n1", NodeScheduling::Enabled),
            summary("n2", NodeScheduling::Disabled),
            summary("n3", NodeScheduling::Enabled),
        ],
        cx,
    );
    assert_eq!(
        t.drain_button(cx),
        crate::row_selection::BulkState::Off("Select rows first".into())
    );
    t.tick(&[0, 1], cx);
    assert_eq!(
        t.drain_button(cx),
        crate::row_selection::BulkState::Ready(crate::resource_actions::ResourceAction::Drain)
    );
    t.t.fixture.with_window(cx, |window, cx| {
        t.t.fixture.shell.update(cx, |shell, cx| {
            shell.run_bulk(crate::resource_actions::ResourceAction::Drain, window, cx);
        });
    });
    let dialog = t.dialog(cx).expect("the bar opens the dialog");
    t.settle(&dialog, cx);
    dialog.read_with(cx, |dialog, _| {
        // Both ticked nodes, in table order; several nodes type the cluster name.
        let nodes: Vec<&str> = dialog
            .plans()
            .iter()
            .map(|plan| plan.node.as_str())
            .collect();
        assert_eq!(nodes, ["n1", "n2"]);
        assert_eq!(dialog.expected_name(), "stg-b");
    });
    // n2 is cordoned already, so only n1 gets a cordon dry-run.
    let cordons: Vec<String> = writes(&t.t.stg_api)
        .into_iter()
        .filter(|request| request.method == "PATCH")
        .map(|request| request.path)
        .collect();
    assert_eq!(cordons, ["/api/v1/nodes/n1"]);
}

#[gpui_kit::test]
fn the_bar_drain_is_a_preview_on_a_locked_cluster(cx: &mut TestAppContext) {
    let t = drain_test("drain-bar-locked", three_pods, cx);
    t.activate_prod(cx);
    t.tick(&[0], cx);
    assert_eq!(
        t.drain_button(cx),
        crate::row_selection::BulkState::Preview(
            crate::resource_actions::ResourceAction::Drain,
            "prod-a is read-only".into()
        )
    );
}

// ---- The run ----

fn key_down(key: &str, is_held: bool) -> gpui_kit::KeyDownEvent {
    gpui_kit::KeyDownEvent {
        keystroke: gpui_kit::Keystroke::parse(key).expect("a valid keystroke"),
        is_held,
        prefer_character_input: false,
    }
}

impl DrainTest {
    fn tab(&self, cx: &mut TestAppContext) -> Option<Entity<DrainTab>> {
        self.t
            .fixture
            .shell
            .read_with(cx, |shell, _| shell.last_drain_tab.clone())
            .and_then(|tab| tab.upgrade())
    }

    fn press_drain(&self, dialog: &Entity<DrainDialog>, cx: &mut TestAppContext) {
        self.t.fixture.with_window(cx, |window, cx| {
            dialog.update(cx, |dialog, cx| dialog.press_drain(window, cx));
        });
    }

    /// Opens the dialog on `node-b`, waits for its dry-runs, and presses Drain.
    fn start(&self, cx: &mut TestAppContext) -> Entity<DrainTab> {
        let dialog = self.open_and_settle(&self.t.stg, &["node-b"], cx);
        self.press_drain(&dialog, cx);
        self.t.wait_for("the tab", cx, |cx| self.tab(cx).is_some());
        self.tab(cx).expect("a drain tab")
    }

    fn wait_for_end(&self, tab: &Entity<DrainTab>, cx: &mut TestAppContext) {
        self.t.wait_for("the run to end", cx, |cx| {
            tab.read_with(cx, |tab, _| !tab.is_running())
        });
    }

    fn dock_tabs(&self, cx: &mut TestAppContext) -> usize {
        self.t
            .fixture
            .shell
            .read_with(cx, |shell, cx| shell.dock.read(cx).tab_count())
    }
}

fn evictions(api: &FakeApi, is_dry_run: bool) -> Vec<String> {
    eviction_posts(api)
        .into_iter()
        .filter(|request| request.has_query("dryRun", "All") == is_dry_run)
        .map(|request| request.path)
        .collect()
}

#[gpui_kit::test]
fn a_drain_cordons_evicts_waits_and_ends_drained(cx: &mut TestAppContext) {
    let t = drain_test("drain-run", three_pods, cx);
    let dir = t.t.enable_audit_folder("drain-run", cx);
    let tab = t.start(cx);
    t.wait_for_end(&tab, cx);
    tab.read_with(cx, |tab, _| {
        assert!(tab.run().is_drained());
        assert_eq!(tab.run().cordoned(), ["node-b".to_owned()]);
        assert_eq!(
            tab.run().end_notice().as_deref(),
            Some("Drain: node-b drained")
        );
    });
    // The cordon, then each eviction, committed once each; the DaemonSet pod was left alone.
    let sent = writes(&t.t.stg_api);
    let commits: Vec<(&str, &str)> = sent
        .iter()
        .filter(|request| !request.has_query_key("dryRun"))
        .map(|request| (request.method.as_str(), request.path.as_str()))
        .collect();
    assert_eq!(
        commits,
        [
            ("PATCH", "/api/v1/nodes/node-b"),
            ("POST", "/api/v1/namespaces/payments/pods/api-1/eviction"),
            ("POST", "/api/v1/namespaces/payments/pods/api-2/eviction"),
        ]
    );
    // The dialog already dry-ran the pods, so the run did not dry-run them again.
    assert_eq!(evictions(&t.t.stg_api, true).len(), 2);
    // The commit sent the uid-pinned body.
    let body: Value = serde_json::from_str(
        &eviction_posts(&t.t.stg_api)
            .into_iter()
            .find(|request| !request.has_query_key("dryRun"))
            .expect("a committed eviction")
            .body,
    )
    .expect("a JSON body");
    assert_eq!(body["deleteOptions"]["preconditions"]["uid"], "uid-api-1");
    // One line per commit and one summary for the node.
    t.t.wait_for("the audit lines", cx, |_| audit_lines(&dir).len() == 4);
    let lines = audit_lines(&dir);
    let actions: Vec<&str> = lines
        .iter()
        .map(|line| line["action"].as_str().unwrap_or_default())
        .collect();
    assert_eq!(actions, ["Cordon", "Evict", "Evict", "Drain"]);
    let summary = &lines[3];
    assert_eq!(summary["outcome"], "drained");
    assert_eq!(summary["object"]["name"], "node-b");
    assert_eq!(summary["cluster"], "stg-b");
    assert_eq!(
        summary["fields"],
        json!([
            {"path": "evicted", "value": "2"},
            {"path": "refused", "value": "0"},
            {"path": "failed", "value": "0"},
            {"path": "skipped", "value": "1"},
        ])
    );
    assert!(lines[1]["outcome"] == "applied" && lines[1]["object"]["kind"] == "Pod");
}

#[gpui_kit::test]
fn the_dialog_closes_and_one_tab_shows_the_run(cx: &mut TestAppContext) {
    let t = drain_test("drain-tab-opens", three_pods, cx);
    let dialog = t.open_and_settle(&t.t.stg, &["node-b"], cx);
    t.press_drain(&dialog, cx);
    t.t.wait_for("the tab", cx, |cx| t.tab(cx).is_some());
    t.t.wait_for("the dialog to close", cx, |cx| {
        dialog.read_with(cx, |dialog, _| !dialog.is_open())
    });
    assert_eq!(t.dock_tabs(cx), 1);
    let tab = t.tab(cx).expect("a tab");
    tab.read_with(cx, |tab, _| {
        assert_eq!(tab.label(), "Drain node-b");
        assert_eq!(tab.cluster_name().as_ref(), "stg-b");
    });
}

fn refusing_api_2(server: &mut DrainServer) {
    three_pods(server);
    server.refusing = vec!["api-2".to_owned()];
}

#[gpui_kit::test]
fn refusals_are_not_audited_and_cancel_keeps_the_node_cordoned(cx: &mut TestAppContext) {
    let t = drain_test("drain-refused", refusing_api_2, cx);
    let dir = t.t.enable_audit_folder("drain-refused", cx);
    let tab = t.start(cx);
    // api-1 is evicted; api-2 is refused by its budget and waits for the backoff.
    t.t.wait_for("the refusal", cx, |cx| {
        tab.read_with(cx, |tab, _| {
            tab.run()
                .pod_rows(Duration::ZERO)
                .iter()
                .any(|row| row.text.starts_with("Refused by PDB"))
        })
    });
    let before = evictions(&t.t.stg_api, false).len();
    assert_eq!(before, 2, "api-1 accepted and api-2 refused were both sent");
    tab.update(cx, |tab, cx| tab.cancel(cx));
    t.wait_for_end(&tab, cx);
    cx.run_until_parked();
    // Nothing was sent after the click, and nothing was uncordoned.
    assert_eq!(evictions(&t.t.stg_api, false).len(), before);
    assert!(writes(&t.t.stg_api).iter().all(|request| {
        !(request.method == "PATCH"
            && !request.has_query_key("dryRun")
            && request.body.contains("false"))
    }));
    tab.read_with(cx, |tab, _| {
        assert_eq!(tab.run().cordoned(), ["node-b".to_owned()]);
        assert_eq!(tab.run().end_notice().as_deref(), Some("Drain cancelled"));
    });
    // The audit has the cordon, the accepted eviction, and the cancelled summary: the 429 left no
    // line.
    t.t.wait_for("the audit lines", cx, |_| audit_lines(&dir).len() == 3);
    let lines = audit_lines(&dir);
    let actions: Vec<&str> = lines
        .iter()
        .map(|line| line["action"].as_str().unwrap_or_default())
        .collect();
    assert_eq!(actions, ["Cordon", "Evict", "Drain"]);
    assert_eq!(lines[2]["outcome"], "cancelled");
    assert_eq!(
        lines[2]["fields"][0],
        json!({"path": "evicted", "value": "1"})
    );
    assert_eq!(
        lines[2]["fields"][1],
        json!({"path": "refused", "value": "1"})
    );
}

#[gpui_kit::test]
fn drain_tab_cannot_close_while_running(cx: &mut TestAppContext) {
    let t = drain_test("drain-tab-close", refusing_api_2, cx);
    let tab = t.start(cx);
    t.t.wait_for("the refusal", cx, |cx| {
        tab.read_with(cx, |tab, _| tab.run().pod_rows(Duration::ZERO).len() == 3)
    });
    assert_eq!(t.dock_tabs(cx), 1);
    // The close button and Ctrl W both refuse a running drain.
    t.t.fixture.shell.update(cx, |shell, cx| {
        shell.dock.update(cx, |dock, cx| {
            dock.close_tab(0, cx);
            dock.close_active_tab(cx);
        });
    });
    assert_eq!(t.dock_tabs(cx), 1);
    // After Cancel it closes.
    tab.update(cx, |tab, cx| tab.cancel(cx));
    t.wait_for_end(&tab, cx);
    t.t.fixture.shell.update(cx, |shell, cx| {
        shell.dock.update(cx, |dock, cx| dock.close_active_tab(cx));
    });
    assert_eq!(t.dock_tabs(cx), 0);
}

#[gpui_kit::test]
fn second_drain_on_the_cluster_is_disabled(cx: &mut TestAppContext) {
    let t = drain_test("drain-second", refusing_api_2, cx);
    let tab = t.start(cx);
    t.t.wait_for("the refusal", cx, |cx| {
        tab.read_with(cx, |tab, _| tab.run().pod_rows(Duration::ZERO).len() == 3)
    });
    // Opening another dialog on the cluster is refused with the reason.
    t.open(&t.t.stg, &["node-b"], cx);
    // The first dialog closed when the run started, and no second one opened.
    assert!(t.dialog(cx).is_none(), "no second dialog opened");
    t.set_nodes(
        &t.t.stg,
        vec![
            summary("node-b", NodeScheduling::Disabled),
            summary("node-c", NodeScheduling::Enabled),
        ],
        cx,
    );
    t.tick(&[0], cx);
    assert_eq!(
        t.drain_button(cx),
        crate::row_selection::BulkState::Off("A drain is already running on stg-b".into())
    );
    tab.update(cx, |tab, cx| tab.cancel(cx));
    t.wait_for_end(&tab, cx);
}

#[gpui_kit::test]
fn held_enter_does_not_drain(cx: &mut TestAppContext) {
    let t = drain_test("drain-held-enter", three_pods, cx);
    let dialog = t.open_and_settle(&t.t.stg, &["node-b"], cx);
    t.t.fixture.draw_twice(cx);
    // The Enter that opened the dialog from a menu repeats while it is held.
    t.t.fixture.with_window(cx, |window, cx| {
        window.dispatch_event(key_down("enter", true).to_platform_input(), cx);
    });
    cx.run_until_parked();
    assert!(t.tab(cx).is_none());
    assert!(dialog.read_with(cx, |dialog, _| dialog.is_open()));
    // A fresh press starts the run.
    t.t.fixture.with_window(cx, |window, cx| {
        window.dispatch_event(key_down("enter", false).to_platform_input(), cx);
    });
    t.t.wait_for("the tab", cx, |cx| t.tab(cx).is_some());
    let tab = t.tab(cx).expect("a tab");
    t.wait_for_end(&tab, cx);
}

#[gpui_kit::test]
fn the_drain_waits_for_the_typed_name_on_prod(cx: &mut TestAppContext) {
    let t = drain_test("drain-typed", three_pods, cx);
    t.activate_prod(cx);
    t.t.set_lock(&t.t.prod, WriteLock::Unlocked, cx);
    t.prod_state.lock().expect("state").pods =
        vec![pod_json("payments", "api-1", Some("ReplicaSet"), false)];
    let dialog = t.open_and_settle(&t.t.prod, &["node-a"], cx);
    assert_eq!(
        dialog
            .read_with(cx, |dialog, cx| dialog.drain_blocked_by(cx))
            .as_deref(),
        Some("Type node-a to confirm")
    );
    t.press_drain(&dialog, cx);
    cx.run_until_parked();
    assert!(t.tab(cx).is_none(), "the name is not typed");
    t.t.fixture.with_window(cx, |window, cx| {
        dialog.update(cx, |dialog, cx| dialog.type_text("node-a", window, cx));
    });
    t.press_drain(&dialog, cx);
    t.t.wait_for("the tab", cx, |cx| t.tab(cx).is_some());
    let tab = t.tab(cx).expect("a tab");
    t.wait_for_end(&tab, cx);
    // Everything went to prod's own server.
    assert!(writes(&t.t.stg_api).is_empty());
}

#[gpui_kit::test]
fn a_multi_node_drain_cordons_every_node_first(cx: &mut TestAppContext) {
    let t = drain_test("drain-multi", three_pods, cx);
    t.set_nodes(
        &t.t.stg,
        vec![
            summary("node-b", NodeScheduling::Enabled),
            summary("node-c", NodeScheduling::Enabled),
        ],
        cx,
    );
    let dialog = t.open_and_settle(&t.t.stg, &["node-b", "node-c"], cx);
    t.press_drain(&dialog, cx);
    t.t.wait_for("the tab", cx, |cx| t.tab(cx).is_some());
    let tab = t.tab(cx).expect("a tab");
    t.wait_for_end(&tab, cx);
    let commits: Vec<String> = writes(&t.t.stg_api)
        .into_iter()
        .filter(|request| request.method == "PATCH" && !request.has_query_key("dryRun"))
        .map(|request| request.path)
        .collect();
    assert_eq!(
        commits,
        [
            "/api/v1/nodes/node-b".to_owned(),
            "/api/v1/nodes/node-c".to_owned()
        ]
    );
    // Both cordons were sent before the first eviction.
    let order: Vec<String> = writes(&t.t.stg_api)
        .into_iter()
        .filter(|request| !request.has_query_key("dryRun"))
        .map(|request| format!("{} {}", request.method, request.path))
        .collect();
    let first_eviction = order
        .iter()
        .position(|text| text.ends_with("/eviction"))
        .expect("an eviction");
    assert_eq!(first_eviction, 2, "{order:?}");
    tab.read_with(cx, |tab, _| {
        assert_eq!(tab.run().cordoned().len(), 2);
        assert_eq!(tab.label(), "Drain 2 nodes");
    });
}

#[gpui_kit::test]
fn a_switch_stops_the_drain(cx: &mut TestAppContext) {
    let t = drain_test("drain-release", refusing_api_2, cx);
    let dir = t.t.enable_audit_folder("drain-release", cx);
    let tab = t.start(cx);
    t.t.wait_for("the refusal", cx, |cx| {
        tab.read_with(cx, |tab, _| tab.run().pod_rows(Duration::ZERO).len() == 3)
    });
    let prod = t.t.prod.clone();
    t.t.fixture
        .shell
        .update(cx, |shell, cx| shell.switch_cluster(&prod, cx));
    cx.run_until_parked();
    // The switch asks first and names the drain.
    assert_eq!(
        t.t.fixture
            .shell
            .read_with(cx, |shell, _| shell.last_leaving.clone()),
        Some(vec![
            "A drain on stg-b will stop; its nodes stay cordoned".to_owned()
        ])
    );
    assert_eq!(t.dock_tabs(cx), 1);
    t.t.fixture.with_window(cx, |window, cx| {
        window.dispatch_action(
            Box::new(gpui_kit::component::dialog::Confirm { secondary: false }),
            cx,
        );
    });
    cx.run_until_parked();
    // The run ended `stopped`, its tab went with the session, and the node got one summary line.
    assert_eq!(t.dock_tabs(cx), 0);
    assert!(!tab.read_with(cx, |tab, _| tab.is_running()));
    t.t.wait_for("the summary", cx, |_| {
        audit_lines(&dir)
            .iter()
            .any(|line| line["action"] == "Drain" && line["outcome"] == "stopped")
    });
    let summaries = audit_lines(&dir)
        .into_iter()
        .filter(|line| line["action"] == "Drain")
        .count();
    assert_eq!(summaries, 1);
}

#[gpui_kit::test]
fn quitting_with_a_running_drain_asks_first(cx: &mut TestAppContext) {
    let t = drain_test("drain-quit", refusing_api_2, cx);
    let tab = t.start(cx);
    t.t.wait_for("the refusal", cx, |cx| {
        tab.read_with(cx, |tab, _| tab.run().pod_rows(Duration::ZERO).len() == 3)
    });
    let may_close =
        t.t.fixture
            .shell
            .update(cx, |shell, cx| shell.main_window_may_close(cx));
    assert!(!may_close, "the window waits for the answer");
    cx.run_until_parked();
    assert_eq!(
        t.t.fixture
            .shell
            .read_with(cx, |shell, _| shell.last_leaving.clone()),
        Some(vec![
            "A drain on stg-b will stop; its nodes stay cordoned".to_owned()
        ])
    );
    // Keeping everything leaves the drain running.
    t.t.fixture.with_window(cx, |window, cx| {
        window.dispatch_action(Box::new(gpui_kit::component::dialog::Cancel), cx);
    });
    assert!(tab.read_with(cx, |tab, _| tab.is_running()));
    tab.update(cx, |tab, cx| tab.cancel(cx));
    t.wait_for_end(&tab, cx);
}

#[gpui_kit::test]
fn quit_stops_a_drain_of_a_cluster_just_left(cx: &mut TestAppContext) {
    let t = drain_test("drain-quit-left", refusing_api_2, cx);
    let dir = t.t.enable_audit_folder("drain-quit-left", cx);
    let tab = t.start(cx);
    t.t.wait_for("the refusal", cx, |cx| {
        tab.read_with(cx, |tab, _| tab.run().pod_rows(Duration::ZERO).len() == 3)
    });
    // Synthetic state: a switch stops its drains first, so no real path leaves a drain tab
    // without a session. Taking the session out stands for the moment a drain is still ending
    // after its cluster was left (spec 0046 decision 20); the session is kept alive by `_left`.
    let _left =
        t.t.fixture
            .shell
            .update(cx, |shell, _| shell.active_session.take());
    let may_close =
        t.t.fixture
            .shell
            .update(cx, |shell, cx| shell.main_window_may_close(cx));
    assert!(!may_close, "the drain of the left cluster still asks");
    cx.run_until_parked();
    assert_eq!(
        t.t.fixture
            .shell
            .read_with(cx, |shell, _| shell.last_leaving.clone()),
        Some(vec![
            "A drain on stg-b will stop; its nodes stay cordoned".to_owned()
        ])
    );
    t.t.fixture.with_window(cx, |window, cx| {
        window.dispatch_action(
            Box::new(gpui_kit::component::dialog::Confirm { secondary: false }),
            cx,
        );
    });
    assert!(!tab.read_with(cx, |tab, _| tab.is_running()));
    let summaries = audit_lines(&dir)
        .into_iter()
        .filter(|line| line["action"] == "Drain" && line["outcome"] == "stopped")
        .count();
    assert_eq!(summaries, 1);
}

#[gpui_kit::test]
fn the_uncordon_button_offers_a_batch_over_the_cordoned_nodes(cx: &mut TestAppContext) {
    let t = drain_test("drain-uncordon", three_pods, cx);
    let tab = t.start(cx);
    t.wait_for_end(&tab, cx);
    // The cluster reports the node cordoned, as the watch would after the patch.
    t.set_nodes(
        &t.t.stg,
        vec![summary("node-b", NodeScheduling::Disabled)],
        cx,
    );
    let (stg, cordoned) = (
        t.t.stg.clone(),
        tab.read_with(cx, |tab, _| tab.run().cordoned().to_vec()),
    );
    t.t.fixture.with_window(cx, |window, cx| {
        t.t.fixture.shell.update(cx, |shell, cx| {
            shell.uncordon_drained(&stg, &cordoned, window, cx);
        });
    });
    t.t.wait_for_dry_run(cx);
    let dry_runs: Vec<_> = writes(&t.t.stg_api)
        .into_iter()
        .filter(|request| request.method == "PATCH" && request.has_query("dryRun", "All"))
        .collect();
    // The drain's own cordon dry-run, then the batch's uncordon dry-run.
    let last = dry_runs.last().expect("a dry-run");
    assert_eq!(last.path, "/api/v1/nodes/node-b");
    assert_eq!(
        serde_json::from_str::<Value>(&last.body).expect("JSON"),
        json!({"spec": {"unschedulable": false}})
    );
}

#[gpui_kit::test]
fn closing_a_finished_drain_removes_its_tab(cx: &mut TestAppContext) {
    let t = drain_test("drain-close-finished", three_pods, cx);
    let tab = t.start(cx);
    t.wait_for_end(&tab, cx);
    t.t.fixture
        .shell
        .update(cx, |shell, cx| shell.close_drain_tab(&tab, cx));
    assert_eq!(t.dock_tabs(cx), 0);
}

// ---- Fixes after review ----

#[gpui_kit::test]
fn cordon_uncordon_and_taint_edits_are_refused_while_a_drain_runs(cx: &mut TestAppContext) {
    let t = drain_test("drain-conflict", refusing_api_2, cx);
    let tab = t.start(cx);
    t.t.wait_for("the refusal", cx, |cx| {
        tab.read_with(cx, |tab, _| tab.run().pod_rows(Duration::ZERO).len() == 3)
    });
    t.set_nodes(
        &t.t.stg,
        vec![
            summary("node-b", NodeScheduling::Enabled),
            summary("node-c", NodeScheduling::Disabled),
        ],
        cx,
    );
    let stg = t.t.stg.clone();
    // The C key and the menu: no confirm dialog.
    t.t.fixture.with_window(cx, |window, cx| {
        t.t.fixture.shell.update(cx, |shell, cx| {
            shell.start_cordon(&stg, "node-b", None, window, cx);
            shell.start_cordon(&stg, "node-c", None, window, cx);
        });
    });
    assert!(!t.t.has_dialog(cx), "no cordon or uncordon dialog");
    // The taint editor does not open; the label editor, which cannot undo the drain, does.
    t.t.fixture.with_window(cx, |window, cx| {
        t.t.fixture.shell.update(cx, |shell, cx| {
            shell.open_node_editor(NodeEditKind::Taints, &stg, "node-b", None, window, cx);
        });
    });
    assert!(
        t.t.fixture
            .shell
            .read_with(cx, |shell, _| shell.last_node_editor.clone())
            .is_none()
    );
    // The bulk buttons say why.
    t.tick(&[0], cx);
    let states = t.t.fixture.shell.read_with(cx, |shell, cx| {
        shell
            .bulk_buttons(cx)
            .into_iter()
            .map(|button| (button.label.to_string(), button.state))
            .collect::<Vec<_>>()
    });
    for label in ["Cordon", "Uncordon"] {
        let state = states
            .iter()
            .find(|(name, _)| name == label)
            .map(|(_, s)| s.clone());
        assert_eq!(
            state,
            Some(crate::row_selection::BulkState::Off(
                "A drain is running on stg-b".into()
            )),
            "{label}"
        );
    }
    t.t.fixture.with_window(cx, |window, cx| {
        t.t.fixture.shell.update(cx, |shell, cx| {
            shell.run_bulk(
                crate::resource_actions::ResourceAction::Uncordon,
                window,
                cx,
            );
        });
    });
    assert!(!t.t.has_dialog(cx), "no bulk dialog");
    // Nothing but the drain's own requests reached the server: no uncordon patch.
    assert!(
        writes(&t.t.stg_api)
            .iter()
            .all(|request| { !(request.method == "PATCH" && request.body.contains("false")) })
    );
    // After Cancel the node actions work again.
    tab.update(cx, |tab, cx| tab.cancel(cx));
    t.wait_for_end(&tab, cx);
    t.t.fixture.with_window(cx, |window, cx| {
        t.t.fixture.shell.update(cx, |shell, cx| {
            shell.start_cordon(&stg, "node-b", None, window, cx);
        });
    });
    assert!(t.t.has_dialog(cx));
}

#[gpui_kit::test]
fn a_drain_is_refused_while_a_batch_runs_on_the_cluster(cx: &mut TestAppContext) {
    let t = drain_test("drain-batch", three_pods, cx);
    let stg = t.t.stg.clone();
    t.t.fixture.shell.update(cx, |shell, _| {
        shell.running_batches.insert(stg.clone());
    });
    // The dialog does not open.
    t.open(&stg, &["node-b"], cx);
    assert!(t.dialog(cx).is_none());
    assert!(pod_lists(&t.t.stg_api).is_empty());
    // The bar button says why.
    t.tick(&[0], cx);
    assert_eq!(
        t.drain_button(cx),
        crate::row_selection::BulkState::Off("A batch is running".into())
    );
    // A dialog opened before the batch cannot start its run either.
    t.t.fixture.shell.update(cx, |shell, _| {
        shell.running_batches.remove(&stg);
    });
    let dialog = t.open_and_settle(&stg, &["node-b"], cx);
    t.t.fixture.shell.update(cx, |shell, _| {
        shell.running_batches.insert(stg.clone());
    });
    t.press_drain(&dialog, cx);
    cx.run_until_parked();
    assert!(t.tab(cx).is_none(), "no run while a batch commits");
    assert!(
        writes(&t.t.stg_api)
            .iter()
            .all(|request| request.has_query("dryRun", "All"))
    );
}

#[gpui_kit::test]
fn quitting_mid_eviction_writes_an_unknown_line_and_counts_it(cx: &mut TestAppContext) {
    let t = drain_test("drain-quit-in-flight", three_pods, cx);
    let dir = t.t.enable_audit_folder("drain-quit-in-flight", cx);
    let (release, gate) = std::sync::mpsc::channel();
    *t.stg_gate.lock().expect("the gate") = Some(gate);
    let tab = t.start(cx);
    // The first committed eviction is held at the server: it is in the air.
    t.t.wait_for("a request in the air", cx, |cx| {
        tab.read_with(cx, |tab, _| {
            matches!(
                tab.run().in_flight(),
                Some(crate::drain_run::NextStep::Evict(_))
            )
        })
    });
    let may_close =
        t.t.fixture
            .shell
            .update(cx, |shell, cx| shell.main_window_may_close(cx));
    assert!(!may_close);
    cx.run_until_parked();
    t.t.fixture.with_window(cx, |window, cx| {
        window.dispatch_action(
            Box::new(gpui_kit::component::dialog::Confirm { secondary: false }),
            cx,
        );
    });
    let lines = audit_lines(&dir);
    let unknown: Vec<_> = lines
        .iter()
        .filter(|line| line["action"] == "Evict" && line["outcome"] == "unknown")
        .collect();
    assert_eq!(unknown.len(), 1, "{lines:?}");
    assert_eq!(unknown[0]["object"]["kind"], "Pod");
    let summary = lines
        .iter()
        .find(|line| line["action"] == "Drain")
        .expect("the stopped summary");
    assert_eq!(summary["outcome"], "stopped");
    assert!(
        summary["fields"]
            .as_array()
            .is_some_and(|fields| fields.contains(&json!({"path": "unknown", "value": "1"})))
    );
    let _ = release.send(());
}

// ---- Skip PodDisruptionBudgets (spec 0040) ----

fn delete_requests(api: &FakeApi, is_dry_run: bool) -> Vec<RecordedRequest> {
    writes(api)
        .into_iter()
        .filter(|request| {
            request.method == "DELETE" && request.body.contains("\"dryRun\"") == is_dry_run
        })
        .collect()
}

impl DrainTest {
    fn tick_skip(&self, dialog: &Entity<DrainDialog>, is_on: bool, cx: &mut TestAppContext) {
        self.t.fixture.with_window(cx, |window, cx| {
            dialog.update(cx, |dialog, cx| dialog.tick_skip(is_on, window, cx));
        });
    }

    /// Waits until the Skip checkbox can change: the lazy review answered and no dry-run runs.
    fn wait_for_skip(&self, dialog: &Entity<DrainDialog>, cx: &mut TestAppContext) {
        self.t.wait_for("the Skip checkbox", cx, |cx| {
            dialog.read_with(cx, |dialog, cx| dialog.skip_blocked_by(cx).is_none())
        });
    }

    fn skip_reason(&self, dialog: &Entity<DrainDialog>, cx: &mut TestAppContext) -> Option<String> {
        dialog.read_with(cx, |dialog, cx| {
            dialog.skip_blocked_by(cx).map(|reason| reason.to_string())
        })
    }

    fn type_name(&self, dialog: &Entity<DrainDialog>, text: &str, cx: &mut TestAppContext) {
        self.t.fixture.with_window(cx, |window, cx| {
            dialog.update(cx, |dialog, cx| dialog.type_text(text, window, cx));
        });
    }
}

#[gpui_kit::test]
fn skip_pdbs_is_off_without_delete_pods(cx: &mut TestAppContext) {
    // Allowed: on once the review answered and the dry-runs are done.
    let t = drain_test("skip-allowed", three_pods, cx);
    // While a dry-run runs, the checkbox is off: its answer would be of the other request kind.
    let (release, gate) = std::sync::mpsc::channel();
    *t.stg_dry_run_gate.lock().expect("the gate") = Some(gate);
    t.open(&t.t.stg, &["node-b"], cx);
    let dialog = t.dialog(cx).expect("a dialog");
    t.t.wait_for("the dry-run to be held", cx, |cx| {
        t.skip_reason(&dialog, cx).as_deref() == Some("Waiting for the dry-runs")
    });
    assert!(dialog.read_with(cx, |dialog, _| dialog.is_checking()));
    t.tick_skip(&dialog, true, cx);
    assert!(dialog.read_with(cx, |dialog, _| {
        dialog.options().budgets == crate::drain_plan::BudgetPolicy::Respect
    }));
    release.send(()).expect("the server waits for the release");
    t.settle(&dialog, cx);
    t.wait_for_skip(&dialog, cx);
    assert!(!dialog.read_with(cx, |dialog, _| dialog.is_checking()));
    // Denied: off with the reason of the delete pods check.
    let denied = drain_test(
        "skip-denied",
        |server| {
            three_pods(server);
            server.is_delete_denied = true;
        },
        cx,
    );
    let dialog = denied.open_and_settle(&denied.t.stg, &["node-b"], cx);
    denied.t.wait_for("the review", cx, |cx| {
        denied.skip_reason(&dialog, cx).as_deref() == Some("Not permitted: delete pods")
    });
    // And ticking it does nothing.
    denied.tick_skip(&dialog, true, cx);
    assert!(dialog.read_with(cx, |dialog, _| {
        dialog.options().budgets == crate::drain_plan::BudgetPolicy::Respect
    }));
}

#[gpui_kit::test]
fn skip_pdbs_starts_off_each_time(cx: &mut TestAppContext) {
    use crate::drain_plan::BudgetPolicy;
    let t = drain_test("skip-off", three_pods, cx);
    let dialog = t.open_and_settle(&t.t.stg, &["node-b"], cx);
    t.wait_for_skip(&dialog, cx);
    t.tick_skip(&dialog, true, cx);
    assert_eq!(
        dialog.read_with(cx, |dialog, _| dialog.options().budgets),
        BudgetPolicy::Skip
    );
    t.t.fixture.with_window(cx, |window, cx| {
        dialog.update(cx, |dialog, cx| dialog.close_for_test(window, cx));
    });
    // A new open builds a fresh default: the choice is never remembered.
    let again = t.open_and_settle(&t.t.stg, &["node-b"], cx);
    assert_eq!(
        again.read_with(cx, |dialog, _| dialog.options().budgets),
        BudgetPolicy::Respect
    );
}

#[gpui_kit::test]
fn toggling_skip_pdbs_reruns_every_dry_run(cx: &mut TestAppContext) {
    let t = drain_test("skip-toggle", three_pods, cx);
    let dialog = t.open_and_settle(&t.t.stg, &["node-b"], cx);
    t.wait_for_skip(&dialog, cx);
    assert_eq!(
        evictions(&t.t.stg_api, true).len(),
        2,
        "the eviction dry-runs"
    );
    let cordon_dry_runs = |api: &FakeApi| {
        writes(api)
            .iter()
            .filter(|request| request.method == "PATCH")
            .count()
    };
    assert_eq!(cordon_dry_runs(&t.t.stg_api), 1);
    assert!(dialog.read_with(cx, |dialog, _| dialog.recorded_elapsed() > Duration::ZERO));
    t.tick_skip(&dialog, true, cx);
    // At once: every answer of the old request kind is gone, so Drain waits for the new ones.
    dialog.read_with(cx, |dialog, cx| {
        assert_eq!(dialog.recorded_elapsed(), Duration::ZERO);
        assert!(matches!(dialog.state(), DryRunState::Running));
        assert!(dialog.drain_blocked_by(cx).is_some());
    });
    t.type_name(&dialog, "node-b", cx);
    t.settle(&dialog, cx);
    // Every pod was checked again as a delete; the cordon stood and no eviction was sent.
    let dry_runs = delete_requests(&t.t.stg_api, true);
    let paths: Vec<&str> = dry_runs
        .iter()
        .map(|request| request.path.as_str())
        .collect();
    assert_eq!(
        paths,
        [
            "/api/v1/namespaces/payments/pods/api-1",
            "/api/v1/namespaces/payments/pods/api-2"
        ]
    );
    for request in &dry_runs {
        let body: Value = serde_json::from_str(&request.body).expect("a body");
        assert_eq!(body["propagationPolicy"], "Background");
        assert!(body["preconditions"]["uid"].as_str().is_some());
    }
    assert_eq!(evictions(&t.t.stg_api, true).len(), 2, "no new eviction");
    assert_eq!(
        cordon_dry_runs(&t.t.stg_api),
        1,
        "the cordon was not asked again"
    );
    dialog.read_with(cx, |dialog, cx| {
        assert_eq!(dialog.drain_blocked_by(cx), None);
        assert_eq!(
            dialog.dry_run_line(),
            "Server dry-run: cordon passed · 2 of 2 deletes accepted"
        );
    });
    // Unticking asks the eviction again.
    t.wait_for_skip(&dialog, cx);
    t.tick_skip(&dialog, false, cx);
    t.settle(&dialog, cx);
    assert_eq!(evictions(&t.t.stg_api, true).len(), 4);
}

#[gpui_kit::test]
fn skip_pdbs_types_the_name_in_every_tier(cx: &mut TestAppContext) {
    let t = drain_test("skip-tier", three_pods, cx);
    let dialog = t.open_and_settle(&t.t.stg, &["node-b"], cx);
    t.wait_for_skip(&dialog, cx);
    let tiers = |cx: &mut TestAppContext| dialog.read_with(cx, |dialog, cx| dialog.live_tiers(cx));
    // A click cluster: both buttons click.
    assert_eq!(tiers(cx), (DialogConfirm::Click, DialogConfirm::Click));
    t.tick_skip(&dialog, true, cx);
    t.settle(&dialog, cx);
    // Drain types the node name; Cordon only keeps its tier.
    assert_eq!(
        tiers(cx),
        (
            DialogConfirm::TypeName {
                expected: "node-b".to_owned()
            },
            DialogConfirm::Click
        )
    );
    dialog.read_with(cx, |dialog, cx| {
        assert_eq!(
            dialog.drain_blocked_by(cx).as_deref(),
            Some("Type node-b to confirm")
        );
        assert_eq!(
            dialog.cordon_blocked_by(cx),
            None,
            "Cordon only does not wait for it"
        );
    });
    t.type_name(&dialog, "node-b", cx);
    dialog.read_with(cx, |dialog, cx| {
        assert_eq!(dialog.drain_blocked_by(cx), None)
    });
    // Unticking returns to the tier the dialog opened with.
    t.wait_for_skip(&dialog, cx);
    t.tick_skip(&dialog, false, cx);
    assert_eq!(tiers(cx), (DialogConfirm::Click, DialogConfirm::Click));
}

#[gpui_kit::test]
fn skip_pdbs_of_several_nodes_types_the_cluster_name(cx: &mut TestAppContext) {
    let t = drain_test("skip-several", three_pods, cx);
    t.set_nodes(
        &t.t.stg,
        vec![
            summary("node-b", NodeScheduling::Enabled),
            summary("node-c", NodeScheduling::Enabled),
        ],
        cx,
    );
    let dialog = t.open_and_settle(&t.t.stg, &["node-b", "node-c"], cx);
    t.wait_for_skip(&dialog, cx);
    t.tick_skip(&dialog, true, cx);
    dialog.read_with(cx, |dialog, cx| {
        let (drain, cordon) = dialog.live_tiers(cx);
        assert_eq!(
            drain,
            DialogConfirm::TypeName {
                expected: "stg-b".to_owned()
            }
        );
        assert_eq!(cordon, DialogConfirm::Click);
    });
}

#[gpui_kit::test]
fn skip_pdbs_run_deletes_with_uid(cx: &mut TestAppContext) {
    let t = drain_test("skip-run", three_pods, cx);
    let dir = t.t.enable_audit_folder("skip-run", cx);
    let dialog = t.open_and_settle(&t.t.stg, &["node-b"], cx);
    t.wait_for_skip(&dialog, cx);
    t.tick_skip(&dialog, true, cx);
    t.settle(&dialog, cx);
    // Without the typed name nothing starts; with it the run starts.
    t.press_drain(&dialog, cx);
    cx.run_until_parked();
    assert!(t.tab(cx).is_none());
    t.type_name(&dialog, "node-b", cx);
    t.press_drain(&dialog, cx);
    t.t.wait_for("the tab", cx, |cx| t.tab(cx).is_some());
    let tab = t.tab(cx).expect("a drain tab");
    t.wait_for_end(&tab, cx);
    // The pods went by DELETE with the uid, and no eviction was committed.
    let commits = delete_requests(&t.t.stg_api, false);
    let paths: Vec<&str> = commits
        .iter()
        .map(|request| request.path.as_str())
        .collect();
    assert_eq!(
        paths,
        [
            "/api/v1/namespaces/payments/pods/api-1",
            "/api/v1/namespaces/payments/pods/api-2"
        ]
    );
    let body: Value = serde_json::from_str(&commits[0].body).expect("a body");
    assert_eq!(body["preconditions"]["uid"], "uid-api-1");
    assert_eq!(body["propagationPolicy"], "Background");
    assert!(evictions(&t.t.stg_api, false).is_empty(), "never /eviction");
    // The run reuses the dialog's dry-runs: only the two of the toggle were sent.
    assert_eq!(delete_requests(&t.t.stg_api, true).len(), 2);
    // The texts and the audit follow the policy.
    tab.read_with(cx, |tab, _| {
        assert_eq!(
            tab.run().end_notice().as_deref(),
            Some("Drain: node-b drained")
        );
    });
    t.t.wait_for("the audit lines", cx, |_| audit_lines(&dir).len() == 4);
    let lines = audit_lines(&dir);
    let actions: Vec<&str> = lines
        .iter()
        .map(|line| line["action"].as_str().unwrap_or_default())
        .collect();
    assert_eq!(actions, ["Cordon", "Delete", "Delete", "Drain"]);
    assert_eq!(
        lines[1]["fields"][0]["path"],
        "deleteOptions.propagationPolicy"
    );
    assert_eq!(
        lines[3]["fields"]
            .as_array()
            .and_then(|fields| fields.last()),
        Some(&json!({"path": "disable_eviction", "value": "true"}))
    );
}

#[gpui_kit::test]
fn skip_pdbs_run_skips_dry_runs_the_dialog_recorded(cx: &mut TestAppContext) {
    let t = drain_test("skip-recorded", three_pods, cx);
    let dialog = t.open_and_settle(&t.t.stg, &["node-b"], cx);
    t.wait_for_skip(&dialog, cx);
    t.tick_skip(&dialog, true, cx);
    t.settle(&dialog, cx);
    t.type_name(&dialog, "node-b", cx);
    t.press_drain(&dialog, cx);
    t.t.wait_for("the tab", cx, |cx| t.tab(cx).is_some());
    let tab = t.tab(cx).expect("a drain tab");
    t.wait_for_end(&tab, cx);
    // Two delete dry-runs of the dialog and two commits: the run asked nothing again.
    assert_eq!(delete_requests(&t.t.stg_api, true).len(), 2);
    assert_eq!(delete_requests(&t.t.stg_api, false).len(), 2);
}

#[gpui_kit::test]
fn skip_pdbs_run_words_the_tab_by_the_policy(cx: &mut TestAppContext) {
    let t = drain_test(
        "skip-words",
        |server| {
            three_pods(server);
            server.throttled = vec!["api-2".to_owned()];
        },
        cx,
    );
    let dialog = t.open_and_settle(&t.t.stg, &["node-b"], cx);
    t.wait_for_skip(&dialog, cx);
    t.tick_skip(&dialog, true, cx);
    t.settle(&dialog, cx);
    t.type_name(&dialog, "node-b", cx);
    t.press_drain(&dialog, cx);
    t.t.wait_for("the tab", cx, |cx| t.tab(cx).is_some());
    let tab = t.tab(cx).expect("a drain tab");
    // api-2 is rate limited, not refused by a budget: the row says so and the run retries it.
    t.t.wait_for("the refusal", cx, |cx| {
        tab.read_with(cx, |tab, _| {
            tab.run()
                .pod_rows(Duration::ZERO)
                .iter()
                .any(|row| row.text.starts_with("Refused: Too many requests"))
        })
    });
    tab.read_with(cx, |tab, _| {
        let states = tab.run().node_states();
        assert!(
            states[0].1.text().starts_with("Deleting "),
            "{:?}",
            states[0].1.text()
        );
    });
    tab.update(cx, |tab, cx| tab.cancel(cx));
    t.wait_for_end(&tab, cx);
}

#[gpui_kit::test]
fn skip_pdbs_run_stops_on_a_lock(cx: &mut TestAppContext) {
    use crate::drain_run::{NextStep, RunEnd};
    let t = drain_test("skip-lock", three_pods, cx);
    let dialog = t.open_and_settle(&t.t.stg, &["node-b"], cx);
    t.wait_for_skip(&dialog, cx);
    t.tick_skip(&dialog, true, cx);
    t.settle(&dialog, cx);
    t.type_name(&dialog, "node-b", cx);
    // The first committed delete waits at the server: the run is mid-commit.
    let (release, gate) = std::sync::mpsc::channel();
    *t.stg_gate.lock().expect("the gate") = Some(gate);
    t.press_drain(&dialog, cx);
    t.t.wait_for("a request in the air", cx, |cx| {
        t.tab(cx).is_some_and(|tab| {
            tab.read_with(cx, |tab, _| {
                matches!(tab.run().in_flight(), Some(NextStep::Evict(_)))
            })
        })
    });
    let tab = t.tab(cx).expect("a drain tab");
    t.t.set_lock(&t.t.stg, WriteLock::Locked, cx);
    release.send(()).expect("the server waits for the release");
    t.wait_for_end(&tab, cx);
    cx.run_until_parked();
    // The delete that had left is the only one: the next was blocked before it was sent.
    assert_eq!(delete_requests(&t.t.stg_api, false).len(), 1);
    tab.read_with(cx, |tab, _| {
        let Some(RunEnd::Stopped(reason)) = tab.run().end() else {
            panic!("the run stopped: {:?}", tab.run().end());
        };
        assert!(reason.ends_with("; drain stopped"), "{reason}");
    });
}

#[gpui_kit::test]
fn a_held_enter_never_confirms_a_skip_pdbs_drain(cx: &mut TestAppContext) {
    use gpui_kit::test::TestWindowExt as _;
    let t = drain_test("skip-held-enter", three_pods, cx);
    let dialog = t.open_and_settle(&t.t.stg, &["node-b"], cx);
    t.wait_for_skip(&dialog, cx);
    t.tick_skip(&dialog, true, cx);
    t.settle(&dialog, cx);
    t.type_name(&dialog, "node-b", cx);
    t.t.fixture
        .with_window(cx, |window, cx| window.render_frame(cx));
    for _ in 0..3 {
        t.t.fixture.with_window(cx, |window, cx| {
            window.dispatch_event(key_down("enter", true).to_platform_input(), cx);
        });
    }
    cx.run_until_parked();
    assert!(t.tab(cx).is_none(), "a held Enter starts nothing");
    assert!(delete_requests(&t.t.stg_api, false).is_empty());
}

#[gpui_kit::test]
fn skip_pdbs_needs_the_cluster_wide_delete_right(cx: &mut TestAppContext) {
    // A drain deletes the pods of every namespace on the node, so a right that holds in one
    // namespace only does not enable the option.
    let t = drain_test(
        "skip-cluster-wide",
        |server| {
            three_pods(server);
            server.is_delete_namespaced_only = true;
        },
        cx,
    );
    let dialog = t.open_and_settle(&t.t.stg, &["node-b"], cx);
    t.t.wait_for("the review", cx, |cx| {
        t.skip_reason(&dialog, cx).as_deref() == Some("Not permitted: delete pods")
    });
    t.tick_skip(&dialog, true, cx);
    assert!(dialog.read_with(cx, |dialog, _| {
        dialog.options().budgets == crate::drain_plan::BudgetPolicy::Respect
    }));
    // The review the dialog asked had no namespace on it.
    let reviews: Vec<RecordedRequest> =
        t.t.stg_api
            .requests()
            .into_iter()
            .filter(|request| {
                request.path.ends_with("/selfsubjectaccessreviews")
                    && request.body.contains("\"verb\":\"delete\"")
                    && request.body.contains("\"resource\":\"pods\"")
            })
            .collect();
    assert!(!reviews.is_empty(), "the dialog asked about deleting pods");
    assert!(
        reviews
            .iter()
            .all(|request| !request.body.contains("\"namespace\"")),
        "cluster-wide, no namespace"
    );
    // Allowed cluster-wide: on.
    let allowed = drain_test("skip-cluster-wide-ok", three_pods, cx);
    let dialog = allowed.open_and_settle(&allowed.t.stg, &["node-b"], cx);
    allowed.wait_for_skip(&dialog, cx);
}

#[gpui_kit::test]
fn unticking_skip_pdbs_restores_the_grace_the_user_chose(cx: &mut TestAppContext) {
    use cluster::GracePeriod;
    let t = drain_test("skip-grace", three_pods, cx);
    let dialog = t.open_and_settle(&t.t.stg, &["node-b"], cx);
    t.wait_for_skip(&dialog, cx);
    t.t.fixture.with_window(cx, |window, cx| {
        dialog.update(cx, |dialog, cx| dialog.pick_grace(2, window, cx));
    });
    let grace = |cx: &mut TestAppContext| dialog.read_with(cx, |dialog, _| dialog.options().grace);
    assert_eq!(grace(cx), GracePeriod::Seconds(30));
    t.tick_skip(&dialog, true, cx);
    assert_eq!(grace(cx), GracePeriod::PodDefault);
    t.t.wait_for("the dry-runs", cx, |cx| {
        dialog.read_with(cx, |dialog, cx| dialog.skip_blocked_by(cx).is_none())
    });
    t.tick_skip(&dialog, false, cx);
    assert_eq!(grace(cx), GracePeriod::Seconds(30));
}
