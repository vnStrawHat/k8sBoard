//! Delete (spec 0033) in a headless window over two loaded clusters, one active at a time: the
//! fixture starts on `prod-a` (locked at open), switches to `stg-b` (unlocked), and makes it live.
//! `activate` does the same for `prod-a`. Each session answers from its own fake API server, so a
//! test sees which cluster a request reached and nothing leaves the machine: no test sends a real
//! delete.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, mpsc};
use std::time::Duration;

use cluster::fake_api::{FakeApi, RecordedRequest};
use cluster::{ObjectKind, PodStatus, PodSummary, ReadyCount, StatusReason};
use gpui_kit::{Entity, InputEvent as _, KeyDownEvent, Keystroke, TestAppContext};
use serde_json::{Value, json};

use super::app_shell_switch_tests::open_switch_fixture;
use super::app_shell_write_tests::{
    Clusters, audit_lines, go_live_answering, slot_session, switch_to, writes,
};
use super::batch_write::ItemProgress;
use super::object_delete::Removal;
use super::shell_open::ShellOpen;
use super::*;
use crate::app_shell::write_flow::DryRunState;
use crate::batch_rows::job_row;
use crate::kind_access::KindAccess;
use crate::kind_row::KindRow;
use crate::resource_actions::ResourceAction;
use crate::resource_actions::RowAction;
use crate::row_selection::BulkState;
use crate::secret_rows::secret_row;
use crate::workload_actions::workload_actions_tests::{deployment, job};
use crate::workload_rows::deployment_row;
use crate::write_guard::{DialogConfirm, WriteLock};
use crate::yaml_edit::EditBanner;

const NOT_FOUND: &str = r#"{"kind":"Status","apiVersion":"v1","status":"Failure","message":"no","reason":"NotFound","code":404}"#;
const NAMESPACE: &str = "team-a";

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn access_review(is_allowed: bool) -> String {
    format!(
        r#"{{"apiVersion":"authorization.k8s.io/v1","kind":"SelfSubjectAccessReview","metadata":{{}},"spec":{{}},"status":{{"allowed":{is_allowed}}}}}"#
    )
}

fn status(code: u16, reason: &str) -> (u16, String) {
    let body = json!({
        "kind": "Status", "apiVersion": "v1", "status": "Failure",
        "message": "refused", "reason": reason, "code": code,
    });
    (code, body.to_string())
}

/// What both fake API servers know about deletes: the access answer, which objects exist, and
/// what a delete answers.
struct DeleteServer {
    may_delete: AtomicBool,
    /// Objects whose identity read answers 404.
    missing: Mutex<HashSet<String>>,
    /// An HTTP status every identity read answers instead of the object.
    identity_status: Mutex<Option<u16>>,
    /// The finalizers every object carries.
    finalizers: Mutex<Vec<String>>,
    /// What a committed delete answers instead of success.
    commit_answer: Mutex<Option<(u16, String)>>,
    /// Whether a committed delete answers the object under deletion instead of a status.
    is_pending: AtomicBool,
    /// Objects whose dry-run delete is refused.
    refused_dry_runs: Mutex<HashSet<String>>,
    /// Objects that exist for the identity read and are gone by the time they are deleted.
    vanished: Mutex<HashSet<String>>,
    /// What a dry-run eviction and a committed one answer instead of success (a budget's 429).
    eviction_dry_run_answer: Mutex<Option<(u16, String)>>,
    eviction_commit_answer: Mutex<Option<(u16, String)>>,
    /// Pods whose identity read says their deletion has started.
    terminating: Mutex<HashSet<String>>,
    /// Held by the first committed delete until the test releases it: the batch is mid-commit.
    gate: Mutex<Option<mpsc::Receiver<()>>>,
    /// Held by the first identity read until the test releases it: the delete is still reading.
    identity_gate: Mutex<Option<mpsc::Receiver<()>>>,
}

impl DeleteServer {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            may_delete: AtomicBool::new(true),
            missing: Mutex::new(HashSet::new()),
            identity_status: Mutex::new(None),
            finalizers: Mutex::new(Vec::new()),
            commit_answer: Mutex::new(None),
            is_pending: AtomicBool::new(false),
            refused_dry_runs: Mutex::new(HashSet::new()),
            vanished: Mutex::new(HashSet::new()),
            eviction_dry_run_answer: Mutex::new(None),
            eviction_commit_answer: Mutex::new(None),
            terminating: Mutex::new(HashSet::new()),
            gate: Mutex::new(None),
            identity_gate: Mutex::new(None),
        })
    }

    /// `(resource, name)` of a request for one namespaced object or node, else `None`: the watches
    /// and lists of a live session end at the resource.
    fn object_of(path: &str) -> Option<(&str, &str)> {
        let parts: Vec<&str> = path.trim_start_matches('/').split('/').collect();
        let tail = match parts.as_slice() {
            ["api", "v1", "namespaces", _, resource, name] => (*resource, *name),
            ["apis", _, _, "namespaces", _, resource, name] => (*resource, *name),
            ["api", "v1", "nodes", name] => ("nodes", *name),
            _ => return None,
        };
        Some(tail)
    }

    fn object_json(&self, resource: &str, name: &str) -> Value {
        let kind = match resource {
            "pods" => "Pod",
            "deployments" => "Deployment",
            "secrets" => "Secret",
            "jobs" => "Job",
            _ => "Node",
        };
        let finalizers = lock(&self.finalizers).clone();
        let deleted_at = lock(&self.terminating)
            .contains(name)
            .then_some("2026-10-03T08:00:00Z");
        json!({
            "apiVersion": "v1", "kind": kind,
            "metadata": {
                "name": name, "namespace": NAMESPACE, "uid": format!("uid-{name}"),
                "resourceVersion": "100", "finalizers": finalizers,
                "deletionTimestamp": deleted_at,
            },
            "spec": {"replicas": 3, "selector": {"matchLabels": {"app": "api"}},
                "template": {"metadata": {"labels": {"app": "api"}},
                    "spec": {"containers": [{"name": "api", "image": "api:1"}]}}},
        })
    }

    fn answer(&self, request: &RecordedRequest) -> (u16, String) {
        let path = request.path.as_str();
        match request.method.as_str() {
            "POST" if path.ends_with("/selfsubjectaccessreviews") => {
                let asks_delete = request.body.contains("\"verb\":\"delete\"");
                let allowed = !asks_delete || self.may_delete.load(Ordering::SeqCst);
                (201, access_review(allowed))
            }
            "GET" => self.read(path),
            "DELETE" => self.delete(request),
            "POST" if path.ends_with("/eviction") => self.evict(request),
            _ => (404, NOT_FOUND.to_owned()),
        }
    }

    fn evict(&self, request: &RecordedRequest) -> (u16, String) {
        let slot = if request.has_query("dryRun", "All") {
            &self.eviction_dry_run_answer
        } else {
            &self.eviction_commit_answer
        };
        if let Some(answer) = lock(slot).clone() {
            return answer;
        }
        let ok = json!({"kind": "Status", "apiVersion": "v1", "status": "Success", "code": 201});
        (201, ok.to_string())
    }

    fn read(&self, path: &str) -> (u16, String) {
        let Some((resource, name)) = Self::object_of(path) else {
            return (404, NOT_FOUND.to_owned());
        };
        if let Some(gate) = lock(&self.identity_gate).take() {
            let _ = gate.recv_timeout(Duration::from_secs(10));
        }
        if let Some(code) = *lock(&self.identity_status) {
            return status(code, "InternalError");
        }
        if lock(&self.missing).contains(name) {
            return (404, NOT_FOUND.to_owned());
        }
        (200, self.object_json(resource, name).to_string())
    }

    fn delete(&self, request: &RecordedRequest) -> (u16, String) {
        let is_dry_run = request.body.contains("\"dryRun\"");
        let name = request.path.rsplit('/').next().unwrap_or_default();
        if lock(&self.missing).contains(name) || lock(&self.vanished).contains(name) {
            return (404, NOT_FOUND.to_owned());
        }
        if !is_dry_run && let Some(gate) = lock(&self.gate).take() {
            let _ = gate.recv_timeout(Duration::from_secs(10));
        }
        if is_dry_run && lock(&self.refused_dry_runs).contains(name) {
            return status(403, "Forbidden");
        }
        if !is_dry_run && let Some(answer) = lock(&self.commit_answer).clone() {
            return answer;
        }
        if !is_dry_run && self.is_pending.load(Ordering::SeqCst) {
            let mut object = self.object_json("pods", name);
            object["metadata"]["deletionTimestamp"] = json!("2026-10-03T08:00:00Z");
            return (200, object.to_string());
        }
        (
            200,
            json!({"kind": "Status", "apiVersion": "v1", "status": "Success", "code": 200})
                .to_string(),
        )
    }
}

fn answers(server: &Arc<DeleteServer>) -> impl Fn(&RecordedRequest) -> (u16, String) + use<> {
    let server = Arc::clone(server);
    move |request| server.answer(request)
}

fn pod(name: &str, has_controller: bool) -> PodSummary {
    PodSummary {
        is_finished: false,
        namespace: NAMESPACE.to_owned(),
        name: name.to_owned(),
        status: PodStatus::Reason(StatusReason::Running),
        ready: ReadyCount { ready: 1, total: 1 },
        restarts: 0,
        node_name: Some("node-b".to_owned()),
        created_at: None,
        pod_ip: None,
        qos_class: None,
        service_account: None,
        controller: has_controller.then(|| cluster::ControllerRef {
            kind: "ReplicaSet".to_owned(),
            name: "api-7d9f8c".to_owned(),
        }),
        conditions: Vec::new(),
        containers: Vec::new(),
        status_message: None,
        labels: Vec::new(),
        host_network: false,
        image_pull_secrets: Vec::new(),
    }
}

struct DeleteTest {
    t: Clusters,
    server: Arc<DeleteServer>,
}

