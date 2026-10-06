//! Log tab behavior in a headless window over a fake API server (see `log_fixtures`).

use gpui_kit::{Entity, Focusable as _, TestAppContext, Window, point, px};

use super::*;
use crate::log_fixtures::{fixture_container, fixture_pod, oom_killed, open_log_fixture};
use crate::log_window::open_log_window;
use cluster::PodSummary;

const LOG_BODY: &str =
    "2024-05-01T10:00:00.000000000Z first\n2024-05-01T10:00:01.000000000Z second\n";

fn marker_texts(tab: &LogTab) -> Vec<String> {
    tab.buffer
        .visible_lines()
        .filter(|line| line.kind == LineKind::Marker)
        .map(|line| line.line.text.clone())
        .collect()
}

#[gpui_kit::test]
fn restart_marker_shows_after_the_stream_ended(cx: &mut TestAppContext) {
    let pod = fixture_pod("api-0", vec![fixture_container("api", 3, None)]);
    let fixture = open_log_fixture(vec![pod.clone()], LOG_BODY, cx);
    let tab = fixture.open_tab(&pod, "api", cx);
    // The fake server closes the body, so the follow stream ends like a container's does.
    fixture.wait_until("the stream to end", cx, |cx| {
        tab.read_with(cx, |tab, _| tab.phase() == TabPhase::Ended)
    });
    assert_eq!(tab.read_with(cx, |tab, _| tab.buffer.total_len()), 2);

    // The kubelet raises the count only after the stream has ended.
    let restarted = fixture_pod(
        "api-0",
        vec![fixture_container(
            "api",
            14,
            Some(oom_killed("2024-05-01T10:00:02Z")),
        )],
    );
    fixture.session.update(cx, |session, cx| {
        session.set_pods_for_test(vec![restarted], cx)
    });
    cx.run_until_parked();
    assert_eq!(
        tab.read_with(cx, |tab, _| marker_texts(tab)),
        ["── container api terminated: OOMKilled (exit 137) · restart #14 ──"]
    );
    // The marker comes from the pod list: it opens no stream.
    assert_eq!(fixture.log_reads(), 1);
}

fn timestamps_of_window(tab: &LogTab) -> Option<(String, String)> {
    tab.buffer
        .view()
        .window
        .map(|window| (window.start.to_string(), window.end.to_string()))
}

/// A tab in the Full layout whose two lines, a second apart, make a two-bucket histogram.
fn open_full_tab(cx: &mut TestAppContext) -> (crate::log_fixtures::LogFixture, Entity<LogTab>) {
    let pod = fixture_pod("api-0", vec![fixture_container("api", 0, None)]);
    let fixture = open_log_fixture(vec![pod.clone()], LOG_BODY, cx);
    let tab = fixture.open_tab(&pod, "api", cx);
    fixture.wait_until("the stream to end", cx, |cx| {
        tab.read_with(cx, |tab, _| tab.phase() == TabPhase::Ended)
    });
    tab.update(cx, |tab, cx| tab.set_layout(LogLayout::Full, cx));
    fixture.draw(cx);
    (fixture, tab)
}

fn mouse_down(position: gpui_kit::Point<Pixels>) -> gpui_kit::MouseDownEvent {
    gpui_kit::MouseDownEvent {
        button: gpui_kit::MouseButton::Left,
        position,
        modifiers: gpui_kit::Modifiers::default(),
        click_count: 1,
        first_mouse: false,
    }
}

fn mouse_move(position: gpui_kit::Point<Pixels>) -> gpui_kit::MouseMoveEvent {
    gpui_kit::MouseMoveEvent {
        position,
        pressed_button: Some(gpui_kit::MouseButton::Left),
        modifiers: gpui_kit::Modifiers::default(),
    }
}

fn mouse_up(position: gpui_kit::Point<Pixels>) -> gpui_kit::MouseUpEvent {
    gpui_kit::MouseUpEvent {
        button: gpui_kit::MouseButton::Left,
        position,
        modifiers: gpui_kit::Modifiers::default(),
        click_count: 1,
    }
}

#[gpui_kit::test]
fn release_outside_the_chart_ends_the_brush(cx: &mut TestAppContext) {
    let (fixture, tab) = open_full_tab(cx);
    let mut visual = gpui_kit::VisualTestContext::from_window(fixture.window.into(), cx);
    let chart = visual
        .debug_bounds("log-volume-brush")
        .expect("the histogram is drawn");
    let y = chart.center().y;
    visual.simulate_event(mouse_down(point(chart.left() + chart.size.width * 0.25, y)));
    assert!(tab.read_with(cx, |tab, _| tab.brush.is_some()));
    // The drag shows on the next frame, which is when the window listeners are registered.
    fixture.draw(cx);
    let beyond = point(chart.right() + px(120.), y);
    visual.simulate_event(mouse_move(beyond));
    visual.simulate_event(mouse_up(beyond));
    assert!(tab.read_with(cx, |tab, _| tab.brush.is_none()));
    // The release clamps to the last bucket: the window spans both buckets.
    assert_eq!(
        tab.read_with(cx, |tab, _| timestamps_of_window(tab)),
        Some((
            "2024-05-01T10:00:00Z".to_owned(),
            "2024-05-01T10:00:02Z".to_owned()
        ))
    );
}

