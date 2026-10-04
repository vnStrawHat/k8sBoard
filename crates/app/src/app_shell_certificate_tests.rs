//! Certificate Renew now (spec 0018 step 6) in a headless window over two loaded clusters: the key
//! and the header button open the shared 0030 dialog on the cursor Certificate of the open cluster,
//! and the requests, the audit line, and the blocks are checked against a fake API server.

use cluster::fake_api::RecordedRequest;
use cluster::{CrdState, CrdSummary, CrdVersion, ResourceScope, SchemaOutline};
use gpui_kit::{InputEvent as _, KeyDownEvent, Keystroke, TestAppContext};

use super::app_shell_switch_tests::open_switch_fixture;
use super::app_shell_write_tests::{Clusters, audit_lines, go_live_answering, switch_to, writes};
use super::*;
use crate::custom_kind::{CustomKind, CustomKindCache, custom_kinds};
use crate::resource_actions::RowAction;
use crate::write_guard::WriteLock;

const NOT_FOUND: &str = r#"{"kind":"Status","apiVersion":"v1","status":"Failure","message":"no","reason":"NotFound","code":404}"#;
const CERTIFICATE_PATH: &str = "/apis/cert-manager.io/v1/namespaces/shop/certificates/tls";
const CERTIFICATE_JSON: &str = r#"{"apiVersion":"cert-manager.io/v1","kind":"Certificate","metadata":{"name":"tls","namespace":"shop","resourceVersion":"812","generation":2},"spec":{"secretName":"tls-secret"},"status":{"conditions":[{"type":"Ready","status":"True"}],"revision":1}}"#;

fn certificate_kind(version: &str) -> CustomKind {
    let crd = CrdSummary {
        name: "certificates.cert-manager.io".to_owned(),
        group: "cert-manager.io".to_owned(),
        kind: "Certificate".to_owned(),
        plural: "certificates".to_owned(),
        singular: "certificate".to_owned(),
        scope: ResourceScope::Namespaced,
        versions: vec![CrdVersion {
            name: version.to_owned(),
            is_served: true,
            is_storage: true,
            is_deprecated: false,
            deprecation_warning: None,
            printer_columns: Vec::new(),
            schema: SchemaOutline::default(),
        }],
        state: CrdState::Established,
        created_at: None,
    };
    custom_kinds(&[crd], &mut CustomKindCache::default())[0]
}

const ALLOWED_REVIEW: &str = r#"{"apiVersion":"authorization.k8s.io/v1","kind":"SelfSubjectAccessReview","metadata":{},"spec":{},"status":{"allowed":true}}"#;
const CERTIFICATE_LIST: &str = r#"{"apiVersion":"cert-manager.io/v1","kind":"CertificateList","metadata":{"resourceVersion":"900"},"items":[{"apiVersion":"cert-manager.io/v1","kind":"Certificate","metadata":{"name":"tls","namespace":"shop","resourceVersion":"812","generation":2},"spec":{"secretName":"tls-secret"},"status":{"conditions":[{"type":"Ready","status":"True"}],"revision":1}}]}"#;

/// Allows every review, lists the one Certificate, serves it, and accepts the `PUT` of its status;
/// finds nothing else.
fn certificate_answers(request: &RecordedRequest) -> (u16, String) {
    let path = request.path.as_str();
    match request.method.as_str() {
        "POST" if path.ends_with("/selfsubjectaccessreviews") => (201, ALLOWED_REVIEW.to_owned()),
        "GET" if path == CERTIFICATE_PATH => (200, CERTIFICATE_JSON.to_owned()),
        "GET" if path.ends_with("/certificates") => (200, CERTIFICATE_LIST.to_owned()),
        "PUT" if path == format!("{CERTIFICATE_PATH}/status") => (200, CERTIFICATE_JSON.to_owned()),
        _ => (404, NOT_FOUND.to_owned()),
    }
}

/// `stg-b` live over `certificate_answers`, showing the Certificates kind with its one listed row
/// and the cursor on it.
fn certificate_clusters(name: &str, kind: CustomKind, cx: &mut TestAppContext) -> Clusters {
    cx.update(|cx| cx.set_reduce_motion(true));
    let fixture = open_switch_fixture(name, cx);
    switch_to(&fixture, "stg-b", cx);
    let (prod, stg) = (fixture.cluster("prod-a", cx), fixture.cluster("stg-b", cx));
    let stg_api = go_live_answering(&fixture, &stg, "node-b", certificate_answers, cx);
    let t = Clusters {
        fixture,
        stg_api,
        prod,
        stg,
    };
    let resource_kind = ResourceKind::Custom(kind);
    t.fixture.shell.update(cx, |shell, cx| {
        shell.show_screen(Screen::Kind(resource_kind), cx);
    });
    let key = ResourceKey::Kind {
        kind: resource_kind,
        namespace: Some("shop".to_owned()),
        name: "tls".to_owned(),
    };
    t.wait_for("the Certificate row", cx, |cx| {
        t.fixture.shell.read_with(cx, |shell, cx| {
            shell
                .live_of(&t.stg, cx)
                .is_some_and(|live| live.row_of(&key).is_some())
        })
    });
    let object = ClusterObject::new(t.stg.clone(), key);
    t.fixture.shell.update(cx, |shell, cx| {
        shell.change_selection(Some(object), cx);
    });
    cx.run_until_parked();
    t
}