fn delete_test(name: &str, cx: &mut TestAppContext) -> DeleteTest {
    // Dialogs open without their animation, so the confirm button takes input at once.
    cx.update(|cx| cx.set_reduce_motion(true));
    let fixture = open_switch_fixture(name, cx);
    switch_to(&fixture, "stg-b", cx);
    let (prod, stg) = (fixture.cluster("prod-a", cx), fixture.cluster("stg-b", cx));
    let server = DeleteServer::new();
    let stg_api = go_live_answering(&fixture, &stg, "node-b", answers(&server), cx);
    DeleteTest {
        t: Clusters {
            fixture,
            stg_api,
            prod,
            stg,
        },
        server,
    }
}

impl DeleteTest {
    fn shell(&self) -> &Entity<AppShell> {
        &self.t.fixture.shell
    }

    /// Switches to `cluster` and makes it live over a new fake server that serves the same
    /// objects. The old session is gone, so a test never has both clusters live at once.
    fn activate(&self, cluster: &ClusterRef, cx: &mut TestAppContext) -> FakeApi {
        let node = if *cluster == self.t.prod {
            "node-a"
        } else {
            "node-b"
        };
        self.t
            .activate_answering(cluster, node, answers(&self.server), cx)
    }

    /// Shows Pods with `pods` on the open cluster and waits for its delete answer.
    fn show_pods(&self, pods: &[PodSummary], cx: &mut TestAppContext) {
        self.show_pods_on(pods, pods, cx);
    }

    /// Shows Pods with `prod` pods when `prod-a` is open and `stg` pods when `stg-b` is.
    fn show_pods_on(&self, prod: &[PodSummary], stg: &[PodSummary], cx: &mut TestAppContext) {
        self.shell()
            .update(cx, |shell, cx| shell.show_screen(Screen::Pods, cx));
        cx.run_until_parked();
        for (cluster, pods) in [(&self.t.prod, prod), (&self.t.stg, stg)] {
            let session = self
                .shell()
                .read_with(cx, |shell, _| shell.slot_session(cluster).cloned());
            if let Some(session) = session {
                session.update(cx, |session, cx| {
                    session.set_pods_for_test(pods.to_vec(), cx);
                });
            }
        }
        cx.run_until_parked();
        self.t.fixture.draw_twice(cx);
        self.wait_for_answers(ObjectKind::Pod, cx);
    }

    fn show_deployments(&self, names: &[&str], cx: &mut TestAppContext) {
        let rows = |names: &[&str]| -> Vec<KindRow> {
            names
                .iter()
                .map(|name| deployment_row(&deployment(name)))
                .collect()
        };
        self.t
            .show_kind(ResourceKind::Deployments, rows(names), rows(names), cx);
        self.wait_for_answers(ObjectKind::Deployment, cx);
    }

    /// Waits until the lazy review of `kind` has an answer on the open cluster.
    fn wait_for_answers(&self, kind: ObjectKind, cx: &mut TestAppContext) {
        for cluster in [&self.t.prod, &self.t.stg] {
            let is_open = self
                .shell()
                .read_with(cx, |shell, _| shell.slot_session(cluster).is_some());
            if !is_open {
                continue;
            }
            self.t.wait_for("the delete review", cx, |cx| {
                self.shell().read_with(cx, |shell, cx| {
                    shell.guard_for(cluster, cx).is_some_and(|guard| {
                        matches!(
                            guard.kind_access.get(kind),
                            Some(KindAccess::Known(_) | KindAccess::Unknown)
                        )
                    })
                })
            });
        }
    }

    fn cursor_on_pod(&self, cluster: &ClusterRef, name: &str, cx: &mut TestAppContext) {
        let key = ResourceKey::Pod {
            namespace: NAMESPACE.to_owned(),
            name: name.to_owned(),
        };
        let object = ClusterObject::new(cluster.clone(), key);
        self.shell().update(cx, |shell, cx| {
            shell.change_selection(Some(object), cx);
        });
        cx.run_until_parked();
    }

    /// Del on the cursor row, and the dialog it opens once the uid is read.
    fn press_delete(&self, cx: &mut TestAppContext) {
        self.t.fixture.press("delete", cx);
    }

    fn wait_for_dialog(&self, cx: &mut TestAppContext) {
        self.t
            .wait_for("the dialog", cx, |cx| self.t.has_dialog(cx));
    }

    fn open_dialog(&self, cx: &mut TestAppContext) {
        self.press_delete(cx);
        self.wait_for_dialog(cx);
        self.t.wait_for_dry_run(cx);
    }

    fn identity_reads(&self, api: &FakeApi) -> usize {
        api.requests()
            .iter()
            .filter(|request| {
                request.method == "GET" && DeleteServer::object_of(&request.path).is_some()
            })
            .count()
    }

    fn notification_count(&self, cx: &mut TestAppContext) -> usize {
        self.t
            .fixture
            .with_window(cx, |window, cx| window.notifications(cx).len())
    }

    fn is_dialog_open(&self, cx: &mut TestAppContext) -> bool {
        let dialog = self
            .shell()
            .read_with(cx, |shell, _| shell.last_dialog.clone())
            .and_then(|dialog| dialog.upgrade());
        dialog.is_some_and(|dialog| dialog.read_with(cx, |dialog, _| dialog.is_open()))
    }

    fn dialog_label(&self, cx: &mut TestAppContext) -> String {
        let dialog = self.t.dialog(cx);
        dialog
            .read_with(cx, |dialog, _| dialog.label())
            .expect("a write dialog")
            .to_string()
    }

    fn items(&self, cx: &mut TestAppContext) -> Vec<ItemProgress> {
        self.t
            .dialog(cx)
            .read_with(cx, |dialog, _| dialog.item_states())
    }
}

fn key_down(key: &str, is_held: bool) -> KeyDownEvent {
    KeyDownEvent {
        keystroke: Keystroke::parse(key).expect("a valid keystroke"),
        is_held,
        prefer_character_input: false,
    }
}

fn body_of(request: &RecordedRequest) -> Value {
    serde_json::from_str(&request.body).expect("the body is JSON")
}

// ---- Opening ----

#[gpui_kit::test]
fn del_reads_the_uid_then_opens_a_click_dialog_on_staging(cx: &mut TestAppContext) {
    let t = delete_test("delete-open", cx);
    t.show_pods(&[pod("api-x", true), pod("api-y", false)], cx);
    t.cursor_on_pod(&t.t.stg, "api-x", cx);
    t.press_delete(cx);
    t.wait_for_dialog(cx);
    // The identity read came first and was a GET; nothing was deleted yet.
    assert_eq!(t.identity_reads(&t.t.stg_api), 1);
    assert_eq!(t.dialog_label(cx), "Delete pod");
    // A single delete is a one-item batch.
    assert_eq!(t.items(cx).len(), 1);
    t.t.dialog(cx).read_with(cx, |dialog, _| {
        assert_eq!(*dialog.tier(), DialogConfirm::Click);
        assert_eq!(dialog.confirm_text().as_deref(), Some("Delete"));
    });
}

#[gpui_kit::test]
fn delete_always_dry_runs_first_and_pins_the_uid(cx: &mut TestAppContext) {
    let t = delete_test("delete-dry-run", cx);
    t.show_pods(&[pod("api-x", true)], cx);
    t.cursor_on_pod(&t.t.stg, "api-x", cx);
    t.open_dialog(cx);
    let sent = writes(&t.t.stg_api);
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert_eq!(sent[0].method, "DELETE");
    assert_eq!(sent[0].path, "/api/v1/namespaces/team-a/pods/api-x");
    let body = body_of(&sent[0]);
    assert_eq!(body["dryRun"], json!(["All"]));
    assert_eq!(body["propagationPolicy"], "Background");
    assert_eq!(body["preconditions"]["uid"], "uid-api-x");
    // The dialog asks to be confirmed before anything real is sent.
    assert_eq!(writes(&t.t.stg_api).len(), 1);
    t.t.confirm(cx);
    t.t.wait_for("the commit", cx, |_| writes(&t.t.stg_api).len() == 2);
    let committed = &writes(&t.t.stg_api)[1];
    let body = body_of(committed);
    assert!(body.get("dryRun").is_none(), "{body}");
    assert_eq!(body["preconditions"]["uid"], "uid-api-x");
}

#[gpui_kit::test]
fn delete_always_opens_a_dialog_on_both_tiers(cx: &mut TestAppContext) {
    let t = delete_test("delete-both-tiers", cx);
    t.show_pods(&[pod("api-x", true)], cx);
    t.cursor_on_pod(&t.t.stg, "api-x", cx);
    t.press_delete(cx);
    t.wait_for_dialog(cx);
    assert!(t.t.has_dialog(cx));
    t.t.fixture.press("escape", cx);
    cx.run_until_parked();
    let prod_api = t.activate(&t.t.prod, cx);
    t.t.set_lock(&t.t.prod, WriteLock::Unlocked, cx);
    t.show_pods(&[pod("api-x", true)], cx);
    t.cursor_on_pod(&t.t.prod, "api-x", cx);
    t.press_delete(cx);
    t.wait_for_dialog(cx);
    assert!(t.t.has_dialog(cx));
    t.t.fixture.press("escape", cx);
    cx.run_until_parked();
    assert!(
        writes(&prod_api)
            .iter()
            .all(|request| body_of(request).get("dryRun").is_some()),
        "only dry-runs were sent"
    );
}

