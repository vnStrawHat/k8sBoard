use cluster::fake_api::{FakeApi, RecordedRequest};
use cluster::{ObjectKind, WritePolicy};
use gpui_kit::{AppContext as _, Entity, TestAppContext};
use serde_json::{Value, json};

use super::*;
use crate::resource_kind::ResourceKind;

fn deployment_key() -> ResourceKey {
    ResourceKey::Kind {
        kind: ResourceKind::Deployments,
        namespace: Some("shop".to_owned()),
        name: "api".to_owned(),
    }
}

fn deployment_object() -> ObjectRef {
    ObjectRef::new(
        ObjectKind::Deployment,
        Some("shop".to_owned()),
        "api".to_owned(),
    )
    .expect("a deployment is namespaced")
}

fn replica_set(name: &str, revision: u32) -> Value {
    json!({
        "apiVersion": "apps/v1", "kind": "ReplicaSet",
        "metadata": {
            "name": name, "namespace": "shop",
            "annotations": {"deployment.kubernetes.io/revision": revision.to_string()},
            "ownerReferences": [{
                "apiVersion": "apps/v1", "kind": "Deployment", "name": "api",
                "uid": "u1", "controller": true,
            }],
        },
        "spec": {"template": {"spec": {"containers": [
            {"name": "api", "image": format!("api:v{revision}")},
        ]}}},
    })
}

fn answer(sets: Vec<Value>) -> impl Fn(&RecordedRequest) -> (u16, String) + Send + Sync + 'static {
    move |request| {
        let body = if request.path.ends_with("/replicasets") {
            json!({"apiVersion": "apps/v1", "kind": "ReplicaSetList", "metadata": {}, "items": sets})
        } else {
            replica_set("one", 1)
        };
        (200, body.to_string())
    }
}

struct Fixture {
    // Held so the runtime outlives the entity's tasks.
    _runtime: tokio::runtime::Runtime,
    api: FakeApi,
    history: Entity<RevisionHistory>,
}

fn open(sets: Vec<Value>, cx: &mut TestAppContext) -> Fixture {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .expect("a tokio runtime");
    // The tokio threads wake gpui tasks, which the deterministic scheduler forbids by default.
    cx.executor().allow_parking();
    cx.update(|cx| cx.set_global(ClusterRuntime::new(runtime.handle().clone())));
    let (connection, api) = {
        let _guard = runtime.enter();
        FakeApi::connection(WritePolicy::Blocked, answer(sets))
    };
    let inputs = HistoryInputs::Ready {
        connection,
        selector: "app=api".to_owned(),
    };
    let history =
        cx.new(|cx| RevisionHistory::new(deployment_key(), deployment_object(), inputs, cx));
    Fixture {
        _runtime: runtime,
        api,
        history,
    }
}

