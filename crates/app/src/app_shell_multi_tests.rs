//! Viewing several clusters at once in a headless window. The fixture kubeconfig of the switch
//! tests points at a closed local port, so a connect fails fast; a slot that must be Live is made
//! so over a client that never connects (`go_live_for_test`), and its pods are seeded.

use cluster::{PodStatus, PodSummary, ReadyCount, StatusReason};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{Entity, TestAppContext};

use super::app_shell_switch_tests::{
    FIXTURE_YAML, SwitchFixture, chord, open_switch_fixture, open_switch_fixture_over,
    open_switch_fixture_with,
};
use super::workspace::SlotNotice;
use super::*;
use crate::cluster_switcher::ToggleClusterTick;
use crate::kubelet_metrics::KubeletDemand;
use crate::log_target::LogTarget;
use crate::palette_search::PaletteTarget;
use crate::write_guard::{ActionRisk, DialogConfirm, WriteLock, confirm_step};
use cluster::ClusterConnection;

fn pod(namespace: &str, name: &str, node: &str) -> PodSummary {
    PodSummary {
        namespace: namespace.to_owned(),
        name: name.to_owned(),
        status: PodStatus::Reason(StatusReason::Running),
        ready: ReadyCount { ready: 1, total: 1 },
        restarts: 0,
        node_name: Some(node.to_owned()),
        created_at: None,
        pod_ip: None,
        qos_class: None,
        service_account: None,
        controller: None,
        conditions: Vec::new(),
        containers: Vec::new(),
        status_message: None,
        labels: Vec::new(),
        host_network: false,
        image_pull_secrets: Vec::new(),
    }
}

fn pod_key(name: &str) -> ResourceKey {
    ResourceKey::Pod {
        namespace: "shop".to_owned(),
        name: name.to_owned(),
    }
}

/// The contexts of the viewed slots, in slot order.
fn viewed(fixture: &SwitchFixture, cx: &mut TestAppContext) -> Vec<String> {
    fixture.shell.read_with(cx, |shell, _| {
        shell
            .view
            .slots()
            .iter()
            .map(|slot| slot.cluster.context.clone())
            .collect()
    })
}

fn view(fixture: &SwitchFixture, contexts: &[&str], cx: &mut TestAppContext) {
    let wanted: Vec<ClusterRef> = contexts
        .iter()
        .map(|context| fixture.cluster(context, cx))
        .collect();
    fixture
        .shell
        .update(cx, |shell, cx| shell.view_clusters(&wanted, cx));
    cx.run_until_parked();
}

fn slot_session(
    fixture: &SwitchFixture,
    context: &str,
    cx: &mut TestAppContext,
) -> Entity<ClusterSession> {
    let cluster = fixture.cluster(context, cx);
    fixture
        .shell
        .read_with(cx, |shell, _| shell.slot_session(&cluster).cloned())
        .expect("a viewed slot")
}

/// Makes the session of `context` Live, as if its connect had answered.
fn go_live(fixture: &SwitchFixture, context: &str, scope: NamespaceScope, cx: &mut TestAppContext) {
    let kubeconfig = Kubeconfig::parse(FIXTURE_YAML, &fixture.path).expect("the fixture parses");
    let connection = fixture
        .runtime
        .block_on(ClusterConnection::open(&kubeconfig, context))
        .expect("a client builds without a round trip");
    let session = slot_session(fixture, context, cx);
    session.update(cx, |session, cx| {
        session.go_live_for_test(connection, scope, cx)
    });
    cx.run_until_parked();
}

fn seed_pods(
    fixture: &SwitchFixture,
    context: &str,
    pods: Vec<PodSummary>,
    cx: &mut TestAppContext,
) {
    let session = slot_session(fixture, context, cx);
    session.update(cx, |session, cx| session.set_pods_for_test(pods, cx));
    cx.run_until_parked();
}

/// Two viewed clusters, both Live, with the pod `api-0` in each; the Pods screen is shown.
fn two_live_clusters(name: &str, cx: &mut TestAppContext) -> SwitchFixture {
    let fixture = open_switch_fixture(name, cx);
    view(&fixture, &["prod-a", "stg-b"], cx);
    for (context, node) in [("prod-a", "node-a"), ("stg-b", "node-b")] {
        go_live(&fixture, context, NamespaceScope::All, cx);
        seed_pods(&fixture, context, vec![pod("shop", "api-0", node)], cx);
    }
    fixture
        .shell
        .update(cx, |shell, cx| shell.show_screen(Screen::Pods, cx));
    cx.run_until_parked();
    fixture.draw_twice(cx);
    fixture
}

fn select_pod_row(fixture: &SwitchFixture, row: usize, cx: &mut TestAppContext) {
    let table = fixture
        .shell
        .read_with(cx, |shell, _| shell.pod_table.clone());
    cx.update(|cx| table.update(cx, |table, cx| table.set_selected_row(row, cx)));
    cx.run_until_parked();
}

fn selected_context(fixture: &SwitchFixture, cx: &mut TestAppContext) -> Option<String> {
    fixture.shell.read_with(cx, |shell, _| {
        shell
            .selected
            .as_ref()
            .map(|object| object.cluster.context.clone())
    })
}

// ---- Step 1: the view model ----

#[gpui_kit::test]
fn apply_keeps_the_staying_session(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture("keep-staying", cx);
    view(&fixture, &["prod-a", "stg-b"], cx);
    assert_eq!(viewed(&fixture, cx), ["prod-a", "stg-b"]);
    let staying = slot_session(&fixture, "stg-b", cx).entity_id();
    view(&fixture, &["stg-b", "dev-c"], cx);
    assert_eq!(viewed(&fixture, cx), ["stg-b", "dev-c"]);
    assert_eq!(slot_session(&fixture, "stg-b", cx).entity_id(), staying);
}

#[gpui_kit::test]
fn apply_releases_before_connecting(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture("release-first", cx);
    view(&fixture, &["prod-a", "stg-b"], cx);
    view(&fixture, &["stg-b", "dev-c"], cx);
    let check = fixture.shell.read_with(cx, |shell, _| {
        let last = shell.view_connects.last().expect("a connect ran");
        (last.released_gone, last.slots + last.connects)
    });
    // `prod-a` was gone when `dev-c` connected, and never more than five sessions exist.
    assert_eq!(check, (true, 2));
    assert!(check.1 <= crate::cluster_view::MAX_VIEWED_CLUSTERS);
}

#[gpui_kit::test]
fn single_switch_from_multi_releases_all(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture("single-from-multi", cx);
    view(&fixture, &["prod-a", "stg-b"], cx);
    let released = [
        slot_session(&fixture, "prod-a", cx).downgrade(),
        slot_session(&fixture, "stg-b", cx).downgrade(),
    ];
    // Ctrl 3 is the third row of the switcher, `dev-c`, which is not viewed: every viewed session
    // is released.
    fixture.press(&chord("3"), cx);
    cx.run_until_parked();
    fixture.draw_twice(cx);
    assert_eq!(viewed(&fixture, cx), ["dev-c"]);
    assert!(
        released.iter().all(|session| session.upgrade().is_none()),
        "an old session is still held"
    );
}