#[gpui_kit::test]
fn prod_delete_types_the_object_name(cx: &mut TestAppContext) {
    let t = delete_test("delete-prod-tier", cx);
    let prod_api = t.activate(&t.t.prod, cx);
    t.t.set_lock(&t.t.prod, WriteLock::Unlocked, cx);
    t.show_pods(&[pod("api-x", true)], cx);
    t.cursor_on_pod(&t.t.prod, "api-x", cx);
    t.open_dialog(cx);
    t.t.dialog(cx).read_with(cx, |dialog, _| {
        assert_eq!(
            *dialog.tier(),
            DialogConfirm::TypeName {
                expected: "api-x".to_owned()
            }
        );
    });
    t.t.confirm(cx);
    cx.run_until_parked();
    assert_eq!(writes(&prod_api).len(), 1, "the name was not typed");
    // The cluster name is not the object name.
    t.t.type_name("prod-a", cx);
    t.t.confirm(cx);
    cx.run_until_parked();
    assert_eq!(writes(&prod_api).len(), 1);
    t.t.type_name("api-x", cx);
    t.t.confirm(cx);
    t.t.wait_for("the commit", cx, |_| writes(&prod_api).len() == 2);
    assert!(writes(&t.t.stg_api).is_empty());
}

#[gpui_kit::test]
fn held_enter_does_not_delete(cx: &mut TestAppContext) {
    let t = delete_test("delete-held-enter", cx);
    t.show_pods(&[pod("api-x", true)], cx);
    t.cursor_on_pod(&t.t.stg, "api-x", cx);
    t.open_dialog(cx);
    t.t.fixture.draw_twice(cx);
    for _ in 0..3 {
        t.t.fixture.with_window(cx, |window, cx| {
            window.dispatch_event(key_down("enter", true).to_platform_input(), cx);
        });
    }
    cx.run_until_parked();
    assert_eq!(writes(&t.t.stg_api).len(), 1, "only the dry-run was sent");
    // A fresh press confirms once.
    t.t.fixture.with_window(cx, |window, cx| {
        window.dispatch_event(key_down("enter", false).to_platform_input(), cx);
    });
    t.t.wait_for("the commit", cx, |_| writes(&t.t.stg_api).len() == 2);
}

#[gpui_kit::test]
fn a_held_del_opens_one_dialog_and_reads_once(cx: &mut TestAppContext) {
    let t = delete_test("delete-held-del", cx);
    t.show_pods(&[pod("api-x", true)], cx);
    t.cursor_on_pod(&t.t.stg, "api-x", cx);
    // Key repeats arrive before the first read has finished.
    for _ in 0..4 {
        t.press_delete(cx);
    }
    t.wait_for_dialog(cx);
    t.t.wait_for_dry_run(cx);
    // And while the dialog is open.
    for _ in 0..3 {
        t.press_delete(cx);
    }
    cx.run_until_parked();
    assert_eq!(t.identity_reads(&t.t.stg_api), 1);
    assert_eq!(writes(&t.t.stg_api).len(), 1, "one dry-run, no commit");
    assert!(
        t.shell()
            .read_with(cx, |shell, _| shell.delete_start.is_none()),
        "the read finished"
    );
}

// ---- The gate and the cluster ----

#[gpui_kit::test]
fn delete_uses_the_tier_of_the_active_cluster(cx: &mut TestAppContext) {
    let t = delete_test("delete-cursor-slot", cx);
    t.show_pods(&[pod("api-x", true)], cx);
    t.cursor_on_pod(&t.t.stg, "api-x", cx);
    t.open_dialog(cx);
    let environment =
        t.t.dialog(cx)
            .read_with(cx, |dialog, _| dialog.environment());
    assert_ne!(
        environment,
        crate::environment::Environment::Production,
        "the tier is staging's, not production's"
    );
    t.t.confirm(cx);
    t.t.wait_for("the commit", cx, |_| writes(&t.t.stg_api).len() == 2);
}

#[gpui_kit::test]
fn a_locked_cluster_deletes_nothing(cx: &mut TestAppContext) {
    let t = delete_test("delete-locked", cx);
    let prod_api = t.activate(&t.t.prod, cx);
    t.show_pods(&[pod("api-x", true)], cx);
    t.cursor_on_pod(&t.t.prod, "api-x", cx);
    let before = t.notification_count(cx);
    t.press_delete(cx);
    cx.run_until_parked();
    assert!(!t.t.has_dialog(cx));
    assert_eq!(t.identity_reads(&prod_api), 0, "the gate stops first");
    assert!(writes(&prod_api).is_empty());
    assert!(t.notification_count(cx) > before, "Del says why");
}

#[gpui_kit::test]
fn a_denied_delete_permission_sends_nothing(cx: &mut TestAppContext) {
    let t = delete_test("delete-denied", cx);
    t.server.may_delete.store(false, Ordering::SeqCst);
    t.show_pods(&[pod("api-x", true)], cx);
    t.cursor_on_pod(&t.t.stg, "api-x", cx);
    let reason = t.shell().read_with(cx, |shell, cx| {
        let guard = shell.guard_for(&t.t.stg, cx).expect("a live guard");
        crate::resource_actions::action_availability(
            ResourceAction::Delete(ObjectKind::Pod),
            &guard,
        )
    });
    assert_eq!(
        reason,
        crate::resource_actions::ActionAvailability::Disabled {
            reason: "Not permitted: delete pods".into()
        }
    );
    t.press_delete(cx);
    cx.run_until_parked();
    assert!(!t.t.has_dialog(cx));
    assert_eq!(t.identity_reads(&t.t.stg_api), 0);
    assert!(writes(&t.t.stg_api).is_empty());
}

#[gpui_kit::test]
fn the_palette_and_the_menu_item_run_the_same_arm(cx: &mut TestAppContext) {
    let t = delete_test("delete-palette", cx);
    t.show_pods(&[pod("api-x", true)], cx);
    t.cursor_on_pod(&t.t.stg, "api-x", cx);
    let snapshot = t
        .shell()
        .read_with(cx, |shell, cx| shell.palette_snapshot(&parse_query(""), cx));
    let entry = snapshot
        .entries
        .iter()
        .find(|entry| {
            matches!(
                entry.target,
                crate::palette_search::PaletteTarget::RowAction(RowAction::Delete)
            )
        })
        .expect("the palette offers Delete");
    assert!(entry.is_enabled());
    // The palette and the menu item dispatch the key action of Delete.
    let action = RowAction::Delete.key_action();
    t.t.fixture
        .with_window(cx, |window, cx| window.dispatch_action(action, cx));
    t.wait_for_dialog(cx);
    t.t.wait_for_dry_run(cx);
    assert_eq!(t.dialog_label(cx), "Delete pod");
    assert_eq!(t.identity_reads(&t.t.stg_api), 1);
}

// ---- The identity read ----

#[gpui_kit::test]
fn a_missing_object_says_so_and_sends_no_delete(cx: &mut TestAppContext) {
    let t = delete_test("delete-gone", cx);
    t.show_pods(&[pod("api-x", true)], cx);
    t.cursor_on_pod(&t.t.stg, "api-x", cx);
    lock(&t.server.missing).insert("api-x".to_owned());
    let before = t.notification_count(cx);
    t.press_delete(cx);
    t.t.wait_for("the notice", cx, |cx| t.notification_count(cx) > before);
    assert!(!t.t.has_dialog(cx));
    assert!(writes(&t.t.stg_api).is_empty());
}

#[gpui_kit::test]
fn a_failed_identity_read_stops_before_the_dialog(cx: &mut TestAppContext) {
    let t = delete_test("delete-identity-error", cx);
    t.show_pods(&[pod("api-x", true)], cx);
    t.cursor_on_pod(&t.t.stg, "api-x", cx);
    *lock(&t.server.identity_status) = Some(500);
    let before = t.notification_count(cx);
    t.press_delete(cx);
    t.t.wait_for("the notice", cx, |cx| t.notification_count(cx) > before);
    assert!(!t.t.has_dialog(cx));
    assert!(writes(&t.t.stg_api).is_empty(), "no DELETE of any kind");
}

#[gpui_kit::test]
fn a_helm_release_secret_is_never_deleted(cx: &mut TestAppContext) {
    let t = delete_test("delete-helm-secret", cx);
    let release = secret_row(&cluster::SecretSummary {
        namespace: NAMESPACE.to_owned(),
        name: "sh.helm.release.v1.shop.v1".to_owned(),
        secret_type: cluster::HELM_RELEASE_SECRET_TYPE.to_owned(),
        ..crate::topology_fixtures::secret("x", "x")
    });
    t.t.show_kind(
        ResourceKind::Secrets,
        vec![release.clone()],
        vec![release],
        cx,
    );
    t.wait_for_answers(ObjectKind::Secret, cx);
    let key = ResourceKey::Kind {
        kind: ResourceKind::Secrets,
        namespace: Some(NAMESPACE.to_owned()),
        name: "sh.helm.release.v1.shop.v1".to_owned(),
    };
    t.shell().update(cx, |shell, cx| {
        shell.change_selection(Some(ClusterObject::new(t.t.stg.clone(), key)), cx);
    });
    cx.run_until_parked();
    let before = t.notification_count(cx);
    t.press_delete(cx);
    cx.run_until_parked();
    assert!(!t.t.has_dialog(cx));
    assert_eq!(t.identity_reads(&t.t.stg_api), 0);
    assert!(writes(&t.t.stg_api).is_empty());
    assert!(t.notification_count(cx) > before);
}

// ---- Warnings, propagation, results ----

#[gpui_kit::test]
fn the_dialog_warns_about_a_pod_without_a_controller_and_finalizers(cx: &mut TestAppContext) {
    let t = delete_test("delete-warnings", cx);
    *lock(&t.server.finalizers) = vec!["foregroundDeletion".to_owned()];
    t.show_pods(&[pod("api-x", false)], cx);
    t.cursor_on_pod(&t.t.stg, "api-x", cx);
    t.open_dialog(cx);
    let lines: Vec<String> =
        t.t.dialog(cx)
            .read_with(cx, |dialog, _| dialog.warning_lines())
            .iter()
            .map(ToString::to_string)
            .collect();
    assert_eq!(
        lines,
        [
            "Not managed by a controller; it will not come back",
            "Has finalizers: foregroundDeletion. Deletion waits until their controllers remove them"
        ]
    );
}

