//! Edit values (spec 0047) in a headless window over one live cluster, `stg-b` (unlocked, a
//! Staging cluster that confirms with a click). The cluster answers from a fake API server, so a
//! test sees every request, and nothing leaves the machine. No test sets `K8SBOARD_ALLOW_WRITES`:
//! the fake connection is built with an allowing policy of its own.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use cluster::fake_api::{FakeApi, RecordedRequest};
use cluster::{ConfigMapSummary, ObjectKind, SecretDetails, SecretKey, SecretSummary, WritePolicy};
use gpui_kit::InputEvent as _;
use gpui_kit::component::dialog::{Cancel, Confirm};
use gpui_kit::{Entity, KeyDownEvent, Keystroke, TestAppContext};
use serde_json::{Value, json};

use super::app_shell_switch_tests::{SwitchFixture, chord, open_switch_fixture};
use super::app_shell_write_tests::{allowed, audit_lines, writes};
use super::values_edit_flow::values_success_notice;
use super::write_flow::DryRunState;
use super::*;
use crate::confirm_dialog::ConfirmDialog;
use crate::kind_access::KindAccess;
use crate::kind_row::KindRow;
use crate::resource_actions::{ActionAvailability, ResourceAction, RowAction, action_availability};
use crate::values_edit::{ValuesBanner, ValuesEditView};
use crate::write_guard::{DialogConfirm, WriteLock};

const SECRET_PATH: &str = "/api/v1/namespaces/team-a/secrets/api-db";
const CONFIG_MAP_PATH: &str = "/api/v1/namespaces/team-a/configmaps/api-config";
const SECRET_VALUE: &str = "S3cr3t-0047-ZZ";
const SECRET_BASE64: &str = "UzNjcjN0LTAwNDctWlo=";
const NOT_FOUND: &str = r#"{"kind":"Status","apiVersion":"v1","status":"Failure","message":"no","reason":"NotFound","code":404}"#;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn secret_object(resource_version: &str) -> Value {
    json!({
        "apiVersion": "v1", "kind": "Secret",
        "metadata": {"name": "api-db", "namespace": "team-a", "resourceVersion": resource_version},
        "type": "Opaque",
        "data": {"DB_HOST": "ZGIuaW50ZXJuYWw=", "DB_PASSWORD": "b2xkLXNlY3JldA==", "ca.der": "//4A"},
    })
}

fn config_map_object(resource_version: &str) -> Value {
    json!({
        "apiVersion": "v1", "kind": "ConfigMap",
        "metadata": {"name": "api-config", "namespace": "team-a", "resourceVersion": resource_version},
        "data": {"mode": "fast"},
    })
}

fn access_review(is_allowed: bool) -> String {
    format!(
        r#"{{"apiVersion":"authorization.k8s.io/v1","kind":"SelfSubjectAccessReview","metadata":{{}},"spec":{{}},"status":{{"allowed":{is_allowed}}}}}"#
    )
}

/// What the fake API server knows: both objects, whether `patch` is allowed, and what a dry-run or a
/// commit `PATCH` answers instead of the object.
struct ValuesServer {
    secret: Mutex<Value>,
    config_map: Mutex<Value>,
    may_patch: AtomicBool,
    dry_run_answer: Mutex<Option<(u16, String)>>,
    commit_answer: Mutex<Option<(u16, String)>>,
}

impl ValuesServer {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            secret: Mutex::new(secret_object("100")),
            config_map: Mutex::new(config_map_object("200")),
            may_patch: AtomicBool::new(true),
            dry_run_answer: Mutex::new(None),
            commit_answer: Mutex::new(None),
        })
    }

    fn answer(&self, request: &RecordedRequest) -> (u16, String) {
        let path = request.path.as_str();
        let object = |path: &str| match path {
            SECRET_PATH => Some(lock(&self.secret).clone()),
            CONFIG_MAP_PATH => Some(lock(&self.config_map).clone()),
            _ => None,
        };
        match request.method.as_str() {
            "POST" if path.ends_with("/selfsubjectaccessreviews") => {
                let is_patch_check = request.body.contains("\"verb\":\"patch\"");
                let is_allowed = !is_patch_check || self.may_patch.load(Ordering::SeqCst);
                (201, access_review(is_allowed))
            }
            "GET" => object(path).map_or((404, NOT_FOUND.to_owned()), |object| {
                (200, object.to_string())
            }),
            "PATCH" => {
                let slot = if request.has_query_key("dryRun") {
                    &self.dry_run_answer
                } else {
                    &self.commit_answer
                };
                if let Some(answer) = lock(slot).clone() {
                    return answer;
                }
                object(path).map_or((404, NOT_FOUND.to_owned()), |object| {
                    (200, object.to_string())
                })
            }
            _ => (404, NOT_FOUND.to_owned()),
        }
    }
}