#[gpui_kit::test]
fn fan_out_reaches_every_slot(cx: &mut TestAppContext) {
    let fixture = two_live_clusters("fan-out", cx);
    let scope = NamespaceScope::Named("web".to_owned());
    fixture.shell.update(cx, |shell, cx| {
        shell.apply_namespace_scope(scope.clone(), cx);
        shell.show_screen(Screen::Kind(ResourceKind::Events), cx);
    });
    for context in ["prod-a", "stg-b"] {
        let session = slot_session(&fixture, context, cx);
        session.update(cx, |session, cx| session.seed_explorer(cx));
    }
    fixture.shell.update(cx, |shell, cx| {
        shell.toggle_warnings_only(cx);
        shell.toggle_explorer_paused(cx);
    });
    for context in ["prod-a", "stg-b"] {
        let session = slot_session(&fixture, context, cx);
        cx.read_entity(&session, |session, _| {
            let live = session.live().expect("a live session");
            assert_eq!(live.scope, scope, "{context}");
            assert!(
                live.kind_list(ResourceKind::Events).is_some(),
                "{context} follows the explorer kind"
            );
            assert_eq!(
                session.event_filter(),
                EventFilter::WarningsOnly,
                "{context}"
            );
        });
    }
    // The pause reached both lists, which were reloaded by the filter before: seed again, pause.
    for context in ["prod-a", "stg-b"] {
        let session = slot_session(&fixture, context, cx);
        session.update(cx, |session, cx| session.seed_explorer(cx));
    }
    fixture
        .shell
        .update(cx, |shell, cx| shell.toggle_explorer_paused(cx));
    for context in ["prod-a", "stg-b"] {
        let session = slot_session(&fixture, context, cx);
        cx.read_entity(&session, |session, _| {
            let flow = session.live().and_then(LiveCluster::explorer_flow);
            assert!(matches!(flow, Some(FlowState::Paused { .. })), "{context}");
        });
    }
}

#[gpui_kit::test]
fn slot_going_live_gets_the_view_scope(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture("view-scope", cx);
    view(&fixture, &["prod-a", "stg-b"], cx);
    let web = NamespaceScope::Named("web".to_owned());
    // The primary decides the scope of the view.
    go_live(&fixture, "prod-a", web.clone(), cx);
    // The other cluster answers with its own default; the view scope replaces it.
    go_live(&fixture, "stg-b", NamespaceScope::All, cx);
    let session = slot_session(&fixture, "stg-b", cx);
    let scope = cx.read_entity(&session, |session, _| {
        session.live().map(|live| live.scope.clone())
    });
    assert_eq!(scope, Some(web));
}

fn last_used(cx: &mut TestAppContext) -> Option<ClusterRef> {
    cx.update(|cx| AppSettings::get(cx).registry.last_used.clone())
}

#[gpui_kit::test]
fn last_used_is_written_on_live(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture("last-used-live", cx);
    assert_eq!(last_used(cx), None);
    go_live(&fixture, "prod-a", NamespaceScope::All, cx);
    assert_eq!(last_used(cx), Some(fixture.cluster("prod-a", cx)));
}

#[gpui_kit::test]
fn last_used_is_not_written_on_failure(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture("last-used-failed", cx);
    fixture.wait_until_failed(cx);
    assert_eq!(last_used(cx), None);
}

#[gpui_kit::test]
fn last_used_is_written_once_per_session(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture("last-used-once", cx);
    go_live(&fixture, "prod-a", NamespaceScope::All, cx);
    // Another cluster is saved meanwhile; the same session going on does not write it back.
    let other = fixture.cluster("dev-c", cx);
    cx.update(|cx| {
        AppSettings::update(cx, |settings| {
            settings.registry.last_used = Some(other.clone())
        });
    });
    fixture
        .session(cx)
        .update(cx, |session, cx| session.set_pods_for_test(Vec::new(), cx));
    cx.run_until_parked();
    assert_eq!(last_used(cx), Some(other));
}

#[gpui_kit::test]
fn last_used_follows_the_primary(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture("last-used-primary", cx);
    view(&fixture, &["prod-a", "stg-b"], cx);
    go_live(&fixture, "prod-a", NamespaceScope::All, cx);
    go_live(&fixture, "stg-b", NamespaceScope::All, cx);
    // A viewed cluster that is not the primary does not write it.
    assert_eq!(last_used(cx), Some(fixture.cluster("prod-a", cx)));
    // The primary leaves: `stg-b` is first in display order and already live, so it is saved.
    view(&fixture, &["stg-b", "dev-c"], cx);
    assert_eq!(last_used(cx), Some(fixture.cluster("stg-b", cx)));
}

fn quick_filter_text(fixture: &SwitchFixture, cx: &mut TestAppContext) -> Option<String> {
    fixture.shell.read_with(cx, |shell, cx| {
        shell.toolkit_state(cx).map(|state| state.text.to_string())
    })
}

#[gpui_kit::test]
fn filters_kept_when_primary_stays(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture("filters-kept", cx);
    fixture.shell.update(cx, |shell, cx| {
        shell.show_screen(Screen::Pods, cx);
        shell.update_view(cx, |view| view.filter.text = "api".into());
    });
    view(&fixture, &["prod-a", "stg-b"], cx);
    assert_eq!(quick_filter_text(&fixture, cx).as_deref(), Some("api"));
}

#[gpui_kit::test]
fn filters_reset_on_new_primary(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture("filters-reset", cx);
    fixture.shell.update(cx, |shell, cx| {
        shell.show_screen(Screen::Pods, cx);
        shell.update_view(cx, |view| view.filter.text = "api".into());
    });
    // `prod-a` leaves; `stg-b` and `dev-c` start with a new primary, `stg-b`.
    view(&fixture, &["stg-b", "dev-c"], cx);
    assert_eq!(quick_filter_text(&fixture, cx).as_deref(), Some(""));
}

#[gpui_kit::test]
fn primary_follows_the_current_cluster(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture("primary-rule", cx);
    // `prod-a` is where the user stands, and first in display order; `dev-c` is added.
    view(&fixture, &["dev-c", "prod-a"], cx);
    let primary = fixture
        .shell
        .read_with(cx, |shell, _| shell.primary_cluster());
    assert_eq!(primary, Some(fixture.cluster("prod-a", cx)));
    // The current cluster leaves: the first in display order is primary.
    view(&fixture, &["dev-c", "stg-b"], cx);
    let primary = fixture
        .shell
        .read_with(cx, |shell, _| shell.primary_cluster());
    assert_eq!(primary, Some(fixture.cluster("stg-b", cx)));
}

