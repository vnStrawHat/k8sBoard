//! New from templates (spec 0042) in a headless window over `stg-b`, an unlocked Staging cluster that
//! confirms with a click. The cluster answers from a fake API server, so a test sees every request,
//! and nothing leaves the machine. No test sets `K8SBOARD_ALLOW_WRITES`: the fake connection is
//! built with an allowing policy of its own.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use cluster::ConfigMapSummary;
use cluster::fake_api::RecordedRequest;
use gpui_kit::InputEvent as _;
use gpui_kit::component::dialog::Confirm;
use gpui_kit::{Entity, KeyDownEvent, Keystroke, TestAppContext};
use serde_json::{Value, json};

use super::app_shell_switch_tests::open_switch_fixture;
use super::app_shell_write_tests::{Clusters, audit_lines, go_live_answering, switch_to, writes};
use super::*;
use crate::kind_access::KindAccess;
use crate::kind_row::KindRow;
use crate::object_create_view::{CreateCheck, CreateFailure, ObjectCreateView};
use crate::write_guard::{DialogConfirm, WriteLock};

const CONFIG_MAPS: &str = "/api/v1/namespaces/default/configmaps";
const NOT_FOUND: &str = r#"{"kind":"Status","apiVersion":"v1","status":"Failure","message":"no","reason":"NotFound","code":404}"#;
const TOO_MANY: &str = r#"{"kind":"Status","apiVersion":"v1","status":"Failure","message":"slow down","reason":"TooManyRequests","code":429}"#;
const EXISTS: &str = r#"{"kind":"Status","apiVersion":"v1","status":"Failure","message":"configmaps \"new-config\" already exists","reason":"AlreadyExists","code":409}"#;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn access_review(is_allowed: bool) -> String {
    format!(
        r#"{{"apiVersion":"authorization.k8s.io/v1","kind":"SelfSubjectAccessReview","metadata":{{}},"spec":{{}},"status":{{"allowed":{is_allowed}}}}}"#
    )
}

/// What the fake API server knows: whether `create` is allowed, and what a dry-run or a commit
/// `POST` answers instead of an echo of the body.
struct CreateServer {
    may_create: AtomicBool,
    dry_run_answer: Mutex<Option<(u16, String)>>,
    commit_answer: Mutex<Option<(u16, String)>>,
}

impl CreateServer {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            may_create: AtomicBool::new(true),
            dry_run_answer: Mutex::new(None),
            commit_answer: Mutex::new(None),
        })
    }

    fn answer(&self, request: &RecordedRequest) -> (u16, String) {
        if request.method != "POST" {
            return (404, NOT_FOUND.to_owned());
        }
        if request.path.ends_with("/selfsubjectaccessreviews") {
            let is_create_check = request.body.contains("\"verb\":\"create\"");
            let is_allowed = !is_create_check || self.may_create.load(Ordering::SeqCst);
            return (201, access_review(is_allowed));
        }
        let slot = if request.has_query_key("dryRun") {
            &self.dry_run_answer
        } else {
            &self.commit_answer
        };
        if let Some(answer) = lock(slot).clone() {
            return answer;
        }
        // The echo of the body with the metadata the server adds.
        let mut body: Value = serde_json::from_str(&request.body).unwrap_or(Value::Null);
        body["metadata"]["uid"] = json!("uid-1");
        (201, body.to_string())
    }
}

fn config_map_rows() -> Vec<KindRow> {
    let existing = ConfigMapSummary {
        namespace: "default".to_owned(),
        name: "existing".to_owned(),
        created_at: None,
        labels: Vec::new(),
        keys: Vec::new(),
        is_immutable: false,
    };
    vec![crate::config_map_rows::config_map_row(&existing)]
}

struct CreateTest {
    t: Clusters,
    server: Arc<CreateServer>,
}

