//! New docker-registry Secret, New TLS Secret, and Replace certificate in a headless window over two
//! loaded clusters, each with its own fake API server: a test sees which cluster a request reached,
//! and nothing leaves the machine.

use cluster::fake_api::{FakeApi, RecordedRequest};
use gpui_kit::TestAppContext;
use serde_json::{Value, json};

use super::app_shell_switch_tests::open_switch_fixture;
use super::app_shell_write_tests::{Clusters, go_live_answering, switch_to, writes};
use super::secret_form::{SecretForm, SecretFormKind, SecretFormStart};
use super::*;
use crate::kind_access::KindAccess;
use crate::resource_kind::ResourceKind;
use crate::write_guard::WriteLock;

const RSA_CERT: &str = include_str!("../../cluster/tests/fixtures/tls/pair-rsa.crt");
const RSA_KEY: &str = include_str!("../../cluster/tests/fixtures/tls/pair-rsa-pkcs8.key");
const OTHER_KEY: &str = include_str!("../../cluster/tests/fixtures/tls/other-rsa-pkcs8.key");
const TLS_PATH: &str = "/api/v1/namespaces/shop/secrets/api-tls";
const SECRETS_PATH: &str = "/api/v1/namespaces/shop/secrets";
const ALLOWED: &str = r#"{"apiVersion":"authorization.k8s.io/v1","kind":"SelfSubjectAccessReview","metadata":{},"spec":{},"status":{"allowed":true}}"#;

fn tls_secret_json() -> String {
    json!({
        "apiVersion": "v1", "kind": "Secret",
        "metadata": {"name": "api-tls", "namespace": "shop", "resourceVersion": "9"},
        "type": "kubernetes.io/tls",
        "data": {"tls.crt": "b2xk", "tls.key": "b2xk"},
    })
    .to_string()
}

fn server() -> impl Fn(&RecordedRequest) -> (u16, String) + Send + Sync + 'static {
    move |request| {
        if request.method == "POST" && request.path.ends_with("/selfsubjectaccessreviews") {
            return (201, ALLOWED.to_owned());
        }
        if request.method == "GET" && request.path == TLS_PATH {
            return (200, tls_secret_json());
        }
        if request.method == "POST" && request.path == SECRETS_PATH {
            let mut created: Value = serde_json::from_str(&request.body).expect("the body is JSON");
            created["metadata"]["uid"] = json!("uid-1");
            return (201, created.to_string());
        }
        if request.method == "PATCH" {
            return (200, tls_secret_json());
        }
        (
            404,
            r#"{"kind":"Status","status":"Failure","code":404}"#.to_owned(),
        )
    }
}

struct FormTest {
    t: Clusters,
}

fn form_test(name: &str, cx: &mut TestAppContext) -> FormTest {
    form_test_on(name, Screen::Kind(ResourceKind::Secrets), cx)
}

/// `form_test` with `screen` shown. The permissions of Secrets are reviewed only when the Secrets
/// screen shows.
fn form_test_on(name: &str, screen: Screen, cx: &mut TestAppContext) -> FormTest {
    form_test_serving(name, screen, server(), cx)
}