#[gpui_kit::test]
fn a_missing_cluster_in_the_set_is_dropped_with_a_notice(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture("missing-in-set", cx);
    let ghost = ClusterRef {
        kubeconfig: fixture.path.clone(),
        context: "ghost".to_owned(),
    };
    let wanted = [
        fixture.cluster("prod-a", cx),
        fixture.cluster("stg-b", cx),
        ghost,
    ];
    fixture
        .shell
        .update(cx, |shell, cx| shell.view_clusters(&wanted, cx));
    cx.run_until_parked();
    assert_eq!(viewed(&fixture, cx), ["prod-a", "stg-b"]);
    let notices = cx.update(|cx| fixture.shell.read(cx).notices(cx));
    assert_eq!(notices, ["'ghost' is no longer in its kubeconfig"]);
}

// ---- Step 2: the switcher ticks ----

fn ticked(fixture: &SwitchFixture, cx: &mut TestAppContext) -> Vec<String> {
    fixture.shell.read_with(cx, |shell, _| {
        shell
            .switcher
            .ticked()
            .iter()
            .map(|cluster| cluster.context.clone())
            .collect()
    })
}

#[gpui_kit::test]
fn ticks_start_as_viewed_set(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture("ticks-start", cx);
    fixture.open_switcher(cx);
    // One cluster: the draft is that one.
    assert_eq!(ticked(&fixture, cx), ["prod-a"]);
    fixture
        .shell
        .update(cx, |shell, cx| shell.close_cluster_switcher(cx));
    view(&fixture, &["prod-a", "stg-b"], cx);
    fixture.open_switcher(cx);
    assert_eq!(ticked(&fixture, cx), ["prod-a", "stg-b"]);
}

#[gpui_kit::test]
fn footer_only_when_ticks_differ(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture("footer", cx);
    fixture.open_switcher(cx);
    fixture.draw_twice(cx);
    assert!(!fixture.is_drawn("switcher-apply", cx));
    let stg = fixture.cluster("stg-b", cx);
    fixture
        .shell
        .update(cx, |shell, cx| shell.toggle_cluster_tick(&stg, cx));
    fixture.draw_twice(cx);
    assert!(fixture.is_drawn("switcher-apply", cx));
    // Unticking again leaves nothing to apply.
    fixture
        .shell
        .update(cx, |shell, cx| shell.toggle_cluster_tick(&stg, cx));
    fixture.draw_twice(cx);
    assert!(!fixture.is_drawn("switcher-apply", cx));
}

#[gpui_kit::test]
fn space_ticks_in_filter(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture("space-filter", cx);
    fixture.open_switcher(cx);
    // The filter has the focus; the highlight moves to `stg-b`, the second row.
    fixture.press("down", cx);
    assert_eq!(fixture.highlight(cx).as_deref(), Some("stg-b"));
    fixture.press("space", cx);
    assert_eq!(ticked(&fixture, cx), ["prod-a", "stg-b"]);
    // The space did not type, and the kit popover did not take it as Confirm.
    let text = fixture.shell.read_with(cx, |shell, cx| {
        shell.switcher.filter().read(cx).value().to_string()
    });
    assert_eq!(text, "");
    assert!(fixture.is_switcher_open(cx));
}

#[gpui_kit::test]
fn space_ticks_on_row(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture("space-row", cx);
    fixture.open_switcher(cx);
    fixture.draw_twice(cx);
    // Tab walks off the filter onto the buttons of the popover.
    let is_in_filter = |fixture: &SwitchFixture, cx: &mut TestAppContext| {
        fixture.with_window(cx, |window, cx| {
            let filter = fixture.shell.read(cx).switcher.filter().clone();
            filter.read(cx).focus_handle(cx).is_focused(window)
        })
    };
    for _ in 0..8 {
        if !is_in_filter(&fixture, cx) {
            break;
        }
        fixture.press("tab", cx);
    }
    assert!(!is_in_filter(&fixture, cx), "the focus left the filter");
    let before = ticked(&fixture, cx);
    fixture.press("space", cx);
    assert_ne!(ticked(&fixture, cx), before);
    assert!(fixture.is_switcher_open(cx));
}

#[gpui_kit::test]
fn space_binding_belongs_to_the_switcher_contexts(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture("space-action", cx);
    fixture.open_switcher(cx);
    let is_available = fixture.with_window(cx, |window, cx| {
        window.is_action_available(&ToggleClusterTick, cx)
    });
    assert!(is_available);
}

#[gpui_kit::test]
fn enter_applies_when_ticks_differ(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture("enter-applies", cx);
    fixture.open_switcher(cx);
    let stg = fixture.cluster("stg-b", cx);
    fixture
        .shell
        .update(cx, |shell, cx| shell.toggle_cluster_tick(&stg, cx));
    fixture.press("enter", cx);
    cx.run_until_parked();
    assert_eq!(viewed(&fixture, cx), ["prod-a", "stg-b"]);
    assert!(!fixture.is_switcher_open(cx));
}

#[gpui_kit::test]
fn enter_switches_when_ticks_equal(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture("enter-switches", cx);
    fixture.open_switcher(cx);
    fixture.press("down", cx);
    assert_eq!(fixture.highlight(cx).as_deref(), Some("stg-b"));
    fixture.press("enter", cx);
    cx.run_until_parked();
    assert_eq!(viewed(&fixture, cx), ["stg-b"]);
}

#[gpui_kit::test]
fn ticking_never_connects(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture("tick-no-connect", cx);
    fixture.open_switcher(cx);
    let (before_scopes, before_view) = fixture.shell.read_with(cx, |shell, _| {
        (shell.connected_scopes.len(), shell.view.clusters())
    });
    for context in ["stg-b", "dev-c"] {
        let cluster = fixture.cluster(context, cx);
        fixture
            .shell
            .update(cx, |shell, cx| shell.toggle_cluster_tick(&cluster, cx));
    }
    cx.run_until_parked();
    fixture.shell.read_with(cx, |shell, _| {
        assert_eq!(shell.connected_scopes.len(), before_scopes);
        assert_eq!(shell.view.clusters(), before_view);
        assert!(shell.view_connects.is_empty());
    });
}

// ---- Step 3: merged rows and per-slot state ----

#[gpui_kit::test]
fn header_counts_all_clusters_headless(cx: &mut TestAppContext) {
    let fixture = two_live_clusters("header-count", cx);
    seed_pods(
        &fixture,
        "stg-b",
        vec![
            pod("shop", "api-0", "node-b"),
            pod("shop", "api-1", "node-b"),
        ],
        cx,
    );
    let text = fixture
        .shell
        .read_with(cx, |shell, cx| shell.multi_header_count(cx));
    assert_eq!(text.as_deref(), Some("2 clusters · 3 pods"));
}

