//! Edit labels / annotations in a headless window over two loaded clusters, each with its own fake
//! API server: a test sees which cluster a request reached, and nothing leaves the machine.

use std::sync::{Arc, Mutex};

use cluster::fake_api::{FakeApi, RecordedRequest};
use gpui_kit::TestAppContext;
use serde_json::{Value, json};

use super::app_shell_switch_tests::open_switch_fixture;
use super::app_shell_write_tests::{Clusters, go_live_answering, switch_to, writes};
use super::metadata_editor::MetadataEditor;
use super::*;
use crate::kind_access::KindAccess;
use crate::metadata_edits::MetadataList;
use crate::resource_kind::ResourceKind;
use crate::write_guard::WriteLock;

const WEB_PATH: &str = "/apis/apps/v1/namespaces/shop/deployments/web";
const GHOST_PATH: &str = "/apis/apps/v1/namespaces/shop/deployments/ghost";
const APPLIED: &str = "kubectl.kubernetes.io/last-applied-configuration";
const ALLOWED: &str = r#"{"apiVersion":"authorization.k8s.io/v1","kind":"SelfSubjectAccessReview","metadata":{},"spec":{},"status":{"allowed":true}}"#;

fn deployment_json() -> String {
    json!({
        "apiVersion": "apps/v1", "kind": "Deployment",
        "metadata": {
            "name": "web", "namespace": "shop",
            "labels": {"app": "web"},
            "annotations": {"team": "storefront", APPLIED: "{\"secret\":\"s3cret\"}"},
        },
    })
    .to_string()
}

fn server(
    patch: Arc<Mutex<(u16, String)>>,
) -> impl Fn(&RecordedRequest) -> (u16, String) + Send + Sync + 'static {
    move |request| {
        if request.method == "POST" && request.path.ends_with("/selfsubjectaccessreviews") {
            return (201, ALLOWED.to_owned());
        }
        if request.method == "GET" && request.path == WEB_PATH {
            return (200, deployment_json());
        }
        if request.method == "PATCH" {
            return patch.lock().expect("the answer").clone();
        }
        (
            404,
            r#"{"kind":"Status","status":"Failure","code":404}"#.to_owned(),
        )
    }
}

struct MetadataTest {
    t: Clusters,
}

