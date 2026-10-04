//! Opening a node shell over two loaded clusters (see `debug_open_tests` for the fixture): the
//! always-typed node name, the permit before the create, the namespace and labels of the pod, the
//! audit lines, and the setting. A fake never upgrades a connection to a stream: a pod read that
//! finds nothing fails the attach at once, which ends the tab and so runs the cleanup.

use cluster::AccessCheck;
use cluster::fake_api::RecordedRequest;
use gpui_kit::TestAppContext;
use gpui_kit::component::WindowExt as _;

use super::*;
use crate::app_shell::debug_open::debug_open_tests::{
    Answers, CREATED_UID, Debugs, MIRROR_IMAGE, audit_lines, go_live_answering, report_denying,
    respond, two_clusters,
};
use crate::confirm_dialog::ConfirmDialog;
use crate::settings::AppSettings;
use crate::write_guard::{DialogConfirm, WriteLock};

pub(in crate::app_shell) fn creates(api: &cluster::fake_api::FakeApi) -> Vec<RecordedRequest> {
    api.requests()
        .into_iter()
        .filter(|request| request.method == "POST" && request.path.ends_with("/pods"))
        .collect()
}

pub(in crate::app_shell) fn deletes(api: &cluster::fake_api::FakeApi) -> Vec<RecordedRequest> {
    api.requests()
        .into_iter()
        .filter(|request| request.method == "DELETE")
        .collect()
}

fn commits(api: &cluster::fake_api::FakeApi) -> Vec<RecordedRequest> {
    creates(api)
        .into_iter()
        .filter(|request| !request.has_query_key("dryRun"))
        .collect()
}

/// The two clusters with the node shell switched on in both: a guessed Staging and a Production
/// cluster have it off by default.
pub(in crate::app_shell) fn node_clusters(
    name: &str,
    answers: Answers,
    cx: &mut TestAppContext,
) -> Debugs {
    let debugs = two_clusters(name, answers, cx);
    cx.update(|cx| {
        crate::clusters_page::set_allow_node_shell(&debugs.stg, Some(true), cx);
        crate::clusters_page::set_allow_node_shell(&debugs.prod, Some(true), cx);
    });
    cx.run_until_parked();
    debugs
}

/// `node_clusters` with settings that save into a fresh folder, so the audit log has one. The
/// folder is set up first: installing settings resets the registry, and with it the switches.
pub(in crate::app_shell) fn audited_node_clusters(
    name: &str,
    answers: Answers,
    cx: &mut TestAppContext,
) -> (Debugs, std::path::PathBuf) {
    let debugs = two_clusters(name, answers, cx);
    let dir = debugs.audit_folder(name, cx);
    cx.update(|cx| {
        crate::clusters_page::set_allow_node_shell(&debugs.stg, Some(true), cx);
    });
    cx.run_until_parked();
    (debugs, dir)
}

impl Debugs {
    /// The options dialog of the Open node shell item, as the key, the menu, and the palette open it.
    pub(in crate::app_shell) fn open_node_options(
        &self,
        cluster: &ClusterRef,
        node: &str,
        cx: &mut TestAppContext,
    ) {
        self.fixture.with_window(cx, |window, cx| {
            self.fixture.shell.update(cx, |shell, cx| {
                shell.open_node_shell_options(cluster, node, window, cx);
            });
        });
    }

    /// What Continue of the options dialog does.
    pub(in crate::app_shell) fn start_node(
        &self,
        cluster: &ClusterRef,
        node: &str,
        namespace: &str,
        image: &str,
        cx: &mut TestAppContext,
    ) {
        let chosen = NodeShellChosen {
            namespace: namespace.to_owned(),
            image: image.to_owned(),
        };
        self.fixture.with_window(cx, |window, cx| {
            self.fixture.shell.update(cx, |shell, cx| {
                shell.start_node_shell(cluster, node, chosen, window, cx);
            });
        });
    }

    pub(in crate::app_shell) fn run_id(&self, cx: &mut TestAppContext) -> String {
        self.fixture
            .shell
            .read_with(cx, |shell, _| shell.run_id.clone())
    }
}