#[gpui_kit::test]
fn a_propagation_change_rebuilds_the_items_and_checks_again(cx: &mut TestAppContext) {
    let t = delete_test("delete-propagation", cx);
    t.show_deployments(&["api"], cx);
    t.t.cursor_on(&t.t.stg, ResourceKind::Deployments, "api", cx);
    t.open_dialog(cx);
    let first = writes(&t.t.stg_api);
    assert_eq!(body_of(&first[0])["propagationPolicy"], "Background");
    // Orphan is the third choice.
    let dialog = t.t.dialog(cx);
    dialog.update(cx, |dialog, cx| dialog.choose_propagation_for_test(2, cx));
    assert_eq!(t.items(cx), [ItemProgress::Waiting]);
    t.t.wait_for_dry_run(cx);
    let sent = writes(&t.t.stg_api);
    assert_eq!(sent.len(), 2, "the dry-run ran again");
    assert_eq!(body_of(&sent[1])["propagationPolicy"], "Orphan");
    assert_eq!(body_of(&sent[1])["dryRun"], json!(["All"]));
    t.t.confirm(cx);
    t.t.wait_for("the commit", cx, |_| writes(&t.t.stg_api).len() == 3);
    let committed = body_of(&writes(&t.t.stg_api)[2]);
    assert_eq!(committed["propagationPolicy"], "Orphan");
    assert!(committed.get("dryRun").is_none());
}

#[gpui_kit::test]
fn a_kind_without_dependents_sends_background(cx: &mut TestAppContext) {
    let t = delete_test("delete-no-radio", cx);
    t.show_pods(&[pod("api-x", true)], cx);
    t.cursor_on_pod(&t.t.stg, "api-x", cx);
    t.open_dialog(cx);
    // A choice for a kind that owns nothing changes nothing.
    t.t.dialog(cx)
        .update(cx, |dialog, cx| dialog.choose_propagation_for_test(2, cx));
    cx.run_until_parked();
    assert_eq!(writes(&t.t.stg_api).len(), 1);
    assert_eq!(
        body_of(&writes(&t.t.stg_api)[0])["propagationPolicy"],
        "Background"
    );
}

#[gpui_kit::test]
fn delete_appends_one_audit_line_with_the_propagation(cx: &mut TestAppContext) {
    let t = delete_test("delete-audit", cx);
    let dir = t.t.enable_audit_folder("delete-audit", cx);
    t.show_pods(&[pod("api-x", true)], cx);
    t.cursor_on_pod(&t.t.stg, "api-x", cx);
    t.open_dialog(cx);
    assert!(audit_lines(&dir).is_empty(), "a dry-run is not recorded");
    t.t.confirm(cx);
    t.t.wait_for("the audit line", cx, |_| audit_lines(&dir).len() == 1);
    let line = &audit_lines(&dir)[0];
    assert_eq!(line["action"], "Delete");
    assert_eq!(line["cluster"], "stg-b");
    assert_eq!(line["object"]["kind"], "Pod");
    assert_eq!(line["object"]["name"], "api-x");
    assert_eq!(line["outcome"], "applied");
    assert_eq!(
        line["fields"],
        json!([{"path": "deleteOptions.propagationPolicy", "value": "Background"}])
    );
}

#[gpui_kit::test]
fn a_uid_conflict_fails_the_delete_and_is_audited_as_failed(cx: &mut TestAppContext) {
    let t = delete_test("delete-conflict", cx);
    let dir = t.t.enable_audit_folder("delete-conflict", cx);
    t.show_pods(&[pod("api-x", true)], cx);
    t.cursor_on_pod(&t.t.stg, "api-x", cx);
    t.open_dialog(cx);
    *lock(&t.server.commit_answer) = Some(status(409, "Conflict"));
    t.t.confirm(cx);
    t.t.wait_for("the audit line", cx, |_| audit_lines(&dir).len() == 1);
    assert_eq!(audit_lines(&dir)[0]["outcome"], "failed");
    t.t.wait_for("the dialog to close", cx, |cx| !t.is_dialog_open(cx));
}

#[gpui_kit::test]
fn a_delete_that_waits_for_finalizers_is_applied(cx: &mut TestAppContext) {
    let t = delete_test("delete-pending", cx);
    let dir = t.t.enable_audit_folder("delete-pending", cx);
    t.server.is_pending.store(true, Ordering::SeqCst);
    *lock(&t.server.finalizers) = vec!["f1".to_owned()];
    t.show_pods(&[pod("api-x", true)], cx);
    t.cursor_on_pod(&t.t.stg, "api-x", cx);
    t.open_dialog(cx);
    t.t.confirm(cx);
    t.t.wait_for("the audit line", cx, |_| audit_lines(&dir).len() == 1);
    assert_eq!(audit_lines(&dir)[0]["outcome"], "applied");
}

// ---- The object open in the editor or the drawer ----

#[gpui_kit::test]
fn deleting_the_object_open_in_the_editor_shows_it_gone(cx: &mut TestAppContext) {
    let t = delete_test("delete-open-edit", cx);
    t.show_deployments(&["api"], cx);
    t.t.cursor_on(&t.t.stg, ResourceKind::Deployments, "api", cx);
    t.open_dialog(cx);
    // The editor is opened on the same object while the dialog waits.
    let subject = t
        .shell()
        .read_with(cx, |shell, _| shell.selected.clone())
        .expect("the cursor is on a row");
    t.t.fixture.with_window(cx, |window, cx| {
        t.shell()
            .update(cx, |shell, cx| shell.open_edit(subject, window, cx));
    });
    assert!(t.shell().read_with(cx, |shell, _| shell.is_editing()));
    t.t.confirm(cx);
    t.t.wait_for("the editor to learn it", cx, |cx| {
        let edit = t
            .shell()
            .read_with(cx, |shell, _| shell.edit.as_ref().and_then(OpenEdit::yaml));
        edit.is_some_and(|edit| {
            edit.read_with(cx, |view, _| {
                matches!(view.banner(), Some(EditBanner::Deleted))
            })
        })
    });
}

#[gpui_kit::test]
fn deleting_the_drawer_object_closes_the_drawer_once_the_row_is_gone(cx: &mut TestAppContext) {
    let t = delete_test("delete-open-drawer", cx);
    t.show_pods(&[pod("api-x", true)], cx);
    t.cursor_on_pod(&t.t.stg, "api-x", cx);
    t.shell()
        .update(cx, |shell, cx| shell.set_drawer_open(true, cx));
    t.open_dialog(cx);
    t.t.confirm(cx);
    t.t.wait_for("the commit", cx, |_| writes(&t.t.stg_api).len() == 2);
    // The watch drops the row.
    let session = slot_session(&t.t.fixture, &t.t.stg, cx);
    session.update(cx, |session, cx| session.set_pods_for_test(Vec::new(), cx));
    cx.run_until_parked();
    t.t.fixture.draw_twice(cx);
    let (selected, is_open) = t.shell().read_with(cx, |shell, _| {
        (shell.selected.clone(), shell.drawer.is_open)
    });
    assert_eq!(selected, None);
    assert!(!is_open);
}

// ---- Several rows ----

fn jobs(names: &[&str]) -> Vec<KindRow> {
    names.iter().map(|name| job_row(&job(name))).collect()
}

fn pods_named(names: &[&str]) -> Vec<PodSummary> {
    names.iter().map(|name| pod(name, true)).collect()
}

impl DeleteTest {
    fn tick(&self, rows: &[usize], cx: &mut TestAppContext) {
        for &row in rows {
            self.shell().update(cx, |shell, cx| {
                shell.check_rows(crate::table_view::RowCheck::Toggle(row), cx);
            });
        }
        cx.run_until_parked();
        self.t.fixture.draw_twice(cx);
    }

    /// Staging lists `names`, the primary lists nothing, and the first `count` rows are ticked.
    fn tick_staging_pods(&self, names: &[&str], count: usize, cx: &mut TestAppContext) {
        self.show_pods_on(&[], &pods_named(names), cx);
        let rows: Vec<usize> = (0..count).collect();
        self.tick(&rows, cx);
    }

    /// The names of the ticked pods, in table order, with the cursor put on the first of them.
    fn cursor_on_first_ticked(&self, cx: &mut TestAppContext) -> Vec<String> {
        let checked = self
            .shell()
            .read_with(cx, |shell, cx| shell.checked_objects(cx));
        let names: Vec<String> = checked
            .iter()
            .map(|object| match &object.key {
                ResourceKey::Pod { name, .. } => name.clone(),
                other => panic!("a pod, not {other:?}"),
            })
            .collect();
        self.cursor_on_pod(&self.t.stg, &names[0], cx);
        names
    }
}

#[gpui_kit::test]
fn del_on_a_ticked_row_deletes_the_ticked_set_in_order(cx: &mut TestAppContext) {
    let t = delete_test("delete-bulk-key", cx);
    t.tick_staging_pods(&["api-0", "api-1", "api-2"], 2, cx);
    let names = t.cursor_on_first_ticked(cx);
    assert_eq!(names.len(), 2);
    t.press_delete(cx);
    t.wait_for_dialog(cx);
    t.t.wait_for_dry_run(cx);
    assert_eq!(t.dialog_label(cx), "Delete 2 pods");
    t.t.dialog(cx).read_with(cx, |dialog, _| {
        assert_eq!(dialog.confirm_text().as_deref(), Some("Delete 2 of 2"));
        assert_eq!(
            dialog.item_states(),
            vec![ItemProgress::Passed, ItemProgress::Passed]
        );
        // Staging clicks.
        assert_eq!(*dialog.tier(), DialogConfirm::Click);
    });
    // The identity reads and the dry-runs went one after the other, in list order.
    let order: Vec<String> = writes(&t.t.stg_api)
        .iter()
        .map(|request| {
            request
                .path
                .rsplit('/')
                .next()
                .unwrap_or_default()
                .to_owned()
        })
        .collect();
    assert_eq!(order, names);
    assert_eq!(t.identity_reads(&t.t.stg_api), 2);
}