impl Fixture {
    fn wait_until_ready(&self, cx: &mut TestAppContext) {
        for _ in 0..1_500 {
            cx.run_until_parked();
            if self.history.read_with(cx, |history, _| {
                matches!(history.state, HistoryState::Ready(_))
            }) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        panic!("the revisions never loaded");
    }

    fn selected(&self, cx: &mut TestAppContext) -> Option<usize> {
        self.history.read_with(cx, |history, _| history.selected)
    }

    fn has_diff(&self, cx: &mut TestAppContext) -> bool {
        self.history
            .read_with(cx, |history, _| history.diff.is_some())
    }
}

fn three_sets() -> Vec<Value> {
    vec![
        replica_set("api-36", 36),
        replica_set("api-38", 38),
        replica_set("api-37", 37),
    ]
}

#[gpui_kit::test]
fn history_selects_previous_revision_on_load(cx: &mut TestAppContext) {
    let fixture = open(three_sets(), cx);
    fixture.wait_until_ready(cx);
    // Newest first: rev 38 (current), rev 37, rev 36. The previous one is selected, with its diff.
    assert_eq!(fixture.selected(cx), Some(1));
    assert!(fixture.has_diff(cx));
    let names: Vec<String> = fixture
        .history
        .read_with(cx, |history, _| match &history.state {
            HistoryState::Ready(sides) => sides.iter().map(RevisionSide::title).collect(),
            _ => Vec::new(),
        });
    assert_eq!(names, ["rev 38 · v38", "rev 37 · v37", "rev 36 · v36"]);
    // One bounded LIST; the diff's own GETs are the only other requests.
    let lists: Vec<RecordedRequest> = fixture
        .api
        .requests()
        .into_iter()
        .filter(|request| request.path.ends_with("/replicasets"))
        .collect();
    assert_eq!(lists.len(), 1);
    assert!(lists[0].has_query("labelSelector", "app%3Dapi"));
}

#[gpui_kit::test]
fn history_current_row_has_no_diff(cx: &mut TestAppContext) {
    let fixture = open(three_sets(), cx);
    fixture.wait_until_ready(cx);
    fixture
        .history
        .update(cx, |history, cx| history.select(0, cx));
    assert_eq!(fixture.selected(cx), Some(0));
    assert!(!fixture.has_diff(cx));
    let note = fixture
        .history
        .read_with(cx, |history, _| match &history.state {
            HistoryState::Ready(sides) => {
                detail_note(sides, history.selected, history.diff.is_some())
            }
            _ => None,
        });
    assert_eq!(note, Some("This is the current revision."));
}

#[gpui_kit::test]
fn history_denied_sends_nothing(cx: &mut TestAppContext) {
    // No connection is handed over for a denied list, so there is nothing that could send.
    let history = cx.new(|cx| {
        RevisionHistory::new(
            deployment_key(),
            deployment_object(),
            HistoryInputs::Denied,
            cx,
        )
    });
    history.read_with(cx, |history, _| {
        assert!(matches!(history.state, HistoryState::Denied));
        assert!(history.connection.is_none());
    });
    assert_eq!(DENIED_NOTE, "Not permitted: list replicasets");
}

#[gpui_kit::test]
fn history_unavailable_is_a_failure_with_its_reason(cx: &mut TestAppContext) {
    let history = cx.new(|cx| {
        RevisionHistory::new(
            deployment_key(),
            deployment_object(),
            HistoryInputs::Unavailable("the deployment is not loaded yet".into()),
            cx,
        )
    });
    history.read_with(cx, |history, _| {
        assert!(matches!(
            &history.state,
            HistoryState::Failed(reason) if reason.as_ref() == "the deployment is not loaded yet"
        ));
    });
}

#[gpui_kit::test]
fn history_single_revision_text(cx: &mut TestAppContext) {
    let fixture = open(vec![replica_set("api-38", 38)], cx);
    fixture.wait_until_ready(cx);
    assert_eq!(fixture.selected(cx), None);
    assert!(!fixture.has_diff(cx));
    let note = fixture
        .history
        .read_with(cx, |history, _| match &history.state {
            HistoryState::Ready(sides) => {
                detail_note(sides, history.selected, history.diff.is_some())
            }
            _ => None,
        });
    assert_eq!(
        note,
        Some("No earlier revision kept (revisionHistoryLimit)")
    );
}

#[test]
fn detail_note_is_none_only_with_a_diff() {
    let side = |is_current| RevisionSide {
        replica_set: "x".to_owned(),
        revision: Some(1),
        tag: None,
        is_current,
        created_at: None,
        change_cause: None,
    };
    assert_eq!(detail_note(&[], None, false), Some("No revisions found"));
    assert_eq!(detail_note(&[side(false)], Some(0), true), None);
    assert_eq!(
        detail_note(&[side(true)], Some(0), true),
        Some(CURRENT_NOTE)
    );
}

#[test]
fn detail_note_is_neutral_without_revision_numbers() {
    let side = |revision| RevisionSide {
        replica_set: "x".to_owned(),
        revision,
        tag: None,
        is_current: false,
        created_at: None,
        change_cause: None,
    };
    assert_eq!(
        detail_note(&[side(None), side(None)], None, false),
        Some(UNNUMBERED_NOTE)
    );
    // One numbered revision keeps the history-limit wording.
    assert_eq!(
        detail_note(&[side(Some(3)), side(None)], None, false),
        Some(SINGLE_NOTE)
    );
}

#[gpui_kit::test]
fn history_failure_can_be_retried(cx: &mut TestAppContext) {
    let unavailable = cx.new(|cx| {
        RevisionHistory::new(
            deployment_key(),
            deployment_object(),
            HistoryInputs::Unavailable("the deployment is not loaded yet".into()),
            cx,
        )
    });
    assert!(unavailable.read_with(cx, |history, _| history.has_failed()));
    // A denied list is not asked again by showing the tab, and a loaded one never is.
    let denied = cx.new(|cx| {
        RevisionHistory::new(
            deployment_key(),
            deployment_object(),
            HistoryInputs::Denied,
            cx,
        )
    });
    assert!(!denied.read_with(cx, |history, _| history.has_failed()));
    let fixture = open(three_sets(), cx);
    fixture.wait_until_ready(cx);
    assert!(
        !fixture
            .history
            .read_with(cx, |history, _| history.has_failed())
    );
}