#[gpui_kit::test]
fn a_node_shell_always_types_the_node_name(cx: &mut TestAppContext) {
    let debugs = node_clusters("ns-types", Answers::Accepts, cx);
    // stg-b is a click tier, and still asks for the node name.
    debugs.start_node(
        &debugs.stg,
        "wk-03",
        "kube-system",
        cluster::DEFAULT_DEBUG_IMAGE,
        cx,
    );
    assert!(debugs.has_dialog(cx));
    let dialog = debugs.dialog(cx);
    let tier = dialog.read_with(cx, |dialog, _| dialog.tier().clone());
    assert_eq!(
        tier,
        DialogConfirm::TypeName {
            expected: "wk-03".to_owned()
        }
    );
    debugs.wait_for_dry_run(&dialog, cx);
    // Until it is typed the button stays off, and nothing was created.
    let block = cx.read(|cx| dialog.read(cx).block_reason(cx));
    assert_eq!(block.as_deref(), Some("Type wk-03 to confirm"));
    assert!(commits(&debugs.stg_api).is_empty());
    // The production cluster types the node name too, not its own name.
    debugs
        .fixture
        .with_window(cx, |window, cx| window.close_dialog(cx));
    debugs.activate(&debugs.prod, cx);
    debugs.set_lock(&debugs.prod, WriteLock::Unlocked, cx);
    debugs.start_node(
        &debugs.prod,
        "wk-03",
        "kube-system",
        cluster::DEFAULT_DEBUG_IMAGE,
        cx,
    );
    let tier = debugs
        .dialog(cx)
        .read_with(cx, |dialog, _| dialog.tier().clone());
    assert_eq!(
        tier,
        DialogConfirm::TypeName {
            expected: "wk-03".to_owned()
        }
    );
}

#[gpui_kit::test]
fn the_dialog_names_the_privileged_change(cx: &mut TestAppContext) {
    let debugs = node_clusters("ns-dialog", Answers::Accepts, cx);
    debugs.start_node(
        &debugs.stg,
        "wk-03",
        "kube-system",
        cluster::DEFAULT_DEBUG_IMAGE,
        cx,
    );
    let dialog = debugs.dialog(cx);
    let (label, warnings, button) = dialog.read_with(cx, |dialog, _| {
        (
            dialog.label(),
            dialog.warning_lines(),
            dialog.confirm_text(),
        )
    });
    assert_eq!(label.as_deref(), Some("Open node shell for wk-03"));
    assert_eq!(button.as_deref(), Some("Open node shell"));
    assert_eq!(
        warnings,
        [crate::debug_dialogs::node_shell_warning("wk-03")]
    );
}

#[gpui_kit::test]
fn a_held_enter_never_confirms_the_node_shell(cx: &mut TestAppContext) {
    use gpui_kit::InputEvent as _;
    use gpui_kit::test::TestWindowExt as _;
    let debugs = node_clusters("ns-enter", Answers::Accepts, cx);
    debugs.start_node(
        &debugs.stg,
        "wk-03",
        "kube-system",
        cluster::DEFAULT_DEBUG_IMAGE,
        cx,
    );
    let dialog = debugs.dialog(cx);
    debugs.wait_for_dry_run(&dialog, cx);
    debugs.fixture.with_window(cx, |window, cx| {
        dialog.update(cx, |dialog, cx| dialog.type_text("wk-03", window, cx));
        window.render_frame(cx);
    });
    let enter = |is_held: bool| gpui_kit::KeyDownEvent {
        keystroke: gpui_kit::Keystroke::parse("enter").expect("a valid keystroke"),
        is_held,
        prefer_character_input: false,
    };
    for _ in 0..3 {
        debugs.fixture.with_window(cx, |window, cx| {
            window.dispatch_event(enter(true).to_platform_input(), cx);
        });
    }
    cx.run_until_parked();
    assert!(
        commits(&debugs.stg_api).is_empty(),
        "the repeat of a held Enter creates nothing"
    );
    assert!(debugs.has_dialog(cx));
    // A fresh Enter confirms.
    debugs.fixture.with_window(cx, |window, cx| {
        window.dispatch_event(enter(false).to_platform_input(), cx);
    });
    debugs.wait_for("the create", cx, |_| !commits(&debugs.stg_api).is_empty());
}

