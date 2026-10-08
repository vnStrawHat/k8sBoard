//! Edit YAML (spec 0031) in a headless window over two loaded clusters, one active at a time: the
//! fixture starts on `prod-a` (locked at open), switches to `stg-b` (unlocked), and makes it live;
//! `activate` does the same for `prod-a`. Each session answers from its own fake API server, so a
//! test sees which cluster a request reached, and nothing leaves the machine. The fake connection is
//! built with an allowing policy of its own.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use cluster::fake_api::{FakeApi, RecordedRequest};
use gpui_kit::InputEvent as _;
use gpui_kit::component::dialog::{Cancel, Confirm};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{Entity, KeyDownEvent, Keystroke, TestAppContext};
use serde_json::{Value, json};

use super::app_shell_switch_tests::open_switch_fixture;
use super::app_shell_write_tests::{Clusters, audit_lines, go_live_answering, switch_to, writes};
use super::*;
use crate::kind_access::KindAccess;
use crate::kind_row::KindRow;
use crate::resource_actions::{ActionAvailability, ResourceAction, RowAction, action_availability};
use crate::workload_actions::workload_actions_tests::deployment;
use crate::workload_rows::deployment_row;
use crate::write_guard::{DialogConfirm, WriteLock};
use crate::yaml_edit::{EditBanner, EditTab, PreviewFailure, PreviewState, YamlEditView};

const PATH: &str = "/apis/apps/v1/namespaces/team-a/deployments/api";
const NOT_FOUND: &str = r#"{"kind":"Status","apiVersion":"v1","status":"Failure","message":"no","reason":"NotFound","code":404}"#;
const INVALID: &str = r#"{"kind":"Status","apiVersion":"v1","status":"Failure","message":"Deployment.apps \"api\" is invalid: spec.replicas: Invalid value: -1","reason":"Invalid","details":{"name":"api","group":"apps","kind":"Deployment","causes":[{"reason":"FieldValueInvalid","message":"Invalid value: -1","field":"spec.replicas"}]},"code":422}"#;
const CONFLICT: &str = r#"{"kind":"Status","apiVersion":"v1","status":"Failure","message":"the object has been modified","reason":"Conflict","code":409}"#;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn deployment_object(resource_version: &str) -> Value {
    json!({
        "apiVersion": "apps/v1",
        "kind": "Deployment",
        "metadata": {
            "name": "api",
            "namespace": "team-a",
            "uid": "uid-1",
            "resourceVersion": resource_version,
            "labels": {"app": "api"},
            "annotations": {
                "kubectl.kubernetes.io/last-applied-configuration": "{\"applied\":true}",
            },
        },
        "spec": {
            "replicas": 3,
            "selector": {"matchLabels": {"app": "api"}},
            "template": {
                "metadata": {"labels": {"app": "api"}},
                "spec": {"containers": [
                    {"name": "api", "image": "api:1", "env": [
                        {"name": "DB_PASS", "value": "s3cr3t-env"},
                    ]},
                    {"name": "sidecar", "image": "proxy:1"},
                ]},
            },
        },
        "status": {"readyReplicas": 3},
    })
}

fn replica_set(name: &str, revision: u32) -> Value {
    json!({
        "apiVersion": "apps/v1", "kind": "ReplicaSet",
        "metadata": {
            "name": name, "namespace": "team-a",
            "annotations": {"deployment.kubernetes.io/revision": revision.to_string()},
            "ownerReferences": [{
                "apiVersion": "apps/v1", "kind": "Deployment", "name": "api",
                "uid": "uid-1", "controller": true,
            }],
        },
        "spec": {"template": {"spec": {"containers": [
            {"name": "api", "image": format!("api:{revision}")},
        ]}}},
    })
}

fn replica_set_list(items: &[Value]) -> Value {
    json!({
        "apiVersion": "apps/v1", "kind": "ReplicaSetList", "metadata": {},
        "items": items,
    })
}

fn access_review(is_allowed: bool) -> String {
    format!(
        r#"{{"apiVersion":"authorization.k8s.io/v1","kind":"SelfSubjectAccessReview","metadata":{{}},"spec":{{}},"status":{{"allowed":{is_allowed}}}}}"#
    )
}

/// What both fake API servers know: the Deployment they serve, whether `update` is allowed, and
/// what a dry-run or a commit `PUT` answers instead of an echo.
struct EditServer {
    object: Mutex<Value>,
    /// The ReplicaSets the list route answers with.
    replica_sets: Mutex<Vec<Value>>,
    may_update: AtomicBool,
    /// Whether the ReplicaSet list route answers with an error.
    fail_lists: AtomicBool,
    dry_run_answer: Mutex<Option<(u16, String)>>,
    commit_answer: Mutex<Option<(u16, String)>>,
}

impl EditServer {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            object: Mutex::new(deployment_object("100")),
            replica_sets: Mutex::new(vec![replica_set("api-a", 1), replica_set("api-b", 2)]),
            may_update: AtomicBool::new(true),
            fail_lists: AtomicBool::new(false),
            dry_run_answer: Mutex::new(None),
            commit_answer: Mutex::new(None),
        })
    }

    fn answer(&self, request: &RecordedRequest) -> (u16, String) {
        let path = request.path.as_str();
        match request.method.as_str() {
            "POST" if path.ends_with("/selfsubjectaccessreviews") => {
                (201, access_review(self.may_update.load(Ordering::SeqCst)))
            }
            "GET" if path == PATH => (200, lock(&self.object).to_string()),
            "GET"
                if path == "/apis/apps/v1/namespaces/team-a/replicasets"
                    && self.fail_lists.load(Ordering::SeqCst) =>
            {
                (404, NOT_FOUND.to_owned())
            }
            "GET" if path == "/apis/apps/v1/namespaces/team-a/replicasets" => {
                (200, replica_set_list(&lock(&self.replica_sets)).to_string())
            }
            "GET" if path.starts_with("/apis/apps/v1/namespaces/team-a/replicasets/") => {
                (200, replica_set("api-x", 1).to_string())
            }
            "PUT" if path == PATH => self.answer_put(request),
            _ => (404, NOT_FOUND.to_owned()),
        }
    }

    /// The echo of the body with a bumped `resourceVersion`, as the server answers a replace.
    fn answer_put(&self, request: &RecordedRequest) -> (u16, String) {
        let slot = if request.has_query_key("dryRun") {
            &self.dry_run_answer
        } else {
            &self.commit_answer
        };
        if let Some(answer) = lock(slot).clone() {
            return answer;
        }
        let mut body: Value = serde_json::from_str(&request.body).unwrap_or(Value::Null);
        body["metadata"]["resourceVersion"] = json!("101");
        (200, body.to_string())
    }
}

fn answers(server: &Arc<EditServer>) -> impl Fn(&RecordedRequest) -> (u16, String) + use<> {
    let server = Arc::clone(server);
    move |request| server.answer(request)
}

fn deployments() -> Vec<KindRow> {
    vec![deployment_row(&deployment("api"))]
}

struct EditTest {
    t: Clusters,
    server: Arc<EditServer>,
}

fn edit_test(name: &str, cx: &mut TestAppContext) -> EditTest {
    // Dialogs open without their animation, so the confirm button takes input at once.
    cx.update(|cx| cx.set_reduce_motion(true));
    let fixture = open_switch_fixture(name, cx);
    switch_to(&fixture, "stg-b", cx);
    let (prod, stg) = (fixture.cluster("prod-a", cx), fixture.cluster("stg-b", cx));
    let server = EditServer::new();
    let stg_api = go_live_answering(&fixture, &stg, "node-b", answers(&server), cx);
    let t = Clusters {
        fixture,
        stg_api,
        prod,
        stg,
    };
    t.show_kind(ResourceKind::Deployments, deployments(), deployments(), cx);
    EditTest { t, server }
}

impl EditTest {
    fn shell(&self) -> &Entity<AppShell> {
        &self.t.fixture.shell
    }

    /// Switches to `cluster`, makes it live over a new fake server, and lists the Deployment on it.
    /// The old session is gone, so a test never has both clusters live at once.
    fn activate(&self, cluster: &ClusterRef, cx: &mut TestAppContext) -> FakeApi {
        let node = if *cluster == self.t.prod {
            "node-a"
        } else {
            "node-b"
        };
        let api = self
            .t
            .activate_answering(cluster, node, answers(&self.server), cx);
        self.t
            .show_kind(ResourceKind::Deployments, deployments(), deployments(), cx);
        api
    }

    /// Waits until the lazy `update deployments` review of `cluster` has an answer.
    fn wait_for_update_answer(&self, cluster: &ClusterRef, cx: &mut TestAppContext) {
        self.t.wait_for("the update review", cx, |cx| {
            self.shell().read_with(cx, |shell, cx| {
                shell.guard_for(cluster, cx).is_some_and(|guard| {
                    matches!(
                        guard.kind_access.get(cluster::ObjectKind::Deployment),
                        Some(KindAccess::Known(_) | KindAccess::Unknown)
                    )
                })
            })
        });
    }

    /// The cursor on the Deployment of `cluster`, once its permission is known.
    fn cursor_on(&self, cluster: &ClusterRef, cx: &mut TestAppContext) {
        self.t
            .cursor_on(cluster, ResourceKind::Deployments, "api", cx);
        self.wait_for_update_answer(cluster, cx);
    }

    /// E on the cursor row of staging, and the object read.
    fn open(&self, cx: &mut TestAppContext) {
        self.cursor_on(&self.t.stg, cx);
        self.t.fixture.press("e", cx);
        self.wait_for_base(cx);
    }

    fn wait_for_base(&self, cx: &mut TestAppContext) {
        self.t.wait_for("the object to load", cx, |cx| {
            self.edit(cx).is_some_and(|edit| {
                edit.read_with(cx, |view, _| {
                    view.base_text().is_some() || view.load_error().is_some()
                })
            })
        });
    }

    fn edit(&self, cx: &mut TestAppContext) -> Option<Entity<YamlEditView>> {
        self.shell()
            .read_with(cx, |shell, _| shell.edit.as_ref().and_then(OpenEdit::yaml))
    }

    fn view(&self, cx: &mut TestAppContext) -> Entity<YamlEditView> {
        self.edit(cx).expect("an open edit")
    }

    fn base_text(&self, cx: &mut TestAppContext) -> String {
        self.view(cx)
            .read_with(cx, |view, _| view.base_text())
            .expect("a loaded object")
    }

    fn text(&self, cx: &mut TestAppContext) -> String {
        self.view(cx)
            .read_with(cx, |view, cx| view.text(cx).to_string())
    }