#[gpui_kit::test]
fn del_on_an_unticked_row_deletes_that_row_only(cx: &mut TestAppContext) {
    let t = delete_test("delete-bulk-unticked", cx);
    t.tick_staging_pods(&["api-0", "api-1", "api-2"], 2, cx);
    let ticked = t.cursor_on_first_ticked(cx);
    let other = ["api-0", "api-1", "api-2"]
        .into_iter()
        .find(|name| !ticked.iter().any(|ticked| ticked == name))
        .expect("one row is not ticked");
    t.cursor_on_pod(&t.t.stg, other, cx);
    t.press_delete(cx);
    t.wait_for_dialog(cx);
    t.t.wait_for_dry_run(cx);
    assert_eq!(t.dialog_label(cx), "Delete pod");
    assert_eq!(t.identity_reads(&t.t.stg_api), 1);
}

#[gpui_kit::test]
fn a_bulk_delete_types_the_cluster_name_on_production(cx: &mut TestAppContext) {
    let t = delete_test("delete-bulk-prod", cx);
    t.activate(&t.t.prod, cx);
    t.t.set_lock(&t.t.prod, WriteLock::Unlocked, cx);
    t.show_pods_on(&pods_named(&["api-0", "api-1"]), &[], cx);
    t.tick(&[0, 1], cx);
    let checked = t
        .shell()
        .read_with(cx, |shell, cx| shell.checked_objects(cx));
    t.shell().update(cx, |shell, cx| {
        shell.change_selection(checked.first().cloned(), cx);
    });
    cx.run_until_parked();
    t.press_delete(cx);
    t.wait_for_dialog(cx);
    t.t.wait_for_dry_run(cx);
    t.t.dialog(cx).read_with(cx, |dialog, _| {
        assert_eq!(
            *dialog.tier(),
            DialogConfirm::TypeName {
                expected: "prod-a".to_owned()
            }
        );
    });
    assert!(writes(&t.t.stg_api).is_empty());
}

#[gpui_kit::test]
fn a_bulk_delete_needs_every_dry_run_to_pass(cx: &mut TestAppContext) {
    let t = delete_test("delete-bulk-all-pass", cx);
    t.tick_staging_pods(&["api-0", "api-1"], 2, cx);
    lock(&t.server.refused_dry_runs).insert("api-1".to_owned());
    let _ = t.cursor_on_first_ticked(cx);
    t.press_delete(cx);
    t.wait_for_dialog(cx);
    t.t.wait_for_dry_run(cx);
    let states = t.items(cx);
    assert_eq!(states[0], ItemProgress::Passed);
    assert!(matches!(states[1], ItemProgress::Rejected(_)), "{states:?}");
    // One refused check keeps the confirm off, and nothing real is sent.
    assert!(t.t.block(cx).is_some());
    t.t.confirm(cx);
    cx.run_until_parked();
    assert_eq!(writes(&t.t.stg_api).len(), 2, "two dry-runs, no commit");
}

#[gpui_kit::test]
fn an_object_gone_before_the_read_is_listed_and_never_deleted(cx: &mut TestAppContext) {
    let t = delete_test("delete-bulk-gone", cx);
    t.tick_staging_pods(&["api-0", "api-1", "api-2"], 3, cx);
    lock(&t.server.missing).insert("api-1".to_owned());
    let _ = t.cursor_on_first_ticked(cx);
    t.press_delete(cx);
    t.wait_for_dialog(cx);
    t.t.wait_for_dry_run(cx);
    assert_eq!(t.dialog_label(cx), "Delete 2 pods");
    let sent = writes(&t.t.stg_api);
    assert_eq!(sent.len(), 2, "{sent:?}");
    assert!(sent.iter().all(|request| !request.path.ends_with("api-1")));
}

#[gpui_kit::test]
fn an_object_gone_at_the_dry_run_is_skipped_not_failed(cx: &mut TestAppContext) {
    let t = delete_test("delete-bulk-vanished", cx);
    let dir = t.t.enable_audit_folder("delete-bulk-vanished", cx);
    t.tick_staging_pods(&["api-0", "api-1", "api-2"], 3, cx);
    lock(&t.server.vanished).insert("api-1".to_owned());
    let _ = t.cursor_on_first_ticked(cx);
    t.press_delete(cx);
    t.wait_for_dialog(cx);
    t.t.wait_for_dry_run(cx);
    let states = t.items(cx);
    assert_eq!(
        states.iter().filter(|s| **s == ItemProgress::Gone).count(),
        1,
        "{states:?}"
    );
    t.t.dialog(cx).read_with(cx, |dialog, _| {
        assert_eq!(dialog.confirm_text().as_deref(), Some("Delete 2 of 3"));
    });
    t.t.confirm(cx);
    t.t.wait_for("two audit lines", cx, |_| audit_lines(&dir).len() == 2);
    let committed = writes(&t.t.stg_api)
        .iter()
        .filter(|request| !request.body.contains("dryRun"))
        .count();
    assert_eq!(committed, 2, "the gone object got no commit");
}

#[gpui_kit::test]
fn a_bulk_commit_continues_after_one_failure(cx: &mut TestAppContext) {
    let t = delete_test("delete-bulk-continue", cx);
    let dir = t.t.enable_audit_folder("delete-bulk-continue", cx);
    t.tick_staging_pods(&["api-0", "api-1", "api-2"], 3, cx);
    let _ = t.cursor_on_first_ticked(cx);
    t.open_dialog(cx);
    // Every commit is refused: each item fails alone, and each is audited.
    *lock(&t.server.commit_answer) = Some(status(403, "Forbidden"));
    t.t.confirm(cx);
    t.t.wait_for("three audit lines", cx, |_| audit_lines(&dir).len() == 3);
    assert!(
        audit_lines(&dir)
            .iter()
            .all(|line| line["outcome"] == "failed" && line["action"] == "Delete")
    );
    assert_eq!(
        writes(&t.t.stg_api).len(),
        6,
        "three dry-runs, three commits"
    );
}

#[gpui_kit::test]
fn a_bulk_commit_survives_the_dialog_closing(cx: &mut TestAppContext) {
    let t = delete_test("delete-bulk-close", cx);
    let dir = t.t.enable_audit_folder("delete-bulk-close", cx);
    t.tick_staging_pods(&["api-0", "api-1", "api-2"], 3, cx);
    let _ = t.cursor_on_first_ticked(cx);
    t.open_dialog(cx);
    t.t.confirm(cx);
    t.t.fixture.press("escape", cx);
    t.t.wait_for("every commit", cx, |_| audit_lines(&dir).len() == 3);
    assert_eq!(writes(&t.t.stg_api).len(), 6);
    // The last line is on disk before the loop learns it and ends, so wait for the release.
    t.t.wait_for("the guard to be released", cx, |cx| {
        !t.shell()
            .read_with(cx, |shell, _| shell.running_batches.contains(&t.t.stg))
    });
}

#[gpui_kit::test]
fn a_locked_cluster_blocks_the_confirm_of_an_open_delete(cx: &mut TestAppContext) {
    let t = delete_test("delete-lock-after-open", cx);
    t.tick_staging_pods(&["api-0", "api-1"], 2, cx);
    let _ = t.cursor_on_first_ticked(cx);
    t.open_dialog(cx);
    t.t.set_lock(&t.t.stg, WriteLock::Locked, cx);
    assert_eq!(
        t.t.block(cx).as_deref(),
        Some("stg-b was locked; nothing was changed")
    );
    t.t.confirm(cx);
    cx.run_until_parked();
    assert_eq!(writes(&t.t.stg_api).len(), 2, "no commit was sent");
}

#[gpui_kit::test]
fn more_than_fifty_ticks_open_no_delete(cx: &mut TestAppContext) {
    let t = delete_test("delete-cap", cx);
    let names: Vec<String> = (0..=50).map(|n| format!("p-{n:02}")).collect();
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    t.show_pods_on(&[], &pods_named(&names), cx);
    t.shell().update(cx, |shell, cx| {
        shell.check_rows(crate::table_view::RowCheck::All(true), cx);
    });
    cx.run_until_parked();
    let _ = t.cursor_on_first_ticked(cx);
    let before = t.notification_count(cx);
    t.press_delete(cx);
    cx.run_until_parked();
    assert!(!t.t.has_dialog(cx));
    assert_eq!(t.identity_reads(&t.t.stg_api), 0);
    assert!(t.notification_count(cx) > before);
}

#[gpui_kit::test]
fn a_second_delete_waits_while_a_batch_runs(cx: &mut TestAppContext) {
    let t = delete_test("delete-running", cx);
    t.show_pods(&[pod("api-x", true)], cx);
    t.cursor_on_pod(&t.t.stg, "api-x", cx);
    let stg = t.t.stg.clone();
    t.shell()
        .update(cx, |shell, _| shell.running_batches.insert(stg));
    let before = t.notification_count(cx);
    t.press_delete(cx);
    cx.run_until_parked();
    assert!(!t.t.has_dialog(cx));
    assert_eq!(t.identity_reads(&t.t.stg_api), 0);
    assert!(t.notification_count(cx) > before);
}

#[gpui_kit::test]
fn job_rows_are_deletable(cx: &mut TestAppContext) {
    let t = delete_test("delete-jobs", cx);
    t.t.show_kind(ResourceKind::Jobs, jobs(&["etl"]), jobs(&["etl"]), cx);
    t.wait_for_answers(ObjectKind::Job, cx);
    t.t.cursor_on(&t.t.stg, ResourceKind::Jobs, "etl", cx);
    t.open_dialog(cx);
    assert_eq!(t.dialog_label(cx), "Delete job");
    let sent = writes(&t.t.stg_api);
    assert_eq!(sent[0].path, "/apis/batch/v1/namespaces/team-a/jobs/etl");
}