#[gpui_kit::test]
fn the_pod_is_created_on_the_nodes_own_cluster_with_the_labels_the_sweep_selects(
    cx: &mut TestAppContext,
) {
    let debugs = node_clusters("ns-pod", Answers::Waiting, cx);
    debugs.start_node(&debugs.stg, "wk-03", "debug", MIRROR_IMAGE, cx);
    debugs.confirm(cx);
    debugs.wait_for("the tab", cx, |cx| debugs.tab_count(cx) == 1);
    let dry_run = creates(&debugs.stg_api)
        .into_iter()
        .find(|request| request.has_query("dryRun", "All"))
        .expect("a dry-run precedes the commit");
    let commit = commits(&debugs.stg_api).remove(0);
    assert_eq!(commit.path, "/api/v1/namespaces/debug/pods");
    assert!(commit.has_query("fieldManager", "k8sboard"));
    assert_eq!(dry_run.body, commit.body, "the dry-run checks the same pod");
    let body: serde_json::Value = serde_json::from_str(&commit.body).expect("JSON");
    let run_id = debugs.run_id(cx);
    let labels = &body["metadata"]["labels"];
    assert_eq!(labels["app.kubernetes.io/managed-by"], "k8sboard");
    assert_eq!(labels["k8sboard.io/purpose"], "node-shell");
    assert_eq!(labels["k8sboard.io/node"], "wk-03");
    assert_eq!(labels["k8sboard.io/instance"], serde_json::json!(run_id));
    assert_eq!(body["spec"]["nodeName"], "wk-03");
    assert_eq!(body["spec"]["containers"][0]["image"], MIRROR_IMAGE);
    assert_eq!(
        body["spec"]["containers"][0]["securityContext"]["privileged"],
        true
    );
    let tab = debugs.tabs(cx).remove(0);
    let (label, kind) = tab.read_with(cx, |tab, _| (tab.label(), tab.kind().clone()));
    assert_eq!(label, "node shell · wk-03 (debug pod)");
    assert!(kind.is_node_shell());
}