#[gpui_kit::test]
fn loading_only_until_some_slot_is_ready(cx: &mut TestAppContext) {
    use gpui_kit::component::table::TableDelegate as _;
    let fixture = open_switch_fixture("loading", cx);
    view(&fixture, &["prod-a", "stg-b"], cx);
    go_live(&fixture, "prod-a", NamespaceScope::All, cx);
    go_live(&fixture, "stg-b", NamespaceScope::All, cx);
    let is_loading = |cx: &mut TestAppContext| {
        cx.update(|cx| {
            let table = fixture.shell.read(cx).pod_table.clone();
            table.read(cx).delegate().loading(cx)
        })
    };
    // Both lists still wait for their first snapshot.
    assert!(is_loading(cx));
    // One ready list is enough to show rows.
    seed_pods(&fixture, "stg-b", vec![pod("shop", "api-0", "node-b")], cx);
    assert!(!is_loading(cx));
}

#[gpui_kit::test]
fn merged_rows_keep_the_same_pod_of_two_clusters_apart(cx: &mut TestAppContext) {
    let fixture = two_live_clusters("same-pod", cx);
    let rows = fixture.shell.read_with(cx, |shell, cx| {
        shell
            .toolkit_state(cx)
            .map(|state| (state.shown, state.total))
    });
    assert_eq!(rows, Some((2, 2)));
}

#[gpui_kit::test]
fn selection_is_cluster_qualified(cx: &mut TestAppContext) {
    let fixture = two_live_clusters("selection", cx);
    select_pod_row(&fixture, 1, cx);
    assert_eq!(selected_context(&fixture, cx).as_deref(), Some("stg-b"));
    select_pod_row(&fixture, 0, cx);
    assert_eq!(selected_context(&fixture, cx).as_deref(), Some("prod-a"));
}

#[gpui_kit::test]
fn checks_are_cluster_qualified(cx: &mut TestAppContext) {
    let fixture = two_live_clusters("checks", cx);
    fixture.shell.update(cx, |shell, cx| {
        shell.toggle_row_checked(0, cx);
        shell.toggle_row_checked(1, cx);
    });
    // Same namespace and name: were the identity not qualified, the second tick would untick.
    let checked = fixture.shell.read_with(cx, |shell, cx| {
        shell.toolkit_state(cx).map(|state| state.checked)
    });
    assert_eq!(checked, Some(2));
}

#[gpui_kit::test]
fn reveal_targets_the_cluster(cx: &mut TestAppContext) {
    let fixture = two_live_clusters("reveal", cx);
    let stg = fixture.cluster("stg-b", cx);
    fixture.shell.update(cx, |shell, cx| {
        shell.show_screen(Screen::Overview, cx);
        shell.reveal_object(ClusterObject::new(stg.clone(), pod_key("api-0")), cx);
    });
    cx.run_until_parked();
    fixture.draw_twice(cx);
    assert_eq!(selected_context(&fixture, cx).as_deref(), Some("stg-b"));
    let table_row = fixture
        .shell
        .read_with(cx, |shell, cx| shell.pod_table.read(cx).selected_row());
    assert_eq!(table_row, Some(1));
}

#[gpui_kit::test]
fn failed_slot_shows_banner_with_remove(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture("failed-banner", cx);
    view(&fixture, &["prod-a", "stg-b"], cx);
    go_live(&fixture, "prod-a", NamespaceScope::All, cx);
    seed_pods(&fixture, "prod-a", vec![pod("shop", "api-0", "node-a")], cx);
    fixture
        .shell
        .update(cx, |shell, cx| shell.show_screen(Screen::Pods, cx));
    fixture.wait_until("the second connect to fail", cx, |shell, cx| {
        shell.view.slots().get(1).is_some_and(|slot| {
            matches!(slot.session.read(cx).phase(), SessionPhase::Failed { .. })
        })
    });
    fixture.draw_twice(cx);
    // The rows of the live cluster stay; the failed one has a banner with both buttons.
    let rows = fixture.shell.read_with(cx, |shell, cx| {
        shell.toolkit_state(cx).map(|state| state.total)
    });
    assert_eq!(rows, Some(1));
    // The buttons sit inside the banner, so they are found within its scope. The live cluster's
    // own watches fail against the dead port and add a banner of their own before it.
    let index = fixture
        .shell
        .read_with(cx, |shell, cx| {
            shell
                .slot_notice_list(cx)
                .iter()
                .position(|notice| matches!(notice, SlotNotice::Failed { .. }))
        })
        .expect("the failed cluster has a banner");
    let (has_retry, has_remove) = fixture.with_window(cx, |window, _| {
        let banner = window.within(("slot-failed", index));
        (
            banner.try_find(("slot-retry", index)).is_some(),
            banner.try_find(("slot-remove", index)).is_some(),
        )
    });
    assert!(has_retry && has_remove);
    // Remove from view keeps the live session and leaves one cluster.
    let kept = slot_session(&fixture, "prod-a", cx).entity_id();
    let stg = fixture.cluster("stg-b", cx);
    fixture
        .shell
        .update(cx, |shell, cx| shell.remove_from_view(&stg, cx));
    cx.run_until_parked();
    assert_eq!(viewed(&fixture, cx), ["prod-a"]);
    assert_eq!(slot_session(&fixture, "prod-a", cx).entity_id(), kept);
}

#[gpui_kit::test]
fn interrupted_slot_shows_banner(cx: &mut TestAppContext) {
    let fixture = two_live_clusters("interrupted", cx);
    let session = slot_session(&fixture, "stg-b", cx);
    session.update(cx, |session, cx| session.interrupt_pods(cx));
    cx.run_until_parked();
    let notices = fixture
        .shell
        .read_with(cx, |shell, cx| shell.slot_notice_list(cx));
    assert_eq!(
        notices,
        [SlotNotice::Interrupted {
            text: "Live updates interrupted in stg-b".to_owned()
        }]
    );
    // The rows of both clusters stay.
    let total = fixture.shell.read_with(cx, |shell, cx| {
        shell.toolkit_state(cx).map(|state| state.total)
    });
    assert_eq!(total, Some(2));
}

#[gpui_kit::test]
fn all_failed_shows_error_view_with_retry_all(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture("all-failed", cx);
    view(&fixture, &["prod-a", "stg-b"], cx);
    fixture.wait_until("both connects to fail", cx, |shell, cx| {
        shell.view.slots().len() == 2
            && shell
                .view
                .slots()
                .iter()
                .all(|slot| matches!(slot.session.read(cx).phase(), SessionPhase::Failed { .. }))
    });
    fixture
        .shell
        .update(cx, |shell, cx| shell.show_screen(Screen::Pods, cx));
    fixture.draw_twice(cx);
    assert!(fixture.is_drawn("retry-all", cx));
}

// ---- Step 4: the sites that must read the row's own cluster ----

#[gpui_kit::test]
fn yaml_view_is_rebuilt_for_same_name_in_other_cluster(cx: &mut TestAppContext) {
    let fixture = two_live_clusters("yaml-rebuild", cx);
    fixture
        .shell
        .update(cx, |shell, _| shell.drawer.tab = DrawerTab::Yaml);
    select_pod_row(&fixture, 0, cx);
    fixture.draw_twice(cx);
    let first = fixture.shell.read_with(cx, |shell, _| {
        shell.drawer.yaml.as_ref().map(Entity::entity_id)
    });
    assert!(first.is_some(), "the YAML tab has a view");
    select_pod_row(&fixture, 1, cx);
    fixture.draw_twice(cx);
    let second = fixture.shell.read_with(cx, |shell, _| {
        shell.drawer.yaml.as_ref().map(Entity::entity_id)
    });
    assert!(second.is_some());
    assert_ne!(first, second, "the view of the other cluster was reused");
}