#[gpui_kit::test]
fn a_delete_whose_read_is_pending_starts_nothing_more(cx: &mut TestAppContext) {
    let t = delete_test("delete-pending-read", cx);
    t.show_pods(&[pod("api-x", true)], cx);
    t.cursor_on_pod(&t.t.stg, "api-x", cx);
    // A held Del repeats before the first read answered.
    t.shell().update(cx, |shell, _| {
        shell.delete_start = Some(gpui_kit::Task::ready(()));
    });
    t.press_delete(cx);
    cx.run_until_parked();
    assert!(!t.t.has_dialog(cx));
    assert_eq!(t.identity_reads(&t.t.stg_api), 0);
}

#[gpui_kit::test]
fn a_removal_is_inert_behind_an_open_dialog(cx: &mut TestAppContext) {
    let t = delete_test("delete-behind-dialog", cx);
    t.show_pods(&[pod("api-x", true), pod("api-y", true)], cx);
    t.cursor_on_pod(&t.t.stg, "api-x", cx);
    t.open_dialog(cx);
    let other = ClusterObject::new(
        t.t.stg.clone(),
        ResourceKey::Pod {
            namespace: NAMESPACE.to_owned(),
            name: "api-y".to_owned(),
        },
    );
    t.t.fixture.with_window(cx, |window, cx| {
        t.shell().update(cx, |shell, cx| {
            shell.start_removal(Removal::Delete, vec![other], window, cx)
        });
    });
    cx.run_until_parked();
    assert_eq!(t.identity_reads(&t.t.stg_api), 1, "no second read");
    assert_eq!(writes(&t.t.stg_api).len(), 1, "no second dry-run");
}

// ---- The selection bar and the menu label ----

impl DeleteTest {
    fn bar(&self, cx: &mut TestAppContext) -> Vec<crate::row_selection::BulkButton> {
        self.shell()
            .read_with(cx, |shell, cx| shell.bulk_buttons(cx))
    }

    fn delete_button(&self, cx: &mut TestAppContext) -> crate::row_selection::BulkButton {
        self.bar(cx)
            .into_iter()
            .find(|button| button.label.as_ref() == "Delete…")
            .expect("the bar has a Delete… button")
    }
}

#[gpui_kit::test]
fn the_selection_bar_ends_with_a_danger_delete_on_pods(cx: &mut TestAppContext) {
    let t = delete_test("delete-bar-pods", cx);
    t.tick_staging_pods(&["api-0", "api-1"], 2, cx);
    let bar = t.bar(cx);
    let last = bar.last().expect("a button");
    assert_eq!(last.label.as_ref(), "Delete…");
    assert!(last.is_danger);
    assert_eq!(
        last.state,
        BulkState::Ready(ResourceAction::Delete(ObjectKind::Pod))
    );
    assert!(bar[..bar.len() - 1].iter().all(|button| !button.is_danger));
}

#[gpui_kit::test]
fn the_selection_bar_offers_delete_on_nodes_and_kind_screens(cx: &mut TestAppContext) {
    let t = delete_test("delete-bar-screens", cx);
    t.shell()
        .update(cx, |shell, cx| shell.show_screen(Screen::Nodes, cx));
    cx.run_until_parked();
    t.wait_for_answers(ObjectKind::Node, cx);
    t.t.fixture.draw_twice(cx);
    t.tick(&[0], cx);
    assert_eq!(
        t.delete_button(cx).state,
        BulkState::Ready(ResourceAction::Delete(ObjectKind::Node))
    );
    t.show_deployments(&["api", "web"], cx);
    t.tick(&[0, 1], cx);
    assert_eq!(
        t.delete_button(cx).state,
        BulkState::Ready(ResourceAction::Delete(ObjectKind::Deployment))
    );
    // Issues rows are findings, not objects: no Delete there.
    t.shell()
        .update(cx, |shell, cx| shell.show_screen(Screen::Issues, cx));
    cx.run_until_parked();
    assert!(
        t.bar(cx)
            .iter()
            .all(|button| button.label.as_ref() != "Delete…")
    );
}

#[gpui_kit::test]
fn the_bar_button_is_off_while_the_rows_cluster_is_locked(cx: &mut TestAppContext) {
    let t = delete_test("delete-bar-locked", cx);
    t.tick_staging_pods(&["api-0", "api-1"], 2, cx);
    t.t.set_lock(&t.t.stg, WriteLock::Locked, cx);
    assert_eq!(
        t.delete_button(cx).state,
        BulkState::Off("stg-b is read-only".into())
    );
    t.t.set_lock(&t.t.stg, WriteLock::Unlocked, cx);
    assert!(matches!(t.delete_button(cx).state, BulkState::Ready(_)));
}

#[gpui_kit::test]
fn the_bar_button_is_off_without_the_delete_right(cx: &mut TestAppContext) {
    let t = delete_test("delete-bar-denied", cx);
    t.server.may_delete.store(false, Ordering::SeqCst);
    t.tick_staging_pods(&["api-0", "api-1"], 2, cx);
    assert_eq!(
        t.delete_button(cx).state,
        BulkState::Off("Not permitted: delete pods".into())
    );
}

#[gpui_kit::test]
fn the_bar_button_starts_the_delete_of_the_ticked_rows(cx: &mut TestAppContext) {
    let t = delete_test("delete-bar-run", cx);
    t.tick_staging_pods(&["api-0", "api-1", "api-2"], 3, cx);
    let button = t.delete_button(cx);
    let BulkState::Ready(action) = button.state else {
        panic!("the button is ready");
    };
    t.t.fixture.with_window(cx, |window, cx| {
        t.shell()
            .update(cx, |shell, cx| shell.run_bulk(action, window, cx));
    });
    t.wait_for_dialog(cx);
    t.t.wait_for_dry_run(cx);
    assert_eq!(t.dialog_label(cx), "Delete 3 pods");
    assert_eq!(t.identity_reads(&t.t.stg_api), 3);
}

#[gpui_kit::test]
fn the_menu_label_counts_what_del_would_delete(cx: &mut TestAppContext) {
    let t = delete_test("delete-menu-count", cx);
    t.tick_staging_pods(&["api-0", "api-1", "api-2"], 2, cx);
    let ticked = t.cursor_on_first_ticked(cx);
    let size = |cx: &mut TestAppContext| {
        t.shell()
            .read_with(cx, |shell, cx| shell.delete_scope_size(cx))
    };
    assert_eq!(size(cx), 2, "the cursor row is ticked");
    let other = ["api-0", "api-1", "api-2"]
        .into_iter()
        .find(|name| !ticked.iter().any(|ticked| ticked == name))
        .expect("one row is not ticked");
    t.cursor_on_pod(&t.t.stg, other, cx);
    assert_eq!(size(cx), 1, "the cursor row is not ticked");
}

#[gpui_kit::test]
fn a_lock_that_comes_on_mid_batch_stops_the_rest(cx: &mut TestAppContext) {
    let t = delete_test("delete-bulk-blocked", cx);
    let dir = t.t.enable_audit_folder("delete-bulk-blocked", cx);
    t.tick_staging_pods(&["api-0", "api-1", "api-2"], 3, cx);
    let _ = t.cursor_on_first_ticked(cx);
    t.open_dialog(cx);
    let (release, gate) = mpsc::channel();
    *lock(&t.server.gate) = Some(gate);
    t.t.confirm(cx);
    // Three dry-runs and the first commit have reached the server, which holds the commit.
    t.t.wait_for("the first commit", cx, |_| writes(&t.t.stg_api).len() == 4);
    t.t.set_lock(&t.t.stg, WriteLock::Locked, cx);
    release.send(()).expect("the server waits for the release");
    t.t.wait_for("the first audit line", cx, |_| audit_lines(&dir).len() == 1);
    t.t.wait_for("the batch to end", cx, |cx| {
        t.shell()
            .read_with(cx, |shell, _| !shell.running_batches.contains(&t.t.stg))
    });
    // The second and third commits were blocked before they were sent, and are not recorded.
    assert_eq!(writes(&t.t.stg_api).len(), 4);
    assert_eq!(audit_lines(&dir).len(), 1);
}

#[gpui_kit::test]
fn a_running_batch_of_a_sends_nothing_to_b(cx: &mut TestAppContext) {
    let t = delete_test("delete-bulk-switch", cx);
    let dir = t.t.enable_audit_folder("delete-bulk-switch", cx);
    t.tick_staging_pods(&["api-0", "api-1", "api-2"], 3, cx);
    let _ = t.cursor_on_first_ticked(cx);
    t.open_dialog(cx);
    let (release, gate) = mpsc::channel();
    *lock(&t.server.gate) = Some(gate);
    t.t.confirm(cx);
    // Three dry-runs and the first commit have reached the server, which holds the commit.
    t.t.wait_for("the first commit", cx, |_| writes(&t.t.stg_api).len() == 4);
    // The switch to prod-a asks first and names the batch; confirming it releases stg-b.
    let prod = t.t.prod.clone();
    t.shell()
        .update(cx, |shell, cx| shell.switch_cluster(&prod, cx));
    cx.run_until_parked();
    assert_eq!(
        t.shell()
            .read_with(cx, |shell, _| shell.last_leaving.clone()),
        Some(vec![
            "1 running batch will stop; its remaining items are not sent".to_owned()
        ])
    );
    t.t.fixture.with_window(cx, |window, cx| {
        window.dispatch_action(
            Box::new(gpui_kit::component::dialog::Confirm { secondary: false }),
            cx,
        );
    });
    cx.run_until_parked();
    let prod_api = go_live_answering(&t.t.fixture, &prod, "node-a", answers(&t.server), cx);
    release.send(()).expect("the server waits for the release");
    t.t.wait_for("the first audit line", cx, |_| audit_lines(&dir).len() == 1);
    t.t.wait_for("the batch to end", cx, |cx| {
        t.shell()
            .read_with(cx, |shell, _| !shell.running_batches.contains(&t.t.stg))
    });
    // The second and third items were never sent, to either cluster.
    assert_eq!(writes(&t.t.stg_api).len(), 4);
    assert_eq!(audit_lines(&dir).len(), 1);
    assert!(writes(&prod_api).is_empty(), "{:?}", writes(&prod_api));
}