fn create_test(name: &str, cx: &mut TestAppContext) -> CreateTest {
    // Dialogs open without their animation, so the confirm button takes input at once.
    cx.update(|cx| cx.set_reduce_motion(true));
    let fixture = open_switch_fixture(name, cx);
    switch_to(&fixture, "stg-b", cx);
    let (prod, stg) = (fixture.cluster("prod-a", cx), fixture.cluster("stg-b", cx));
    let server = CreateServer::new();
    let respond = {
        let server = Arc::clone(&server);
        move |request: &RecordedRequest| server.answer(request)
    };
    let stg_api = go_live_answering(&fixture, &stg, "node-b", respond, cx);
    let t = Clusters {
        fixture,
        stg_api,
        prod,
        stg,
    };
    CreateTest { t, server }
}

impl CreateTest {
    fn shell(&self) -> &Entity<AppShell> {
        &self.t.fixture.shell
    }

    /// Shows the screen of `kind` and waits for the lazy permission review of the kind.
    fn show(&self, kind: ResourceKind, rows: Vec<KindRow>, cx: &mut TestAppContext) {
        self.t.show_kind(kind, rows.clone(), rows, cx);
        let object = kind.builtin_object().expect("a built-in kind");
        self.t.wait_for("the lazy review", cx, |cx| {
            self.shell().read_with(cx, |shell, cx| {
                shell.guard_for(&self.t.stg, cx).is_some_and(|guard| {
                    matches!(
                        guard.kind_access.get(object),
                        Some(KindAccess::Known(_) | KindAccess::Unknown)
                    )
                })
            })
        });
    }

    fn open(&self, kind: cluster::ObjectKind, cx: &mut TestAppContext) {
        self.t.fixture.with_window(cx, |window, cx| {
            self.shell()
                .update(cx, |shell, cx| shell.open_create(kind, window, cx));
        });
        cx.run_until_parked();
    }

    fn edit(&self, cx: &mut TestAppContext) -> Option<Entity<ObjectCreateView>> {
        self.shell().read_with(cx, |shell, _| {
            shell.edit.as_ref().and_then(OpenEdit::create)
        })
    }

    fn view(&self, cx: &mut TestAppContext) -> Entity<ObjectCreateView> {
        self.edit(cx).expect("an open New view")
    }

    fn text(&self, cx: &mut TestAppContext) -> String {
        self.view(cx)
            .read_with(cx, |view, cx| view.text_for_test(cx).to_string())
    }

    fn set_text(&self, text: &str, cx: &mut TestAppContext) {
        let view = self.view(cx);
        self.t.fixture.with_window(cx, |window, cx| {
            view.update(cx, |view, cx| view.set_text_for_test(text, window, cx));
        });
    }

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

    fn wait_for_check(&self, cx: &mut TestAppContext) {
        self.t.wait_for("the dry-run", cx, |cx| {
            self.view(cx).read_with(cx, |view, _| {
                !matches!(view.check(), CreateCheck::Running { .. })
            })
        });
    }

    fn is_passed(&self, cx: &mut TestAppContext) -> bool {
        self.view(cx)
            .read_with(cx, |view, _| matches!(view.check(), CreateCheck::Passed(_)))
    }

    /// Checks the text, then presses `Create…` for the dialog.
    fn check_then_confirm(&self, cx: &mut TestAppContext) {
        self.apply(cx);
        self.wait_for_check(cx);
        assert!(self.is_passed(cx), "the dry-run passed");
        self.apply(cx);
        self.t.wait_for_dry_run(cx);
    }

    /// The POSTs to a collection (the dry-runs first), not the access reviews.
    fn posts(&self) -> Vec<RecordedRequest> {
        writes(&self.t.stg_api)
    }

    fn footer(&self, cx: &mut TestAppContext) -> String {
        self.view(cx).read_with(cx, |view, cx| {
            crate::object_create_view::footer_text(view.check(), &view.text_for_test(cx))
        })
    }

    fn press_event(&self, event: KeyDownEvent, cx: &mut TestAppContext) {
        self.t.fixture.with_window(cx, |window, cx| {
            window.dispatch_event(event.to_platform_input(), cx);
        });
    }
}

fn key_down(key: &str, is_held: bool) -> KeyDownEvent {
    KeyDownEvent {
        keystroke: Keystroke::parse(key).expect("a valid keystroke"),
        is_held,
        prefer_character_input: false,
    }
}

