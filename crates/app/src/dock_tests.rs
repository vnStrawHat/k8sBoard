//! Pop-out windows of the dock, in a headless window over a fake API server (see `log_fixtures`).

use gpui_kit::base::Root;
use gpui_kit::{Entity, TestAppContext};

use super::*;
use crate::log_fixtures::{LogFixture, fixture_container, fixture_pod, open_log_fixture};
use crate::log_window::LogWindow;

const LOG_BODY: &str =
    "2024-05-01T10:00:00.000000000Z first\n2024-05-01T10:00:01.000000000Z second\n";

fn two_pods() -> Vec<cluster::PodSummary> {
    ["api-0", "worker-0"]
        .map(|name| fixture_pod(name, vec![fixture_container("app", 0, None)]))
        .to_vec()
}

/// A dock with a tab for each pod, and the tab of the first one.
fn open_two_tabs(cx: &mut TestAppContext) -> (LogFixture, Entity<Dock>, Entity<LogTab>) {
    let pods = two_pods();
    let fixture = open_log_fixture(pods.clone(), LOG_BODY, cx);
    let dock = fixture.open_dock(cx);
    for pod in &pods {
        fixture.open_in_dock(&dock, pod, "app", cx);
    }
    let first = dock.read_with(cx, |dock, _| match &dock.tabs[0] {
        DockTab::Logs(tab) => tab.clone(),
        DockTab::Shell(_) | DockTab::Drain(_) => unreachable!("log tabs only"),
    });
    (fixture, dock, first)
}

fn pop_out(tab: &Entity<LogTab>, cx: &mut TestAppContext) {
    tab.update(cx, |_, cx| cx.emit(LogTabEvent::PopOut));
    cx.run_until_parked();
}

#[gpui_kit::test]
fn pop_out_moves_the_tab_and_keeps_its_streams(cx: &mut TestAppContext) {
    let (fixture, dock, tab) = open_two_tabs(cx);
    let reads = fixture.log_reads();
    assert_eq!(reads, 2);
    pop_out(&tab, cx);
    // The same entity lives in the new window, out of the dock, and nothing was read again.
    let popped = dock.read_with(cx, |dock, _| dock.popped[0].tab.upgrade());
    assert_eq!(
        popped.map(|popped| popped.entity_id()),
        Some(tab.entity_id())
    );
    assert_eq!(dock.read_with(cx, |dock, _| dock.tab_count()), 1);
    assert!(tab.read_with(cx, |tab, _| tab.is_popped_out()));
    assert_eq!(fixture.log_reads(), reads);
}

#[gpui_kit::test]
fn pop_out_window_is_titled_with_the_tab_label(cx: &mut TestAppContext) {
    let (_fixture, dock, tab) = open_two_tabs(cx);
    pop_out(&tab, cx);
    let window = dock
        .read_with(cx, |dock, _| dock.popped_window())
        .expect("a pop-out");
    let root = window.downcast::<Root>().expect("a Root window");
    let title = root
        .read_with(cx, |root, _| {
            root.view().clone().downcast::<LogWindow>().ok()
        })
        .expect("a readable window")
        .expect("the root shows a log window")
        .read_with(cx, |window, _| window.title().to_owned());
    assert_eq!(title, "api-0/app");
}

#[gpui_kit::test]
fn view_logs_on_a_popped_target_activates_its_window(cx: &mut TestAppContext) {
    let (fixture, dock, tab) = open_two_tabs(cx);
    pop_out(&tab, cx);
    let windows = cx.update(|cx| cx.windows().len());
    let reads = fixture.log_reads();
    fixture.open_in_dock(&dock, &two_pods()[0], "app", cx);
    // No dock tab, no stream, no new window.
    assert_eq!(dock.read_with(cx, |dock, _| dock.tab_count()), 1);
    assert_eq!(fixture.log_reads(), reads);
    assert_eq!(cx.update(|cx| cx.windows().len()), windows);
}

#[gpui_kit::test]
fn cluster_switch_closes_every_pop_out(cx: &mut TestAppContext) {
    let (_fixture, dock, first) = open_two_tabs(cx);
    let second = dock.read_with(cx, |dock, _| match &dock.tabs[1] {
        DockTab::Logs(tab) => tab.clone(),
        DockTab::Shell(_) | DockTab::Drain(_) => unreachable!("log tabs only"),
    });
    pop_out(&first, cx);
    pop_out(&second, cx);
    assert_eq!(dock.read_with(cx, |dock, _| dock.popped.len()), 2);
    let windows = cx.update(|cx| cx.windows().len());
    dock.update(cx, |dock, cx| dock.close_all(cx));
    cx.run_until_parked();
    assert_eq!(dock.read_with(cx, |dock, _| dock.popped.len()), 0);
    // Both pop-out windows are gone; the main window stays.
    assert_eq!(cx.update(|cx| cx.windows().len()), windows - 2);
}

#[gpui_kit::test]
fn closing_a_pop_out_drops_its_tab(cx: &mut TestAppContext) {
    let (_fixture, dock, tab) = open_two_tabs(cx);
    pop_out(&tab, cx);
    let window = dock
        .read_with(cx, |dock, _| dock.popped_window())
        .expect("a pop-out");
    let weak = tab.downgrade();
    drop(tab);
    window
        .update(cx, |_, window, _| window.remove_window())
        .expect("the window is open");
    cx.run_until_parked();
    assert!(weak.upgrade().is_none());
}