#[gpui_kit::test]
fn restart_clears_the_window(cx: &mut TestAppContext) {
    let (_fixture, tab) = open_full_tab(cx);
    tab.update(cx, |tab, cx| {
        tab.apply_view(
            Some(TimeWindow {
                start: "2024-05-01T10:00:00Z".parse().expect("valid time"),
                end: "2024-05-01T10:00:01Z".parse().expect("valid time"),
            }),
            cx,
        );
        tab.restart_stream(cx);
    });
    assert_eq!(tab.read_with(cx, |tab, _| timestamps_of_window(tab)), None);
}

fn shown_texts(tab: &LogTab) -> Vec<String> {
    tab.buffer
        .visible_lines()
        .map(|line| line.line.text.clone())
        .collect()
}

#[gpui_kit::test]
fn pop_out_keeps_the_filter_text(cx: &mut TestAppContext) {
    let (fixture, tab) = open_full_tab(cx);
    fixture.with_window(cx, |window, cx| {
        let input = tab.read(cx).filter_input.clone();
        input.update(cx, |input, cx| input.replace_all("second", window, cx));
    });
    let shown = tab.read_with(cx, |tab, _| shown_texts(tab));
    assert_eq!(shown, ["second"]);
    let old_input = tab.read_with(cx, |tab, _| tab.filter_input.entity_id());

    fixture.clear(cx);
    let window = cx
        .update(|cx| open_log_window(tab.clone(), "api-0/api".to_owned(), cx))
        .expect("the window opens");
    cx.run_until_parked();

    // The kit input belongs to the window that made it, so the new window has its own, with the
    // same text, and the view did not change.
    let (new_input, text) = tab.read_with(cx, |tab, cx| {
        (
            tab.filter_input.entity_id(),
            tab.filter_input.read(cx).value().to_string(),
        )
    });
    assert_ne!(new_input, old_input);
    assert_eq!(text, "second");
    assert_eq!(tab.read_with(cx, |tab, _| shown_texts(tab)), shown);
    // It takes the focus there.
    let input = tab.read_with(cx, |tab, _| tab.filter_input.clone());
    let is_focused = window
        .update(cx, |_, window, cx| {
            input.read(cx).focus_handle(cx).is_focused(window)
        })
        .expect("the window is open");
    assert!(is_focused);
    // A typed change reaches the tab through the new subscription.
    window
        .update(cx, |_, window, cx| {
            input.update(cx, |input, cx| input.replace_all("first", window, cx));
        })
        .expect("the window is open");
    cx.run_until_parked();
    assert_eq!(tab.read_with(cx, |tab, _| shown_texts(tab)), ["first"]);
}

#[gpui_kit::test]
fn pop_out_button_hides_once_the_tab_is_popped_out(cx: &mut TestAppContext) {
    use gpui_kit::test::TestWindowExt as _;
    let (fixture, tab) = open_full_tab(cx);
    let is_drawn = |window: &mut Window| window.try_find("log-pop-out").is_some();
    assert!(fixture.with_window(cx, |window, _| is_drawn(window)));
    fixture.clear(cx);
    let window = cx
        .update(|cx| open_log_window(tab.clone(), "api-0/api".to_owned(), cx))
        .expect("the window opens");
    cx.run_until_parked();
    let is_drawn_there = window
        .update(cx, |_, window, cx| {
            window.render_frame(cx);
            is_drawn(window)
        })
        .expect("the window is open");
    assert!(!is_drawn_there);
}

fn one_second_window() -> TimeWindow {
    TimeWindow {
        start: "2024-05-01T10:00:00Z".parse().expect("valid time"),
        end: "2024-05-01T10:00:01Z".parse().expect("valid time"),
    }
}

#[gpui_kit::test]
fn a_click_without_a_drag_clears_the_window(cx: &mut TestAppContext) {
    let (_fixture, tab) = open_full_tab(cx);
    tab.update(cx, |tab, cx| {
        tab.apply_view(Some(one_second_window()), cx);
        tab.begin_brush(0.5, cx);
        tab.end_brush(0.5, cx);
    });
    assert_eq!(tab.read_with(cx, |tab, _| timestamps_of_window(tab)), None);
}