    fn set_text(&self, text: &str, cx: &mut TestAppContext) {
        let view = self.view(cx);
        self.t.fixture.with_window(cx, |window, cx| {
            view.update(cx, |view, cx| view.set_text_for_test(text, window, cx));
        });
    }

    /// Replaces `from` by `to` in the text the editor holds.
    fn change(&self, from: &str, to: &str, cx: &mut TestAppContext) {
        let text = self.text(cx);
        assert!(text.contains(from), "{from:?} is not in the text");
        self.set_text(&text.replacen(from, to, 1), cx);
    }

    fn apply(&self, cx: &mut TestAppContext) {
        let view = self.view(cx);
        self.t.fixture.with_window(cx, |window, cx| {
            view.update(cx, |view, cx| view.apply(window, cx));
        });
    }

    fn is_passed(&self, cx: &mut TestAppContext) -> bool {
        self.view(cx).read_with(cx, |view, _| {
            matches!(view.preview_state(), PreviewState::Passed(_))
        })
    }

    fn wait_for_preview(&self, cx: &mut TestAppContext) {
        self.t.wait_for("the preview", cx, |cx| {
            self.view(cx).read_with(cx, |view, _| {
                !matches!(view.preview_state(), PreviewState::Running { .. })
            })
        });
    }

    fn with_view<R>(&self, cx: &mut TestAppContext, read: impl FnOnce(&YamlEditView) -> R) -> R {
        self.view(cx).read_with(cx, |view, _| read(view))
    }

    fn has_edit(&self, cx: &mut TestAppContext) -> bool {
        self.edit(cx).is_some()
    }

    /// The `PUT`s the staging server received, dry-runs first.
    fn puts(&self) -> Vec<RecordedRequest> {
        puts_of(&self.t.stg_api)
    }

    /// Presses the confirm button of the dialog on top, typing the cluster name when asked.
    fn confirm_dialog(&self, cx: &mut TestAppContext) {
        self.t.wait_for_dry_run(cx);
        let dialog = self.t.dialog(cx);
        if let DialogConfirm::TypeName { expected } = dialog.read_with(cx, |d, _| d.tier().clone())
        {
            self.t.type_name(&expected, cx);
        }
        self.t.confirm(cx);
    }

    fn press_dialog(&self, answer: impl gpui_kit::Action, cx: &mut TestAppContext) {
        self.t.fixture.with_window(cx, |window, cx| {
            window.dispatch_action(Box::new(answer), cx);
        });
    }
}

fn puts_of(api: &FakeApi) -> Vec<RecordedRequest> {
    writes(api)
        .into_iter()
        .filter(|request| request.method == "PUT")
        .collect()
}

fn gets_of(api: &FakeApi) -> usize {
    api.requests()
        .iter()
        .filter(|request| request.method == "GET" && request.path == PATH)
        .count()
}

/// The reviews of `update` on deployments: the connect-time review of every check also asks for
/// `update certificates/status` (0018 step 6), which is not what these tests count.
fn update_reviews_of(api: &FakeApi) -> usize {
    api.requests()
        .iter()
        .filter(|request| {
            request.path.ends_with("/selfsubjectaccessreviews")
                && request.body.contains("update")
                && request.body.contains("deployments")
        })
        .count()
}

fn audit_dir(t: &EditTest, name: &str, cx: &mut TestAppContext) -> PathBuf {
    t.t.enable_audit_folder(name, cx)
}

// ---- opening ----

#[gpui_kit::test]
fn e_opens_the_editor_on_the_cursor_row_of_its_own_cluster(cx: &mut TestAppContext) {
    let t = edit_test("edit-open", cx);
    t.open(cx);
    let (cluster, name) = t.with_view(cx, |view| {
        (view.cluster().clone(), view.object().name().to_owned())
    });
    assert_eq!((&cluster, name.as_str()), (&t.t.stg, "api"));
    // One read of the object, on the cluster of the row.
    assert_eq!(gets_of(&t.t.stg_api), 1);
    let text = t.base_text(cx);
    assert!(
        text.starts_with("# Values shown as <hidden>"),
        "the edit header: {text}"
    );
    assert!(text.contains("replicas: 3"));
    // The env literal is masked and the server's own fields are gone.
    assert!(text.contains("value: <hidden>"));
    assert!(!text.contains("s3cr3t-env"));
    assert!(!text.contains("readyReplicas"));
    assert!(!text.contains("resourceVersion"));
    t.t.fixture.draw_twice(cx);
}

#[gpui_kit::test]
fn menus_e_and_palette_share_the_arm(cx: &mut TestAppContext) {
    let t = edit_test("edit-palette", cx);
    t.cursor_on(&t.t.stg, cx);
    // The palette lists the action of the cursor row, enabled, and dispatches its key action.
    let snapshot = t
        .shell()
        .read_with(cx, |shell, cx| shell.palette_snapshot(&parse_query(""), cx));
    let entry = snapshot
        .entries
        .iter()
        .find(|entry| {
            matches!(
                entry.target,
                crate::palette_search::PaletteTarget::RowAction(RowAction::EditYaml)
            )
        })
        .expect("the palette offers Edit YAML");
    assert!(entry.is_enabled());
    let action = RowAction::EditYaml.key_action();
    t.t.fixture
        .with_window(cx, |window, cx| window.dispatch_action(action, cx));
    t.wait_for_base(cx);
    assert!(t.has_edit(cx));
}

#[gpui_kit::test]
fn edit_yaml_is_disabled_with_the_denied_permission(cx: &mut TestAppContext) {
    let t = edit_test("edit-denied", cx);
    // The first review already ran as allowed; a denial arrives with the next scope.
    t.server.may_update.store(false, Ordering::SeqCst);
    t.shell().update(cx, |shell, cx| {
        shell.set_namespace(NamespaceScope::Named("team-a".to_owned()), cx)
    });
    t.cursor_on(&t.t.stg, cx);
    let availability = t.shell().read_with(cx, |shell, cx| {
        let guard = shell.guard_for(&t.t.stg, cx).expect("a live guard");
        action_availability(
            ResourceAction::EditYaml(cluster::ObjectKind::Deployment),
            &guard,
        )
    });
    assert_eq!(
        availability,
        ActionAvailability::Disabled {
            reason: "Not permitted: update deployments".into()
        }
    );
    t.t.fixture.press("e", cx);
    cx.run_until_parked();
    assert!(!t.has_edit(cx));
    assert!(puts_of(&t.t.stg_api).is_empty());
}

#[gpui_kit::test]
fn kind_access_runs_once_per_kind_and_scope(cx: &mut TestAppContext) {
    let t = edit_test("edit-review", cx);
    t.wait_for_update_answer(&t.t.stg, cx);
    assert_eq!(update_reviews_of(&t.t.stg_api), 1);
    // Showing the screen again, or selecting its rows, asks nothing new.
    t.shell().update(cx, |shell, cx| {
        shell.show_screen(Screen::Kind(ResourceKind::Deployments), cx)
    });
    t.cursor_on(&t.t.stg, cx);
    cx.run_until_parked();
    assert_eq!(update_reviews_of(&t.t.stg_api), 1);
    // Another scope forgets the answers, and the screen asks again.
    t.shell().update(cx, |shell, cx| {
        shell.set_namespace(NamespaceScope::Named("team-a".to_owned()), cx)
    });
    t.t.wait_for("the second review", cx, |_| {
        update_reviews_of(&t.t.stg_api) == 2
    });
}

/// The access reviews that asked for `verb` on deployments.
fn reviews_of(api: &FakeApi, verb: &str) -> usize {
    api.requests()
        .iter()
        .filter(|request| {
            request.path.ends_with("/selfsubjectaccessreviews")
                && request.body.contains(&format!("\"verb\":\"{verb}\""))
        })
        .count()
}

#[gpui_kit::test]
fn kind_access_includes_delete(cx: &mut TestAppContext) {
    let t = edit_test("edit-review-delete", cx);
    t.wait_for_update_answer(&t.t.stg, cx);
    // One screen show asks both rights of the kind, once.
    assert_eq!(reviews_of(&t.t.stg_api, "update"), 1);
    assert_eq!(reviews_of(&t.t.stg_api, "delete"), 1);
    let known = t.shell().read_with(cx, |shell, cx| {
        let guard = shell.guard_for(&t.t.stg, cx).expect("a live guard");
        match guard.kind_access.get(cluster::ObjectKind::Deployment) {
            Some(KindAccess::Known(report)) => Some(report.is_allowed(
                cluster::AccessCheck::Delete(cluster::ObjectKind::Deployment),
            )),
            _ => None,
        }
    });
    assert_eq!(known, Some(true));
}

#[gpui_kit::test]
fn row_keys_inert_while_editing(cx: &mut TestAppContext) {
    let t = edit_test("edit-inert", cx);
    t.open(cx);
    let before = t.shell().read_with(cx, |shell, _| shell.selected.clone());
    for key in ["r", "e", "j", "enter", "escape"] {
        t.t.fixture.press(key, cx);
    }
    assert!(!t.t.has_dialog(cx), "no key opened a dialog");
    assert!(t.has_edit(cx));
    let after = t.shell().read_with(cx, |shell, _| shell.selected.clone());
    assert_eq!(before, after, "the hidden cursor did not move");
    assert!(writes(&t.t.stg_api).is_empty());
}

// ---- the editor ----

#[gpui_kit::test]
fn editor_marks_dirty_on_change(cx: &mut TestAppContext) {
    let t = edit_test("edit-dirty", cx);
    t.open(cx);
    assert!(!t.with_view(cx, |view| view.is_dirty()));
    // A change the user makes reaches the view as an event, not as a call.
    let editor = t.with_view(cx, |view| view.editor().clone());
    let changed = t.base_text(cx).replacen("replicas: 3", "replicas: 4", 1);
    t.t.fixture.with_window(cx, |window, cx| {
        editor.update(cx, |editor, cx| editor.replace_all(changed, window, cx));
    });
    assert!(t.with_view(cx, |view| view.is_dirty()));
    let base = t.base_text(cx);
    t.set_text(&base, cx);
    assert!(!t.with_view(cx, |view| view.is_dirty()));
}

#[gpui_kit::test]
fn apply_while_clean_does_nothing(cx: &mut TestAppContext) {
    let t = edit_test("edit-clean", cx);
    t.open(cx);
    t.apply(cx);
    cx.run_until_parked();
    assert!(t.puts().is_empty());
    assert!(!t.t.has_dialog(cx));
    assert!(t.with_view(cx, |view| matches!(
        view.preview_state(),
        PreviewState::NotChecked
    )));
}