#[gpui_kit::test]
fn same_pod_in_two_clusters_opens_two_tabs(cx: &mut TestAppContext) {
    let fixture = two_live_clusters("two-tabs", cx);
    let target = || {
        LogTarget::of_workload(PodOwner::Deployment {
            namespace: "shop".to_owned(),
            name: "web".to_owned(),
        })
        .expect("a deployment has a log target")
    };
    for context in ["prod-a", "stg-b", "prod-a"] {
        let cluster = fixture.cluster(context, cx);
        let kubeconfig =
            Kubeconfig::parse(FIXTURE_YAML, &fixture.path).expect("the fixture parses");
        let connection = fixture
            .runtime
            .block_on(ClusterConnection::open(&kubeconfig, context))
            .expect("a client builds without a round trip");
        fixture.with_window(cx, |window, cx| {
            let shell = fixture.shell.read(cx);
            let row = shell
                .slot_row_context(&cluster, cx)
                .expect("the cluster is viewed");
            let dock = shell.dock.clone();
            dock.update(cx, |dock, cx| {
                dock.open(LogOrigin::new(&row, connection), target(), window, cx)
            });
        });
    }
    // One tab per cluster; the second open of `prod-a` focused its own tab.
    let count = fixture
        .shell
        .read_with(cx, |shell, cx| shell.dock.read(cx).tab_count());
    assert_eq!(count, 2);
}

#[gpui_kit::test]
fn released_slot_closes_its_log_tabs(cx: &mut TestAppContext) {
    let fixture = two_live_clusters("release-tabs", cx);
    let target = LogTarget::of_workload(PodOwner::Deployment {
        namespace: "shop".to_owned(),
        name: "web".to_owned(),
    })
    .expect("a deployment has a log target");
    for context in ["prod-a", "stg-b"] {
        let cluster = fixture.cluster(context, cx);
        let kubeconfig =
            Kubeconfig::parse(FIXTURE_YAML, &fixture.path).expect("the fixture parses");
        let connection = fixture
            .runtime
            .block_on(ClusterConnection::open(&kubeconfig, context))
            .expect("a client builds without a round trip");
        let target = target.clone();
        fixture.with_window(cx, |window, cx| {
            let shell = fixture.shell.read(cx);
            let row = shell
                .slot_row_context(&cluster, cx)
                .expect("the cluster is viewed");
            let dock = shell.dock.clone();
            dock.update(cx, |dock, cx| {
                dock.open(LogOrigin::new(&row, connection), target, window, cx)
            });
        });
    }
    view(&fixture, &["prod-a", "dev-c"], cx);
    let remaining = fixture.shell.read_with(cx, |shell, cx| {
        let dock = shell.dock.read(cx);
        (dock.tab_count(), dock.is_multi())
    });
    // `stg-b` left: its tab closed, `prod-a`'s stayed, and two clusters are still viewed.
    assert_eq!(remaining, (1, true));
}

#[gpui_kit::test]
fn subject_moves_between_clusters_stops_old_starts_new(cx: &mut TestAppContext) {
    let fixture = two_live_clusters("subject-moves", cx);
    let subject_of = |context: &str, cx: &mut TestAppContext| {
        let session = slot_session(&fixture, context, cx);
        cx.read_entity(&session, |session, _| {
            session
                .live()
                .and_then(|live| live.event_subject().cloned())
        })
    };
    select_pod_row(&fixture, 0, cx);
    cx.executor().advance_clock(DRAWER_SUBJECT_DELAY);
    cx.run_until_parked();
    assert!(
        subject_of("prod-a", cx).is_some(),
        "the old cluster watches"
    );
    assert!(subject_of("stg-b", cx).is_none());
    select_pod_row(&fixture, 1, cx);
    // The watches of the cluster the subject left stop at once; the new one waits for the rest.
    assert!(subject_of("prod-a", cx).is_none());
    cx.executor().advance_clock(DRAWER_SUBJECT_DELAY);
    cx.run_until_parked();
    assert!(subject_of("stg-b", cx).is_some(), "the new cluster watches");
}

#[test]
fn pending_subjects_differ_by_cluster() {
    let cluster = |context: &str| ClusterRef {
        kubeconfig: PathBuf::from("kube.yaml"),
        context: context.to_owned(),
    };
    let subject = InvolvedObject {
        kind: "Pod".to_owned(),
        namespace: Some("shop".to_owned()),
        name: "api-0".to_owned(),
    };
    let pending = |context: &str| PendingSubjects {
        cluster: Some(cluster(context)),
        events: Some(subject.clone()),
        ..PendingSubjects::default()
    };
    assert!(pending("a").has_same_subjects(&pending("a")));
    // The same pod name in another cluster is another subject.
    assert!(!pending("a").has_same_subjects(&pending("b")));
}

#[gpui_kit::test]
fn kubelet_demand_only_on_subject_slot(cx: &mut TestAppContext) {
    let fixture = two_live_clusters("kubelet-demand", cx);
    fixture
        .shell
        .update(cx, |shell, _| shell.drawer.tab = DrawerTab::Monitor);
    select_pod_row(&fixture, 1, cx);
    fixture.draw_twice(cx);
    let demand_of = |context: &str, cx: &mut TestAppContext| {
        let session = slot_session(&fixture, context, cx);
        cx.read_entity(&session, |session, _| {
            session
                .live()
                .map(|live| live.metrics.kubelet.demand().clone())
        })
    };
    let subject = demand_of("stg-b", cx).expect("a live session");
    assert!(subject.subject.is_some(), "the subject's cluster polls it");
    assert_eq!(
        demand_of("prod-a", cx),
        Some(KubeletDemand::default()),
        "no other cluster polls anything extra"
    );
}

#[gpui_kit::test]
fn drawer_reads_the_row_cluster(cx: &mut TestAppContext) {
    let fixture = two_live_clusters("drawer-cluster", cx);
    select_pod_row(&fixture, 1, cx);
    fixture.draw_twice(cx);
    // The drawer is drawn, from the live data of the cluster of the row.
    assert!(fixture.is_drawn("drawer-close", cx));
    let node = fixture.shell.read_with(cx, |shell, cx| {
        let live = shell.subject_live(cx)?;
        live.pods.items().first()?.node_name.clone()
    });
    assert_eq!(node.as_deref(), Some("node-b"));
    // The header names the cluster while several are viewed.
    let cluster = fixture
        .shell
        .read_with(cx, |shell, _| shell.drawer.cluster.clone());
    assert_eq!(
        cluster.map(|cluster| cluster.label.to_string()).as_deref(),
        Some("stg-b")
    );
}