fn renew(t: &Clusters, cx: &mut TestAppContext) {
    t.fixture.with_window(cx, |window, cx| {
        t.fixture.shell.update(cx, |shell, cx| {
            shell.run_row_key(RowAction::RenewCertificate, window, cx);
        });
    });
}

fn key_down(key: &str, is_held: bool) -> KeyDownEvent {
    KeyDownEvent {
        keystroke: Keystroke::parse(key).expect("a valid keystroke"),
        is_held,
        prefer_character_input: false,
    }
}

fn puts(t: &Clusters) -> Vec<RecordedRequest> {
    writes(&t.stg_api)
        .into_iter()
        .filter(|request| request.method == "PUT")
        .collect()
}

#[gpui_kit::test]
fn renew_runs_the_dry_run_then_the_commit_and_audits_one_line(cx: &mut TestAppContext) {
    let t = certificate_clusters("renew-flow", certificate_kind("v1"), cx);
    let dir = t.enable_audit_folder("renew-flow", cx);
    renew(&t, cx);
    t.wait_for_dry_run(cx);
    let sent = puts(&t);
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert_eq!(sent[0].path, format!("{CERTIFICATE_PATH}/status"));
    assert!(sent[0].has_query("dryRun", "All"));
    assert!(sent[0].has_query("fieldManager", "k8sboard"));
    assert!(t.block(cx).is_none(), "{:?}", t.block(cx));
    t.confirm(cx);
    t.wait_for("the commit", cx, |_| puts(&t).len() == 2);
    let sent = puts(&t);
    assert!(!sent[1].has_query_key("dryRun"));
    // One body for the check and the commit, with one Issuing condition.
    assert_eq!(sent[0].body, sent[1].body);
    let body: serde_json::Value = serde_json::from_str(&sent[1].body).expect("JSON");
    let issuing: Vec<_> = body["status"]["conditions"]
        .as_array()
        .expect("conditions")
        .iter()
        .filter(|condition| condition["type"] == "Issuing")
        .collect();
    assert_eq!(issuing.len(), 1);
    assert_eq!(issuing[0]["reason"], "ManuallyTriggered");
    t.wait_for("the audit line", cx, |_| audit_lines(&dir).len() == 1);
    let line = &audit_lines(&dir)[0];
    assert_eq!(line["action"], "Renew");
    assert_eq!(line["object"]["kind"], "Certificate");
    assert_eq!(line["fields"][0]["path"], "status.conditions[Issuing]");
    assert_eq!(line["fields"][0]["value"], "True (ManuallyTriggered)");
}

#[gpui_kit::test]
fn held_enter_does_not_confirm_a_renewal(cx: &mut TestAppContext) {
    let t = certificate_clusters("renew-held", certificate_kind("v1"), cx);
    renew(&t, cx);
    t.wait_for_dry_run(cx);
    t.fixture.draw_twice(cx);
    for _ in 0..3 {
        t.fixture.with_window(cx, |window, cx| {
            window.dispatch_event(key_down("enter", true).to_platform_input(), cx);
        });
    }
    cx.run_until_parked();
    assert_eq!(puts(&t).len(), 1, "only the dry-run was sent");
}

#[gpui_kit::test]
fn renew_commit_is_blocked_after_the_cluster_session_changed(cx: &mut TestAppContext) {
    let t = certificate_clusters("renew-generation", certificate_kind("v1"), cx);
    renew(&t, cx);
    t.wait_for_dry_run(cx);
    switch_to(&t.fixture, "prod-a", cx);
    assert_eq!(
        t.block(cx).as_deref(),
        Some("stg-b is no longer open; nothing was changed")
    );
    t.confirm(cx);
    cx.run_until_parked();
    assert_eq!(puts(&t).len(), 1, "only the dry-run reached stg-b");
}

#[gpui_kit::test]
fn renew_on_a_locked_cluster_opens_no_dialog(cx: &mut TestAppContext) {
    let t = certificate_clusters("renew-locked", certificate_kind("v1"), cx);
    t.set_lock(&t.stg, WriteLock::Locked, cx);
    renew(&t, cx);
    assert!(!t.has_dialog(cx));
    assert!(puts(&t).is_empty());
}

#[gpui_kit::test]
fn renew_on_another_cert_manager_version_opens_no_dialog(cx: &mut TestAppContext) {
    let t = certificate_clusters("renew-old-version", certificate_kind("v1alpha2"), cx);
    renew(&t, cx);
    assert!(!t.has_dialog(cx));
    assert!(puts(&t).is_empty());
}

#[gpui_kit::test]
fn renew_header_follows_the_cursor_and_the_gate(cx: &mut TestAppContext) {
    let kind = certificate_kind("v1");
    let t = certificate_clusters("renew-header", kind, cx);
    let state = |t: &Clusters, cx: &mut TestAppContext| {
        t.fixture.shell.read_with(cx, |shell, cx| {
            shell
                .renew_header_state(ResourceKind::Custom(kind), cx)
                .map(|_| ())
                .map_err(|reason| reason.to_string())
        })
    };
    assert_eq!(state(&t, cx), Ok(()));
    t.set_lock(&t.stg, WriteLock::Locked, cx);
    assert_eq!(state(&t, cx), Err("stg-b is read-only".to_owned()));
    t.set_lock(&t.stg, WriteLock::Unlocked, cx);
    t.fixture.shell.update(cx, |shell, cx| {
        shell.change_selection(None, cx);
    });
    cx.run_until_parked();
    assert_eq!(state(&t, cx), Err("Select a certificate".to_owned()));
}