#[gpui_kit::test]
fn env_toggle_disabled_while_dirty_and_shows_values_when_clean(cx: &mut TestAppContext) {
    let t = edit_test("edit-env", cx);
    t.open(cx);
    t.change("replicas: 3", "replicas: 4", cx);
    let reads = gets_of(&t.t.stg_api);
    let view = t.view(cx);
    t.t.fixture.with_window(cx, |window, cx| {
        view.update(cx, |view, cx| view.toggle_env_values(window, cx));
    });
    cx.run_until_parked();
    assert_eq!(gets_of(&t.t.stg_api), reads, "a dirty editor reads nothing");
    assert!(!t.text(cx).contains("s3cr3t-env"));
    // Clean again: the toggle reads the object once more, with the env values shown.
    let base = t.base_text(cx);
    t.set_text(&base, cx);
    t.t.fixture.with_window(cx, |window, cx| {
        view.update(cx, |view, cx| view.toggle_env_values(window, cx));
    });
    t.t.wait_for("the second read", cx, |cx| {
        t.edit(cx).is_some_and(|edit| {
            edit.read_with(cx, |view, _| {
                view.base_text()
                    .is_some_and(|text| text.contains("s3cr3t-env"))
            })
        })
    });
    assert_eq!(gets_of(&t.t.stg_api), reads + 1);
}

#[gpui_kit::test]
fn format_sorts_keys_and_keeps_header(cx: &mut TestAppContext) {
    let t = edit_test("edit-format", cx);
    t.open(cx);
    let shuffled = "# kept\nspec:\n  replicas: 3\nmetadata:\n  namespace: team-a\n  name: api\nkind: Deployment\napiVersion: apps/v1\n";
    t.set_text(shuffled, cx);
    let view = t.view(cx);
    t.t.fixture.with_window(cx, |window, cx| {
        view.update(cx, |view, cx| view.format(window, cx));
    });
    let text = t.text(cx);
    assert!(
        text.starts_with("# kept\napiVersion: apps/v1\nkind: Deployment\n"),
        "{text}"
    );
    assert!(
        text.contains("metadata:\n  name: api\n  namespace: team-a\n"),
        "{text}"
    );
}

#[gpui_kit::test]
fn a_helm_release_record_is_refused_on_load(cx: &mut TestAppContext) {
    let t = edit_test("edit-helm", cx);
    lock(&t.server.object)["type"] = json!("helm.sh/release.v1");
    t.cursor_on(&t.t.stg, cx);
    t.t.fixture.press("e", cx);
    t.wait_for_base(cx);
    let (error, has_base) = t.with_view(cx, |view| {
        (view.load_error().cloned(), view.base_text().is_some())
    });
    assert!(!has_base);
    let error = error.expect("the load failed");
    assert!(error.contains("Helm release"), "{error}");
}

// ---- the check ----

#[gpui_kit::test]
fn ctrl_s_runs_the_preview_and_shows_diff(cx: &mut TestAppContext) {
    let t = edit_test("edit-preview", cx);
    t.open(cx);
    t.change("replicas: 3", "replicas: 5", cx);
    // The real key, from the editor: the chord is bound in the view.
    t.t.fixture.press("ctrl-s", cx);
    t.wait_for_preview(cx);
    let dry_runs = t.puts();
    assert_eq!(dry_runs.len(), 1, "{dry_runs:?}");
    assert_eq!(dry_runs[0].path, PATH);
    assert!(dry_runs[0].has_query("dryRun", "All"));
    assert!(dry_runs[0].has_query("fieldManager", "k8sboard"));
    assert_eq!(t.with_view(cx, |view| view.tab()), EditTab::Diff);
    t.with_view(cx, |view| {
        let PreviewState::Passed(passed) = view.preview_state() else {
            panic!("the dry-run did not pass");
        };
        let paths: Vec<&str> = passed.changes.iter().map(|c| c.path.as_ref()).collect();
        assert_eq!(paths, ["spec.replicas"]);
        assert_eq!(passed.changes[0].old.as_deref(), Some("3"));
        assert_eq!(passed.changes[0].new.as_deref(), Some("5"));
        assert!(
            passed
                .rows
                .iter()
                .any(|row| row.text.contains("replicas: 5"))
        );
        // The object carries last-applied-configuration, which a replace leaves stale.
        assert!(
            passed
                .checks
                .iter()
                .any(|check| check.starts_with("kubectl apply users:"))
        );
        // Nothing the server knows beyond the masked tree reaches the preview.
        assert!(
            passed
                .rows
                .iter()
                .all(|row| !row.text.contains("s3cr3t-env"))
        );
    });
    t.t.fixture.draw_twice(cx);
}

#[gpui_kit::test]
fn a_template_change_warns_that_pods_are_replaced(cx: &mut TestAppContext) {
    let t = edit_test("edit-rollout", cx);
    t.open(cx);
    t.change("image: api:1", "image: api:2", cx);
    t.apply(cx);
    t.wait_for_preview(cx);
    t.with_view(cx, |view| {
        let PreviewState::Passed(passed) = view.preview_state() else {
            panic!("the dry-run did not pass");
        };
        assert!(
            passed
                .checks
                .iter()
                .any(|check| check.as_ref() == "Pods will be replaced (RollingUpdate)"),
            "{:?}",
            passed.checks
        );
    });
}

#[gpui_kit::test]
fn ctrl_s_while_running_does_nothing(cx: &mut TestAppContext) {
    let t = edit_test("edit-running", cx);
    t.open(cx);
    t.change("replicas: 3", "replicas: 5", cx);
    let view = t.view(cx);
    // Two presses before the first answer: the second finds the check running.
    t.t.fixture.with_window(cx, |window, cx| {
        view.update(cx, |view, cx| {
            view.apply(window, cx);
            view.apply(window, cx);
        });
    });
    t.wait_for_preview(cx);
    assert_eq!(t.puts().len(), 1, "one dry-run, not two");
    assert!(!t.t.has_dialog(cx), "and no dialog");
}

#[gpui_kit::test]
fn local_error_stays_on_editor_tab(cx: &mut TestAppContext) {
    let t = edit_test("edit-local", cx);
    t.open(cx);
    t.set_text("kind: [unclosed\n", cx);
    t.apply(cx);
    cx.run_until_parked();
    assert_eq!(t.with_view(cx, |view| view.tab()), EditTab::Editor);
    t.with_view(cx, |view| {
        assert!(matches!(
            view.preview_state(),
            PreviewState::Failed(PreviewFailure::Local(cluster::EditError::Syntax { .. }))
        ));
    });
    assert!(t.puts().is_empty());
}

#[gpui_kit::test]
fn an_unmatched_placeholder_blocks_locally_and_names_its_path(cx: &mut TestAppContext) {
    let t = edit_test("edit-placeholder", cx);
    t.open(cx);
    t.change("name: DB_PASS", "name: DB_PASS2", cx);
    t.apply(cx);
    cx.run_until_parked();
    t.with_view(cx, |view| {
        let PreviewState::Failed(PreviewFailure::Local(error)) = view.preview_state() else {
            panic!("expected a local failure");
        };
        assert!(error.to_string().contains("env[DB_PASS2].value"), "{error}");
    });
    assert!(t.puts().is_empty());
}

#[gpui_kit::test]
fn diff_marker_text_never_leaves_the_editor(cx: &mut TestAppContext) {
    let t = edit_test("edit-marker", cx);
    t.open(cx);
    t.change("value: <hidden>", "value: <hidden, changed>", cx);
    t.apply(cx);
    cx.run_until_parked();
    t.with_view(cx, |view| {
        assert!(matches!(
            view.preview_state(),
            PreviewState::Failed(PreviewFailure::Local(cluster::EditError::MarkerText { .. }))
        ));
    });
    assert!(t.puts().is_empty());
}

#[gpui_kit::test]
fn a_422_of_the_preview_lists_its_fields_verbatim(cx: &mut TestAppContext) {
    let t = edit_test("edit-422", cx);
    *lock(&t.server.dry_run_answer) = Some((422, INVALID.to_owned()));
    t.open(cx);
    t.change("replicas: 3", "replicas: -1", cx);
    t.apply(cx);
    t.wait_for_preview(cx);
    t.with_view(cx, |view| {
        let PreviewState::Failed(PreviewFailure::Invalid { fields, .. }) = view.preview_state()
        else {
            panic!("expected an invalid change");
        };
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].as_ref(), "spec.replicas");
    });
    t.t.fixture.draw_twice(cx);
}

#[gpui_kit::test]
fn a_syntax_error_marks_its_line_and_the_footer_jumps_there(cx: &mut TestAppContext) {
    let t = edit_test("edit-error-line", cx);
    t.open(cx);
    t.set_text("kind: Deployment\nspec: [unclosed\nreplicas: 3\n", cx);
    t.apply(cx);
    cx.run_until_parked();
    let (line, marks) = t.view(cx).read_with(cx, |view, cx| {
        (
            view.error_line(&view.text(cx)),
            view.error_mark_ranges(cx).len(),
        )
    });
    let line = line.expect("the parser names a line");
    assert_eq!(marks, 1, "the line is marked");
    t.t.fixture.draw_twice(cx);
    let view = t.view(cx);
    t.t.fixture.with_window(cx, |window, cx| {
        view.update(cx, |view, cx| view.go_to_line(line, window, cx));
    });
    let cursor = t
        .view(cx)
        .read_with(cx, |view, cx| view.cursor_line_for_test(cx));
    assert_eq!(cursor, line);
    // An edit clears the mark: the failure no longer describes the text.
    let view = t.view(cx);
    t.t.fixture.with_window(cx, |window, cx| {
        view.update(cx, |view, cx| view.insert_for_test("x", window, cx));
    });
    let marks = t
        .view(cx)
        .read_with(cx, |view, cx| view.error_mark_ranges(cx).len());
    assert_eq!(marks, 0);
}