#[gpui_kit::test]
fn a_delete_read_landing_after_a_switch_opens_nothing(cx: &mut TestAppContext) {
    let t = delete_test("delete-read-after-switch", cx);
    t.show_pods(&[pod("api-x", true)], cx);
    t.cursor_on_pod(&t.t.stg, "api-x", cx);
    let (release, gate) = mpsc::channel();
    *lock(&t.server.identity_gate) = Some(gate);
    t.press_delete(cx);
    t.t.wait_for("the read to start", cx, |cx| {
        t.shell()
            .read_with(cx, |shell, _| shell.delete_start.is_some())
    });
    // The shell switches to prod-a while the objects are still being read on stg-b.
    let prod_api = t.activate(&t.t.prod, cx);
    release.send(()).expect("the server waits for the release");
    t.t.wait_for("the read to end", cx, |cx| {
        t.shell()
            .read_with(cx, |shell, _| shell.delete_start.is_none())
    });
    assert!(!t.t.has_dialog(cx), "no dialog for a cluster that left");
    assert!(writes(&t.t.stg_api).is_empty());
    assert!(writes(&prod_api).is_empty());
}

#[gpui_kit::test]
fn a_dialog_that_opens_during_the_reads_stops_the_delete(cx: &mut TestAppContext) {
    let t = delete_test("delete-dialog-during-read", cx);
    t.show_pods(&[pod("api-x", true)], cx);
    t.cursor_on_pod(&t.t.stg, "api-x", cx);
    let (release, gate) = mpsc::channel();
    *lock(&t.server.identity_gate) = Some(gate);
    t.press_delete(cx);
    t.t.wait_for("the read to start", cx, |cx| {
        t.shell()
            .read_with(cx, |shell, _| shell.delete_start.is_some())
    });
    // Another dialog opens while the objects are being read: an Open shell question.
    let open = ShellOpen {
        cluster: t.t.stg.clone(),
        namespace: NAMESPACE.to_owned(),
        pod: "api-x".to_owned(),
        short_pod: "api-x".to_owned(),
        container: "app".to_owned(),
    };
    t.t.fixture.with_window(cx, |window, cx| {
        t.shell()
            .update(cx, |shell, cx| shell.start_shell(open, window, cx));
    });
    cx.run_until_parked();
    let before = t.notification_count(cx);
    release.send(()).expect("the server waits for the release");
    t.t.wait_for("the notice", cx, |cx| t.notification_count(cx) > before);
    t.t.wait_for("the read to end", cx, |cx| {
        t.shell()
            .read_with(cx, |shell, _| shell.delete_start.is_none())
    });
    assert!(
        writes(&t.t.stg_api).is_empty(),
        "no dry-run behind the other dialog"
    );
}

#[gpui_kit::test]
fn an_editor_that_opens_during_the_reads_stops_the_delete(cx: &mut TestAppContext) {
    let t = delete_test("delete-editor-during-read", cx);
    t.show_deployments(&["api"], cx);
    t.t.cursor_on(&t.t.stg, ResourceKind::Deployments, "api", cx);
    let (release, gate) = mpsc::channel();
    *lock(&t.server.identity_gate) = Some(gate);
    t.press_delete(cx);
    t.t.wait_for("the read to start", cx, |cx| {
        t.shell()
            .read_with(cx, |shell, _| shell.delete_start.is_some())
    });
    let subject = t
        .shell()
        .read_with(cx, |shell, _| shell.selected.clone())
        .expect("the cursor is on a row");
    t.t.fixture.with_window(cx, |window, cx| {
        t.shell()
            .update(cx, |shell, cx| shell.open_edit(subject, window, cx));
    });
    release.send(()).expect("the server waits for the release");
    t.t.wait_for("the read to end", cx, |cx| {
        t.shell()
            .read_with(cx, |shell, _| shell.delete_start.is_none())
    });
    assert!(!t.t.has_dialog(cx));
    assert!(writes(&t.t.stg_api).is_empty());
}

#[gpui_kit::test]
fn a_second_batch_cannot_start_committing_on_a_busy_cluster(cx: &mut TestAppContext) {
    let t = delete_test("delete-commit-busy", cx);
    t.show_pods(&[pod("api-x", true)], cx);
    t.cursor_on_pod(&t.t.stg, "api-x", cx);
    t.open_dialog(cx);
    // Another batch got to the cluster while this dialog waited.
    let stg = t.t.stg.clone();
    t.shell()
        .update(cx, |shell, _| shell.running_batches.insert(stg));
    t.t.confirm(cx);
    cx.run_until_parked();
    assert_eq!(writes(&t.t.stg_api).len(), 1, "only the dry-run was sent");
    let state =
        t.t.dialog(cx)
            .read_with(cx, |dialog, _| dialog.dry_run_state());
    assert!(
        matches!(state, Some(DryRunState::Failed(ref text)) if text.contains("A batch is running")),
        "{state:?}"
    );
}

#[gpui_kit::test]
fn a_held_del_on_a_refused_delete_leaves_one_notice(cx: &mut TestAppContext) {
    let t = delete_test("delete-notice-replaces", cx);
    t.show_pods(&[pod("api-x", true)], cx);
    t.cursor_on_pod(&t.t.stg, "api-x", cx);
    let stg = t.t.stg.clone();
    t.shell()
        .update(cx, |shell, _| shell.running_batches.insert(stg));
    let before = t.notification_count(cx);
    for _ in 0..4 {
        t.press_delete(cx);
    }
    cx.run_until_parked();
    assert_eq!(t.notification_count(cx), before + 1);
}

#[gpui_kit::test]
fn the_reads_announce_themselves(cx: &mut TestAppContext) {
    let t = delete_test("delete-reading-notice", cx);
    t.show_pods(&[pod("api-x", true)], cx);
    t.cursor_on_pod(&t.t.stg, "api-x", cx);
    let (release, gate) = mpsc::channel();
    *lock(&t.server.identity_gate) = Some(gate);
    let before = t.notification_count(cx);
    t.press_delete(cx);
    assert!(t.notification_count(cx) > before, "Reading 1 object…");
    release.send(()).expect("the server waits for the release");
    t.wait_for_dialog(cx);
}

// ---- Restart pod and Evict (spec 0040) ----

/// The 429 of an eviction a PodDisruptionBudget refuses: the cause names the budget.
fn budget_refusal() -> (u16, String) {
    let body = json!({
        "kind": "Status", "apiVersion": "v1", "status": "Failure",
        "message": "Cannot evict pod as it would violate the pod's disruption budget.",
        "reason": "TooManyRequests", "code": 429,
        "details": {"causes": [{
            "reason": "DisruptionBudget",
            "message": "The disruption budget api-pdb needs 2 healthy pods and has 2 currently",
        }]},
    });
    (429, body.to_string())
}

/// The same 429 without a cause list: the message of the status stands in.
fn plain_refusal() -> (u16, String) {
    let body = json!({
        "kind": "Status", "apiVersion": "v1", "status": "Failure",
        "message": "Too many requests, try again later",
        "reason": "TooManyRequests", "code": 429,
    });
    (429, body.to_string())
}

impl DeleteTest {
    /// The key action of `row` on the cursor pod, as the menu item and the palette dispatch it.
    fn run(&self, row: RowAction, cx: &mut TestAppContext) {
        let action = row.key_action();
        self.t
            .fixture
            .with_window(cx, |window, cx| window.dispatch_action(action, cx));
    }

    /// Opens the dialog of `row` on the cursor pod and waits for its dry-run.
    fn open_removal(&self, row: RowAction, cx: &mut TestAppContext) {
        self.run(row, cx);
        self.wait_for_dialog(cx);
        self.t.wait_for_dry_run(cx);
    }
}

#[gpui_kit::test]
fn restart_pod_dry_runs_then_deletes_with_uid(cx: &mut TestAppContext) {
    let t = delete_test("restart-pod", cx);
    let dir = t.t.enable_audit_folder("restart-pod", cx);
    t.show_pods(&[pod("api-x", true)], cx);
    t.cursor_on_pod(&t.t.stg, "api-x", cx);
    t.open_removal(RowAction::RestartPod, cx);
    assert_eq!(t.dialog_label(cx), "Restart pod");
    // The uid was read first, then one dry-run delete carried it.
    assert_eq!(t.identity_reads(&t.t.stg_api), 1);
    let sent = writes(&t.t.stg_api);
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert_eq!(sent[0].method, "DELETE");
    assert_eq!(sent[0].path, "/api/v1/namespaces/team-a/pods/api-x");
    let body = body_of(&sent[0]);
    assert_eq!(body["dryRun"], json!(["All"]));
    assert_eq!(body["propagationPolicy"], "Background");
    assert_eq!(body["preconditions"]["uid"], "uid-api-x");
    t.t.dialog(cx).read_with(cx, |dialog, _| {
        assert_eq!(dialog.confirm_text().as_deref(), Some("Restart"));
    });
    t.t.confirm(cx);
    t.t.wait_for("the commit", cx, |_| writes(&t.t.stg_api).len() == 2);
    let committed = body_of(&writes(&t.t.stg_api)[1]);
    assert!(committed.get("dryRun").is_none(), "{committed}");
    assert_eq!(committed["preconditions"]["uid"], "uid-api-x");
    t.t.wait_for("the audit line", cx, |_| audit_lines(&dir).len() == 1);
    let line = &audit_lines(&dir)[0];
    assert_eq!(line["action"], "Restart pod");
    assert_eq!(line["outcome"], "applied");
    assert_eq!(line["fields"][0]["path"], "deleteOptions.propagationPolicy");
    assert_eq!(line["fields"][0]["value"], "Background");
}