const CLUSTER_ADMIN_BINDING: &str = "apiVersion: rbac.authorization.k8s.io/v1\nkind: RoleBinding\nmetadata:\n  name: new-binding\n  namespace: default\nroleRef:\n  apiGroup: rbac.authorization.k8s.io\n  kind: ClusterRole\n  name: cluster-admin\nsubjects:\n  - kind: ServiceAccount\n    name: default\n    namespace: default\n";

// ---- the gate of the button ----

#[gpui_kit::test]
fn new_button_gate_reads_permissions_then_the_lock(cx: &mut TestAppContext) {
    let t = create_test("create-gate", cx);
    t.show(ResourceKind::ConfigMaps, Vec::new(), cx);
    let block = |cx: &mut TestAppContext| {
        t.shell().read_with(cx, |shell, cx| {
            shell
                .new_object_block(cluster::ObjectKind::ConfigMap, cx)
                .map(|reason| reason.to_string())
        })
    };
    assert_eq!(block(cx), None);
    t.t.set_lock(&t.t.stg, WriteLock::Locked, cx);
    assert_eq!(block(cx).as_deref(), Some("stg-b is read-only"));
    t.t.set_lock(&t.t.stg, WriteLock::Unlocked, cx);
    // A denial arrives with the next scope.
    t.server.may_create.store(false, Ordering::SeqCst);
    t.shell().update(cx, |shell, cx| {
        shell.set_namespace(NamespaceScope::Named("team-a".to_owned()), cx)
    });
    t.t.wait_for("the denial", cx, |cx| {
        block(cx).as_deref() == Some("Not permitted: create configmaps")
    });
    // The denied button opens nothing.
    t.open(cluster::ObjectKind::ConfigMap, cx);
    assert!(t.edit(cx).is_none());
}

// ---- opening ----

#[gpui_kit::test]
fn open_create_shows_the_template_of_the_first_scoped_namespace(cx: &mut TestAppContext) {
    let t = create_test("create-open", cx);
    t.show(ResourceKind::ConfigMaps, Vec::new(), cx);
    t.open(cluster::ObjectKind::ConfigMap, cx);
    let text = t.text(cx);
    assert!(text.contains("kind: ConfigMap"), "{text}");
    assert!(text.contains("namespace: default"), "{text}");
    assert!(!t.view(cx).read_with(cx, |view, _| view.is_dirty()));
    // A second New does nothing while one is open.
    t.open(cluster::ObjectKind::ConfigMap, cx);
    assert!(t.edit(cx).is_some());
    t.t.fixture.draw_twice(cx);
}

#[gpui_kit::test]
fn open_create_follows_the_scope_namespace(cx: &mut TestAppContext) {
    let t = create_test("create-scope", cx);
    t.shell().update(cx, |shell, cx| {
        shell.set_namespace(NamespaceScope::Named("team-a".to_owned()), cx)
    });
    t.show(ResourceKind::ConfigMaps, Vec::new(), cx);
    t.open(cluster::ObjectKind::ConfigMap, cx);
    assert!(t.text(cx).contains("namespace: team-a"));
}

#[gpui_kit::test]
fn open_create_slot_has_no_object(cx: &mut TestAppContext) {
    let t = create_test("create-slot", cx);
    t.show(ResourceKind::ConfigMaps, Vec::new(), cx);
    t.open(cluster::ObjectKind::ConfigMap, cx);
    t.shell().read_with(cx, |shell, cx| {
        let edit = shell.edit.as_ref().expect("an open edit");
        assert!(edit.object(cx).is_none());
        assert_eq!(edit.discard_title(cx), "Discard the new ConfigMap?");
        assert_eq!(edit.leaving_line(cx), "Unsaved new ConfigMap");
        assert_eq!(edit.subject_text(cx), "new ConfigMap");
    });
}

// ---- the dry-run ----