#[gpui_kit::test]
fn the_attach_permit_is_taken_before_the_pod_is_created(cx: &mut TestAppContext) {
    let (debugs, dir) = audited_node_clusters("ns-permit", Answers::Accepts, cx);
    debugs.start_node(
        &debugs.stg,
        "wk-03",
        "kube-system",
        cluster::DEFAULT_DEBUG_IMAGE,
        cx,
    );
    let dialog = debugs.dialog(cx);
    debugs.wait_for_dry_run(&dialog, cx);
    debugs.set_access(
        &debugs.stg,
        report_denying(&[AccessCheck::CreatePodAttach]),
        cx,
    );
    debugs.confirm(cx);
    cx.run_until_parked();
    assert!(commits(&debugs.stg_api).is_empty(), "no permit, no pod");
    assert_eq!(debugs.tab_count(cx), 0);
    assert!(
        audit_lines(&dir).is_empty(),
        "nothing created, nothing audited"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn a_refused_create_opens_no_tab_and_leaves_nothing_to_delete(cx: &mut TestAppContext) {
    let (debugs, dir) = audited_node_clusters("ns-refused", Answers::RefusesCommit, cx);
    debugs.start_node(
        &debugs.stg,
        "wk-03",
        "kube-system",
        cluster::DEFAULT_DEBUG_IMAGE,
        cx,
    );
    debugs.confirm(cx);
    debugs.wait_for("the failed line", cx, |_| audit_lines(&dir).len() == 1);
    assert_eq!(audit_lines(&dir)[0]["outcome"], "failed");
    assert_eq!(debugs.tab_count(cx), 0);
    std::thread::sleep(std::time::Duration::from_millis(100));
    cx.run_until_parked();
    assert!(deletes(&debugs.stg_api).is_empty(), "no pod, no delete");
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn nothing_opens_for_a_locked_cluster_a_windows_node_or_a_cluster_that_turned_it_off(
    cx: &mut TestAppContext,
) {
    let debugs = two_clusters("ns-gate", Answers::Accepts, cx);
    // prod-a is locked at open.
    debugs.activate(&debugs.prod, cx);
    debugs.open_node_options(&debugs.prod, "wk-03", cx);
    assert!(!debugs.has_dialog(cx));
    let stg_api = debugs.activate(&debugs.stg, cx);
    // A Windows node, and a node that is not listed.
    debugs.open_node_options(&debugs.stg, "win-01", cx);
    assert!(!debugs.has_dialog(cx));
    debugs.open_node_options(&debugs.stg, "gone-01", cx);
    assert!(!debugs.has_dialog(cx));
    // The setting off: stg-b is a guessed Staging, so the default is off already.
    debugs.open_node_options(&debugs.stg, "wk-03", cx);
    assert!(
        !debugs.has_dialog(cx),
        "a guessed Staging has the shell off"
    );
    // Turning it on opens the options dialog; a denied right closes the door again.
    cx.update(|cx| crate::clusters_page::set_allow_node_shell(&debugs.stg, Some(true), cx));
    cx.run_until_parked();
    debugs.open_node_options(&debugs.stg, "wk-03", cx);
    assert!(debugs.has_dialog(cx));
    debugs
        .fixture
        .with_window(cx, |window, cx| window.close_dialog(cx));
    debugs.set_access(&debugs.stg, report_denying(&[AccessCheck::DeletePods]), cx);
    debugs.open_node_options(&debugs.stg, "wk-03", cx);
    assert!(!debugs.has_dialog(cx));
    assert!(creates(&stg_api).is_empty());
}

#[gpui_kit::test]
fn options_persist_image_and_namespace_after_start(cx: &mut TestAppContext) {
    let debugs = node_clusters("ns-persist", Answers::Waiting, cx);
    let stored = |cx: &mut TestAppContext| {
        cx.read(|cx| {
            AppSettings::get(cx)
                .registry
                .clusters
                .iter()
                .find(|entry| entry.cluster == debugs.stg)
                .map(|entry| {
                    (
                        entry.debug_image.clone(),
                        entry.node_shell_namespace.clone(),
                    )
                })
        })
    };
    // The defaults are not written down.
    debugs.start_node(
        &debugs.stg,
        "wk-03",
        "kube-system",
        cluster::DEFAULT_DEBUG_IMAGE,
        cx,
    );
    debugs.confirm(cx);
    debugs.wait_for("the first tab", cx, |cx| debugs.tab_count(cx) == 1);
    assert_eq!(stored(cx), Some((None, None)));
    // A mirror and another namespace are, once the start went through, and not before.
    debugs.start_node(&debugs.stg, "wk-03", "debug", MIRROR_IMAGE, cx);
    assert_eq!(stored(cx), Some((None, None)));
    debugs.confirm(cx);
    debugs.wait_for("the second tab", cx, |cx| debugs.tab_count(cx) == 2);
    assert_eq!(
        stored(cx),
        Some((Some(MIRROR_IMAGE.to_owned()), Some("debug".to_owned())))
    );
}

#[gpui_kit::test]
fn the_audit_follows_the_pod_from_create_to_delete(cx: &mut TestAppContext) {
    // The fake finds no pod to wait for, so the attach fails at once and the cleanup runs. The
    // Open line (queued by the main thread) and the Delete line (queued by the tokio runtime) go
    // through the one audit writer, so their order is the order they were queued in.
    let (debugs, dir) = audited_node_clusters("ns-audit", Answers::Accepts, cx);
    debugs.start_node(
        &debugs.stg,
        "wk-03",
        "kube-system",
        cluster::DEFAULT_DEBUG_IMAGE,
        cx,
    );
    debugs.confirm(cx);
    debugs.wait_for("three lines", cx, |_| audit_lines(&dir).len() == 3);
    let lines = audit_lines(&dir);
    assert_eq!(lines[0]["action"], "Create node shell pod");
    assert_eq!(lines[0]["outcome"], "applied");
    assert_eq!(lines[0]["cluster"], "stg-b");
    assert_eq!(lines[1]["action"], "Open node shell");
    assert_eq!(lines[1]["outcome"], "failed");
    assert_eq!(lines[2]["action"], "Delete node shell pod");
    assert_eq!(lines[2]["outcome"], "applied");
    // The same pod in all three, and no pod body or session text: fields name paths and the few
    // values the spec lists (node, image, privilege).
    for line in &lines {
        assert_eq!(line["object"]["kind"], "Pod");
        assert_eq!(line["object"]["namespace"], "kube-system");
    }
    let text = serde_json::to_string(&lines).expect("serializes");
    assert!(!text.contains("nsenter"), "{text}");
    // The delete carried the uid the server reported, with no grace and no dry-run.
    let delete = deletes(&debugs.stg_api).remove(0);
    assert!(!delete.has_query_key("dryRun"));
    let body: serde_json::Value = serde_json::from_str(&delete.body).expect("JSON");
    assert_eq!(body["preconditions"]["uid"], CREATED_UID);
    assert_eq!(body["gracePeriodSeconds"], 0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn a_cap_of_tabs_stops_a_node_shell_before_anything_is_created(cx: &mut TestAppContext) {
    let debugs = node_clusters("ns-cap", Answers::Waiting, cx);
    for _ in 0..crate::dock::MAX_SHELL_TABS {
        debugs.start_node(
            &debugs.stg,
            "wk-03",
            "kube-system",
            cluster::DEFAULT_DEBUG_IMAGE,
            cx,
        );
        debugs.confirm(cx);
    }
    debugs.wait_for("eight tabs", cx, |cx| {
        debugs.tab_count(cx) == crate::dock::MAX_SHELL_TABS
    });
    let before = commits(&debugs.stg_api).len();
    debugs.start_node(
        &debugs.stg,
        "wk-03",
        "kube-system",
        cluster::DEFAULT_DEBUG_IMAGE,
        cx,
    );
    assert!(!debugs.has_dialog(cx), "no dialog past the cap");
    assert_eq!(commits(&debugs.stg_api).len(), before);
}

#[gpui_kit::test]
fn s_on_a_node_menu_and_palette_share_the_arm(cx: &mut TestAppContext) {
    use crate::resource_actions::{ResourceAction, RowAction, subject_action};
    use crate::table_selection::{ClusterObject, ResourceKey};
    let debugs = node_clusters("ns-arm", Answers::Accepts, cx);
    let key = ResourceKey::Node {
        name: "wk-03".to_owned(),
    };
    // The key, the node menu item, and the palette entry all dispatch the one row action.
    assert_eq!(
        subject_action(RowAction::OpenShell, &key),
        Some(ResourceAction::OpenNodeShell)
    );
    assert_eq!(
        ResourceAction::OpenNodeShell.row_action(),
        RowAction::OpenShell
    );
    // And the arm opens the options dialog for the cursor node, in the cursor's own cluster.
    debugs.fixture.shell.update(cx, |shell, _| {
        shell.selected = Some(ClusterObject {
            cluster: debugs.stg.clone(),
            key,
        });
    });
    assert!(!debugs.has_dialog(cx));
    debugs.fixture.with_window(cx, |window, cx| {
        debugs.fixture.shell.update(cx, |shell, cx| {
            shell.run_row_key(RowAction::OpenShell, window, cx);
        });
    });
    assert!(debugs.has_dialog(cx), "the options dialog of the node");
    assert!(
        creates(&debugs.stg_api).is_empty(),
        "nothing is created by opening it"
    );
}

#[gpui_kit::test]
fn node_shell_uses_the_rows_cluster(cx: &mut TestAppContext) {
    // A node of the second cluster: its guard, tier, connection, and audit line are that cluster's.
    let debugs = node_clusters("ns-rows", Answers::Waiting, cx);
    let prod_api = debugs.activate(&debugs.prod, cx);
    debugs.set_lock(&debugs.prod, WriteLock::Unlocked, cx);
    debugs.start_node(
        &debugs.prod,
        "wk-03",
        "kube-system",
        cluster::DEFAULT_DEBUG_IMAGE,
        cx,
    );
    debugs.confirm(cx);
    debugs.wait_for("the tab", cx, |cx| debugs.tab_count(cx) == 1);
    assert_eq!(creates(&prod_api).len(), 2, "a dry-run and the commit");
    assert!(creates(&debugs.stg_api).is_empty());
    let tab = debugs.tabs(cx).remove(0);
    let (cluster, label) = tab.read_with(cx, |tab, _| {
        (tab.cluster().clone(), tab.cluster_label().to_owned())
    });
    assert_eq!(cluster, debugs.prod);
    assert_eq!(label, "prod-a");
    // The test holds no handle to the tab, so closing it releases it.
    drop(tab);
    // Closing it deletes on the same cluster.
    debugs.fixture.shell.update(cx, |shell, cx| {
        shell.dock.update(cx, |dock, cx| dock.close_active_tab(cx));
    });
    debugs.wait_for("the delete", cx, |_| !deletes(&prod_api).is_empty());
    assert!(deletes(&debugs.stg_api).is_empty());
}

#[gpui_kit::test]
fn a_node_shell_create_landing_after_a_switch_opens_no_tab(cx: &mut TestAppContext) {
    use std::sync::{Mutex, mpsc};
    use std::time::Duration;
    let (debugs, dir) = audited_node_clusters("ns-mid-create", Answers::Waiting, cx);
    // The commit of the create on stg-b waits until the shell has switched to prod-a.
    let (release, gate) = mpsc::channel::<()>();
    let gate = Mutex::new(gate);
    let held_api = go_live_answering(
        &debugs.fixture,
        &debugs.stg,
        move |request| {
            let is_commit = request.method == "POST"
                && request.path.ends_with("/pods")
                && !request.has_query_key("dryRun");
            if is_commit && let Ok(gate) = gate.lock() {
                let _ = gate.recv_timeout(Duration::from_secs(10));
            }
            respond(Answers::Waiting, request)
        },
        cx,
    );
    debugs.start_node(
        &debugs.stg,
        "wk-03",
        "kube-system",
        cluster::DEFAULT_DEBUG_IMAGE,
        cx,
    );
    debugs.confirm(cx);
    let prod_api = debugs.activate(&debugs.prod, cx);
    let _ = release.send(());
    // The pod exists on stg-b but nothing owns it: it is deleted on the connection the create held.
    debugs.wait_for("the delete", cx, |_| !deletes(&held_api).is_empty());
    debugs.wait_for("the audit line", cx, |_| {
        audit_lines(&dir)
            .iter()
            .any(|line| line["action"] == "Delete node shell pod")
    });
    assert_eq!(debugs.tab_count(cx), 0, "no tab for a cluster that left");
    let delete_line = audit_lines(&dir)
        .into_iter()
        .find(|line| line["action"] == "Delete node shell pod")
        .expect("the delete is audited");
    assert_eq!(delete_line["cluster"], "stg-b");
    assert!(deletes(&prod_api).is_empty());
    assert!(creates(&prod_api).is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

// ---- the gate is read again at confirm time ----

/// Opens the node shell dialog on stg-b, waits for the dry-run, and types the node name: the user
/// is one press from the create.
fn ready_to_confirm(debugs: &Debugs, cx: &mut TestAppContext) -> gpui_kit::Entity<ConfirmDialog> {
    debugs.start_node(
        &debugs.stg,
        "wk-03",
        "kube-system",
        cluster::DEFAULT_DEBUG_IMAGE,
        cx,
    );
    let dialog = debugs.dialog(cx);
    debugs.wait_for_dry_run(&dialog, cx);
    debugs.fixture.with_window(cx, |window, cx| {
        dialog.update(cx, |dialog, cx| dialog.type_text("wk-03", window, cx));
    });
    assert_eq!(cx.read(|cx| dialog.read(cx).block_reason(cx)), None);
    dialog
}

fn press_and_settle(
    debugs: &Debugs,
    dialog: &gpui_kit::Entity<ConfirmDialog>,
    cx: &mut TestAppContext,
) {
    debugs.fixture.with_window(cx, |window, cx| {
        dialog.update(cx, |dialog, cx| dialog.press_confirm(window, cx));
    });
    std::thread::sleep(std::time::Duration::from_millis(150));
    cx.run_until_parked();
}

#[gpui_kit::test]
fn turning_the_setting_off_while_the_dialog_is_open_creates_nothing(cx: &mut TestAppContext) {
    let debugs = node_clusters("ns-late-off", Answers::Waiting, cx);
    let dialog = ready_to_confirm(&debugs, cx);
    cx.update(|cx| crate::clusters_page::set_allow_node_shell(&debugs.stg, Some(false), cx));
    cx.run_until_parked();
    let block = cx.read(|cx| dialog.read(cx).block_reason(cx));
    assert_eq!(
        block.as_deref(),
        Some("Node shell is off for stg-b (Settings › Clusters › Safety)")
    );
    press_and_settle(&debugs, &dialog, cx);
    assert!(
        commits(&debugs.stg_api).is_empty(),
        "no pod after the switch went off"
    );
    assert_eq!(debugs.tab_count(cx), 0);
}

#[gpui_kit::test]
fn making_the_cluster_production_while_the_dialog_is_open_creates_nothing(cx: &mut TestAppContext) {
    let debugs = node_clusters("ns-late-prod", Answers::Waiting, cx);
    let dialog = ready_to_confirm(&debugs, cx);
    // The explicit switch goes, and the environment is now Production: off by default.
    cx.update(|cx| {
        crate::settings::AppSettings::update(cx, |settings| {
            crate::cluster_form::edit_entry(&mut settings.registry, &debugs.stg, |entry| {
                entry.allow_node_shell = None;
                entry.environment = Some(crate::environment::Environment::Production);
            });
        });
    });
    cx.run_until_parked();
    assert!(cx.read(|cx| dialog.read(cx).block_reason(cx)).is_some());
    press_and_settle(&debugs, &dialog, cx);
    assert!(commits(&debugs.stg_api).is_empty());
    assert_eq!(debugs.tab_count(cx), 0);
}

#[gpui_kit::test]
fn a_revoked_create_right_while_the_dialog_is_open_creates_nothing(cx: &mut TestAppContext) {
    for (name, denied) in [
        ("ns-late-create", AccessCheck::CreatePods),
        ("ns-late-delete", AccessCheck::DeletePods),
    ] {
        let debugs = node_clusters(name, Answers::Waiting, cx);
        let dialog = ready_to_confirm(&debugs, cx);
        debugs.set_access(&debugs.stg, report_denying(&[denied]), cx);
        let block = cx.read(|cx| dialog.read(cx).block_reason(cx));
        assert!(
            block
                .as_deref()
                .is_some_and(|text| text.starts_with("Not permitted")),
            "{denied:?}: {block:?}"
        );
        press_and_settle(&debugs, &dialog, cx);
        assert!(commits(&debugs.stg_api).is_empty(), "{denied:?}");
    }
}

#[gpui_kit::test]
fn the_commit_itself_refuses_a_stale_yes(cx: &mut TestAppContext) {
    use crate::app_shell::write_flow::{ConnectCommit, DryRunState, TypedMatch, confirmed};
    let debugs = node_clusters("ns-late-direct", Answers::Waiting, cx);
    let dialog = ready_to_confirm(&debugs, cx);
    let intent = cx
        .read(|cx| dialog.read(cx).connect_intent())
        .expect("a start");
    let generation = cx.read(|cx| dialog.read(cx).generation());
    let proof = confirmed(
        &DryRunState::Passed {
            elapsed: std::time::Duration::ZERO,
        },
        TypedMatch::Matches,
        generation,
    );
    // The dialog's own check is bypassed: the commit must refuse on its own.
    cx.update(|cx| crate::clusters_page::set_allow_node_shell(&debugs.stg, Some(false), cx));
    cx.run_until_parked();
    debugs.fixture.with_window(cx, |window, cx| {
        debugs.fixture.shell.update(cx, |shell, cx| {
            let commit = ConnectCommit {
                generation,
                confirmed: proof,
                note: None,
            };
            shell.commit_connect(&intent, commit, window, cx);
        });
    });
    std::thread::sleep(std::time::Duration::from_millis(150));
    cx.run_until_parked();
    assert!(commits(&debugs.stg_api).is_empty());
    assert_eq!(debugs.tab_count(cx), 0);
}

impl Debugs {
    /// The connection of a viewed cluster, as a tab or a cleanup would hold it.
    pub(in crate::app_shell) fn session_connection(
        &self,
        cluster: &ClusterRef,
        cx: &mut TestAppContext,
    ) -> cluster::ClusterConnection {
        self.fixture
            .shell
            .read_with(cx, |shell, cx| shell.slot_connection(cluster, cx))
            .expect("a live slot")
    }

    /// The proof a started debug shell may attach, from a report that allows both verbs.
    pub(in crate::app_shell) fn attach_permit(&self) -> cluster::AttachPermit {
        report_denying(&[])
            .attach_permit()
            .expect("both attach verbs allowed")
    }
}