#[gpui_kit::test]
fn a_422_field_path_resolves_to_its_line_in_the_editor(cx: &mut TestAppContext) {
    let t = edit_test("edit-422-line", cx);
    *lock(&t.server.dry_run_answer) = Some((422, INVALID.to_owned()));
    t.open(cx);
    t.change("replicas: 3", "replicas: -1", cx);
    t.apply(cx);
    t.wait_for_preview(cx);
    let (line, expected, marks) = t.view(cx).read_with(cx, |view, cx| {
        let text = view.text(cx);
        let expected = text
            .lines()
            .position(|line| line.trim() == "replicas: -1")
            .map(|index| index + 1);
        (
            view.error_line(&text),
            expected,
            view.error_mark_ranges(cx).len(),
        )
    });
    assert_eq!(line, expected);
    assert_eq!(marks, 1);
    t.t.fixture.draw_twice(cx);
    // The side panel row and the footer jump to the same line.
    let view = t.view(cx);
    t.t.fixture.with_window(cx, |window, cx| {
        view.update(cx, |view, cx| {
            view.go_to_line(line.expect("a line"), window, cx)
        });
    });
    let cursor = t
        .view(cx)
        .read_with(cx, |view, cx| view.cursor_line_for_test(cx));
    assert_eq!(Some(cursor), expected);
}
#[gpui_kit::test]
fn the_lock_stops_the_preview_before_anything_is_sent(cx: &mut TestAppContext) {
    let t = edit_test("edit-lock", cx);
    t.open(cx);
    t.change("replicas: 3", "replicas: 5", cx);
    t.t.set_lock(&t.t.stg, WriteLock::Locked, cx);
    t.apply(cx);
    cx.run_until_parked();
    t.with_view(cx, |view| {
        let PreviewState::Failed(PreviewFailure::Server(text)) = view.preview_state() else {
            panic!("expected a refusal");
        };
        assert!(text.contains("stg-b is read-only"), "{text}");
    });
    assert!(t.puts().is_empty());
}

// ---- the commit ----

#[gpui_kit::test]
fn second_ctrl_s_opens_the_confirm_dialog(cx: &mut TestAppContext) {
    let t = edit_test("edit-dialog", cx);
    t.open(cx);
    t.change("replicas: 3", "replicas: 5", cx);
    t.apply(cx);
    t.wait_for_preview(cx);
    assert!(!t.t.has_dialog(cx));
    t.apply(cx);
    assert!(t.t.has_dialog(cx));
    assert_eq!(
        t.t.dialog(cx).read_with(cx, |dialog, _| dialog.label()),
        Some("Apply changes".into())
    );
    // The dialog dry-runs again: the preview and the dialog are two dry-runs.
    t.t.wait_for_dry_run(cx);
    assert_eq!(t.puts().len(), 2);
}

#[gpui_kit::test]
fn dialog_lists_paths_and_check_warnings(cx: &mut TestAppContext) {
    let t = edit_test("edit-dialog-lines", cx);
    t.open(cx);
    t.change("replicas: 3", "replicas: 5", cx);
    t.apply(cx);
    t.wait_for_preview(cx);
    t.apply(cx);
    t.t.dialog(cx).read_with(cx, |dialog, _| {
        assert_eq!(*dialog.tier(), DialogConfirm::Click);
        let lines: Vec<String> = dialog
            .warning_lines()
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(lines[0].starts_with("kubectl apply users:"), "{lines:?}");
        assert_eq!(dialog.dry_run_note(), " · unchanged since you opened it");
    });
}

#[gpui_kit::test]
fn stale_preview_needs_a_new_check(cx: &mut TestAppContext) {
    let t = edit_test("edit-stale", cx);
    t.open(cx);
    t.change("replicas: 3", "replicas: 5", cx);
    t.apply(cx);
    t.wait_for_preview(cx);
    t.change("replicas: 5", "replicas: 6", cx);
    // The preview was for the other text: a press checks again instead of opening the dialog.
    t.apply(cx);
    t.wait_for_preview(cx);
    assert!(!t.t.has_dialog(cx));
    assert_eq!(t.puts().len(), 2);
    assert!(t.is_passed(cx));
}