fn metadata_test(name: &str, cx: &mut TestAppContext) -> MetadataTest {
    let fixture = open_switch_fixture(name, cx);
    switch_to(&fixture, "stg-b", cx);
    let (prod, stg) = (fixture.cluster("prod-a", cx), fixture.cluster("stg-b", cx));
    let patch = Arc::new(Mutex::new((200, deployment_json())));
    let stg_api = go_live_answering(&fixture, &stg, "node-b", server(patch), cx);
    fixture.shell.update(cx, |shell, cx| {
        shell.show_screen(Screen::Kind(ResourceKind::Deployments), cx);
    });
    cx.run_until_parked();
    fixture.draw_twice(cx);
    // The patch right is reviewed once the screen of the kind shows.
    for _ in 0..1_500 {
        cx.run_until_parked();
        let is_known = fixture.shell.read_with(cx, |shell, cx| {
            shell.guard_for(&stg, cx).is_some_and(|guard| {
                matches!(
                    guard.kind_access.get(cluster::ObjectKind::Deployment),
                    Some(KindAccess::Known(_))
                )
            })
        });
        if is_known {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    MetadataTest {
        t: Clusters {
            fixture,
            stg_api,
            prod,
            stg,
        },
    }
}

fn deployment_key() -> ResourceKey {
    ResourceKey::Kind {
        kind: ResourceKind::Deployments,
        namespace: Some("shop".to_owned()),
        name: "web".to_owned(),
    }
}

impl MetadataTest {
    fn open(&self, cluster: &ClusterRef, key: ResourceKey, cx: &mut TestAppContext) {
        self.t.fixture.with_window(cx, |window, cx| {
            self.t.fixture.shell.update(cx, |shell, cx| {
                shell.open_metadata_editor(ClusterObject::new(cluster.clone(), key), window, cx);
            });
        });
    }

    fn editor(&self, cx: &mut TestAppContext) -> Option<Entity<MetadataEditor>> {
        self.t
            .fixture
            .shell
            .read_with(cx, |shell, _| shell.last_metadata_editor.clone())
            .and_then(|editor| editor.upgrade())
    }

    fn wait_for_editor(&self, cx: &mut TestAppContext) -> Entity<MetadataEditor> {
        self.t.wait_for("the editor to load", cx, |cx| {
            self.editor(cx)
                .is_some_and(|editor| editor.read_with(cx, |editor, _| editor.is_loaded()))
        });
        self.editor(cx).expect("an editor")
    }

    fn add_row(
        &self,
        editor: &Entity<MetadataEditor>,
        list: MetadataList,
        key: &str,
        value: &str,
        cx: &mut TestAppContext,
    ) {
        self.t.fixture.with_window(cx, |window, cx| {
            editor.update(cx, |editor, cx| {
                editor.add_row_with(list, key, value, window, cx);
            });
        });
    }

    fn review(&self, editor: &Entity<MetadataEditor>, cx: &mut TestAppContext) {
        self.t.fixture.with_window(cx, |window, cx| {
            editor.update(cx, |editor, cx| editor.press_review(window, cx));
        });
        cx.run_until_parked();
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
fn the_editor_reads_the_object_from_its_own_cluster_and_hides_the_applied_manifest(
    cx: &mut TestAppContext,
) {
    let t = metadata_test("metadata-edit-reads", cx);
    t.open(&t.t.stg, deployment_key(), cx);
    let editor = t.wait_for_editor(cx);
    assert_eq!(reads_of(&t.t.stg_api, WEB_PATH), 1);
    editor.read_with(cx, |editor, cx| {
        let labels = editor.rows_of(MetadataList::Labels, cx);
        assert_eq!(labels.len(), 1);
        assert_eq!(labels[0].key, "app");
        let annotations = editor.rows_of(MetadataList::Annotations, cx);
        assert_eq!(annotations.len(), 1);
        assert_eq!(annotations[0].key, "team");
        assert_eq!(editor.hidden_annotation_count(), 1);
        assert_eq!(
            editor.current_intent(cx).err().map(|text| text.to_string()),
            Some("No changes".to_owned())
        );
    });
}

#[gpui_kit::test]
fn review_lists_old_and_new_and_commit_sends_a_minimal_patch(cx: &mut TestAppContext) {
    let t = metadata_test("metadata-edit-commit", cx);
    t.open(&t.t.stg, deployment_key(), cx);
    let editor = t.wait_for_editor(cx);
    t.add_row(&editor, MetadataList::Annotations, "owner", "infra", cx);
    t.review(&editor, cx);
    t.t.wait_for_dry_run(cx);
    let dialog = t.t.dialog(cx);
    assert_eq!(
        dialog.read_with(cx, |dialog, _| dialog.label()).as_deref(),
        Some("Edit labels / annotations of deployment web")
    );
    assert_eq!(
        dialog
            .read_with(cx, |dialog, _| dialog.change_lines())
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        ["metadata.annotations.owner: (none) → infra"]
    );
    // The dry-run is the same patch with the server flag, on stg only.
    let sent = writes(&t.t.stg_api);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].path, WEB_PATH);
    assert!(sent[0].has_query("dryRun", "All"));
    t.t.confirm(cx);
    t.t.wait_for("the commit", cx, |_| writes(&t.t.stg_api).len() == 2);
    let commit = &writes(&t.t.stg_api)[1];
    assert!(!commit.has_query_key("dryRun"));
    assert_eq!(
        commit.content_type.as_deref(),
        Some("application/merge-patch+json")
    );
    // Only the added key goes out: the label, the other annotation, and the hidden one stay.
    assert_eq!(
        body_of(commit),
        json!({"metadata": {"annotations": {"owner": "infra"}}})
    );
}

#[gpui_kit::test]
fn a_removed_label_goes_out_as_null(cx: &mut TestAppContext) {
    let t = metadata_test("metadata-edit-remove", cx);
    t.open(&t.t.stg, deployment_key(), cx);
    let editor = t.wait_for_editor(cx);
    t.t.fixture.with_window(cx, |window, cx| {
        editor.update(cx, |editor, cx| editor.remove_label_for_test(0, window, cx));
    });
    t.review(&editor, cx);
    t.t.wait_for_dry_run(cx);
    let dialog = t.t.dialog(cx);
    assert_eq!(
        dialog
            .read_with(cx, |dialog, _| dialog.change_lines())
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        ["metadata.labels.app: web → (removed)"]
    );
    assert_eq!(
        body_of(&writes(&t.t.stg_api)[0]),
        json!({"metadata": {"labels": {"app": null}}})
    );
}

#[gpui_kit::test]
fn a_locked_cluster_opens_no_editor(cx: &mut TestAppContext) {
    let t = metadata_test("metadata-edit-locked", cx);
    t.t.set_lock(&t.t.stg, WriteLock::Locked, cx);
    t.open(&t.t.stg, deployment_key(), cx);
    assert!(t.editor(cx).is_none());
    assert_eq!(reads_of(&t.t.stg_api, WEB_PATH), 0);
}

#[gpui_kit::test]
fn a_kind_without_the_editor_opens_none(cx: &mut TestAppContext) {
    let t = metadata_test("metadata-edit-kind", cx);
    let key = ResourceKey::Kind {
        kind: ResourceKind::Services,
        namespace: Some("shop".to_owned()),
        name: "web".to_owned(),
    };
    t.open(&t.t.stg, key, cx);
    assert!(t.editor(cx).is_none());
}

#[gpui_kit::test]
fn an_unreadable_object_shows_the_error_and_no_rows(cx: &mut TestAppContext) {
    let t = metadata_test("metadata-edit-unreadable", cx);
    // The server finds the deployment `web` but not `ghost`.
    let key = ResourceKey::Kind {
        kind: ResourceKind::Deployments,
        namespace: Some("shop".to_owned()),
        name: "ghost".to_owned(),
    };
    t.open(&t.t.stg, key, cx);
    t.t.wait_for("the failure", cx, |cx| {
        t.editor(cx)
            .is_some_and(|editor| editor.read_with(cx, |editor, _| editor.failure().is_some()))
    });
    let editor = t.editor(cx).expect("an editor");
    let text = editor.read_with(cx, |editor, _| editor.failure());
    assert!(
        text.is_some_and(|text| text.starts_with("Could not read deployment ghost")),
        "the error names the deployment"
    );
    assert_eq!(reads_of(&t.t.stg_api, GHOST_PATH), 1);
}