#[gpui_kit::test]
fn the_clear_button_shows_every_line_again(cx: &mut TestAppContext) {
    let (_fixture, tab) = open_full_tab(cx);
    tab.update(cx, |tab, cx| {
        tab.apply_view(Some(one_second_window()), cx);
        assert_eq!(tab.buffer.visible_len(), 1);
        tab.clear_time_window(cx);
    });
    assert_eq!(tab.read_with(cx, |tab, _| tab.buffer.visible_len()), 2);
}

fn stream_of(namespace: &str, pod: &str, state: LogStreamState) -> TabStream {
    TabStream {
        namespace: namespace.to_owned(),
        pod: pod.to_owned(),
        container: "api".to_owned(),
        prefix: "p/api".into(),
        full_prefix: "pod/api".into(),
        color_slot: 0,
        is_member: true,
        state,
        _stream: None,
        _grace: None,
    }
}

fn pod_in(namespace: &str, restarts: u32) -> PodSummary {
    let mut pod = fixture_pod("api-0", vec![fixture_container("api", restarts, None)]);
    pod.namespace = namespace.to_owned();
    pod
}

#[test]
fn restart_markers_keep_same_named_pods_of_two_namespaces_apart() {
    let streams = [
        stream_of("a", "api-0", LogStreamState::Streaming),
        stream_of("b", "api-0", LogStreamState::Streaming),
    ];
    let mut seen = RestartBaselines::new();
    let listed = [pod_in("a", 5), pod_in("b", 1)];
    assert!(restart_markers(&streams, &mut seen, &listed).is_empty());
    // Nothing changed: the other namespace's count is not a rise.
    assert!(restart_markers(&streams, &mut seen, &listed).is_empty());
    let after = [pod_in("a", 5), pod_in("b", 2)];
    assert_eq!(restart_markers(&streams, &mut seen, &after).len(), 1);
}

#[test]
fn baselines_of_gone_pods_with_no_live_stream_are_dropped() {
    let streams = [
        stream_of("a", "api-0", LogStreamState::Ended),
        stream_of("b", "api-0", LogStreamState::Streaming),
    ];
    let mut seen = RestartBaselines::new();
    restart_markers(&streams, &mut seen, &[pod_in("a", 1), pod_in("b", 1)]);
    assert_eq!(seen.len(), 2);
    // Both pods left the list: the ended stream's baseline goes, the live one's stays.
    restart_markers(&streams, &mut seen, &[]);
    let kept: Vec<_> = seen
        .keys()
        .map(|(namespace, _, _)| namespace.as_str())
        .collect();
    assert_eq!(kept, ["b"]);
}

#[gpui_kit::test]
fn leaving_the_full_layout_drops_a_drag_in_progress(cx: &mut TestAppContext) {
    let (_fixture, tab) = open_full_tab(cx);
    tab.update(cx, |tab, cx| {
        tab.begin_brush(0.3, cx);
        assert!(tab.brush.is_some());
        tab.set_layout(LogLayout::Compact, cx);
        assert!(tab.brush.is_none());
    });
}

#[test]
fn the_docked_toolbar_keeps_every_action_in_the_overflow_menu() {
    let placement = toolbar_actions(LogLayout::Compact, false, false);
    assert!(placement.inline.is_empty());
    assert_eq!(
        placement.overflow,
        [
            ToolbarAction::Copy,
            ToolbarAction::Export,
            ToolbarAction::PopOut,
            ToolbarAction::Reconnect
        ]
    );
}

#[test]
fn the_zoomed_toolbar_keeps_a_button_per_action() {
    let placement = toolbar_actions(LogLayout::Full, false, false);
    assert!(placement.overflow.is_empty());
    assert_eq!(placement.inline.len(), 4);
}

#[test]
fn a_popped_out_or_connecting_tab_offers_fewer_actions() {
    let popped_out = toolbar_actions(LogLayout::Full, true, false);
    assert!(!popped_out.inline.contains(&ToolbarAction::PopOut));
    let connecting = toolbar_actions(LogLayout::Compact, false, true);
    assert!(!connecting.overflow.contains(&ToolbarAction::Reconnect));
}

#[gpui_kit::test]
fn picking_a_since_window_restarts_the_stream_with_since_seconds(cx: &mut TestAppContext) {
    let (fixture, tab) = open_full_tab(cx);
    let first = fixture
        .api
        .requests()
        .into_iter()
        .find(|request| request.path.ends_with("/log"))
        .expect("the first log request");
    assert!(first.has_query_key("tailLines"));
    assert!(!first.has_query_key("sinceSeconds"));

    tab.update(cx, |tab, cx| tab.pick_since(LogSince::Minutes15, cx));
    fixture.wait_until("the second log request", cx, |_| fixture.log_reads() == 2);
    let second = fixture
        .api
        .requests()
        .into_iter()
        .filter(|request| request.path.ends_with("/log"))
        .nth(1)
        .expect("the restarted log request");
    assert!(second.has_query("sinceSeconds", "900"), "{}", second.query);
    assert!(!second.has_query_key("tailLines"), "{}", second.query);
}