fn secret_summary(
    name: &str,
    secret_type: &str,
    labels: &[&str],
    is_immutable: bool,
) -> SecretSummary {
    SecretSummary {
        namespace: "team-a".to_owned(),
        name: name.to_owned(),
        created_at: None,
        labels: labels.iter().map(|label| (*label).to_owned()).collect(),
        secret_type: secret_type.to_owned(),
        keys: ["DB_HOST", "DB_PASSWORD", "ca.der"]
            .into_iter()
            .map(|key| SecretKey {
                name: key.to_owned(),
                size_bytes: 8,
                is_binary: key == "ca.der",
            })
            .collect(),
        details: if secret_type == "kubernetes.io/service-account-token" {
            SecretDetails::ServiceAccountToken { account: None }
        } else {
            SecretDetails::None
        },
        is_immutable,
        is_owned: false,
    }
}

fn config_map_summary(name: &str, labels: &[&str]) -> ConfigMapSummary {
    ConfigMapSummary {
        namespace: "team-a".to_owned(),
        name: name.to_owned(),
        created_at: None,
        labels: labels.iter().map(|label| (*label).to_owned()).collect(),
        keys: Vec::new(),
        is_immutable: false,
    }
}

fn secret_rows() -> Vec<KindRow> {
    [
        secret_summary("api-db", "Opaque", &[], false),
        secret_summary(
            "sh.helm.release.v1.api.v1",
            "helm.sh/release.v1",
            &[],
            false,
        ),
        secret_summary("owned-by-helm", "Opaque", &["owner=helm"], false),
        secret_summary("token", "kubernetes.io/service-account-token", &[], false),
        secret_summary("frozen", "Opaque", &[], true),
    ]
    .iter()
    .map(crate::secret_rows::secret_row)
    .collect()
}

fn config_map_rows() -> Vec<KindRow> {
    [
        config_map_summary("api-config", &[]),
        config_map_summary("api-release", &["owner=helm"]),
    ]
    .iter()
    .map(crate::config_map_rows::config_map_row)
    .collect()
}

struct ValuesTest {
    fixture: SwitchFixture,
    api: FakeApi,
    server: Arc<ValuesServer>,
    cluster: ClusterRef,
}

fn values_test(name: &str, cx: &mut TestAppContext) -> ValuesTest {
    values_test_on(name, "stg-b", cx)
}

/// The same over the cluster `context`: `prod-a` is the one the fixture opens on, locked.
fn values_test_on(name: &str, context: &str, cx: &mut TestAppContext) -> ValuesTest {
    // Dialogs open without their animation, so the confirm button takes input at once.
    cx.update(|cx| cx.set_reduce_motion(true));
    let fixture = open_switch_fixture(name, cx);
    // Staging confirms with a click and starts unlocked; production is locked and types a name.
    if context != "prod-a" {
        fixture.switch(context, cx);
        cx.run_until_parked();
    }
    let cluster = fixture.cluster(context, cx);
    let server = ValuesServer::new();
    let (connection, api) = {
        let _guard = fixture.runtime.enter();
        let server = Arc::clone(&server);
        FakeApi::connection(WritePolicy::Allowed, move |request| server.answer(request))
    };
    fixture.session(cx).update(cx, |session, cx| {
        session.go_live_for_test(connection, NamespaceScope::All, cx);
        session.set_access_for_test(allowed(), cx);
    });
    cx.run_until_parked();
    let t = ValuesTest {
        fixture,
        api,
        server,
        cluster,
    };
    t.show(ResourceKind::Secrets, secret_rows(), cx);
    t
}

impl ValuesTest {
    fn shell(&self) -> &Entity<AppShell> {
        &self.fixture.shell
    }