#[gpui_kit::test]
fn evict_dry_runs_then_evicts_with_uid(cx: &mut TestAppContext) {
    let t = delete_test("evict-pod", cx);
    let dir = t.t.enable_audit_folder("evict-pod", cx);
    t.show_pods(&[pod("api-x", true)], cx);
    t.cursor_on_pod(&t.t.stg, "api-x", cx);
    t.open_removal(RowAction::EvictPod, cx);
    assert_eq!(t.dialog_label(cx), "Evict pod");
    let sent = writes(&t.t.stg_api);
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert_eq!(sent[0].method, "POST");
    assert_eq!(
        sent[0].path,
        "/api/v1/namespaces/team-a/pods/api-x/eviction"
    );
    assert!(sent[0].has_query("dryRun", "All"), "{}", sent[0].query);
    let body = body_of(&sent[0]);
    assert_eq!(body["deleteOptions"]["preconditions"]["uid"], "uid-api-x");
    assert!(body["deleteOptions"].get("gracePeriodSeconds").is_none());
    t.t.confirm(cx);
    t.t.wait_for("the commit", cx, |_| writes(&t.t.stg_api).len() == 2);
    assert!(!writes(&t.t.stg_api)[1].has_query_key("dryRun"));
    t.t.wait_for("the audit line", cx, |_| audit_lines(&dir).len() == 1);
    let line = &audit_lines(&dir)[0];
    assert_eq!(line["action"], "Evict");
    assert_eq!(line["fields"][0]["path"], "pods/eviction");
    assert_eq!(line["fields"][0]["value"], "grace pod default");
}

#[gpui_kit::test]
fn evict_429_dry_run_blocks_the_commit(cx: &mut TestAppContext) {
    for (name, refusal, cause) in [
        (
            "evict-429-cause",
            budget_refusal(),
            "refused for now: The disruption budget api-pdb needs 2 healthy pods and has 2 currently",
        ),
        (
            "evict-429-plain",
            plain_refusal(),
            "refused for now: Too many requests, try again later",
        ),
    ] {
        let t = delete_test(name, cx);
        *lock(&t.server.eviction_dry_run_answer) = Some(refusal);
        t.show_pods(&[pod("api-x", true)], cx);
        t.cursor_on_pod(&t.t.stg, "api-x", cx);
        t.open_removal(RowAction::EvictPod, cx);
        assert_eq!(
            t.items(cx),
            [ItemProgress::Rejected(cause.into())],
            "{name}"
        );
        assert!(t.t.block(cx).is_some(), "Apply stays off");
        t.t.confirm(cx);
        cx.run_until_parked();
        assert_eq!(writes(&t.t.stg_api).len(), 1, "{name}: only the dry-run");
    }
}

#[gpui_kit::test]
fn evict_commit_429_is_not_audited(cx: &mut TestAppContext) {
    let t = delete_test("evict-commit-429", cx);
    let dir = t.t.enable_audit_folder("evict-commit-429", cx);
    t.show_pods(&[pod("api-x", true)], cx);
    t.cursor_on_pod(&t.t.stg, "api-x", cx);
    t.open_removal(RowAction::EvictPod, cx);
    *lock(&t.server.eviction_commit_answer) = Some(budget_refusal());
    t.t.confirm(cx);
    t.t.wait_for("the commit", cx, |_| writes(&t.t.stg_api).len() == 2);
    t.t.wait_for("the batch to end", cx, |cx| {
        t.shell()
            .read_with(cx, |shell, _| !shell.running_batches.contains(&t.t.stg))
    });
    std::thread::sleep(Duration::from_millis(200));
    cx.run_until_parked();
    assert!(
        audit_lines(&dir).is_empty(),
        "a refusal for now is not audited"
    );
}

#[gpui_kit::test]
fn restart_of_a_pod_found_terminating_at_the_uid_read_opens_nothing(cx: &mut TestAppContext) {
    for row in [RowAction::RestartPod, RowAction::EvictPod] {
        let t = delete_test("removal-terminating", cx);
        // The row still reads Running; the uid read learns that its deletion has started.
        lock(&t.server.terminating).insert("api-x".to_owned());
        t.show_pods(&[pod("api-x", true)], cx);
        t.cursor_on_pod(&t.t.stg, "api-x", cx);
        let before = t.notification_count(cx);
        t.run(row, cx);
        t.t.wait_for("the notice", cx, |cx| t.notification_count(cx) > before);
        assert!(!t.t.has_dialog(cx), "no dialog for a pod on its way out");
        assert!(writes(&t.t.stg_api).is_empty(), "{row:?}");
    }
}

#[gpui_kit::test]
fn restart_and_evict_refuse_a_blocked_pod_without_reading(cx: &mut TestAppContext) {
    let t = delete_test("removal-blocked", cx);
    t.show_pods(&[pod("bare", false)], cx);
    t.cursor_on_pod(&t.t.stg, "bare", cx);
    let before = t.notification_count(cx);
    t.run(RowAction::RestartPod, cx);
    cx.run_until_parked();
    assert!(!t.t.has_dialog(cx));
    assert_eq!(t.identity_reads(&t.t.stg_api), 0, "the gate stops first");
    assert!(t.notification_count(cx) > before, "the key says why");
    // A bare pod can be evicted: the dialog opens and warns once.
    t.open_removal(RowAction::EvictPod, cx);
    let lines: Vec<String> =
        t.t.dialog(cx)
            .read_with(cx, |dialog, _| dialog.warning_lines())
            .iter()
            .map(ToString::to_string)
            .collect();
    assert_eq!(
        lines,
        ["Not managed by a controller; it will not come back"]
    );
}

#[gpui_kit::test]
fn a_restart_acts_on_the_cursor_pod_never_the_ticked_set(cx: &mut TestAppContext) {
    let t = delete_test("restart-cursor-only", cx);
    t.tick_staging_pods(&["api-0", "api-1", "api-2"], 3, cx);
    let ticked = t.cursor_on_first_ticked(cx);
    t.open_removal(RowAction::RestartPod, cx);
    assert_eq!(t.items(cx).len(), 1, "one pod, not the three ticked");
    assert_eq!(t.identity_reads(&t.t.stg_api), 1);
    assert_eq!(
        writes(&t.t.stg_api)[0].path.rsplit('/').next(),
        ticked.first().map(String::as_str)
    );
}

#[gpui_kit::test]
fn prod_restart_and_evict_type_the_pod_name(cx: &mut TestAppContext) {
    for row in [RowAction::RestartPod, RowAction::EvictPod] {
        let t = delete_test("removal-prod-tier", cx);
        let prod_api = t.activate(&t.t.prod, cx);
        t.t.set_lock(&t.t.prod, WriteLock::Unlocked, cx);
        t.show_pods(&[pod("api-x", true)], cx);
        t.cursor_on_pod(&t.t.prod, "api-x", cx);
        t.open_removal(row, cx);
        t.t.dialog(cx).read_with(cx, |dialog, _| {
            assert_eq!(
                *dialog.tier(),
                DialogConfirm::TypeName {
                    expected: "api-x".to_owned()
                },
                "{row:?}"
            );
        });
        t.t.confirm(cx);
        cx.run_until_parked();
        assert_eq!(
            writes(&prod_api).len(),
            1,
            "{row:?}: the name was not typed"
        );
    }
}

#[gpui_kit::test]
fn restart_read_landing_after_a_switch_opens_nothing(cx: &mut TestAppContext) {
    let t = delete_test("restart-read-after-switch", cx);
    t.show_pods(&[pod("api-x", true)], cx);
    t.cursor_on_pod(&t.t.stg, "api-x", cx);
    let (release, gate) = mpsc::channel();
    *lock(&t.server.identity_gate) = Some(gate);
    t.run(RowAction::RestartPod, cx);
    t.t.wait_for("the read to start", cx, |cx| {
        t.shell()
            .read_with(cx, |shell, _| shell.delete_start.is_some())
    });
    let prod_api = t.activate(&t.t.prod, cx);
    release.send(()).expect("the server waits for the release");
    t.t.wait_for("the read to end", cx, |cx| {
        t.shell()
            .read_with(cx, |shell, _| shell.delete_start.is_none())
    });
    assert!(!t.t.has_dialog(cx));
    assert!(writes(&t.t.stg_api).is_empty());
    assert!(writes(&prod_api).is_empty());
}

#[gpui_kit::test]
fn evict_confirmed_after_switching_back_sends_nothing(cx: &mut TestAppContext) {
    let t = delete_test("evict-switch-back", cx);
    t.show_pods(&[pod("api-x", true)], cx);
    t.cursor_on_pod(&t.t.stg, "api-x", cx);
    t.open_removal(RowAction::EvictPod, cx);
    // A to B to A: the session of the dialog is gone, and the new one has another generation.
    t.activate(&t.t.prod, cx);
    let again = t.activate(&t.t.stg, cx);
    t.t.confirm(cx);
    cx.run_until_parked();
    assert_eq!(writes(&t.t.stg_api).len(), 1, "only the first dry-run");
    assert!(writes(&again).is_empty(), "nothing reached the new session");
}

#[gpui_kit::test]
fn a_held_enter_never_confirms_a_restart_or_an_evict(cx: &mut TestAppContext) {
    for row in [RowAction::RestartPod, RowAction::EvictPod] {
        let t = delete_test("removal-held-enter", cx);
        t.show_pods(&[pod("api-x", true)], cx);
        t.cursor_on_pod(&t.t.stg, "api-x", cx);
        t.open_removal(row, cx);
        t.t.fixture.draw_twice(cx);
        for _ in 0..3 {
            t.t.fixture.with_window(cx, |window, cx| {
                window.dispatch_event(key_down("enter", true).to_platform_input(), cx);
            });
        }
        cx.run_until_parked();
        assert_eq!(writes(&t.t.stg_api).len(), 1, "{row:?}: only the dry-run");
        t.t.fixture.with_window(cx, |window, cx| {
            window.dispatch_event(key_down("enter", false).to_platform_input(), cx);
        });
        t.t.wait_for("the commit", cx, |_| writes(&t.t.stg_api).len() == 2);
    }
}