#[gpui_kit::test]
fn local_error_blocks_before_request(cx: &mut TestAppContext) {
    let t = create_test("create-local", cx);
    t.show(ResourceKind::ConfigMaps, Vec::new(), cx);
    t.open(cluster::ObjectKind::ConfigMap, cx);
    t.change("name: new-config", "generateName: new-", cx);
    t.apply(cx);
    cx.run_until_parked();
    t.view(cx).read_with(cx, |view, _| {
        assert!(matches!(
            view.check(),
            CreateCheck::Failed(CreateFailure::Local { .. })
        ));
    });
    assert!(t.footer(cx).contains("metadata.generateName"));
    assert!(t.posts().is_empty());
}

const KUBECTL_CONFIG_MAP: &str = "\
apiVersion: v1
kind: ConfigMap
metadata:
  name: pasted
  namespace: team-a
  uid: 5b2f
  resourceVersion: \"881\"
  creationTimestamp: \"2026-10-01T08:00:00Z\"
data:
  K: v
status: {}
";

#[gpui_kit::test]
fn a_kubectl_paste_lists_its_server_fields_and_one_click_strips_them(cx: &mut TestAppContext) {
    let t = create_test("create-server-fields", cx);
    t.show(ResourceKind::ConfigMaps, Vec::new(), cx);
    t.open(cluster::ObjectKind::ConfigMap, cx);
    t.set_text(KUBECTL_CONFIG_MAP, cx);
    t.apply(cx);
    cx.run_until_parked();
    let fix = t.view(cx).read_with(cx, |view, _| match view.check() {
        CreateCheck::Failed(CreateFailure::Local { message, fix }) => {
            assert!(
                message.starts_with("4 fields are set by the server: status, "),
                "{message}"
            );
            *fix
        }
        _ => panic!("expected a local failure"),
    });
    assert_eq!(fix, Some(cluster::DraftFix::RemoveServerFields));
    assert!(t.posts().is_empty());
    let view = t.view(cx);
    t.t.fixture.with_window(cx, |window, cx| {
        view.update(cx, |view, cx| {
            view.apply_fix(cluster::DraftFix::RemoveServerFields, window, cx)
        });
    });
    let text = t.text(cx);
    assert!(!text.contains("uid") && !text.contains("status"), "{text}");
    assert!(text.contains("name: pasted"));
    assert!(t.view(cx).read_with(cx, |view, _| matches!(
        view.check(),
        CreateCheck::NotChecked
    )));
}

#[gpui_kit::test]
fn several_pasted_documents_say_how_many_and_the_first_can_be_kept(cx: &mut TestAppContext) {
    let t = create_test("create-several-documents", cx);
    t.show(ResourceKind::ConfigMaps, Vec::new(), cx);
    t.open(cluster::ObjectKind::ConfigMap, cx);
    let second = KUBECTL_CONFIG_MAP.replace("pasted", "second");
    t.set_text(&format!("{KUBECTL_CONFIG_MAP}---\n{second}"), cx);
    t.apply(cx);
    cx.run_until_parked();
    assert_eq!(t.footer(cx), "Found 2 documents; paste one");
    let view = t.view(cx);
    t.t.fixture.with_window(cx, |window, cx| {
        view.update(cx, |view, cx| {
            view.apply_fix(cluster::DraftFix::KeepFirstDocument, window, cx)
        });
    });
    assert_eq!(t.text(cx), KUBECTL_CONFIG_MAP);
}

#[gpui_kit::test]
fn ctrl_s_during_dry_run_does_nothing(cx: &mut TestAppContext) {
    let t = create_test("create-twice", cx);
    t.show(ResourceKind::ConfigMaps, Vec::new(), cx);
    t.open(cluster::ObjectKind::ConfigMap, cx);
    t.apply(cx);
    // The first check is still running: this press does not start another or open the dialog.
    t.apply(cx);
    t.wait_for_check(cx);
    assert_eq!(t.posts().len(), 1);
    assert!(!t.t.has_dialog(cx));
}