#[gpui_kit::test]
fn commit_success_closes_the_editor(cx: &mut TestAppContext) {
    let t = edit_test("edit-commit", cx);
    let dir = audit_dir(&t, "edit-commit", cx);
    t.open(cx);
    t.change("replicas: 3", "replicas: 5", cx);
    t.apply(cx);
    t.wait_for_preview(cx);
    t.apply(cx);
    t.confirm_dialog(cx);
    t.t.wait_for("the commit", cx, |_| t.puts().len() == 3);
    t.t.wait_for("the editor to close", cx, |cx| !t.has_edit(cx));
    let puts = t.puts();
    let commit = &puts[2];
    assert!(!commit.has_query_key("dryRun"));
    assert!(commit.has_query("fieldManager", "k8sboard"));
    let body: Value = serde_json::from_str(&commit.body).expect("a JSON body");
    // The base version and uid guard the replace; the server owns the rest.
    assert_eq!(body["metadata"]["resourceVersion"], json!("100"));
    assert_eq!(body["metadata"]["uid"], json!("uid-1"));
    assert_eq!(body["spec"]["replicas"], json!(5));
    assert!(body.get("status").is_none());
    // The masked env value went back as the server's own value, never as a placeholder.
    assert!(commit.body.contains("s3cr3t-env"));
    for put in &puts {
        assert!(!put.body.contains("<hidden"), "{}", put.body);
        assert!(!put.body.contains("# Values shown"), "{}", put.body);
    }
    // One line, paths only, no preview text.
    t.t.wait_for("the audit line", cx, |_| audit_lines(&dir).len() == 1);
    let lines = audit_lines(&dir);
    let line = &lines[0];
    assert_eq!(line["action"], json!("Edit YAML"));
    assert_eq!(line["outcome"], json!("applied"));
    assert_eq!(line["cluster"], json!("stg-b"));
    assert_eq!(line["object"]["kind"], json!("Deployment"));
    assert_eq!(line["object"]["name"], json!("api"));
    assert_eq!(line["fields"], json!([{"path": "spec.replicas"}]));
    let raw = std::fs::read_to_string(crate::audit_log::audit_path(&dir)).unwrap_or_default();
    for forbidden in ["replicas: 5", "s3cr3t-env", "<hidden", "kubectl apply"] {
        assert!(
            !raw.contains(forbidden),
            "{forbidden} reached the audit log"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn commit_rechecks_the_row_cluster(cx: &mut TestAppContext) {
    let t = edit_test("edit-prod", cx);
    let prod_api = t.activate(&t.t.prod, cx);
    t.t.set_lock(&t.t.prod, WriteLock::Unlocked, cx);
    let dir = audit_dir(&t, "edit-prod", cx);
    t.cursor_on(&t.t.prod, cx);
    t.t.fixture.press("e", cx);
    t.wait_for_base(cx);
    t.change("replicas: 3", "replicas: 5", cx);
    t.apply(cx);
    t.wait_for_preview(cx);
    t.apply(cx);
    t.t.wait_for_dry_run(cx);
    t.t.dialog(cx).read_with(cx, |dialog, _| {
        assert_eq!(
            *dialog.tier(),
            DialogConfirm::TypeName {
                expected: "api".to_owned()
            }
        );
    });
    // Without the name nothing is committed.
    t.t.confirm(cx);
    cx.run_until_parked();
    assert_eq!(puts_of(&prod_api).len(), 2, "two dry-runs, no commit");
    t.confirm_dialog(cx);
    t.t.wait_for("the commit", cx, |_| puts_of(&prod_api).len() == 3);
    t.t.wait_for("the audit line", cx, |_| audit_lines(&dir).len() == 1);
    assert_eq!(audit_lines(&dir)[0]["cluster"], json!("prod-a"));
    assert!(puts_of(&t.t.stg_api).is_empty(), "staging was not touched");
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn commit_conflict_shows_the_banner(cx: &mut TestAppContext) {
    let t = edit_test("edit-conflict", cx);
    let dir = audit_dir(&t, "edit-conflict", cx);
    *lock(&t.server.commit_answer) = Some((409, CONFLICT.to_owned()));
    t.open(cx);
    t.change("replicas: 3", "replicas: 5", cx);
    t.apply(cx);
    t.wait_for_preview(cx);
    t.apply(cx);
    t.confirm_dialog(cx);
    t.t.wait_for("the banner", cx, |cx| {
        t.edit(cx).is_some_and(|edit| {
            edit.read_with(cx, |view, _| {
                matches!(view.banner(), Some(EditBanner::Conflict))
            })
        })
    });
    t.t.wait_for("the dialog to close", cx, |cx| !t.t.has_dialog(cx));
    assert!(t.has_edit(cx), "the text is kept");
    t.t.wait_for("the audit line", cx, |_| audit_lines(&dir).len() == 1);
    assert_eq!(audit_lines(&dir)[0]["outcome"], json!("failed"));
    t.t.fixture.draw_twice(cx);
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn invalid_lists_fields_in_the_side_panel(cx: &mut TestAppContext) {
    let t = edit_test("edit-commit-422", cx);
    *lock(&t.server.commit_answer) = Some((422, INVALID.to_owned()));
    t.open(cx);
    t.change("replicas: 3", "replicas: -1", cx);
    t.apply(cx);
    t.wait_for_preview(cx);
    t.apply(cx);
    t.confirm_dialog(cx);
    t.t.wait_for("the field list", cx, |cx| {
        t.edit(cx).is_some_and(|edit| {
            edit.read_with(cx, |view, _| {
                matches!(
                    view.preview_state(),
                    PreviewState::Failed(PreviewFailure::Invalid { .. })
                )
            })
        })
    });
    assert!(t.has_edit(cx));
}

#[gpui_kit::test]
fn stale_base_is_a_conflict_without_a_put(cx: &mut TestAppContext) {
    let t = edit_test("edit-stale-base", cx);
    t.open(cx);
    t.change("replicas: 3", "replicas: 5", cx);
    *lock(&t.server.object) = deployment_object("200");
    t.apply(cx);
    t.wait_for_preview(cx);
    assert!(t.puts().is_empty(), "the stale base stops before any PUT");
    t.with_view(cx, |view| {
        assert!(matches!(view.banner(), Some(EditBanner::Conflict)));
    });
}

#[gpui_kit::test]
fn reload_and_keep_my_changes_rebases_and_checks_again(cx: &mut TestAppContext) {
    let t = edit_test("edit-rebase", cx);
    t.open(cx);
    t.change("replicas: 3", "replicas: 5", cx);
    // Somebody else adds a label and moves the version.
    let mut newer = deployment_object("200");
    newer["metadata"]["labels"] = json!({"app": "api", "tier": "backend"});
    *lock(&t.server.object) = newer;
    t.apply(cx);
    t.wait_for_preview(cx);
    let view = t.view(cx);
    t.t.fixture.with_window(cx, |window, cx| {
        view.update(cx, |view, cx| view.keep_my_changes(window, cx));
    });
    t.t.wait_for("the new check", cx, |_| t.puts().len() == 1);
    t.wait_for_preview(cx);
    let text = t.text(cx);
    assert!(text.contains("replicas: 5"), "{text}");
    assert!(text.contains("tier: backend"), "{text}");
    t.with_view(cx, |view| {
        assert!(matches!(
            view.banner(),
            Some(EditBanner::Rebased { unreachable, .. }) if unreachable.is_empty()
        ));
        let changed: Vec<&str> = view.server_changed().iter().map(AsRef::as_ref).collect();
        assert_eq!(changed, ["metadata.labels.tier"]);
    });
    // The check ran against the new base.
    let body: Value = serde_json::from_str(&t.puts()[0].body).expect("a JSON body");
    assert_eq!(body["metadata"]["resourceVersion"], json!("200"));
    assert_eq!(body["metadata"]["labels"]["tier"], json!("backend"));
    t.t.fixture.draw_twice(cx);
}

#[gpui_kit::test]
fn rebase_lists_a_path_that_no_longer_exists(cx: &mut TestAppContext) {
    let t = edit_test("edit-unreachable", cx);
    t.open(cx);
    t.change("image: proxy:1", "image: proxy:9", cx);
    let mut newer = deployment_object("200");
    let containers = newer["spec"]["template"]["spec"]["containers"]
        .as_array_mut()
        .expect("containers");
    containers.retain(|container| container["name"] != "sidecar");
    *lock(&t.server.object) = newer;
    t.apply(cx);
    t.wait_for_preview(cx);
    let view = t.view(cx);
    t.t.fixture.with_window(cx, |window, cx| {
        view.update(cx, |view, cx| view.keep_my_changes(window, cx));
    });
    t.t.wait_for("the rebase", cx, |cx| {
        t.edit(cx).is_some_and(|edit| {
            edit.read_with(cx, |view, _| {
                matches!(view.banner(), Some(EditBanner::Rebased { .. }))
            })
        })
    });
    t.with_view(cx, |view| {
        let Some(EditBanner::Rebased { unreachable, .. }) = view.banner() else {
            panic!("expected the rebase banner");
        };
        assert_eq!(
            unreachable[0].as_ref(),
            "spec.template.spec.containers[sidecar].image: no longer exists on the server"
        );
    });
    t.t.fixture.draw_twice(cx);
}

#[gpui_kit::test]
fn discard_my_changes_reads_the_newest_object(cx: &mut TestAppContext) {
    let t = edit_test("edit-discard-mine", cx);
    t.open(cx);
    t.change("replicas: 3", "replicas: 5", cx);
    let mut newer = deployment_object("200");
    newer["spec"]["replicas"] = json!(8);
    *lock(&t.server.object) = newer;
    t.apply(cx);
    t.wait_for_preview(cx);
    let view = t.view(cx);
    t.t.fixture.with_window(cx, |window, cx| {
        view.update(cx, |view, cx| view.discard_my_changes(window, cx));
    });
    t.t.wait_for("the new text", cx, |cx| {
        t.edit(cx).is_some_and(|edit| {
            edit.read_with(cx, |view, cx| view.text(cx).contains("replicas: 8"))
        })
    });
    assert!(!t.with_view(cx, |view| view.is_dirty()));
    assert!(t.with_view(cx, |view| view.banner().is_none()));
}

// ---- leaving ----

#[gpui_kit::test]
fn navigation_with_changes_asks_to_discard(cx: &mut TestAppContext) {
    let t = edit_test("edit-nav", cx);
    t.open(cx);
    t.change("replicas: 3", "replicas: 5", cx);
    t.shell().update(cx, |shell, cx| {
        shell.show_screen(Screen::Kind(ResourceKind::Services), cx)
    });
    cx.run_until_parked();
    // Nothing moved while the question is open.
    assert_eq!(
        t.shell()
            .read_with(cx, |shell, _| shell.last_discard.clone()),
        Some("Deployment/team-a/api".to_owned())
    );
    assert!(t.has_edit(cx));
    assert_eq!(
        t.shell().read_with(cx, |shell, _| shell.screen),
        Screen::Kind(ResourceKind::Deployments)
    );
    // Keep editing keeps the text.
    t.press_dialog(Cancel, cx);
    assert!(t.has_edit(cx));
    assert!(t.with_view(cx, |view| view.is_dirty()));
    // Asking again and discarding leaves.
    t.shell().update(cx, |shell, cx| {
        shell.show_screen(Screen::Kind(ResourceKind::Services), cx)
    });
    cx.run_until_parked();
    t.press_dialog(Confirm { secondary: false }, cx);
    assert!(!t.has_edit(cx));
    assert_eq!(
        t.shell().read_with(cx, |shell, _| shell.screen),
        Screen::Kind(ResourceKind::Services)
    );
}

#[gpui_kit::test]
fn a_clean_editor_closes_silently_on_navigation(cx: &mut TestAppContext) {
    let t = edit_test("edit-nav-clean", cx);
    t.open(cx);
    t.shell().update(cx, |shell, cx| {
        shell.show_screen(Screen::Kind(ResourceKind::Services), cx)
    });
    cx.run_until_parked();
    assert!(!t.has_edit(cx));
    assert!(!t.t.has_dialog(cx));
    assert_eq!(
        t.shell()
            .read_with(cx, |shell, _| shell.last_discard.clone()),
        None
    );
}

#[gpui_kit::test]
fn a_namespace_change_with_changes_asks_to_discard(cx: &mut TestAppContext) {
    let t = edit_test("edit-namespace", cx);
    t.open(cx);
    t.change("replicas: 3", "replicas: 5", cx);
    t.shell().update(cx, |shell, cx| {
        shell.set_namespace(NamespaceScope::Named("team-a".to_owned()), cx)
    });
    cx.run_until_parked();
    assert_eq!(
        t.shell()
            .read_with(cx, |shell, _| shell.last_discard.clone()),
        Some("Deployment/team-a/api".to_owned())
    );
    assert!(t.has_edit(cx));
}

/// An object of the open cluster that is not the one being edited.
fn other_object(t: &EditTest, cx: &mut TestAppContext) -> ClusterObject {
    let selected = t
        .shell()
        .read_with(cx, |shell, _| shell.selected.clone())
        .expect("the edited row is selected");
    ClusterObject::new(
        selected.cluster,
        ResourceKey::Pod {
            namespace: "team-a".to_owned(),
            name: "api-0".to_owned(),
        },
    )
}

#[gpui_kit::test]
fn a_reveal_with_changes_records_only_after_the_discard(cx: &mut TestAppContext) {
    let t = edit_test("edit-reveal-record", cx);
    t.open(cx);
    t.change("replicas: 3", "replicas: 5", cx);
    let other = other_object(&t, cx);
    t.shell()
        .update(cx, |shell, cx| shell.reveal_object(other, cx));
    cx.run_until_parked();
    let previous = |cx: &mut TestAppContext| {
        t.shell()
            .read_with(cx, |shell, _| shell.navigation.previous().is_some())
    };
    assert!(
        !previous(cx),
        "nothing is recorded while the question is open"
    );
    t.press_dialog(Cancel, cx);
    assert!(!previous(cx), "Keep editing leaves no entry");
    let other = other_object(&t, cx);
    t.shell()
        .update(cx, |shell, cx| shell.reveal_object(other, cx));
    cx.run_until_parked();
    t.press_dialog(Confirm { secondary: false }, cx);
    assert!(previous(cx));
}

#[gpui_kit::test]
fn going_back_with_changes_asks_to_discard(cx: &mut TestAppContext) {
    let t = edit_test("edit-back", cx);
    t.open(cx);
    let other = other_object(&t, cx);
    t.shell()
        .update(cx, |shell, cx| shell.record_place_before_reveal(&other, cx));
    t.change("replicas: 3", "replicas: 5", cx);
    t.shell().update(cx, |shell, cx| shell.go_back(cx));
    cx.run_until_parked();
    assert_eq!(
        t.shell()
            .read_with(cx, |shell, _| shell.last_discard.clone()),
        Some("Deployment/team-a/api".to_owned())
    );
    assert!(t.has_edit(cx));
    t.press_dialog(Confirm { secondary: false }, cx);
    assert!(!t.has_edit(cx));
}
#[gpui_kit::test]
fn cancel_asks_only_when_the_text_has_changes(cx: &mut TestAppContext) {
    let t = edit_test("edit-cancel", cx);
    t.open(cx);
    t.change("replicas: 3", "replicas: 5", cx);
    let view = t.view(cx);
    view.update(cx, |view, cx| view.cancel(cx));
    cx.run_until_parked();
    assert!(t.has_edit(cx), "the question is open");
    assert!(
        t.shell()
            .read_with(cx, |shell, _| shell.last_discard.is_some())
    );
    t.press_dialog(Confirm { secondary: false }, cx);
    assert!(!t.has_edit(cx));
}

#[gpui_kit::test]
fn closing_the_editor_restores_the_cursor_and_the_table(cx: &mut TestAppContext) {
    let t = edit_test("edit-restore", cx);
    t.open(cx);
    let cursor = t.shell().read_with(cx, |shell, _| shell.selected.clone());
    let view = t.view(cx);
    view.update(cx, |view, cx| view.cancel(cx));
    cx.run_until_parked();
    assert!(!t.has_edit(cx));
    assert_eq!(
        t.shell().read_with(cx, |shell, _| shell.selected.clone()),
        cursor
    );
    t.t.fixture.draw_twice(cx);
    // The key layer works again.
    t.t.fixture.press("e", cx);
    t.wait_for_base(cx);
    assert!(t.has_edit(cx));
}

#[gpui_kit::test]
fn cluster_switch_with_changes_asks_to_discard(cx: &mut TestAppContext) {
    let t = edit_test("edit-switch", cx);
    t.open(cx);
    t.change("replicas: 3", "replicas: 5", cx);
    let target = t.t.fixture.cluster("dev-c", cx);
    t.shell()
        .update(cx, |shell, cx| shell.switch_cluster(&target, cx));
    cx.run_until_parked();
    assert_eq!(
        t.shell()
            .read_with(cx, |shell, _| shell.last_leaving.clone()),
        Some(vec!["Unsaved changes to Deployment/team-a/api".to_owned()])
    );
    // Stay: the session and the text are kept.
    t.press_dialog(Cancel, cx);
    assert!(t.has_edit(cx));
    assert!(
        t.shell()
            .read_with(cx, |shell, _| shell.session_of(&t.t.stg).is_some())
    );
    // Leave: the cluster is released and the editor goes with it.
    t.shell()
        .update(cx, |shell, cx| shell.switch_cluster(&target, cx));
    cx.run_until_parked();
    t.press_dialog(Confirm { secondary: false }, cx);
    assert!(!t.has_edit(cx));
    assert!(
        t.shell()
            .read_with(cx, |shell, _| shell.session_of(&t.t.stg).is_none())
    );
}

#[gpui_kit::test]
fn a_clean_editor_closes_with_its_released_cluster_without_asking(cx: &mut TestAppContext) {
    let t = edit_test("edit-release-clean", cx);
    t.open(cx);
    let target = t.t.fixture.cluster("dev-c", cx);
    t.shell()
        .update(cx, |shell, cx| shell.switch_cluster(&target, cx));
    cx.run_until_parked();
    assert!(!t.t.has_dialog(cx));
    assert!(!t.has_edit(cx));
}

#[gpui_kit::test]
fn preview_never_reaches_audit_or_notice(cx: &mut TestAppContext) {
    use super::write_flow::{CheckedWriteError, failure_notice};

    let t = edit_test("edit-no-preview-leak", cx);
    let dir = audit_dir(&t, "edit-no-preview-leak", cx);
    t.t.set_lock(&t.t.stg, WriteLock::Unlocked, cx);
    *lock(&t.server.commit_answer) = Some((422, INVALID.to_owned()));
    t.open(cx);
    t.change("replicas: 3", "replicas: -1", cx);
    t.apply(cx);
    t.wait_for_preview(cx);
    t.apply(cx);
    t.confirm_dialog(cx);
    t.t.wait_for("the audit line", cx, |_| audit_lines(&dir).len() == 1);
    let raw = std::fs::read_to_string(crate::audit_log::audit_path(&dir)).unwrap_or_default();
    for preview_text in ["replicas: -1", "s3cr3t-env", "kubectl apply", "<hidden"] {
        assert!(
            !raw.contains(preview_text),
            "{preview_text} reached the audit"
        );
    }
    // The notice of a failed commit names the change, never the text of the preview.
    let error = CheckedWriteError::Write(cluster::WriteError::Invalid {
        message: "the change is invalid".to_owned(),
        fields: vec!["spec.replicas".to_owned()],
    });
    let notice = failure_notice("Apply changes", &error);
    assert!(!notice.contains("replicas: -1"), "{notice}");
    let _ = std::fs::remove_dir_all(&dir);
}

// ---- review fixes ----

fn key_down(key: &str, is_held: bool) -> KeyDownEvent {
    KeyDownEvent {
        keystroke: Keystroke::parse(key).expect("a valid keystroke"),
        is_held,
        prefer_character_input: false,
    }
}

impl EditTest {
    fn press_event(&self, event: KeyDownEvent, cx: &mut TestAppContext) {
        self.t.fixture.with_window(cx, |window, cx| {
            window.dispatch_event(event.to_platform_input(), cx);
        });
    }

    /// Opens the editor, changes the replicas, and moves the server on to another version in which
    /// `change` was made (the object keeps its uid unless `change` replaces it). The first check
    /// then reports the conflict.
    fn conflict_with(&self, change: impl FnOnce(&mut Value), cx: &mut TestAppContext) {
        self.open(cx);
        self.change("replicas: 3", "replicas: 5", cx);
        let mut newer = deployment_object("200");
        change(&mut newer);
        *lock(&self.server.object) = newer;
        self.apply(cx);
        self.wait_for_preview(cx);
    }

    fn keep_my_changes(&self, cx: &mut TestAppContext) {
        let view = self.view(cx);
        self.t.fixture.with_window(cx, |window, cx| {
            view.update(cx, |view, cx| view.keep_my_changes(window, cx));
        });
    }
}

#[gpui_kit::test]
fn rebase_onto_a_recreated_object_offers_only_discard(cx: &mut TestAppContext) {
    let t = edit_test("edit-recreated", cx);
    t.conflict_with(|object| object["metadata"]["uid"] = json!("uid-2"), cx);
    t.keep_my_changes(cx);
    t.t.wait_for("the banner", cx, |cx| {
        t.with_view(cx, |view| {
            matches!(view.banner(), Some(EditBanner::Recreated))
        })
    });
    // The text is kept, and nothing can be applied to the new object.
    assert!(t.text(cx).contains("replicas: 5"));
    t.apply(cx);
    cx.run_until_parked();
    assert!(t.puts().is_empty());
    assert!(!t.t.has_dialog(cx));
    t.t.fixture.draw_twice(cx);
    // Discard reads the new object.
    let view = t.view(cx);
    t.t.fixture.with_window(cx, |window, cx| {
        view.update(cx, |view, cx| view.discard_my_changes(window, cx));
    });
    t.t.wait_for("the new object", cx, |cx| {
        t.with_view(cx, |view| view.banner().is_none() && !view.is_dirty())
    });
    assert!(t.text(cx).contains("replicas: 3"));
}

#[gpui_kit::test]
fn a_rebase_that_overwrites_a_server_change_says_so_and_warns_in_the_dialog(
    cx: &mut TestAppContext,
) {
    let t = edit_test("edit-overwrite", cx);
    t.conflict_with(|object| object["spec"]["replicas"] = json!(7), cx);
    t.keep_my_changes(cx);
    t.t.wait_for("the new check", cx, |_| t.puts().len() == 1);
    t.wait_for_preview(cx);
    let line = "spec.replicas: your value replaces a change made on the server";
    t.with_view(cx, |view| {
        let Some(EditBanner::Rebased { overwritten, .. }) = view.banner() else {
            panic!("expected the rebase banner");
        };
        assert_eq!(overwritten.len(), 1);
        assert_eq!(overwritten[0].as_ref(), line);
    });
    t.apply(cx);
    let lines: Vec<String> = t.t.dialog(cx).read_with(cx, |dialog, _| {
        dialog
            .warning_lines()
            .iter()
            .map(ToString::to_string)
            .collect()
    });
    assert!(lines.iter().any(|text| text == line), "{lines:?}");
    t.t.fixture.draw_twice(cx);
}

#[gpui_kit::test]
fn a_failed_rebase_keeps_the_old_base_so_the_conflict_stands(cx: &mut TestAppContext) {
    let t = edit_test("edit-rebase-fails", cx);
    t.conflict_with(|object| object["spec"]["replicas"] = json!(7), cx);
    let base_text = t.base_text(cx);
    t.change("replicas: 5", "replicas: 0444", cx);
    t.keep_my_changes(cx);
    t.t.wait_for("the refusal", cx, |cx| {
        t.with_view(cx, |view| {
            matches!(
                view.preview_state(),
                PreviewState::Failed(PreviewFailure::Local(
                    cluster::EditError::LeadingZero { .. }
                ))
            )
        })
    });
    assert_eq!(t.base_text(cx), base_text, "the old base is kept");
    assert!(t.with_view(cx, |view| matches!(
        view.banner(),
        Some(EditBanner::Conflict)
    )));
}

#[gpui_kit::test]
fn format_refuses_a_leading_zero_and_keeps_the_text(cx: &mut TestAppContext) {
    let t = edit_test("edit-format-zero", cx);
    t.open(cx);
    t.change("replicas: 3", "replicas: 0444", cx);
    let before = t.text(cx);
    let view = t.view(cx);
    t.t.fixture.with_window(cx, |window, cx| {
        view.update(cx, |view, cx| view.format(window, cx));
    });
    assert_eq!(t.text(cx), before, "the text is not rewritten");
    t.with_view(cx, |view| {
        let PreviewState::Failed(PreviewFailure::Local(error)) = view.preview_state() else {
            panic!("expected a local failure");
        };
        assert!(error.to_string().starts_with("line "), "{error}");
    });
}

#[gpui_kit::test]
fn a_text_over_two_mebibytes_is_refused_without_a_request(cx: &mut TestAppContext) {
    let t = edit_test("edit-big", cx);
    t.open(cx);
    let big = format!("{}# {}\n", t.base_text(cx), "x".repeat(2 * 1024 * 1024));
    t.set_text(&big, cx);
    t.apply(cx);
    cx.run_until_parked();
    t.with_view(cx, |view| {
        assert!(matches!(
            view.preview_state(),
            PreviewState::Failed(PreviewFailure::Local(cluster::EditError::TooLarge))
        ));
    });
    assert!(t.puts().is_empty());
}

#[gpui_kit::test]
fn the_palette_offers_no_row_write_while_editing(cx: &mut TestAppContext) {
    let t = edit_test("edit-palette-hidden", cx);
    t.cursor_on(&t.t.stg, cx);
    let offered = |t: &EditTest, cx: &mut TestAppContext| {
        t.shell()
            .read_with(cx, |shell, cx| shell.palette_snapshot(&parse_query(""), cx))
            .entries
            .iter()
            .filter(|entry| {
                matches!(
                    entry.target,
                    crate::palette_search::PaletteTarget::RowAction(_)
                        | crate::palette_search::PaletteTarget::RollBack(..)
                )
            })
            .count()
    };
    assert!(
        offered(&t, cx) > 0,
        "the cursor row has actions before the edit"
    );
    t.t.fixture.press("e", cx);
    t.wait_for_base(cx);
    assert_eq!(offered(&t, cx), 0, "none while the cursor is hidden");
    // A direct call, such as the replicas field of the palette, is refused as well.
    t.t.fixture.with_window(cx, |window, cx| {
        t.shell()
            .update(cx, |shell, cx| shell.scale_cursor_row(9, window, cx));
    });
    cx.run_until_parked();
    assert!(!t.t.has_dialog(cx));
    assert!(writes(&t.t.stg_api).is_empty());
}

#[gpui_kit::test]
fn a_held_ctrl_s_never_opens_the_confirm_dialog(cx: &mut TestAppContext) {
    let t = edit_test("edit-held", cx);
    t.open(cx);
    t.change("replicas: 3", "replicas: 5", cx);
    t.press_event(key_down("ctrl-s", false), cx);
    t.wait_for_preview(cx);
    assert!(t.is_passed(cx));
    // The repeat of the held key finds the passed check, and still opens nothing.
    t.press_event(key_down("ctrl-s", true), cx);
    cx.run_until_parked();
    assert!(!t.t.has_dialog(cx));
    // A fresh press does.
    t.press_event(key_down("ctrl-s", false), cx);
    cx.run_until_parked();
    assert!(t.t.has_dialog(cx));
}

// ---- Spec 0041: the Revision history tab ----

fn replica_set_lists_of(api: &FakeApi) -> Vec<RecordedRequest> {
    api.requests()
        .into_iter()
        .filter(|request| request.path == "/apis/apps/v1/namespaces/team-a/replicasets")
        .collect()
}

#[gpui_kit::test]
fn history_never_changes_the_editor_text(cx: &mut TestAppContext) {
    let t = edit_test("edit-history", cx);
    t.open(cx);
    t.change("replicas: 3", "replicas: 4", cx);
    let before = t.text(cx);
    let view = t.view(cx);
    view.update(cx, |view, cx| view.show_tab(EditTab::History, cx));
    t.t.wait_for("the revisions", cx, |cx| {
        view.read_with(cx, |view, cx| {
            view.history()
                .is_some_and(|history| history.read(cx).is_ready())
        })
    });
    view.update(cx, |view, cx| view.show_tab(EditTab::Editor, cx));
    assert_eq!(t.text(cx), before);
    assert!(t.with_view(cx, YamlEditView::is_dirty));
    // One LIST bounded by the Deployment's selector, and showing the tab again asks nothing more.
    view.update(cx, |view, cx| view.show_tab(EditTab::History, cx));
    cx.run_until_parked();
    let lists = replica_set_lists_of(&t.t.stg_api);
    assert_eq!(lists.len(), 1, "{lists:?}");
    assert!(lists[0].has_query("labelSelector", "app%3Dapi"));
    assert!(puts_of(&t.t.stg_api).is_empty());
}

#[gpui_kit::test]
fn history_roll_back_asks_for_the_row_revision_while_editing(cx: &mut TestAppContext) {
    let t = edit_test("edit-history-roll-back", cx);
    t.open(cx);
    let view = t.view(cx);
    view.update(cx, |view, cx| view.show_tab(EditTab::History, cx));
    t.t.wait_for("the revisions", cx, |cx| {
        view.read_with(cx, |view, cx| {
            view.history()
                .is_some_and(|history| history.read(cx).is_ready())
        })
    });
    t.t.fixture.draw_twice(cx);
    // Newest first: rev 2 is current and has no button, rev 1 is the second row.
    t.t.fixture.with_window(cx, |window, cx| {
        window.click(("history-roll-back", 1usize), cx)
    });
    cx.run_until_parked();
    assert_eq!(
        t.t.dialog_label(cx),
        "Roll back deployment api to rev 1 (1)"
    );
    assert!(puts_of(&t.t.stg_api).is_empty());
}

/// The Roll back of the Deployment `api` that the open edit shows, as the write flow built it.
fn roll_back_of_api(t: &EditTest) -> super::write_flow::WriteIntent {
    use crate::workload_actions::{RevisionTarget, WorkloadScope, roll_back_intent};
    let target = RevisionTarget {
        replica_set: "api-1".to_owned(),
        revision: 1,
        tag: None,
    };
    let scope = WorkloadScope {
        cluster: &t.t.stg,
        cluster_name: "stg-b",
    };
    roll_back_intent(&scope, &deployment("api"), &target).expect("a roll back intent")
}

#[gpui_kit::test]
fn a_roll_back_closes_the_edit_that_holds_the_old_revision(cx: &mut TestAppContext) {
    let t = edit_test("edit-roll-back-closes", cx);
    t.open(cx);
    let intent = roll_back_of_api(&t);
    t.shell()
        .update(cx, |shell, cx| shell.roll_back_finished(&intent, cx));
    assert!(!t.has_edit(cx));
}

#[gpui_kit::test]
fn a_roll_back_keeps_an_edit_with_unsaved_text(cx: &mut TestAppContext) {
    let t = edit_test("edit-roll-back-dirty", cx);
    t.open(cx);
    t.change("replicas: 3", "replicas: 4", cx);
    let intent = roll_back_of_api(&t);
    t.shell()
        .update(cx, |shell, cx| shell.roll_back_finished(&intent, cx));
    assert!(t.has_edit(cx));
}

// ---- Spec 0041: the quota line ----

fn quota_snapshot(hard: &str, used: &str) -> Vec<crate::kind_row::KindObject> {
    vec![crate::kind_row::KindObject::ResourceQuota(
        cluster::ResourceQuotaSummary {
            namespace: "team-a".to_owned(),
            name: "compute".to_owned(),
            created_at: None,
            labels: Vec::new(),
            items: vec![cluster::QuotaItem {
                resource: "pods".to_owned(),
                hard: hard.to_owned(),
                used: Some(used.to_owned()),
            }],
            scopes: Vec::new(),
        },
    )]
}

/// Gives the ResourceQuotas feed of the live session this snapshot, as its watch would.
fn set_quotas(t: &EditTest, items: Vec<crate::kind_row::KindObject>, cx: &mut TestAppContext) {
    let session = t.t.fixture.session(cx);
    session.update(cx, |session, _| {
        session.apply_condition_update(
            ResourceKind::ResourceQuotas,
            cluster::WatchUpdate::Snapshot(items),
        );
    });
}

fn quota_of(t: &EditTest, cx: &mut TestAppContext) -> crate::edit_quota::QuotaLine {
    t.view(cx)
        .read_with(cx, |view, _| match view.preview_state() {
            PreviewState::Passed(passed) => passed.quota.clone(),
            _ => panic!("the preview did not pass"),
        })
}

#[gpui_kit::test]
fn apply_stays_enabled_when_quota_exceeds(cx: &mut TestAppContext) {
    let t = edit_test("edit-quota-exceeds", cx);
    t.open(cx);
    // 1 pod left; the change adds 2.
    set_quotas(&t, quota_snapshot("4", "3"), cx);
    t.change("replicas: 3", "replicas: 5", cx);
    t.apply(cx);
    t.wait_for_preview(cx);
    let expected = "Quota compute: pods needs 2 more, 1 left";
    assert_eq!(
        quota_of(&t, cx),
        crate::edit_quota::QuotaLine::Exceeds(vec![expected.into()])
    );
    assert_eq!(t.with_view(cx, YamlEditView::apply_block_reason), None);
    // Apply still opens the confirm dialog, which repeats the warning next to the other checks.
    t.apply(cx);
    t.t.dialog(cx).read_with(cx, |dialog, _| {
        let lines: Vec<String> = dialog
            .warning_lines()
            .iter()
            .map(ToString::to_string)
            .collect();
        assert!(lines.contains(&expected.to_owned()), "{lines:?}");
    });
}

#[gpui_kit::test]
fn quota_fits_reads_the_namespace_headroom(cx: &mut TestAppContext) {
    let t = edit_test("edit-quota-fits", cx);
    t.open(cx);
    set_quotas(&t, quota_snapshot("10", "3"), cx);
    t.change("replicas: 3", "replicas: 5", cx);
    t.apply(cx);
    t.wait_for_preview(cx);
    assert_eq!(
        quota_of(&t, cx),
        crate::edit_quota::QuotaLine::Fits("Namespace quota OK (5 pods left)".into())
    );
}

#[gpui_kit::test]
fn quota_feed_off_says_not_checked_and_adds_no_warning(cx: &mut TestAppContext) {
    let t = edit_test("edit-quota-off", cx);
    t.open(cx);
    t.change("replicas: 3", "replicas: 5", cx);
    t.apply(cx);
    t.wait_for_preview(cx);
    // The fake server has no quotas route, so the feed never loaded.
    let line = quota_of(&t, cx);
    let crate::edit_quota::QuotaLine::NotChecked(text) = &line else {
        panic!("expected NotChecked, got {line:?}");
    };
    assert!(text.starts_with("Quota not checked: "), "{text}");
    assert!(line.warnings().is_empty());
    assert_eq!(t.with_view(cx, YamlEditView::apply_block_reason), None);
}

// ---- Spec 0041: a click on a Deployment row of the timeline ----

fn deployment_key() -> ResourceKey {
    ResourceKey::Kind {
        kind: ResourceKind::Deployments,
        namespace: Some("team-a".to_owned()),
        name: "api".to_owned(),
    }
}

fn click_change(t: &EditTest, named: Option<&str>, cx: &mut TestAppContext) {
    let named = named.map(str::to_owned);
    t.t.fixture.with_window(cx, |window, cx| {
        t.shell().update(cx, |shell, cx| {
            shell.open_change_diff(deployment_key(), named, window, cx)
        });
    });
}

fn has_dialog_open(t: &EditTest, cx: &mut TestAppContext) -> bool {
    t.t.fixture
        .with_window(cx, |window, cx| window.has_active_dialog(cx))
}

fn notice_count(t: &EditTest, cx: &mut TestAppContext) -> usize {
    t.t.fixture
        .with_window(cx, |window, cx| window.notifications(cx).len())
}

/// The names of the ReplicaSets whose templates the dialog read, sorted.
fn template_reads(api: &FakeApi) -> Vec<String> {
    let mut names: Vec<String> = api
        .requests()
        .into_iter()
        .filter(|request| request.method == "GET")
        .filter_map(|request| {
            request
                .path
                .strip_prefix("/apis/apps/v1/namespaces/team-a/replicasets/")
                .map(str::to_owned)
        })
        .collect();
    names.sort();
    names
}

fn wait_for_template_reads(t: &EditTest, count: usize, cx: &mut TestAppContext) {
    t.t.wait_for("the template reads", cx, |_| {
        template_reads(&t.t.stg_api).len() >= count
    });
}

#[gpui_kit::test]
fn deployment_row_click_opens_latest_diff(cx: &mut TestAppContext) {
    let t = edit_test("change-latest", cx);
    // The message names no listed set: the newest revision against the one before.
    click_change(&t, Some("api-gone"), cx);
    wait_for_template_reads(&t, 2, cx);
    assert!(has_dialog_open(&t, cx));
    let lists = replica_set_lists_of(&t.t.stg_api);
    assert_eq!(lists.len(), 1, "{lists:?}");
    assert!(lists[0].has_query("labelSelector", "app%3Dapi"));
    assert_eq!(template_reads(&t.t.stg_api), ["api-a", "api-b"]);
}

#[gpui_kit::test]
fn deployment_row_click_diffs_the_named_set(cx: &mut TestAppContext) {
    let t = edit_test("change-named", cx);
    *lock(&t.server.replica_sets) = vec![
        replica_set("api-a", 1),
        replica_set("api-b", 2),
        replica_set("api-c", 3),
    ];
    click_change(&t, Some("api-b"), cx);
    wait_for_template_reads(&t, 2, cx);
    assert!(has_dialog_open(&t, cx));
    // The named set against its predecessor, not the newest pair.
    assert_eq!(template_reads(&t.t.stg_api), ["api-a", "api-b"]);
}

#[gpui_kit::test]
fn deployment_row_click_denied_shows_notice(cx: &mut TestAppContext) {
    let t = edit_test("change-denied", cx);
    let report = cluster::AccessReport {
        reviews: cluster::AccessCheck::ALL
            .into_iter()
            .map(|check| cluster::AccessReview {
                check,
                decision: if check == cluster::AccessCheck::ListReplicaSets {
                    cluster::AccessDecision::Denied { reason: None }
                } else {
                    cluster::AccessDecision::Allowed
                },
            })
            .collect(),
    };
    t.t.fixture.session(cx).update(cx, |session, cx| {
        session.set_access_for_test(crate::cluster_session::AccessState::Known(report), cx);
    });
    let before = notice_count(&t, cx);
    click_change(&t, None, cx);
    cx.run_until_parked();
    assert_eq!(notice_count(&t, cx), before + 1);
    assert!(!has_dialog_open(&t, cx));
    assert!(replica_set_lists_of(&t.t.stg_api).is_empty());
}

#[gpui_kit::test]
fn single_revision_click_shows_notice(cx: &mut TestAppContext) {
    let t = edit_test("change-single", cx);
    *lock(&t.server.replica_sets) = vec![replica_set("api-a", 1)];
    let before = notice_count(&t, cx);
    click_change(&t, None, cx);
    t.t.wait_for("the list", cx, |_| {
        !replica_set_lists_of(&t.t.stg_api).is_empty()
    });
    cx.run_until_parked();
    assert_eq!(notice_count(&t, cx), before + 1);
    assert!(!has_dialog_open(&t, cx));
    assert!(template_reads(&t.t.stg_api).is_empty());
}

#[gpui_kit::test]
fn go_to_deployment_reveals_and_closes(cx: &mut TestAppContext) {
    let t = edit_test("change-go-to", cx);
    click_change(&t, None, cx);
    wait_for_template_reads(&t, 2, cx);
    assert!(has_dialog_open(&t, cx));
    assert!(!t.shell().read_with(cx, |shell, _| shell.drawer.is_open));
    // The footer button runs `go_to_deployment` of the view the dialog holds; a twin view with the
    // same key and shell does the same to the dialog that is open.
    let connection = t
        .shell()
        .read_with(cx, |shell, cx| shell.edit_connection(&t.t.stg, cx))
        .expect("a live connection");
    let request = crate::revision_diff::diff_request(
        deployment_key(),
        crate::revision_diff::RevisionSide {
            replica_set: "api-a".to_owned(),
            revision: Some(1),
            tag: None,
            is_current: false,
            created_at: None,
            change_cause: None,
        },
        crate::revision_diff::RevisionSide {
            replica_set: "api-b".to_owned(),
            revision: Some(2),
            tag: None,
            is_current: true,
            created_at: None,
            change_cause: None,
        },
    );
    let shell = t.shell().downgrade();
    let view = t.t.fixture.with_window(cx, |_, cx| {
        cx.new(|cx| {
            crate::revision_diff::RevisionDiffView::new(request, connection, cx)
                .with_go_to(deployment_key(), shell)
        })
    });
    t.t.fixture.with_window(cx, |window, cx| {
        view.update(cx, |view, cx| view.go_to_deployment_for_test(window, cx));
    });
    assert!(!has_dialog_open(&t, cx));
    assert!(t.shell().read_with(cx, |shell, _| shell.drawer.is_open));
}

#[gpui_kit::test]
fn history_tab_asks_again_after_a_failed_list(cx: &mut TestAppContext) {
    let t = edit_test("edit-history-retry", cx);
    t.open(cx);
    t.server.fail_lists.store(true, Ordering::SeqCst);
    let view = t.view(cx);
    let has_failed = |cx: &mut TestAppContext| {
        view.read_with(cx, |view, cx| {
            view.history()
                .is_some_and(|history| history.read(cx).has_failed())
        })
    };
    view.update(cx, |view, cx| view.show_tab(EditTab::History, cx));
    t.t.wait_for("the failed list", cx, |cx| has_failed(cx));
    // The list works again: leaving the tab and coming back asks again.
    t.server.fail_lists.store(false, Ordering::SeqCst);
    view.update(cx, |view, cx| view.show_tab(EditTab::Editor, cx));
    view.update(cx, |view, cx| view.show_tab(EditTab::History, cx));
    t.t.wait_for("the revisions", cx, |cx| {
        view.read_with(cx, |view, cx| {
            view.history()
                .is_some_and(|history| history.read(cx).is_ready())
        })
    });
    assert_eq!(replica_set_lists_of(&t.t.stg_api).len(), 2);
}

#[gpui_kit::test]
fn a_missing_quota_feed_is_off_not_loading(cx: &mut TestAppContext) {
    let t = edit_test("edit-quota-missing", cx);
    t.t.fixture.session(cx).update(cx, |session, _| {
        session.drop_condition_feeds_for_test();
    });
    let input = t
        .shell()
        .read_with(cx, |shell, cx| shell.quota_input(&t.t.stg, "team-a", cx));
    assert!(matches!(
        input,
        crate::edit_quota::QuotaInput::Off(reason) if reason == "quotas are not watched"
    ));
}

#[gpui_kit::test]
fn closing_the_main_window_releases_the_open_editor(cx: &mut TestAppContext) {
    let t = edit_test("edit-window-close", cx);
    t.open(cx);
    assert!(t.edit(cx).is_some());
    t.t.fixture
        .with_window(cx, |window, _| window.remove_window());
    cx.run_until_parked();
    // The editor's inputs must not outlive the window: a debug build reports them at exit.
    assert!(t.shell().read_with(cx, |shell, _| shell.edit.is_none()));
}

// ---- the Diff tab and the confirm (UX round 3, O16 and O17) ----

impl EditTest {
    fn show_tab(&self, tab: EditTab, cx: &mut TestAppContext) {
        self.view(cx).update(cx, |view, cx| view.show_tab(tab, cx));
    }
}

#[gpui_kit::test]
fn opening_the_diff_runs_the_dry_run_without_ctrl_s(cx: &mut TestAppContext) {
    let t = edit_test("edit-diff-opens", cx);
    t.open(cx);
    t.change("replicas: 3", "replicas: 5", cx);
    assert!(t.puts().is_empty());
    t.show_tab(EditTab::Diff, cx);
    t.wait_for_preview(cx);
    let dry_runs = t.puts();
    assert_eq!(dry_runs.len(), 1, "{dry_runs:?}");
    assert!(dry_runs[0].has_query("dryRun", "All"));
    assert!(t.is_passed(cx));
    assert_eq!(t.with_view(cx, |view| view.tab()), EditTab::Diff);
}

#[gpui_kit::test]
fn the_diff_asks_again_only_for_a_text_no_check_covers(cx: &mut TestAppContext) {
    let t = edit_test("edit-diff-once", cx);
    t.open(cx);
    // An unchanged text has nothing to check.
    t.show_tab(EditTab::Diff, cx);
    assert!(t.puts().is_empty());
    t.change("replicas: 3", "replicas: 5", cx);
    t.show_tab(EditTab::Diff, cx);
    t.wait_for_preview(cx);
    // Back and forth over the same text: the passed check stands.
    t.show_tab(EditTab::Editor, cx);
    t.show_tab(EditTab::Diff, cx);
    t.wait_for_preview(cx);
    assert_eq!(t.puts().len(), 1);
    // A new text is a new question.
    t.change("replicas: 5", "replicas: 6", cx);
    t.show_tab(EditTab::Editor, cx);
    t.show_tab(EditTab::Diff, cx);
    t.wait_for_preview(cx);
    assert_eq!(t.puts().len(), 2);
}

#[gpui_kit::test]
fn ctrl_s_after_the_diff_opened_goes_straight_to_the_confirm(cx: &mut TestAppContext) {
    let t = edit_test("edit-diff-then-confirm", cx);
    t.open(cx);
    t.change("replicas: 3", "replicas: 5", cx);
    t.show_tab(EditTab::Diff, cx);
    t.wait_for_preview(cx);
    assert!(!t.t.has_dialog(cx));
    t.t.fixture.press("ctrl-s", cx);
    cx.run_until_parked();
    assert!(t.t.has_dialog(cx), "the one Ctrl S opens the confirm");
    t.t.wait_for_dry_run(cx);
    // The editor's check and the dialog's own.
    assert_eq!(t.puts().len(), 2);
}

#[gpui_kit::test]
fn the_confirm_lists_old_and_new_for_each_scalar_and_the_helm_warning(cx: &mut TestAppContext) {
    let t = edit_test("edit-confirm-lines", cx);
    {
        let mut object = lock(&t.server.object);
        object["metadata"]["labels"]["app.kubernetes.io/managed-by"] = json!("Helm");
    }
    t.open(cx);
    t.change("replicas: 3", "replicas: 5", cx);
    t.change("image: api:1", "image: api:2", cx);
    t.apply(cx);
    t.wait_for_preview(cx);
    t.apply(cx);
    cx.run_until_parked();
    let dialog = t.t.dialog(cx);
    let lines = dialog.read_with(cx, |dialog, _| dialog.change_lines());
    let lines: Vec<&str> = lines.iter().map(AsRef::as_ref).collect();
    assert!(lines.contains(&"spec.replicas: 3 → 5"), "{lines:?}");
    assert!(
        lines.contains(&"spec.template.spec.containers[api].image: api:1 → api:2"),
        "{lines:?}"
    );
    let warnings = dialog.read_with(cx, |dialog, _| dialog.warning_lines());
    assert_eq!(
        warnings.first().map(AsRef::as_ref),
        Some(cluster::HELM_MANAGED_WARNING)
    );
    // The values in the dialog are the masked ones of the preview: the env literal stays out.
    assert!(lines.iter().all(|line| !line.contains("s3cr3t-env")));
}

#[gpui_kit::test]
fn apply_is_off_after_a_secret_value_is_refused_and_on_again_after_an_edit(
    cx: &mut TestAppContext,
) {
    let t = edit_test("edit-secret-refused", cx);
    t.open(cx);
    t.change("replicas: 3", "replicas: 5", cx);
    assert_eq!(t.with_view(cx, YamlEditView::apply_block_reason), None);
    t.view(cx)
        .update(cx, |view, _| view.refuse_secret_values_for_test());
    assert_eq!(
        t.with_view(cx, YamlEditView::apply_block_reason),
        Some("Secret values are changed with Edit values")
    );
    // Apply sends nothing while the refusal stands.
    t.apply(cx);
    assert!(!t.is_passed(cx));
    // Any change of the text is a new text: the refusal belonged to the old one.
    t.change("replicas: 5", "replicas: 6", cx);
    assert_eq!(t.with_view(cx, YamlEditView::apply_block_reason), None);
}