#[gpui_kit::test]
fn drawer_closes_when_its_slot_is_released(cx: &mut TestAppContext) {
    let fixture = two_live_clusters("drawer-release", cx);
    select_pod_row(&fixture, 1, cx);
    assert_eq!(selected_context(&fixture, cx).as_deref(), Some("stg-b"));
    view(&fixture, &["prod-a", "dev-c"], cx);
    assert_eq!(selected_context(&fixture, cx), None);
}

#[gpui_kit::test]
fn menu_uses_row_cluster_context(cx: &mut TestAppContext) {
    let fixture = two_live_clusters("menu-context", cx);
    let stg = fixture.cluster("stg-b", cx);
    let row = fixture
        .shell
        .read_with(cx, |shell, cx| shell.slot_row_context(&stg, cx))
        .expect("the cluster is viewed");
    assert_eq!(row.cluster, stg);
    assert_eq!(row.context, "stg-b");
    assert!(!row.is_primary);
    // `Copy kubectl command` names the row's own context, never the primary's.
    assert_eq!(
        crate::resource_actions::kubectl_describe_command(&row.context, "shop", "api-0"),
        "kubectl --context stg-b -n shop describe pod api-0"
    );
    // The actions of the row act on the row's cluster.
    assert_eq!(row.object(pod_key("api-0")).cluster, stg);
}

// ---- Launch ----

#[gpui_kit::test]
fn the_view_flag_starts_the_clusters_together(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture_with(
        "view-flag",
        &["--view", "stg-b,dev-c", "--screen", "pods-multi"],
        cx,
    );
    cx.run_until_parked();
    // No current cluster at launch: the first in display order is primary.
    assert_eq!(viewed(&fixture, cx), ["stg-b", "dev-c"]);
    let primary = fixture
        .shell
        .read_with(cx, |shell, _| shell.primary_cluster());
    assert_eq!(primary, Some(fixture.cluster("stg-b", cx)));
}

// ---- Budget ----

fn watch_count(fixture: &SwitchFixture, context: &str, cx: &mut TestAppContext) -> Option<usize> {
    let session = slot_session(fixture, context, cx);
    cx.read_entity(&session, |session, _| {
        session.live().map(LiveCluster::watch_count)
    })
}

#[gpui_kit::test]
fn closed_drawer_watch_count_matches_a_single_session(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture("watch-count", cx);
    // Pods: Overview's change feeds, which only the primary cluster runs, are not part of it.
    fixture
        .shell
        .update(cx, |shell, cx| shell.show_screen(Screen::Pods, cx));
    go_live(&fixture, "prod-a", NamespaceScope::All, cx);
    let single = watch_count(&fixture, "prod-a", cx).expect("a live session");
    // Another cluster costs what one session costs, no more: no extra watch per slot.
    view(&fixture, &["prod-a", "stg-b"], cx);
    go_live(&fixture, "stg-b", NamespaceScope::All, cx);
    assert_eq!(watch_count(&fixture, "prod-a", cx), Some(single));
    assert_eq!(watch_count(&fixture, "stg-b", cx), Some(single));
}

#[gpui_kit::test]
fn open_drawer_adds_watches_to_its_slot_only(cx: &mut TestAppContext) {
    let fixture = two_live_clusters("watch-drawer", cx);
    let closed = watch_count(&fixture, "stg-b", cx).expect("a live session");
    select_pod_row(&fixture, 1, cx);
    cx.executor().advance_clock(DRAWER_SUBJECT_DELAY);
    cx.run_until_parked();
    fixture.draw_twice(cx);
    assert!(watch_count(&fixture, "stg-b", cx) > Some(closed));
    assert_eq!(watch_count(&fixture, "prod-a", cx), Some(closed));
}

#[gpui_kit::test]
fn filter_by_this_cluster_keeps_the_rows_of_one_cluster(cx: &mut TestAppContext) {
    let fixture = two_live_clusters("filter-cluster", cx);
    fixture
        .shell
        .update(cx, |shell, cx| shell.filter_by_cluster("stg-b", cx));
    let shown = fixture.shell.read_with(cx, |shell, cx| {
        shell
            .toolkit_state(cx)
            .map(|state| (state.shown, state.total, state.chips.len()))
    });
    assert_eq!(shown, Some((1, 2, 1)));
    // Removing the chip brings the other cluster back.
    fixture
        .shell
        .update(cx, |shell, cx| shell.remove_chip(0, cx));
    let shown = fixture.shell.read_with(cx, |shell, cx| {
        shell.toolkit_state(cx).map(|state| state.shown)
    });
    assert_eq!(shown, Some(2));
}

#[gpui_kit::test]
fn leaving_multi_drops_the_cluster_column_state(cx: &mut TestAppContext) {
    let fixture = two_live_clusters("leave-multi", cx);
    let column = fixture.shell.read_with(cx, |shell, cx| {
        shell.pod_table.read(cx).delegate().cluster_column()
    });
    assert_eq!(
        column,
        Some(8),
        "Pods list eight columns before the Cluster one"
    );
    // The user hid and sorted by it.
    fixture.shell.update(cx, |shell, cx| {
        shell.toggle_column(8, cx);
        shell.cycle_sort(8, cx);
    });
    // The sort and the hidden state are not saved: no pref names the column.
    let saved = cx.update(|cx| AppSettings::get(cx).tables.get("pods").cloned());
    assert!(
        saved.is_none_or(|prefs| prefs.hidden.iter().all(|name| name != "Cluster")
            && prefs.sort.is_none_or(|sort| sort.column != "Cluster"))
    );
    view(&fixture, &["prod-a"], cx);
    let (hidden, sort, column) = fixture.shell.read_with(cx, |shell, cx| {
        let delegate = shell.pod_table.read(cx).delegate();
        let view = delegate.view().expect("a view");
        (
            view.hidden.contains(&8),
            view.sort,
            delegate.cluster_column(),
        )
    });
    assert!(!hidden);
    assert_eq!(sort, None);
    assert_eq!(column, None);
}

// ---- Review fixes ----

#[gpui_kit::test]
fn dialog_link_reveals_in_dialog_cluster(cx: &mut TestAppContext) {
    let fixture = two_live_clusters("dialog-link", cx);
    let stg = fixture.cluster("stg-b", cx);
    // The drawer is closed, so a bare key would mean the primary cluster.
    fixture
        .shell
        .update(cx, |shell, cx| shell.show_screen(Screen::Overview, cx));
    assert_eq!(selected_context(&fixture, cx), None);
    let origin = DialogOrigin {
        shell: fixture.shell.downgrade(),
        cluster: stg,
    };
    cx.update(|cx| origin.reveal(pod_key("api-0"), cx));
    cx.run_until_parked();
    fixture.draw_twice(cx);
    assert_eq!(selected_context(&fixture, cx).as_deref(), Some("stg-b"));
}