#[gpui_kit::test]
fn changed_text_needs_new_dry_run(cx: &mut TestAppContext) {
    let t = create_test("create-stale", cx);
    t.show(ResourceKind::ConfigMaps, Vec::new(), cx);
    t.open(cluster::ObjectKind::ConfigMap, cx);
    t.apply(cx);
    t.wait_for_check(cx);
    assert!(t.is_passed(cx));
    t.change("KEY: value", "KEY: other", cx);
    assert_eq!(t.footer(cx), "Changed since the last check");
    t.apply(cx);
    t.wait_for_check(cx);
    assert!(!t.t.has_dialog(cx));
    assert_eq!(t.posts().len(), 2);
    assert!(t.is_passed(cx));
}

#[gpui_kit::test]
fn the_dry_run_is_a_post_of_the_collection(cx: &mut TestAppContext) {
    let t = create_test("create-request", cx);
    t.show(ResourceKind::ConfigMaps, Vec::new(), cx);
    t.open(cluster::ObjectKind::ConfigMap, cx);
    t.apply(cx);
    t.wait_for_check(cx);
    let posts = t.posts();
    assert_eq!(posts[0].method, "POST");
    assert_eq!(posts[0].path, CONFIG_MAPS);
    assert!(posts[0].has_query("dryRun", "All"));
    assert!(posts[0].has_query("fieldManager", "k8sboard"));
}

#[gpui_kit::test]
fn a_taken_name_reads_as_invalid_and_blocks(cx: &mut TestAppContext) {
    let t = create_test("create-exists", cx);
    *lock(&t.server.dry_run_answer) = Some((409, EXISTS.to_owned()));
    t.show(ResourceKind::ConfigMaps, Vec::new(), cx);
    t.open(cluster::ObjectKind::ConfigMap, cx);
    t.apply(cx);
    t.wait_for_check(cx);
    t.view(cx).read_with(cx, |view, _| match view.check() {
        CreateCheck::Failed(CreateFailure::Invalid { message, fields }) => {
            assert_eq!(message.as_ref(), "ConfigMap new-config already exists");
            assert_eq!(fields.len(), 1);
            assert_eq!(fields[0].as_ref(), "metadata.name");
        }
        _ => panic!("a name clash is invalid"),
    });
    assert!(!t.t.has_dialog(cx));
}

// ---- the confirm ----

#[gpui_kit::test]
fn passed_dry_run_opens_confirm_with_warnings(cx: &mut TestAppContext) {
    let t = create_test("create-confirm", cx);
    // The server drops the whole `data` map from its answer.
    let answer = json!({
        "apiVersion": "v1", "kind": "ConfigMap",
        "metadata": {"name": "new-config", "namespace": "default", "uid": "u"},
    });
    *lock(&t.server.dry_run_answer) = Some((201, answer.to_string()));
    t.show(ResourceKind::ConfigMaps, Vec::new(), cx);
    t.open(cluster::ObjectKind::ConfigMap, cx);
    t.apply(cx);
    t.wait_for_check(cx);
    t.apply(cx);
    t.t.wait_for_dry_run(cx);
    let dialog = t.t.dialog(cx);
    dialog.read_with(cx, |dialog, _| {
        assert_eq!(
            dialog.label().as_deref(),
            Some("Create ConfigMap default/new-config")
        );
        assert_eq!(*dialog.tier(), DialogConfirm::Click);
        let lines: Vec<String> = dialog
            .warning_lines()
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(
            lines,
            ["data.KEY is not a known field; the server dropped it"]
        );
    });
}

#[gpui_kit::test]
fn risky_binding_types_its_name_everywhere(cx: &mut TestAppContext) {
    let t = create_test("create-risky", cx);
    t.show(ResourceKind::RoleBindings, Vec::new(), cx);
    t.open(cluster::ObjectKind::RoleBinding, cx);
    t.set_text(CLUSTER_ADMIN_BINDING, cx);
    t.check_then_confirm(cx);
    let dialog = t.t.dialog(cx);
    dialog.read_with(cx, |dialog, _| {
        // Staging confirms with a click for any other change; this one types the binding name.
        assert_eq!(
            *dialog.tier(),
            DialogConfirm::TypeName {
                expected: "new-binding".to_owned()
            }
        );
        let lines: Vec<String> = dialog
            .warning_lines()
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(lines[0].contains("cluster-admin"), "{lines:?}");
    });
    // Without the name nothing is committed.
    t.t.confirm(cx);
    cx.run_until_parked();
    assert_eq!(t.posts().len(), 2, "two dry-runs, no commit");
}