/// `form_test_on` over a server that answers as `answer` says.
fn form_test_serving(
    name: &str,
    screen: Screen,
    answer: impl Fn(&RecordedRequest) -> (u16, String) + Send + Sync + 'static,
    cx: &mut TestAppContext,
) -> FormTest {
    let fixture = open_switch_fixture(name, cx);
    switch_to(&fixture, "stg-b", cx);
    let (prod, stg) = (fixture.cluster("prod-a", cx), fixture.cluster("stg-b", cx));
    let stg_api = go_live_answering(&fixture, &stg, "node-b", answer, cx);
    fixture
        .shell
        .update(cx, |shell, cx| shell.show_screen(screen, cx));
    cx.run_until_parked();
    fixture.draw_twice(cx);
    // The create and patch rights are reviewed once the screen of the kind shows.
    for _ in 0..if screen == Screen::Pods { 0 } else { 1_500 } {
        cx.run_until_parked();
        let is_known = fixture.shell.read_with(cx, |shell, cx| {
            shell.guard_for(&stg, cx).is_some_and(|guard| {
                matches!(
                    guard.kind_access.get(cluster::ObjectKind::Secret),
                    Some(KindAccess::Known(_))
                )
            })
        });
        if is_known {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    FormTest {
        t: Clusters {
            fixture,
            stg_api,
            prod,
            stg,
        },
    }
}

impl FormTest {
    fn open_new(&self, start: SecretFormStart, cx: &mut TestAppContext) {
        self.t.fixture.with_window(cx, |window, cx| {
            self.t.fixture.shell.update(cx, |shell, cx| {
                shell.open_secret_form(start, window, cx);
            });
        });
    }

    fn open_replace(&self, cx: &mut TestAppContext) {
        let key = ResourceKey::Kind {
            kind: ResourceKind::Secrets,
            namespace: Some("shop".to_owned()),
            name: "api-tls".to_owned(),
        };
        self.t.fixture.with_window(cx, |window, cx| {
            self.t.fixture.shell.update(cx, |shell, cx| {
                shell.open_certificate_replace(
                    ClusterObject::new(self.t.stg.clone(), key),
                    window,
                    cx,
                );
            });
        });
    }

    fn form(&self, cx: &mut TestAppContext) -> Option<Entity<SecretForm>> {
        self.t
            .fixture
            .shell
            .read_with(cx, |shell, _| shell.secret_form.clone())
            .and_then(|form| form.upgrade())
    }

    fn wait_for_form(&self, cx: &mut TestAppContext) -> Entity<SecretForm> {
        self.t.wait_for("the form to load", cx, |cx| {
            self.form(cx)
                .is_some_and(|form| form.read_with(cx, |form, _| form.is_loaded()))
        });
        self.form(cx).expect("a form")
    }

    fn review(&self, form: &Entity<SecretForm>, cx: &mut TestAppContext) {
        self.t.fixture.with_window(cx, |window, cx| {
            form.update(cx, |form, cx| form.press_review(window, cx));
        });
        cx.run_until_parked();
    }
}

fn body_of(request: &RecordedRequest) -> Value {
    serde_json::from_str(&request.body).expect("the body is JSON")
}

fn reads_of(api: &FakeApi, path: &str) -> usize {
    api.requests()
        .iter()
        .filter(|request| request.method == "GET" && request.path == path)
        .count()
}

fn registry_start() -> SecretFormStart {
    SecretFormStart {
        kind: SecretFormKind::DockerRegistry,
        namespace: "shop".to_owned(),
        name: "regcred".to_owned(),
    }
}

#[gpui_kit::test]
fn the_pull_secret_form_opens_with_the_name_and_namespace(cx: &mut TestAppContext) {
    let t = form_test("secret-form-opens", cx);
    t.open_new(registry_start(), cx);
    let form = t.wait_for_form(cx);
    form.read_with(cx, |form, cx| {
        assert_eq!(form.kind(), SecretFormKind::DockerRegistry);
        assert_eq!(form.name_text(cx), "regcred");
        assert_eq!(form.namespace_text(cx), "shop");
        // Nothing is typed yet, so there is no change to review.
        assert!(form.current_intent(cx).is_err());
    });
}

#[gpui_kit::test]
fn a_form_without_a_namespace_starts_in_the_scope_or_default(cx: &mut TestAppContext) {
    let t = form_test("secret-form-namespace", cx);
    t.open_new(
        SecretFormStart {
            kind: SecretFormKind::DockerRegistry,
            namespace: String::new(),
            name: String::new(),
        },
        cx,
    );
    let form = t.wait_for_form(cx);
    let namespace = form.read_with(cx, |form, cx| form.namespace_text(cx));
    assert!(!namespace.is_empty());
}

#[gpui_kit::test]
fn the_pull_secret_is_created_after_a_dry_run_with_a_built_dockerconfigjson(
    cx: &mut TestAppContext,
) {
    let t = form_test("secret-form-registry", cx);
    t.open_new(registry_start(), cx);
    let form = t.wait_for_form(cx);
    t.t.fixture.with_window(cx, |window, cx| {
        form.update(cx, |form, cx| {
            form.fill_registry("me", "hunter2-pw", window, cx);
        });
    });
    t.review(&form, cx);
    t.t.wait_for_dry_run(cx);
    let dialog = t.t.dialog(cx);
    assert_eq!(
        dialog.read_with(cx, |dialog, _| dialog.label()).as_deref(),
        Some("Create Secret shop/regcred")
    );
    // The dialog says what is built, never the password.
    let lines: Vec<String> = dialog
        .read_with(cx, |dialog, _| dialog.change_lines())
        .iter()
        .map(ToString::to_string)
        .collect();
    assert!(lines.iter().any(|line| line.contains("user me")));
    assert!(!lines.join(" ").contains("hunter2-pw"));
    let sent = writes(&t.t.stg_api);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].method, "POST");
    assert_eq!(sent[0].path, SECRETS_PATH);
    assert!(sent[0].has_query("dryRun", "All"));
    let body = body_of(&sent[0]);
    assert_eq!(body["type"], "kubernetes.io/dockerconfigjson");
    assert!(body.get("stringData").is_none());
    t.t.confirm(cx);
    t.t.wait_for("the commit", cx, |_| writes(&t.t.stg_api).len() == 2);
    let commit = &writes(&t.t.stg_api)[1];
    assert!(!commit.has_query_key("dryRun"));
    assert_eq!(body_of(commit)["metadata"]["name"], "regcred");
}