/// A kubeconfig of `count` contexts named `c0`, `c1`, and so on.
fn many_contexts_yaml(count: usize) -> String {
    let mut yaml = String::from(
        "apiVersion: v1\nkind: Config\nclusters:\n  - name: c\n    cluster: { server: 'https://127.0.0.1:1' }\ncontexts:\n",
    );
    for index in 0..count {
        yaml.push_str(&format!(
            "  - name: c{index}\n    context: {{ cluster: c }}\n"
        ));
    }
    yaml
}

#[gpui_kit::test]
fn refused_sixth_tick_shows_its_notice_without_a_footer(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture_over(
        "sixth-tick",
        &many_contexts_yaml(6),
        &["--context", "c0"],
        cx,
    );
    view(&fixture, &["c0", "c1", "c2", "c3", "c4"], cx);
    fixture.open_switcher(cx);
    let sixth = fixture.cluster("c5", cx);
    fixture
        .shell
        .update(cx, |shell, cx| shell.toggle_cluster_tick(&sixth, cx));
    fixture.draw_twice(cx);
    // Five viewed and five ticked: nothing to apply, so no footer, but the reason is shown.
    assert_eq!(ticked(&fixture, cx).len(), 5);
    assert!(!fixture.is_drawn("switcher-apply", cx));
    let content = fixture.shell.read_with(cx, |shell, cx| {
        let content = shell.switcher_content(fixture.shell.downgrade(), cx);
        (content.has_pending_ticks, content.tick_notice)
    });
    assert_eq!(
        content,
        (false, Some("View at most 5 clusters at once.".into()))
    );
}

#[gpui_kit::test]
fn single_switch_invalidates_a_pending_multi_connect(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture("stale-connect", cx);
    let wanted = [fixture.cluster("prod-a", cx), fixture.cluster("dev-c", cx)];
    let target = fixture.cluster("stg-b", cx);
    // One update, so the deferred connect of `dev-c` has not run when the single switch comes.
    fixture.shell.update(cx, |shell, cx| {
        shell.view_clusters(&wanted, cx);
        shell.switch_cluster(&target, cx);
    });
    cx.run_until_parked();
    assert_eq!(viewed(&fixture, cx), ["stg-b"]);
}

#[gpui_kit::test]
fn name_click_on_a_viewed_cluster_keeps_its_session(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture("click-viewed", cx);
    view(&fixture, &["prod-a", "stg-b"], cx);
    let kept = slot_session(&fixture, "stg-b", cx).entity_id();
    let released = slot_session(&fixture, "prod-a", cx).downgrade();
    let stg = fixture.cluster("stg-b", cx);
    fixture
        .shell
        .update(cx, |shell, cx| shell.switch_cluster(&stg, cx));
    cx.run_until_parked();
    fixture.draw_twice(cx);
    assert_eq!(viewed(&fixture, cx), ["stg-b"]);
    assert_eq!(slot_session(&fixture, "stg-b", cx).entity_id(), kept);
    assert!(released.upgrade().is_none());
}

#[gpui_kit::test]
fn view_flag_wins_over_the_context_flag(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture_with("view-wins", &["--view", "stg-b"], cx);
    // The fixture passes `--context prod-a` first; the one viewed context is what opens.
    assert_eq!(viewed(&fixture, cx), ["stg-b"]);
}

#[gpui_kit::test]
fn select_all_follows_the_ticks(cx: &mut TestAppContext) {
    let fixture = two_live_clusters("select-all", cx);
    let is_all_checked = |cx: &mut TestAppContext| {
        fixture.shell.read_with(cx, |shell, cx| {
            shell.pod_table.read(cx).delegate().all_checked()
        })
    };
    assert!(!is_all_checked(cx));
    fixture
        .shell
        .update(cx, |shell, cx| shell.set_all_checked(true, cx));
    assert!(is_all_checked(cx));
    fixture
        .shell
        .update(cx, |shell, cx| shell.toggle_row_checked(0, cx));
    assert!(!is_all_checked(cx));
}

// ---- Rebase onto the keymap and the palette ----

#[gpui_kit::test]
fn a_closed_drawer_keeps_its_cursor_but_does_not_steer_a_bare_reveal(cx: &mut TestAppContext) {
    let fixture = two_live_clusters("closed-drawer-context", cx);
    select_pod_row(&fixture, 1, cx);
    assert_eq!(selected_context(&fixture, cx).as_deref(), Some("stg-b"));
    fixture.shell.update(cx, |shell, cx| shell.close_drawer(cx));
    // The cursor stays on the row of the second cluster, but no drawer means no context.
    assert_eq!(selected_context(&fixture, cx).as_deref(), Some("stg-b"));
    let context = fixture
        .shell
        .read_with(cx, |shell, _| shell.context_cluster());
    let primary = fixture.cluster("prod-a", cx);
    assert_eq!(context, Some(primary));
    fixture
        .shell
        .update(cx, |shell, cx| shell.reveal(pod_key("api-0"), cx));
    cx.run_until_parked();
    assert_eq!(selected_context(&fixture, cx).as_deref(), Some("prod-a"));
}

#[gpui_kit::test]
fn a_palette_resource_entry_of_cluster_b_reveals_in_b(cx: &mut TestAppContext) {
    let fixture = two_live_clusters("palette-reveal", cx);
    let stg = fixture.cluster("stg-b", cx);
    let snapshot = fixture
        .shell
        .read_with(cx, |shell, cx| shell.palette_snapshot(true, cx));
    let (object, detail) = snapshot
        .entries
        .iter()
        .find_map(|entry| match &entry.target {
            PaletteTarget::Resource(object) if object.cluster == stg => {
                Some((object.clone(), entry.detail.clone()))
            }
            _ => None,
        })
        .expect("the second cluster lists its pod");
    // The row tells the same name of two clusters apart.
    assert_eq!(detail.as_deref(), Some("shop/api-0 · stg-b"));
    // Confirming an entry reveals its object in the cluster of the entry.
    fixture
        .shell
        .update(cx, |shell, cx| shell.reveal_object(object, cx));
    cx.run_until_parked();
    fixture.draw_twice(cx);
    assert_eq!(selected_context(&fixture, cx).as_deref(), Some("stg-b"));
    let (is_open, table_row) = fixture.shell.read_with(cx, |shell, cx| {
        (
            shell.drawer.is_open,
            shell.pod_table.read(cx).selected_row(),
        )
    });
    assert!(is_open);
    assert_eq!(table_row, Some(1));
}

#[gpui_kit::test]
fn the_palette_of_a_single_cluster_adds_no_cluster_label(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture("palette-single", cx);
    go_live(&fixture, "prod-a", NamespaceScope::All, cx);
    seed_pods(&fixture, "prod-a", vec![pod("shop", "api-0", "node-a")], cx);
    let snapshot = fixture
        .shell
        .read_with(cx, |shell, cx| shell.palette_snapshot(true, cx));
    let detail = snapshot.entries.iter().find_map(|entry| {
        matches!(entry.target, PaletteTarget::Resource(_)).then(|| entry.detail.clone())
    });
    assert_eq!(detail.flatten().as_deref(), Some("shop/api-0"));
}