#[gpui_kit::test]
fn a_plain_binding_keeps_the_cluster_tier(cx: &mut TestAppContext) {
    let t = create_test("create-plain", cx);
    t.show(ResourceKind::RoleBindings, Vec::new(), cx);
    t.open(cluster::ObjectKind::RoleBinding, cx);
    t.check_then_confirm(cx);
    t.t.dialog(cx).read_with(cx, |dialog, _| {
        assert_eq!(*dialog.tier(), DialogConfirm::Click);
        assert!(dialog.warning_lines().is_empty());
    });
}

#[gpui_kit::test]
fn a_held_ctrl_s_never_opens_the_confirm_dialog(cx: &mut TestAppContext) {
    let t = create_test("create-held", cx);
    t.show(ResourceKind::ConfigMaps, Vec::new(), cx);
    t.open(cluster::ObjectKind::ConfigMap, cx);
    t.press_event(key_down("ctrl-s", false), cx);
    t.wait_for_check(cx);
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

// ---- the commit ----

#[gpui_kit::test]
fn commit_closes_and_shows_the_kind_screen(cx: &mut TestAppContext) {
    let t = create_test("create-commit", cx);
    let dir = t.t.enable_audit_folder("create-commit", cx);
    t.show(ResourceKind::ConfigMaps, config_map_rows(), cx);
    let existing = ClusterObject::new(
        t.t.stg.clone(),
        ResourceKey::Kind {
            kind: ResourceKind::ConfigMaps,
            namespace: Some("default".to_owned()),
            name: "existing".to_owned(),
        },
    );
    t.shell().update(cx, |shell, cx| {
        shell.change_selection(Some(existing.clone()), cx)
    });
    t.open(cluster::ObjectKind::ConfigMap, cx);
    t.change("KEY: value", "PASSWORD: S3cr3t-0042", cx);
    t.check_then_confirm(cx);
    t.t.confirm(cx);
    t.t.wait_for("the commit", cx, |_| t.posts().len() == 3);
    t.t.wait_for("the view to close", cx, |cx| t.edit(cx).is_none());
    let commit = &t.posts()[2];
    assert!(!commit.has_query_key("dryRun"));
    assert!(commit.has_query("fieldManager", "k8sboard"));
    assert_eq!(commit.path, CONFIG_MAPS);
    // The kind's screen, with the cursor where it was.
    t.shell().read_with(cx, |shell, _| {
        assert_eq!(shell.screen, Screen::Kind(ResourceKind::ConfigMaps));
        assert_eq!(shell.selected.as_ref(), Some(&existing));
    });
    // One audit line: the action, the names, and no value.
    t.t.wait_for("the audit line", cx, |_| audit_lines(&dir).len() == 1);
    let lines = audit_lines(&dir);
    assert_eq!(lines[0]["action"], json!("Create"));
    assert_eq!(lines[0]["outcome"], json!("applied"));
    assert_eq!(lines[0]["object"]["kind"], json!("ConfigMap"));
    let paths: Vec<_> = lines[0]["fields"]
        .as_array()
        .expect("fields")
        .iter()
        .filter_map(|field| field["path"].as_str())
        .collect();
    assert!(paths.contains(&"data[PASSWORD]"), "{paths:?}");
    let raw = std::fs::read_to_string(crate::audit_log::audit_path(&dir)).unwrap_or_default();
    assert!(
        !raw.contains("S3cr3t-0042"),
        "a value reached the audit log"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn failed_create_offers_no_dialog_retry(cx: &mut TestAppContext) {
    let t = create_test("create-no-retry", cx);
    *lock(&t.server.commit_answer) = Some((429, TOO_MANY.to_owned()));
    t.show(ResourceKind::ConfigMaps, Vec::new(), cx);
    t.open(cluster::ObjectKind::ConfigMap, cx);
    t.check_then_confirm(cx);
    t.t.confirm(cx);
    t.t.wait_for("the commit", cx, |_| t.posts().len() == 3);
    // The dialog closes instead of offering Retry, and the view shows the refusal.
    t.t.wait_for("the dialog to close", cx, |cx| !t.t.has_dialog(cx));
    assert!(t.edit(cx).is_some(), "the text is kept");
    assert!(t.footer(cx).contains("slow down"), "{}", t.footer(cx));
    assert_eq!(t.posts().len(), 3, "no retry");
}

#[gpui_kit::test]
fn outcome_unknown_keeps_view_without_retry(cx: &mut TestAppContext) {
    let t = create_test("create-unknown", cx);
    t.show(ResourceKind::ConfigMaps, Vec::new(), cx);
    t.open(cluster::ObjectKind::ConfigMap, cx);
    let view = t.view(cx);
    view.update(cx, |view, cx| {
        view.commit_failed(crate::yaml_edit::EditFailure::OutcomeUnknown, cx)
    });
    assert_eq!(
        t.footer(cx),
        "No answer in time; the object may have been created. Check the list before trying again."
    );
    assert!(t.edit(cx).is_some());
    assert!(!t.t.has_dialog(cx));
}

// ---- leaving ----

#[gpui_kit::test]
fn dirty_cancel_asks_clean_cancel_closes(cx: &mut TestAppContext) {
    let t = create_test("create-cancel", cx);
    t.show(ResourceKind::ConfigMaps, Vec::new(), cx);
    t.open(cluster::ObjectKind::ConfigMap, cx);
    let view = t.view(cx);
    // An unchanged template closes without a question.
    view.update(cx, |view, cx| view.cancel(cx));
    cx.run_until_parked();
    assert!(t.edit(cx).is_none());
    t.open(cluster::ObjectKind::ConfigMap, cx);
    t.change("KEY: value", "KEY: other", cx);
    let view = t.view(cx);
    view.update(cx, |view, cx| view.cancel(cx));
    cx.run_until_parked();
    assert!(t.edit(cx).is_some(), "the question is open");
    assert_eq!(
        t.shell()
            .read_with(cx, |shell, _| shell.last_discard.clone())
            .as_deref(),
        Some("new ConfigMap")
    );
    t.t.fixture.with_window(cx, |window, cx| {
        window.dispatch_action(Box::new(Confirm { secondary: false }), cx);
    });
    cx.run_until_parked();
    assert!(t.edit(cx).is_none());
}

#[gpui_kit::test]
fn a_screen_change_with_a_changed_template_asks_to_discard(cx: &mut TestAppContext) {
    let t = create_test("create-nav", cx);
    t.show(ResourceKind::ConfigMaps, Vec::new(), cx);
    t.open(cluster::ObjectKind::ConfigMap, cx);
    t.change("KEY: value", "KEY: other", cx);
    t.shell().update(cx, |shell, cx| {
        shell.show_screen(Screen::Kind(ResourceKind::Services), cx)
    });
    cx.run_until_parked();
    assert!(t.edit(cx).is_some());
    assert_eq!(
        t.shell().read_with(cx, |shell, _| shell.screen),
        Screen::Kind(ResourceKind::ConfigMaps)
    );
}

#[gpui_kit::test]
fn cluster_switch_lists_unsaved_new_object(cx: &mut TestAppContext) {
    let t = create_test("create-leaving", cx);
    t.show(ResourceKind::ConfigMaps, Vec::new(), cx);
    t.open(cluster::ObjectKind::ConfigMap, cx);
    let leaving = [t.t.stg.clone()];
    let clean = t
        .shell()
        .read_with(cx, |shell, cx| shell.leaving_work(&leaving, cx));
    assert_eq!(
        clean.unsaved_edit, None,
        "an unchanged template is not work"
    );
    t.change("KEY: value", "KEY: other", cx);
    let work = t
        .shell()
        .read_with(cx, |shell, cx| shell.leaving_work(&leaving, cx));
    assert_eq!(work.unsaved_edit.as_deref(), Some("Unsaved new ConfigMap"));
    assert_eq!(work.lines(), ["Unsaved new ConfigMap".to_owned()]);
}