#[gpui_kit::test]
fn a_form_with_a_missing_field_sends_nothing(cx: &mut TestAppContext) {
    let t = form_test("secret-form-incomplete", cx);
    t.open_new(registry_start(), cx);
    let form = t.wait_for_form(cx);
    t.review(&form, cx);
    assert!(!t.t.has_dialog(cx));
    assert!(writes(&t.t.stg_api).is_empty());
}

#[gpui_kit::test]
fn a_locked_cluster_opens_no_form(cx: &mut TestAppContext) {
    let t = form_test("secret-form-locked", cx);
    t.t.set_lock(&t.t.stg, WriteLock::Locked, cx);
    t.open_new(registry_start(), cx);
    assert!(t.form(cx).is_none());
}

#[gpui_kit::test]
fn replace_certificate_reads_the_secret_and_patches_both_keys(cx: &mut TestAppContext) {
    let t = form_test("secret-form-replace", cx);
    t.open_replace(cx);
    let form = t.wait_for_form(cx);
    assert_eq!(reads_of(&t.t.stg_api, TLS_PATH), 1);
    form.read_with(cx, |form, _| {
        assert_eq!(form.kind(), SecretFormKind::TlsReplace);
    });
    t.t.fixture.with_window(cx, |window, cx| {
        form.update(cx, |form, cx| form.fill_pair(RSA_CERT, RSA_KEY, window, cx));
    });
    t.review(&form, cx);
    t.t.wait_for_dry_run(cx);
    let dialog = t.t.dialog(cx);
    assert_eq!(
        dialog.read_with(cx, |dialog, _| dialog.label()).as_deref(),
        Some("Replace certificate of secret api-tls")
    );
    let sent = writes(&t.t.stg_api);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].method, "PATCH");
    assert_eq!(sent[0].path, TLS_PATH);
    assert!(sent[0].has_query("dryRun", "All"));
    let body = body_of(&sent[0]);
    assert_eq!(body["metadata"]["resourceVersion"], "9");
    assert!(body["data"]["tls.crt"].is_string());
    assert!(body["data"]["tls.key"].is_string());
    t.t.confirm(cx);
    t.t.wait_for("the commit", cx, |_| writes(&t.t.stg_api).len() == 2);
    assert!(!writes(&t.t.stg_api)[1].has_query_key("dryRun"));
}

#[gpui_kit::test]
fn replace_certificate_refuses_a_key_that_does_not_match(cx: &mut TestAppContext) {
    let t = form_test("secret-form-mismatch", cx);
    t.open_replace(cx);
    let form = t.wait_for_form(cx);
    t.t.fixture.with_window(cx, |window, cx| {
        form.update(cx, |form, cx| {
            form.fill_pair(RSA_CERT, OTHER_KEY, window, cx);
        });
    });
    let problem = form.read_with(cx, |form, cx| form.current_intent(cx).err());
    assert_eq!(
        problem.as_deref(),
        Some("The key does not match the certificate")
    );
    t.review(&form, cx);
    assert!(!t.t.has_dialog(cx));
    assert!(writes(&t.t.stg_api).is_empty());
}

#[gpui_kit::test]
fn the_private_key_starts_hidden(cx: &mut TestAppContext) {
    let t = form_test("secret-form-key-hidden", cx);
    t.open_replace(cx);
    let form = t.wait_for_form(cx);
    assert!(!form.read_with(cx, |form, _| form.is_key_shown()));
}

#[gpui_kit::test]
fn an_unreadable_secret_shows_the_error_and_no_review(cx: &mut TestAppContext) {
    let t = form_test("secret-form-unreadable", cx);
    let key = ResourceKey::Kind {
        kind: ResourceKind::Secrets,
        namespace: Some("shop".to_owned()),
        name: "ghost".to_owned(),
    };
    t.t.fixture.with_window(cx, |window, cx| {
        t.t.fixture.shell.update(cx, |shell, cx| {
            shell.open_certificate_replace(ClusterObject::new(t.t.stg.clone(), key), window, cx);
        });
    });
    t.t.wait_for("the failure", cx, |cx| {
        t.form(cx)
            .is_some_and(|form| form.read_with(cx, |form, _| form.failure().is_some()))
    });
    let form = t.form(cx).expect("a form");
    assert!(
        form.read_with(cx, |form, _| form.failure())
            .is_some_and(|text| text.starts_with("Cannot replace"))
    );
}