    /// Shows `kind` with `rows` loaded, and waits for the lazy review of its objects.
    fn show(&self, kind: ResourceKind, rows: Vec<KindRow>, cx: &mut TestAppContext) {
        self.shell()
            .update(cx, |shell, cx| shell.show_screen(Screen::Kind(kind), cx));
        cx.run_until_parked();
        self.fixture.session(cx).update(cx, |session, cx| {
            session.set_kind_rows_for_test(kind, rows, cx);
        });
        cx.run_until_parked();
        self.fixture.draw_twice(cx);
        let object = kind.builtin_object().expect("a built-in kind");
        self.wait_for("the lazy review", cx, |cx| {
            self.shell().read_with(cx, |shell, cx| {
                shell.guard_for(&self.cluster, cx).is_some_and(|guard| {
                    matches!(
                        guard.kind_access.get(object),
                        Some(KindAccess::Known(_) | KindAccess::Unknown)
                    )
                })
            })
        });
    }

    fn wait_for(
        &self,
        what: &str,
        cx: &mut TestAppContext,
        done: impl Fn(&mut TestAppContext) -> bool,
    ) {
        for _ in 0..500 {
            cx.run_until_parked();
            if done(cx) {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("timed out waiting for {what}");
    }

    fn cursor_on(&self, kind: ResourceKind, name: &str, cx: &mut TestAppContext) {
        let key = ResourceKey::Kind {
            kind,
            namespace: Some("team-a".to_owned()),
            name: name.to_owned(),
        };
        let object = ClusterObject::new(self.cluster.clone(), key);
        self.shell().update(cx, |shell, cx| {
            shell.change_selection(Some(object), cx);
        });
        cx.run_until_parked();
    }

    /// E on the cursor row, and the object read.
    fn open(&self, cx: &mut TestAppContext) {
        self.cursor_on(ResourceKind::Secrets, "api-db", cx);
        self.fixture.press("e", cx);
        self.wait_for_base(cx);
    }

    fn wait_for_base(&self, cx: &mut TestAppContext) {
        self.wait_for("the object to load", cx, |cx| {
            self.view(cx).is_some_and(|view| {
                view.read_with(cx, |view, _| {
                    view.is_loaded() || view.load_error().is_some()
                })
            })
        });
    }

    fn view(&self, cx: &mut TestAppContext) -> Option<Entity<ValuesEditView>> {
        self.shell().read_with(cx, |shell, _| {
            shell.edit.as_ref().and_then(OpenEdit::values)
        })
    }

    fn edit_view(&self, cx: &mut TestAppContext) -> Entity<ValuesEditView> {
        self.view(cx).expect("an open values editor")
    }

    fn is_editing(&self, cx: &mut TestAppContext) -> bool {
        self.shell().read_with(cx, |shell, _| shell.is_editing())
    }

    fn insert(&self, key: &str, text: &str, cx: &mut TestAppContext) {
        let view = self.edit_view(cx);
        let field = view
            .read_with(cx, |view, _| view.field_of(key))
            .expect("the key has a field");
        self.fixture.with_window(cx, |window, cx| {
            field.update(cx, |state, cx| state.insert(text.to_owned(), window, cx))
        });
    }

    fn apply(&self, cx: &mut TestAppContext) {
        let view = self.edit_view(cx);
        self.fixture.with_window(cx, |window, cx| {
            view.update(cx, |view, cx| view.apply(window, cx));
        });
    }

    fn dialog(&self, cx: &mut TestAppContext) -> Entity<ConfirmDialog> {
        self.shell()
            .read_with(cx, |shell, _| shell.last_dialog.clone())
            .and_then(|dialog| dialog.upgrade())
            .expect("a confirm dialog is open")
    }

    fn dialog_unchecked(&self, cx: &mut App) -> Entity<ConfirmDialog> {
        self.shell()
            .read(cx)
            .last_dialog
            .clone()
            .and_then(|dialog| dialog.upgrade())
            .expect("a confirm dialog is open")
    }

    fn press_dialog(&self, answer: impl gpui_kit::Action, cx: &mut TestAppContext) {
        self.fixture.with_window(cx, |window, cx| {
            window.dispatch_action(Box::new(answer), cx);
        });
    }

    fn has_dialog(&self, cx: &mut TestAppContext) -> bool {
        self.shell()
            .read_with(cx, |shell, _| shell.last_dialog.clone())
            .is_some_and(|dialog| dialog.upgrade().is_some())
    }

    fn wait_for_dry_run(&self, cx: &mut TestAppContext) {
        self.wait_for("the dry-run", cx, |cx| {
            let dialog = self.dialog(cx);
            !matches!(
                dialog.read_with(cx, |dialog, _| dialog.dry_run_state()),
                Some(DryRunState::Running)
            )
        });
    }

    fn confirm(&self, cx: &mut TestAppContext) {
        let dialog = self.dialog(cx);
        self.fixture.with_window(cx, |window, cx| {
            dialog.update(cx, |dialog, cx| dialog.press_confirm(window, cx));
        });
    }

    /// The `PATCH`es the server received, dry-runs first.
    fn patches(&self) -> Vec<RecordedRequest> {
        writes(&self.api)
            .into_iter()
            .filter(|request| request.method == "PATCH")
            .collect()
    }

    fn gets(&self, path: &str) -> usize {
        self.api
            .requests()
            .iter()
            .filter(|request| request.method == "GET" && request.path == path)
            .count()
    }

    /// Settings that save into a folder of their own, so the audit log has one.
    fn audit_folder(&self, name: &str, cx: &mut TestAppContext) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("k8sboard-0047-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create the temp dir");
        cx.update(|cx| {
            AppSettings::install(
                crate::settings_store::LoadedSettings {
                    settings: crate::settings::Settings::default(),
                    writes: crate::settings_store::WriteMode::Enabled(dir.clone()),
                    notice: None,
                },
                cx,
            );
        });
        dir
    }

    /// Opens the editor, types one new value, and presses Apply until the dialog's dry-run is in.
    fn open_dialog(&self, cx: &mut TestAppContext) {
        self.open(cx);
        self.insert("DB_PASSWORD", SECRET_VALUE, cx);
        self.apply(cx);
        self.wait_for("the dialog", cx, |cx| self.has_dialog(cx));
        self.wait_for_dry_run(cx);
    }
}

// ---- entry ----

#[gpui_kit::test]
fn e_opens_edit_values_on_config_maps_and_secrets(cx: &mut TestAppContext) {
    let t = values_test("values-e", cx);
    t.open(cx);
    let view = t.edit_view(cx);
    let (cluster, name) = view.read_with(cx, |view, _| {
        (view.cluster().clone(), view.object().name().to_owned())
    });
    assert_eq!((&cluster, name.as_str()), (&t.cluster, "api-db"));
    // One read of the Secret, over the cluster of the row.
    assert_eq!(t.gets(SECRET_PATH), 1);
    t.fixture.draw_twice(cx);
    // Close it and try a ConfigMap.
    t.shell().update(cx, |shell, cx| shell.close_edit(cx));
    t.show(ResourceKind::ConfigMaps, config_map_rows(), cx);
    t.cursor_on(ResourceKind::ConfigMaps, "api-config", cx);
    t.fixture.press("e", cx);
    t.wait_for_base(cx);
    let name = t
        .edit_view(cx)
        .read_with(cx, |view, _| view.object().name().to_owned());
    assert_eq!(name, "api-config");
    assert_eq!(t.gets(CONFIG_MAP_PATH), 1);
}

#[gpui_kit::test]
fn e_keeps_edit_yaml_on_other_kinds(cx: &mut TestAppContext) {
    let t = values_test("values-e-other", cx);
    // A Deployments screen: E is Edit YAML there (the fake server allows `update`).
    let rows = vec![crate::workload_rows::deployment_row(
        &crate::workload_actions::workload_actions_tests::deployment("api"),
    )];
    t.show(ResourceKind::Deployments, rows, cx);
    t.cursor_on(ResourceKind::Deployments, "api", cx);
    t.wait_for("the update review", cx, |cx| {
        t.shell().read_with(cx, |shell, cx| {
            shell.guard_for(&t.cluster, cx).is_some_and(|guard| {
                matches!(
                    guard.kind_access.get(ObjectKind::Deployment),
                    Some(KindAccess::Known(_))
                )
            })
        })
    });
    t.fixture.press("e", cx);
    cx.run_until_parked();
    assert!(t.view(cx).is_none());
    let is_yaml = t.shell().read_with(cx, |shell, _| {
        shell
            .edit
            .as_ref()
            .is_some_and(|edit| edit.yaml().is_some())
    });
    assert!(is_yaml);
    let context = t
        .shell()
        .read_with(cx, |shell, _| shell_key_context(shell.screen));
    assert_eq!(context, "AppShell");
}

#[gpui_kit::test]
fn e_does_nothing_on_helm_releases(cx: &mut TestAppContext) {
    let t = values_test("values-e-helm", cx);
    t.show(ResourceKind::HelmReleases, Vec::new(), cx);
    let key = ResourceKey::Kind {
        kind: ResourceKind::HelmReleases,
        namespace: Some("team-a".to_owned()),
        name: "api".to_owned(),
    };
    t.shell().update(cx, |shell, cx| {
        shell.change_selection(Some(ClusterObject::new(t.cluster.clone(), key)), cx);
    });
    cx.run_until_parked();
    t.fixture.press("e", cx);
    cx.run_until_parked();
    assert!(!t.is_editing(cx));
    assert!(
        t.api
            .requests()
            .iter()
            .all(|request| request.method != "GET" || !request.path.contains("secrets/"))
    );
}

#[gpui_kit::test]
fn e_on_denied_secret_shows_the_reason(cx: &mut TestAppContext) {
    let t = values_test("values-denied", cx);
    // The first review already ran as allowed; a denial arrives with the next scope.
    t.server.may_patch.store(false, Ordering::SeqCst);
    t.shell().update(cx, |shell, cx| {
        shell.set_namespace(NamespaceScope::Named("team-a".to_owned()), cx)
    });
    t.show(ResourceKind::Secrets, secret_rows(), cx);
    t.cursor_on(ResourceKind::Secrets, "api-db", cx);
    let availability = t.shell().read_with(cx, |shell, cx| {
        let guard = shell.guard_for(&t.cluster, cx).expect("a live guard");
        action_availability(ResourceAction::EditValues(ObjectKind::Secret), &guard)
    });
    assert_eq!(
        availability,
        ActionAvailability::Disabled {
            reason: "Not permitted: patch secrets".into()
        }
    );
    t.fixture.press("e", cx);
    cx.run_until_parked();
    assert!(!t.is_editing(cx));
    assert!(t.patches().is_empty());
    // No read of the Secret was made either.
    assert_eq!(t.gets(SECRET_PATH), 0);
}

#[gpui_kit::test]
fn refused_objects_open_no_editor(cx: &mut TestAppContext) {
    let t = values_test("values-refused", cx);
    for name in [
        "sh.helm.release.v1.api.v1",
        "owned-by-helm",
        "token",
        "frozen",
    ] {
        t.cursor_on(ResourceKind::Secrets, name, cx);
        t.fixture.press("e", cx);
        cx.run_until_parked();
        assert!(!t.is_editing(cx), "{name}");
    }
    assert_eq!(t.gets(SECRET_PATH), 0);
    t.show(ResourceKind::ConfigMaps, config_map_rows(), cx);
    t.cursor_on(ResourceKind::ConfigMaps, "api-release", cx);
    t.fixture.press("e", cx);
    cx.run_until_parked();
    assert!(!t.is_editing(cx));
}

#[gpui_kit::test]
fn edit_yaml_still_opens_from_menu_and_palette_on_secrets(cx: &mut TestAppContext) {
    let t = values_test("values-yaml", cx);
    t.cursor_on(ResourceKind::Secrets, "api-db", cx);
    // The update right is answered as allowed by the fake server.
    let action = RowAction::EditYaml.key_action();
    t.fixture
        .with_window(cx, |window, cx| window.dispatch_action(action, cx));
    t.wait_for("the Edit YAML view", cx, |cx| {
        t.shell().read_with(cx, |shell, _| {
            shell
                .edit
                .as_ref()
                .is_some_and(|edit| edit.yaml().is_some())
        })
    });
    assert!(t.view(cx).is_none());
}

#[gpui_kit::test]
fn one_edit_at_a_time_with_edit_yaml(cx: &mut TestAppContext) {
    let t = values_test("values-one", cx);
    t.open(cx);
    // Another Edit values and an Edit YAML are both refused while one editor is open.
    let reads = t.gets(SECRET_PATH);
    t.fixture.press("e", cx);
    cx.run_until_parked();
    let action = RowAction::EditYaml.key_action();
    t.fixture
        .with_window(cx, |window, cx| window.dispatch_action(action, cx));
    cx.run_until_parked();
    assert!(t.view(cx).is_some());
    assert_eq!(t.gets(SECRET_PATH), reads);
}

#[gpui_kit::test]
fn table_keys_inert_while_open(cx: &mut TestAppContext) {
    let t = values_test("values-inert", cx);
    t.open(cx);
    assert!(t.is_editing(cx));
    for key in ["j", "k", "l", "y", "delete"] {
        t.fixture.press(key, cx);
    }
    cx.run_until_parked();
    assert!(t.is_editing(cx));
    assert!(t.patches().is_empty());
}

// ---- apply ----

#[gpui_kit::test]
fn apply_runs_dry_run_then_confirm_then_commit(cx: &mut TestAppContext) {
    let t = values_test("values-apply", cx);
    let dir = t.audit_folder("apply", cx);
    t.open_dialog(cx);
    let patches = t.patches();
    assert_eq!(patches.len(), 1, "{patches:?}");
    assert!(patches[0].has_query("dryRun", "All"));
    assert_eq!(patches[0].path, SECRET_PATH);
    assert_eq!(
        patches[0].content_type.as_deref(),
        Some("application/merge-patch+json")
    );
    assert!(patches[0].body.contains(SECRET_BASE64));
    assert!(patches[0].body.contains("\"resourceVersion\":\"100\""));
    // Nothing was committed before the click.
    assert!(audit_lines(&dir).is_empty());
    t.confirm(cx);
    t.wait_for("the commit", cx, |_| t.patches().len() == 2);
    let patches = t.patches();
    assert!(!patches[1].has_query_key("dryRun"), "{}", patches[1].query);
    // Success closes the editor and leaves one audit line.
    t.wait_for("the editor to close", cx, |cx| !t.is_editing(cx));
    t.wait_for("the audit line", cx, |_| audit_lines(&dir).len() == 1);
    let lines = audit_lines(&dir);
    assert_eq!(lines[0]["action"], json!("Edit values"));
    assert_eq!(lines[0]["outcome"], json!("applied"));
    assert_eq!(lines[0]["object"]["name"], json!("api-db"));
    assert_eq!(
        lines[0]["fields"][0]["path"],
        json!("data[DB_PASSWORD] value changed")
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[gpui_kit::test]
fn audit_line_and_dialog_hold_names_and_markers_only(cx: &mut TestAppContext) {
    let t = values_test("values-audit", cx);
    let dir = t.audit_folder("audit", cx);
    t.open_dialog(cx);
    let dialog = t.dialog(cx);
    dialog.read_with(cx, |dialog, _| {
        let mut text = dialog
            .label()
            .map(|label| label.to_string())
            .unwrap_or_default();
        text.push_str(&dialog.confirm_text().unwrap_or_default());
        for line in dialog.warning_lines() {
            text.push_str(&line);
        }
        assert!(!text.contains(SECRET_VALUE), "{text}");
        assert!(!text.contains(SECRET_BASE64), "{text}");
        assert_eq!(
            dialog.label().as_deref(),
            Some("Edit values of Secret api-db")
        );
        assert_eq!(dialog.confirm_text().as_deref(), Some("Apply changes"));
    });
    t.confirm(cx);
    t.wait_for("the audit line", cx, |_| audit_lines(&dir).len() == 1);
    let raw = std::fs::read_to_string(crate::audit_log::audit_path(&dir)).expect("an audit log");
    assert!(!raw.contains(SECRET_VALUE), "{raw}");
    assert!(!raw.contains(SECRET_BASE64), "{raw}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn notices_hold_no_value() {
    let notice = values_success_notice("Secret", "api-db", 3);
    assert_eq!(notice, "Updated 3 keys of Secret api-db");
    assert_eq!(
        values_success_notice("ConfigMap", "api-config", 1),
        "Updated 1 key of ConfigMap api-config"
    );
    assert!(!notice.contains(SECRET_VALUE) && !notice.contains(SECRET_BASE64));
}

#[gpui_kit::test]
fn conflict_shows_banner(cx: &mut TestAppContext) {
    let t = values_test("values-conflict", cx);
    *lock(&t.server.commit_answer) = Some((
        409,
        r#"{"kind":"Status","apiVersion":"v1","status":"Failure","message":"the object has been modified","reason":"Conflict","code":409}"#
            .to_owned(),
    ));
    t.open_dialog(cx);
    t.confirm(cx);
    t.wait_for("the banner", cx, |cx| {
        t.view(cx).is_some_and(|view| {
            view.read_with(cx, |view, _| {
                matches!(view.banner(), Some(ValuesBanner::Conflict))
            })
        })
    });
    // The editor stays open with the typed value.
    assert!(t.is_editing(cx));
    let dirty = t.edit_view(cx).read_with(cx, |view, _| view.is_dirty());
    assert!(dirty);
}

#[gpui_kit::test]
fn a_held_ctrl_s_never_opens_the_dialog(cx: &mut TestAppContext) {
    let t = values_test("values-held", cx);
    t.open(cx);
    t.insert("DB_PASSWORD", SECRET_VALUE, cx);
    let held = KeyDownEvent {
        keystroke: Keystroke::parse(&chord("s")).expect("a valid keystroke"),
        is_held: true,
        prefer_character_input: false,
    };
    t.fixture.with_window(cx, |window, cx| {
        window.dispatch_event(held.to_platform_input(), cx);
    });
    cx.run_until_parked();
    assert!(!t.has_dialog(cx));
    assert!(t.patches().is_empty());
    // A fresh press of the same chord does open it.
    t.fixture.press(&chord("s"), cx);
    t.wait_for("the dialog", cx, |cx| t.has_dialog(cx));
}

// ---- leaving ----

#[gpui_kit::test]
fn dirty_edit_asks_before_cluster_switch(cx: &mut TestAppContext) {
    let t = values_test("values-leave", cx);
    t.open(cx);
    t.insert("DB_PASSWORD", SECRET_VALUE, cx);
    let weak = t.edit_view(cx).downgrade();
    let target = t.fixture.cluster("dev-c", cx);
    t.shell()
        .update(cx, |shell, cx| shell.switch_cluster(&target, cx));
    cx.run_until_parked();
    assert_eq!(
        t.shell()
            .read_with(cx, |shell, _| shell.last_leaving.clone()),
        Some(vec!["Unsaved changes to Secret/team-a/api-db".to_owned()])
    );
    // Stay: the editor and its text are kept.
    t.press_dialog(Cancel, cx);
    assert!(t.is_editing(cx));
    // Leave: the cluster is released and the editor goes with it, buffers and all.
    t.shell()
        .update(cx, |shell, cx| shell.switch_cluster(&target, cx));
    cx.run_until_parked();
    t.press_dialog(Confirm { secondary: false }, cx);
    assert!(!t.is_editing(cx));
    cx.run_until_parked();
    assert!(weak.upgrade().is_none());
}

#[gpui_kit::test]
fn a_clean_edit_closes_without_asking(cx: &mut TestAppContext) {
    let t = values_test("values-clean", cx);
    t.open(cx);
    let asked = t
        .shell()
        .update(cx, |shell, cx| shell.parks_for_discard(|_, _| {}, cx));
    assert!(!asked);
    assert!(!t.is_editing(cx));
}

#[gpui_kit::test]
fn namespace_change_asks_then_drops_view(cx: &mut TestAppContext) {
    let t = values_test("values-ns", cx);
    t.open(cx);
    t.insert("DB_PASSWORD", SECRET_VALUE, cx);
    let weak = t.edit_view(cx).downgrade();
    let parked = t
        .shell()
        .update(cx, |shell, cx| shell.parks_for_discard(|_, _| {}, cx));
    assert!(parked);
    cx.run_until_parked();
    // The prompt names the object; the editor stays until the user agrees.
    let asked = t
        .shell()
        .read_with(cx, |shell, _| shell.last_discard.clone());
    assert_eq!(asked.as_deref(), Some("Secret/team-a/api-db"));
    assert!(t.is_editing(cx));
    t.shell().update(cx, |shell, cx| shell.close_edit(cx));
    cx.run_until_parked();
    assert!(weak.upgrade().is_none());
}

#[gpui_kit::test]
fn the_view_holds_no_value_after_it_closes(cx: &mut TestAppContext) {
    let t = values_test("values-drop", cx);
    t.open(cx);
    t.insert("DB_PASSWORD", SECRET_VALUE, cx);
    let view = t.edit_view(cx);
    let field = view
        .read_with(cx, |view, _| view.field_of("DB_PASSWORD"))
        .expect("the key has a field")
        .downgrade();
    drop(view);
    t.shell().update(cx, |shell, cx| shell.close_edit(cx));
    cx.run_until_parked();
    assert!(field.upgrade().is_none());
}

#[gpui_kit::test]
fn prod_tier_needs_typed_name(cx: &mut TestAppContext) {
    let t = values_test_on("values-prod", "prod-a", cx);
    t.fixture
        .session(cx)
        .update(cx, |session, cx| session.set_lock(WriteLock::Unlocked, cx));
    cx.run_until_parked();
    t.open_dialog(cx);
    let tier = t
        .dialog(cx)
        .read_with(cx, |dialog, _| dialog.tier().clone());
    let DialogConfirm::TypeName { expected } = tier else {
        panic!("production types a name");
    };
    // Without the name the commit stays blocked and sends nothing.
    t.confirm(cx);
    cx.run_until_parked();
    assert_eq!(t.patches().len(), 1);
    t.fixture.with_window(cx, |window, cx| {
        let dialog = t.dialog_unchecked(cx);
        dialog.update(cx, |dialog, cx| dialog.type_text(&expected, window, cx));
    });
    t.confirm(cx);
    t.wait_for("the commit", cx, |_| t.patches().len() == 2);
}

#[gpui_kit::test]
fn a_locked_cluster_opens_no_dialog(cx: &mut TestAppContext) {
    let t = values_test("values-locked", cx);
    t.open(cx);
    t.insert("DB_PASSWORD", SECRET_VALUE, cx);
    t.fixture
        .session(cx)
        .update(cx, |session, cx| session.set_lock(WriteLock::Locked, cx));
    cx.run_until_parked();
    t.apply(cx);
    cx.run_until_parked();
    assert!(!t.has_dialog(cx));
    assert!(t.patches().is_empty());
    // The text stays.
    let dirty = t.edit_view(cx).read_with(cx, |view, _| view.is_dirty());
    assert!(dirty);
}

#[gpui_kit::test]
fn a_failed_dry_run_blocks_apply_and_does_not_quote_the_value(cx: &mut TestAppContext) {
    let t = values_test("values-dry-fail", cx);
    let message = format!(
        "Secret \"api-db\" is invalid: data[DB_PASSWORD]: Invalid value: \"{SECRET_VALUE}\""
    );
    *lock(&t.server.dry_run_answer) = Some((
        422,
        json!({
            "kind": "Status", "apiVersion": "v1", "status": "Failure",
            "message": message, "reason": "Invalid", "code": 422,
            "details": {"causes": [{"reason": "FieldValueInvalid", "message": message, "field": "data[DB_PASSWORD]"}]},
        })
        .to_string(),
    ));
    t.open_dialog(cx);
    let state = t
        .dialog(cx)
        .read_with(cx, |dialog, _| dialog.dry_run_state());
    let Some(DryRunState::Failed(text)) = state else {
        panic!("the dry-run failed: {state:?}");
    };
    assert!(!text.contains(SECRET_VALUE), "{text}");
    assert!(text.contains("data[DB_PASSWORD]"), "{text}");
    t.confirm(cx);
    cx.run_until_parked();
    // Only the dry-run was sent.
    assert_eq!(t.patches().len(), 1);
}

#[gpui_kit::test]
fn deleted_object_shows_the_banner(cx: &mut TestAppContext) {
    let t = values_test("values-gone", cx);
    *lock(&t.server.commit_answer) = Some((404, NOT_FOUND.to_owned()));
    t.open_dialog(cx);
    t.confirm(cx);
    t.wait_for("the banner", cx, |cx| {
        t.view(cx).is_some_and(|view| {
            view.read_with(cx, |view, _| {
                matches!(view.banner(), Some(ValuesBanner::Deleted))
            })
        })
    });
}

#[gpui_kit::test]
fn menu_hint_follows_the_key_context(cx: &mut TestAppContext) {
    let t = values_test("values-hint", cx);
    let hints = |t: &ValuesTest, cx: &mut TestAppContext| {
        t.fixture.with_window(cx, |window, cx| {
            let focus = t.shell().read(cx).focus_handle.clone();
            focus.focus(window, cx);
        });
        t.fixture.draw_twice(cx);
        t.fixture.with_window(cx, |window, cx| {
            let focus = t.shell().read(cx).focus_handle.clone();
            (
                window
                    .bindings_for_action_in(&crate::keymap::EditValues, &focus)
                    .len(),
                window
                    .bindings_for_action_in(&crate::keymap::EditYaml, &focus)
                    .len(),
            )
        })
    };
    // On Secrets the menu shows E on Edit values only; Edit YAML has no key there.
    assert_eq!(hints(&t, cx), (1, 0));
    let rows = vec![crate::workload_rows::deployment_row(
        &crate::workload_actions::workload_actions_tests::deployment("api"),
    )];
    t.show(ResourceKind::Deployments, rows, cx);
    assert_eq!(hints(&t, cx), (0, 1));
}