#[gpui_kit::test]
fn picking_the_same_since_window_keeps_the_stream(cx: &mut TestAppContext) {
    let (fixture, tab) = open_full_tab(cx);
    tab.update(cx, |tab, cx| tab.pick_since(LogSince::Tail, cx));
    cx.run_until_parked();
    assert_eq!(fixture.log_reads(), 1);
}

fn select(
    fixture: &crate::log_fixtures::LogFixture,
    tab: &Entity<LogTab>,
    index: usize,
    extends: bool,
    cx: &mut TestAppContext,
) {
    fixture.with_window(cx, |window, cx| {
        tab.update(cx, |tab, cx| tab.select_row(index, extends, window, cx));
    });
}

fn clipboard_text(cx: &mut TestAppContext) -> Option<String> {
    cx.read_from_clipboard().and_then(|item| item.text())
}

#[gpui_kit::test]
fn copying_a_selection_writes_the_selected_rows_with_their_times(cx: &mut TestAppContext) {
    let (fixture, tab) = open_full_tab(cx);
    tab.update(cx, |tab, _| tab.time_zone = TimeZone::UTC);
    select(&fixture, &tab, 0, false, cx);
    tab.update(cx, |tab, cx| tab.copy_selected_lines(cx));
    assert_eq!(clipboard_text(cx).as_deref(), Some("10:00:00.000 first"));

    select(&fixture, &tab, 1, true, cx);
    tab.update(cx, |tab, cx| tab.copy_selected_lines(cx));
    assert_eq!(
        clipboard_text(cx).as_deref(),
        Some("10:00:00.000 first\n10:00:01.000 second")
    );
}

#[gpui_kit::test]
fn copying_without_a_selection_leaves_the_clipboard(cx: &mut TestAppContext) {
    let (_fixture, tab) = open_full_tab(cx);
    cx.write_to_clipboard(ClipboardItem::new_string("before".to_owned()));
    tab.update(cx, |tab, cx| tab.copy_selected_lines(cx));
    assert_eq!(clipboard_text(cx).as_deref(), Some("before"));
}

#[gpui_kit::test]
fn clearing_the_selection_and_changing_the_view_drop_it(cx: &mut TestAppContext) {
    let (fixture, tab) = open_full_tab(cx);
    select(&fixture, &tab, 0, false, cx);
    tab.update(cx, |tab, cx| tab.clear_selection(cx));
    assert_eq!(tab.read_with(cx, |tab, _| tab.selection), None);

    select(&fixture, &tab, 1, false, cx);
    tab.update(cx, |tab, cx| {
        tab.hidden_levels = tab.hidden_levels.toggled(LogLevel::Debug);
        tab.refresh_view(cx);
    });
    assert_eq!(tab.read_with(cx, |tab, _| tab.selection), None);
}

#[gpui_kit::test]
fn copy_visible_lines_still_copies_every_row(cx: &mut TestAppContext) {
    let (fixture, tab) = open_full_tab(cx);
    tab.update(cx, |tab, _| tab.time_zone = TimeZone::UTC);
    select(&fixture, &tab, 1, false, cx);
    tab.update(cx, |tab, cx| {
        tab.run_toolbar_action(ToolbarAction::Copy, cx)
    });
    assert_eq!(
        clipboard_text(cx).as_deref(),
        Some("10:00:00.000 first\n10:00:01.000 second")
    );
}

#[gpui_kit::test]
fn the_selection_actions_copy_and_clear_through_the_focused_tab(cx: &mut TestAppContext) {
    use gpui_kit::Action as _;

    let (fixture, tab) = open_full_tab(cx);
    tab.update(cx, |tab, _| tab.time_zone = TimeZone::UTC);
    select(&fixture, &tab, 0, true, cx);
    fixture.draw(cx);
    fixture.with_window(cx, |window, cx| {
        window.dispatch_action(CopyLogLines.boxed_clone(), cx);
    });
    assert_eq!(clipboard_text(cx).as_deref(), Some("10:00:00.000 first"));

    fixture.with_window(cx, |window, cx| {
        window.dispatch_action(ClearLogSelection.boxed_clone(), cx);
    });
    assert_eq!(tab.read_with(cx, |tab, _| tab.selection), None);
}