#[gpui_kit::test]
fn a_palette_cluster_switch_leaves_the_multi_view(cx: &mut TestAppContext) {
    let fixture = two_live_clusters("palette-switch", cx);
    let dev = fixture.cluster("dev-c", cx);
    let snapshot = fixture
        .shell
        .read_with(cx, |shell, cx| shell.palette_snapshot(false, cx));
    let row = snapshot
        .entries
        .iter()
        .find_map(|entry| match &entry.target {
            PaletteTarget::Cluster(row) if row.cluster == dev => Some(row.clone()),
            _ => None,
        })
        .expect("the palette lists every cluster");
    let before = fixture.shell.read_with(cx, |shell, _| shell.view_request);
    fixture
        .shell
        .update(cx, |shell, cx| shell.switch_cluster(&row.cluster, cx));
    // The switch releases every slot first, which also invalidates a pending multi connect.
    let after = fixture.shell.read_with(cx, |shell, _| shell.view_request);
    assert!(after > before);
    cx.run_until_parked();
    assert_eq!(viewed(&fixture, cx), ["dev-c"]);
}

#[gpui_kit::test]
fn the_yaml_key_acts_on_the_cursor_row_of_its_cluster(cx: &mut TestAppContext) {
    let fixture = two_live_clusters("yaml-key", cx);
    select_pod_row(&fixture, 1, cx);
    fixture.shell.update(cx, |shell, cx| shell.close_drawer(cx));
    fixture.draw_twice(cx);
    fixture.press("y", cx);
    cx.run_until_parked();
    let stg = fixture.cluster("stg-b", cx);
    fixture.shell.read_with(cx, |shell, _| {
        assert!(shell.drawer.is_open);
        assert_eq!(shell.drawer.tab, DrawerTab::Yaml);
        assert_eq!(
            shell.drawer_subject().map(|object| &object.cluster),
            Some(&stg)
        );
    });
}

#[gpui_kit::test]
fn right_click_menu_action_acts_on_the_clicked_row(cx: &mut TestAppContext) {
    let fixture = two_live_clusters("right-click-menu", cx);
    // The cursor sits on the row of the first cluster, with the drawer closed.
    select_pod_row(&fixture, 0, cx);
    fixture.shell.update(cx, |shell, cx| shell.close_drawer(cx));
    fixture.draw_twice(cx);
    assert_eq!(selected_context(&fixture, cx).as_deref(), Some("prod-a"));
    // A right click on the row of the second cluster opens its menu.
    fixture.with_window(cx, |window, cx| window.right_click(("row", 1usize), cx));
    cx.run_until_parked();
    fixture.draw_twice(cx);
    cx.run_until_parked();
    // The kit leaves the cursor alone, so the shell moves it: key actions run on the cursor.
    assert_eq!(selected_context(&fixture, cx).as_deref(), Some("stg-b"));
    fixture
        .shell
        .read_with(cx, |shell, _| assert!(!shell.drawer.is_open));
    // The second clickable item of the menu is View YAML.
    for key in ["down", "down", "enter"] {
        fixture.press(key, cx);
        cx.run_until_parked();
    }
    let stg = fixture.cluster("stg-b", cx);
    fixture.shell.read_with(cx, |shell, _| {
        assert!(shell.drawer.is_open);
        assert_eq!(shell.drawer.tab, DrawerTab::Yaml);
        assert_eq!(
            shell.drawer_subject().map(|object| &object.cluster),
            Some(&stg)
        );
    });
}

#[gpui_kit::test]
fn a_disabled_menu_item_confirmed_by_key_acts_on_the_clicked_row(cx: &mut TestAppContext) {
    let fixture = two_live_clusters("right-click-key", cx);
    select_pod_row(&fixture, 0, cx);
    fixture.shell.update(cx, |shell, cx| shell.close_drawer(cx));
    fixture.draw_twice(cx);
    fixture.with_window(cx, |window, cx| window.right_click(("row", 1usize), cx));
    cx.run_until_parked();
    fixture.draw_twice(cx);
    cx.run_until_parked();
    // The first item is confirmed without a handler of its own: the kit dispatches its action,
    // which runs on the cursor. It must be the clicked row, in the clicked cluster.
    for key in ["down", "enter"] {
        fixture.press(key, cx);
        cx.run_until_parked();
    }
    assert_eq!(selected_context(&fixture, cx).as_deref(), Some("stg-b"));
}

// ---- The write guard reads the row's own cluster (0030 meets 0027) ----

/// What the gate and the confirm step read for `context`: its lock, and how it confirms a
/// change. `None` when the cluster is not viewed or not live.
fn guard_facts(
    fixture: &SwitchFixture,
    context: &str,
    cx: &mut TestAppContext,
) -> Option<(ClusterRef, WriteLock, DialogConfirm)> {
    let cluster = fixture.cluster(context, cx);
    fixture.shell.read_with(cx, |shell, cx| {
        let guard = shell.guard_for(&cluster, cx)?;
        let confirm = confirm_step(
            guard.profile.confirm,
            ActionRisk::Change,
            guard.display_name(),
        );
        Some((guard.cluster.clone(), guard.lock, confirm))
    })
}

/// Production locks and types its name; Staging is open and clicks. Each cluster answers for itself.
fn assert_each_cluster_has_its_own_guard(fixture: &SwitchFixture, cx: &mut TestAppContext) {
    let prod = fixture.cluster("prod-a", cx);
    let stg = fixture.cluster("stg-b", cx);
    assert_eq!(
        guard_facts(fixture, "prod-a", cx),
        Some((
            prod,
            WriteLock::Locked,
            DialogConfirm::TypeName {
                expected: "prod-a".to_owned()
            }
        ))
    );
    assert_eq!(
        guard_facts(fixture, "stg-b", cx),
        Some((stg, WriteLock::Unlocked, DialogConfirm::Click))
    );
    // A cluster that is not viewed has no guard, whatever the primary is.
    assert_eq!(guard_facts(fixture, "dev-c", cx), None);
}

#[gpui_kit::test]
fn gate_and_confirm_use_the_rows_cluster_with_a_production_primary(cx: &mut TestAppContext) {
    let fixture = two_live_clusters("guard-prod-primary", cx);
    let primary = fixture
        .shell
        .read_with(cx, |shell, _| shell.primary_cluster());
    assert_eq!(primary, Some(fixture.cluster("prod-a", cx)));
    assert_each_cluster_has_its_own_guard(&fixture, cx);
}

#[gpui_kit::test]
fn gate_and_confirm_use_the_rows_cluster_with_a_staging_primary(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture_with("guard-stg-primary", &["--context", "stg-b"], cx);
    view(&fixture, &["stg-b", "prod-a"], cx);
    for context in ["stg-b", "prod-a"] {
        go_live(&fixture, context, NamespaceScope::All, cx);
    }
    let primary = fixture
        .shell
        .read_with(cx, |shell, _| shell.primary_cluster());
    assert_eq!(primary, Some(fixture.cluster("stg-b", cx)));
    assert_each_cluster_has_its_own_guard(&fixture, cx);
}