#[gpui_kit::test]
fn the_form_asks_for_the_permissions_of_secrets_when_a_pod_opens_it(cx: &mut TestAppContext) {
    let t = form_test_on("secret-form-from-pods", Screen::Pods, cx);
    let asked_for_secrets = |t: &FormTest| {
        t.t.stg_api.requests().iter().any(|request| {
            request.method == "POST"
                && request.path.ends_with("/selfsubjectaccessreviews")
                && request.body.contains("\"resource\":\"secrets\"")
        })
    };
    assert!(!asked_for_secrets(&t));
    t.open_new(registry_start(), cx);
    // The answer arrives over the runtime and the form opens after a poll of the clock.
    for _ in 0..1_500 {
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(150));
        cx.run_until_parked();
        if t.form(cx).is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(asked_for_secrets(&t));
    let form = t.wait_for_form(cx);
    assert_eq!(form.read_with(cx, |form, cx| form.name_text(cx)), "regcred");
}

/// The server of `server()`, except that a create of a Secret is refused as already there: on its
/// dry-run, or, with `is_refused_on_commit`, only on the commit.
fn refusing_server(
    is_refused_on_commit: bool,
) -> impl Fn(&RecordedRequest) -> (u16, String) + Send + Sync + 'static {
    let answer = server();
    move |request| {
        let is_create = request.method == "POST" && request.path == SECRETS_PATH;
        if is_create && request.has_query_key("dryRun") != is_refused_on_commit {
            return (
                409,
                json!({
                    "kind": "Status", "apiVersion": "v1", "status": "Failure",
                    "message": "secrets \"regcred\" already exists",
                    "reason": "AlreadyExists", "code": 409,
                })
                .to_string(),
            );
        }
        answer(request)
    }
}

fn is_dialog_open(t: &FormTest, cx: &mut TestAppContext) -> bool {
    t.t.fixture
        .with_window(cx, |window, cx| window.has_active_dialog(cx))
}

fn fill_and_review(t: &FormTest, form: &Entity<SecretForm>, cx: &mut TestAppContext) {
    t.t.fixture.with_window(cx, |window, cx| {
        form.update(cx, |form, cx| {
            form.fill_registry("me", "hunter2-pw", window, cx);
        });
    });
    t.review(form, cx);
}

#[gpui_kit::test]
fn a_refused_dry_run_keeps_the_form_open_with_the_server_message(cx: &mut TestAppContext) {
    let t = form_test_serving(
        "secret-form-dry-run-refused",
        Screen::Kind(ResourceKind::Secrets),
        refusing_server(false),
        cx,
    );
    t.open_new(registry_start(), cx);
    let form = t.wait_for_form(cx);
    fill_and_review(&t, &form, cx);
    t.t.wait_for("the refusal", cx, |cx| {
        form.read_with(cx, |form, _| form.refusal().is_some())
    });
    assert_eq!(
        form.read_with(cx, |form, _| form.refusal()).as_deref(),
        Some(
            "Dry-run failed: the change is invalid: Secret regcred already exists (metadata.name)"
        )
    );
    assert!(is_dialog_open(&t, cx), "the form is still open");
    // The fields are kept, so Review… again needs no retyping and sends a new check.
    t.review(&form, cx);
    t.t.wait_for("the second check", cx, |_| writes(&t.t.stg_api).len() == 2);
}

#[gpui_kit::test]
fn a_refused_commit_keeps_the_form_open_with_the_server_message(cx: &mut TestAppContext) {
    let t = form_test_serving(
        "secret-form-commit-refused",
        Screen::Kind(ResourceKind::Secrets),
        refusing_server(true),
        cx,
    );
    t.open_new(registry_start(), cx);
    let form = t.wait_for_form(cx);
    fill_and_review(&t, &form, cx);
    t.t.wait_for_dry_run(cx);
    t.t.confirm(cx);
    t.t.wait_for("the refusal", cx, |cx| {
        form.read_with(cx, |form, _| form.refusal().is_some())
    });
    let text = form.read_with(cx, |form, _| form.refusal());
    assert!(
        text.is_some_and(|text| text.contains("Secret regcred already exists")),
        "the form says what the server said"
    );
    assert!(is_dialog_open(&t, cx), "the form is still open");
}

#[gpui_kit::test]
fn a_commit_that_went_through_closes_the_form(cx: &mut TestAppContext) {
    let t = form_test("secret-form-closes", cx);
    t.open_new(registry_start(), cx);
    let form = t.wait_for_form(cx);
    fill_and_review(&t, &form, cx);
    t.t.wait_for_dry_run(cx);
    assert!(is_dialog_open(&t, cx));
    t.t.confirm(cx);
    t.t.wait_for("the commit", cx, |_| writes(&t.t.stg_api).len() == 2);
    t.t.wait_for("the dialogs to close", cx, |cx| !is_dialog_open(&t, cx));
}
