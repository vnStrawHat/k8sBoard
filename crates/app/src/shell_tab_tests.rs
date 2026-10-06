use cluster::{ClusterError, ShellExit};
use futures::channel::mpsc::{self, UnboundedReceiver};
use gpui_kit::base::Root;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::dialog::{Cancel, Confirm};
use gpui_kit::component::input::InputEvent;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AppContext as _, Bounds, ClipboardItem, Entity, Keystroke, ParentElement as _, Point, Render,
    TestAppContext, WindowBounds, WindowHandle, WindowOptions, div, size,
};
use oneterm_vt::{CellWidth, SnapshotContent, SnapshotRow};

use super::*;
use crate::keymap::{TerminalCopy, TerminalFind, TerminalPaste};

fn target() -> ShellTarget {
    ShellTarget {
        cluster: ClusterRef {
            kubeconfig: std::path::PathBuf::from("test.yaml"),
            context: "stg-b".to_owned(),
        },
        namespace: "payments".to_owned(),
        pod: "api-7d9f8c-m8n2p".to_owned(),
        short_pod: "api-m8n2p".to_owned(),
        container: "api".to_owned(),
    }
}

struct Host {
    tab: Entity<ShellTab>,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(self.tab.clone())
    }
}

struct Fixture {
    window: WindowHandle<Root>,
    tab: Entity<ShellTab>,
    _host: Entity<Host>,
    input: UnboundedReceiver<ShellInput>,
}

fn open_tab(width: f32, height: f32, cx: &mut TestAppContext) -> Fixture {
    let (window, host, input) = cx.update(|cx| {
        gpui_kit::init(cx);
        crate::keymap::bind_keys(cx);
        let bounds = Bounds {
            origin: Point::default(),
            size: size(gpui_kit::px(width), gpui_kit::px(height)),
        };
        let (input_sender, input) = mpsc::unbounded();
        let (window, host) = gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            cx,
            |window, cx| {
                let tab = cx.new(|cx| {
                    let mut tab = ShellTab::new(
                        target(),
                        "stg-b".to_owned(),
                        WeakEntity::new_invalid(),
                        window,
                        cx,
                    );
                    // No session runs in these tests: the fake receiver stands in for the one
                    // the transport would own.
                    tab.input = input_sender;
                    tab
                });
                cx.new(|_| Host { tab })
            },
        )
        .expect("open the test window");
        (
            window.downcast::<Root>().expect("a Root window"),
            host,
            input,
        )
    });
    let tab = host.read_with(cx, |host, _| host.tab.clone());
    Fixture {
        window,
        tab,
        _host: host,
        input,
    }
}

fn render(fixture: &Fixture, cx: &mut TestAppContext) {
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx)
    })
    .expect("the window is open");
    cx.run_until_parked();
}

fn apply(fixture: &Fixture, update: ShellUpdate, cx: &mut TestAppContext) {
    fixture.tab.update(cx, |tab, cx| tab.apply(update, cx));
}

fn row_text(row: &SnapshotRow) -> String {
    let mut text = String::new();
    for cell in &row.cells {
        if cell.width == CellWidth::WideSpacer {
            continue;
        }
        match cell.content {
            SnapshotContent::Scalar(scalar) => text.push(scalar),
            SnapshotContent::Cluster { start, len } => text.extend(row.cluster(start, len)),
        }
    }
    text.trim_end().to_owned()
}

fn screen_text(fixture: &Fixture, cx: &mut TestAppContext) -> Vec<String> {
    fixture.tab.read_with(cx, |tab, _| {
        tab.session
            .borrow_mut()
            .snapshot(Instant::now())
            .rows()
            .iter()
            .map(row_text)
            .collect()
    })
}

fn resizes(fixture: &mut Fixture) -> Vec<GridSize> {
    let mut sizes = Vec::new();
    while let Ok(input) = fixture.input.try_recv() {
        if let ShellInput::Resize(size) = input {
            sizes.push(size);
        }
    }
    sizes
}

#[gpui_kit::test]
fn shell_tab_renders_fed_bytes(cx: &mut TestAppContext) {
    let fixture = open_tab(800., 500., cx);
    apply(&fixture, ShellUpdate::Started, cx);
    apply(
        &fixture,
        ShellUpdate::Output(b"hello\r\nworld".to_vec()),
        cx,
    );
    render(&fixture, cx);
    let rows = screen_text(&fixture, cx);
    // The banner note comes only from `connect`; the fed lines follow whatever is above them.
    assert!(rows.iter().any(|row| row == "hello"), "{rows:?}");
    assert!(rows.iter().any(|row| row == "world"), "{rows:?}");
    // The measured pane gave the grid its size.
    let (grid, metrics) = fixture.tab.read_with(cx, |tab, _| {
        (tab.session.borrow().grid_size(), tab.metrics.get())
    });
    let metrics = metrics.expect("the first frame measured the pane");
    assert!(grid.cols > 2 && grid.rows > 1, "{grid:?}");
    assert!(f32::from(metrics.cell.width) > 0.);
    assert_eq!(
        usize::from(grid.rows),
        rows.len(),
        "the snapshot has one row per grid row"
    );
}

#[gpui_kit::test]
fn resizing_the_window_queues_one_resize(cx: &mut TestAppContext) {
    let mut fixture = open_tab(800., 500., cx);
    render(&fixture, cx);
    let first = resizes(&mut fixture);
    assert_eq!(first.len(), 1, "the first frame sizes the grid once");
    render(&fixture, cx);
    assert!(resizes(&mut fixture).is_empty(), "same size, no resize");

    cx.simulate_window_resize(
        fixture.window.into(),
        size(gpui_kit::px(640.), gpui_kit::px(360.)),
    );
    render(&fixture, cx);
    render(&fixture, cx);
    let after = resizes(&mut fixture);
    assert_eq!(after.len(), 1, "two frames at the new size, one resize");
    assert_ne!(after[0], first[0]);
}

#[gpui_kit::test]
fn header_shows_auto_until_started(cx: &mut TestAppContext) {
    let fixture = open_tab(800., 500., cx);
    let label = |cx: &mut TestAppContext| fixture.tab.read_with(cx, |tab, _| tab.shell_label());
    assert_eq!(label(cx), "Auto");
    apply(&fixture, ShellUpdate::Started, cx);
    assert_eq!(label(cx), "Auto", "starting does not name the shell");
    apply(
        &fixture,
        ShellUpdate::Output(b"\x1b]7770;bash\x07".to_vec()),
        cx,
    );
    assert_eq!(label(cx), "bash");
}

#[gpui_kit::test]
fn a_session_walks_connecting_live_ended(cx: &mut TestAppContext) {
    let fixture = open_tab(800., 500., cx);
    let state = |cx: &mut TestAppContext| fixture.tab.read_with(cx, |tab, _| tab.state().clone());
    assert_eq!(state(cx), ShellState::Connecting);
    apply(&fixture, ShellUpdate::Started, cx);
    assert_eq!(state(cx), ShellState::Live);
    apply(
        &fixture,
        ShellUpdate::Exited(ShellExit {
            code: Some(1),
            message: None,
        }),
        cx,
    );
    assert_eq!(
        state(cx),
        ShellState::Ended(ShellEnd::Exited { code: Some(1) })
    );
    render(&fixture, cx);
    let rows = screen_text(&fixture, cx);
    assert!(
        rows.iter().any(|row| row == "[process exited with code 1]"),
        "{rows:?}"
    );
}

#[gpui_kit::test]
fn a_missing_executable_reads_as_no_shell(cx: &mut TestAppContext) {
    let fixture = open_tab(800., 500., cx);
    apply(&fixture, ShellUpdate::Started, cx);
    apply(
        &fixture,
        ShellUpdate::Exited(ShellExit {
            code: None,
            message: Some("exec: \"sh\": executable file not found in $PATH".to_owned()),
        }),
        cx,
    );
    let state = fixture.tab.read_with(cx, |tab, _| tab.state().clone());
    assert_eq!(state, ShellState::Ended(ShellEnd::NoShell));
}

#[gpui_kit::test]
fn a_failed_open_is_reported_once_and_ends_the_tab(cx: &mut TestAppContext) {
    let fixture = open_tab(800., 500., cx);
    let events = std::rc::Rc::new(RefCell::new(Vec::new()));
    let seen = std::rc::Rc::clone(&events);
    cx.update(|cx| {
        cx.subscribe(&fixture.tab, move |_, event: &ShellEvent, _| {
            seen.borrow_mut().push(event.clone());
        })
        .detach();
    });
    fixture
        .tab
        .update(cx, |tab, _| tab.is_start_unreported = true);
    apply(
        &fixture,
        ShellUpdate::Failed(ClusterError::Rendered {
            message: "writes are blocked".to_owned(),
        }),
        cx,
    );
    assert_eq!(
        *events.borrow(),
        [
            ShellEvent::OpenFailed {
                error: "writes are blocked".to_owned()
            },
            ShellEvent::Ended
        ]
    );
    let state = fixture.tab.read_with(cx, |tab, _| tab.state().clone());
    assert!(matches!(state, ShellState::Ended(ShellEnd::Failed { .. })));
}

#[gpui_kit::test]
fn the_start_is_reported_once(cx: &mut TestAppContext) {
    let fixture = open_tab(800., 500., cx);
    let events = std::rc::Rc::new(RefCell::new(Vec::new()));
    let seen = std::rc::Rc::clone(&events);
    cx.update(|cx| {
        cx.subscribe(&fixture.tab, move |_, event: &ShellEvent, _| {
            seen.borrow_mut().push(event.clone());
        })
        .detach();
    });
    fixture
        .tab
        .update(cx, |tab, _| tab.is_start_unreported = true);
    apply(&fixture, ShellUpdate::Started, cx);
    // A later break of the connection is not a second start.
    apply(
        &fixture,
        ShellUpdate::Failed(ClusterError::Rendered {
            message: "gone".to_owned(),
        }),
        cx,
    );
    // The break ends the session; it is no second start.
    assert_eq!(*events.borrow(), [ShellEvent::Opened, ShellEvent::Ended]);
}

#[gpui_kit::test]
fn clear_empties_the_screen_and_tells_nobody(cx: &mut TestAppContext) {
    let mut fixture = open_tab(800., 500., cx);
    apply(&fixture, ShellUpdate::Started, cx);
    apply(&fixture, ShellUpdate::Output(b"secret line".to_vec()), cx);
    render(&fixture, cx);
    resizes(&mut fixture);
    fixture.tab.update(cx, |tab, cx| tab.clear(cx));
    render(&fixture, cx);
    let rows = screen_text(&fixture, cx);
    assert!(rows.iter().all(|row| row.is_empty()), "{rows:?}");
    assert!(
        fixture.input.try_recv().is_err(),
        "the remote shell is not told"
    );
}

#[test]
fn the_banner_names_pod_container_shell_and_cluster() {
    assert_eq!(
        banner(&target(), ShellCommand::Auto, "stg-b"),
        "# exec -n payments api-7d9f8c-m8n2p -c api -- auto (stg-b)"
    );
    assert_eq!(
        banner(&target(), ShellCommand::Bash, "stg-b"),
        "# exec -n payments api-7d9f8c-m8n2p -c api -- bash (stg-b)"
    );
}

#[test]
fn an_exit_maps_to_the_state_the_tab_shows() {
    let exit = |code, message: Option<&str>| ShellExit {
        code,
        message: message.map(str::to_owned),
    };
    assert_eq!(
        exit_end(&exit(Some(0), None)),
        ShellEnd::Exited { code: Some(0) }
    );
    assert_eq!(
        exit_end(&exit(None, Some("boom"))),
        ShellEnd::Failed {
            reason: "boom".into()
        }
    );
    assert_eq!(exit_end(&exit(None, None)), ShellEnd::Exited { code: None });
}

#[test]
fn end_notes_read_as_the_wireframe_says() {
    assert_eq!(
        end_note(&ShellEnd::Exited { code: Some(1) }, true),
        "[process exited with code 1]"
    );
    assert_eq!(
        end_note(
            &ShellEnd::Failed {
                reason: "reset".into()
            },
            true
        ),
        "[connection lost: reset]"
    );
    assert_eq!(
        end_note(
            &ShellEnd::Failed {
                reason: "denied".into()
            },
            false
        ),
        "[could not open the shell: denied]"
    );
}

#[gpui_kit::test]
fn the_tab_label_names_pod_and_container(cx: &mut TestAppContext) {
    let fixture = open_tab(800., 500., cx);
    let label = fixture.tab.read_with(cx, |tab, _| tab.label());
    assert_eq!(label, "shell · api-m8n2p/api");
}

// ---- input (step 3b) ----

fn press(fixture: &Fixture, key: &str, cx: &mut TestAppContext) {
    let keystroke = Keystroke::parse(key).expect("a valid keystroke");
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.dispatch_keystroke(keystroke, cx);
    })
    .expect("the window is open");
    cx.run_until_parked();
}

fn dispatch(fixture: &Fixture, action: impl gpui_kit::Action, cx: &mut TestAppContext) {
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.dispatch_action(Box::new(action), cx);
    })
    .expect("the window is open");
    cx.run_until_parked();
}

/// The bytes the program was sent, in order, without the resizes.
fn sent_bytes(fixture: &mut Fixture) -> Vec<Vec<u8>> {
    let mut sent = Vec::new();
    while let Ok(input) = fixture.input.try_recv() {
        if let ShellInput::Bytes(bytes) = input {
            sent.push(bytes);
        }
    }
    sent
}

fn has_dialog(fixture: &Fixture, cx: &mut TestAppContext) -> bool {
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.has_active_dialog(cx)
    })
    .expect("the window is open")
}

/// A live tab after its first frame, with the resize of that frame already read.
fn live_tab(cx: &mut TestAppContext) -> Fixture {
    let mut fixture = open_tab(800., 500., cx);
    apply(&fixture, ShellUpdate::Started, cx);
    render(&fixture, cx);
    resizes(&mut fixture);
    fixture
}

#[gpui_kit::test]
fn typing_sends_encoded_bytes(cx: &mut TestAppContext) {
    let mut fixture = live_tab(cx);
    press(&fixture, "l", cx);
    press(&fixture, "ctrl-c", cx);
    press(&fixture, "enter", cx);
    assert_eq!(
        sent_bytes(&mut fixture),
        [b"l".to_vec(), vec![0x03], b"\r".to_vec()]
    );
}

#[gpui_kit::test]
fn shell_chords_reach_the_program_instead_of_the_app(cx: &mut TestAppContext) {
    let mut fixture = live_tab(cx);
    for (key, byte) in [("ctrl-k", 0x0b), ("ctrl-n", 0x0e), ("ctrl-w", 0x17)] {
        press(&fixture, key, cx);
        assert_eq!(sent_bytes(&mut fixture), [vec![byte]], "{key}");
    }
    press(&fixture, "tab", cx);
    assert_eq!(sent_bytes(&mut fixture), [b"\t".to_vec()]);
}

#[gpui_kit::test]
fn platform_chords_are_never_sent(cx: &mut TestAppContext) {
    let mut fixture = live_tab(cx);
    press(&fixture, "cmd-k", cx);
    assert!(sent_bytes(&mut fixture).is_empty());
}

#[gpui_kit::test]
fn ended_session_ignores_input(cx: &mut TestAppContext) {
    let mut fixture = live_tab(cx);
    apply(
        &fixture,
        ShellUpdate::Exited(ShellExit {
            code: Some(0),
            message: None,
        }),
        cx,
    );
    press(&fixture, "a", cx);
    cx.write_to_clipboard(ClipboardItem::new_string("echo hi".to_owned()));
    dispatch(&fixture, TerminalPaste, cx);
    assert!(sent_bytes(&mut fixture).is_empty());
}

#[gpui_kit::test]
fn a_program_reply_goes_back_to_the_program(cx: &mut TestAppContext) {
    let mut fixture = live_tab(cx);
    // A device attributes query: the terminal answers on its own.
    apply(&fixture, ShellUpdate::Output(b"\x1b[c".to_vec()), cx);
    let sent = sent_bytes(&mut fixture);
    assert_eq!(sent.len(), 1);
    assert!(sent[0].starts_with(b"\x1b["));
}

#[gpui_kit::test]
fn a_single_line_paste_is_sent_cleaned(cx: &mut TestAppContext) {
    let mut fixture = live_tab(cx);
    cx.write_to_clipboard(ClipboardItem::new_string("ls\x03 -la".to_owned()));
    dispatch(&fixture, TerminalPaste, cx);
    assert_eq!(sent_bytes(&mut fixture), [b"ls -la".to_vec()]);
    assert!(!has_dialog(&fixture, cx));
}

#[gpui_kit::test]
fn multi_line_paste_asks_first_and_sends_on_confirm(cx: &mut TestAppContext) {
    let mut fixture = live_tab(cx);
    cx.write_to_clipboard(ClipboardItem::new_string("a\nb".to_owned()));
    dispatch(&fixture, TerminalPaste, cx);
    assert!(has_dialog(&fixture, cx), "two lines ask first");
    assert!(
        sent_bytes(&mut fixture).is_empty(),
        "nothing is sent before the answer"
    );
    dispatch(&fixture, Confirm { secondary: false }, cx);
    assert_eq!(sent_bytes(&mut fixture), [b"a\rb".to_vec()]);
    assert!(!has_dialog(&fixture, cx));
}

#[gpui_kit::test]
fn multi_line_paste_cancel_sends_nothing(cx: &mut TestAppContext) {
    let mut fixture = live_tab(cx);
    cx.write_to_clipboard(ClipboardItem::new_string("a\nb\nc".to_owned()));
    dispatch(&fixture, TerminalPaste, cx);
    dispatch(&fixture, Cancel, cx);
    assert!(sent_bytes(&mut fixture).is_empty());
    assert!(!has_dialog(&fixture, cx));
}

#[gpui_kit::test]
fn bracketed_paste_skips_the_dialog_and_keeps_its_newlines(cx: &mut TestAppContext) {
    let mut fixture = live_tab(cx);
    // The program turns bracketed paste on.
    apply(&fixture, ShellUpdate::Output(b"\x1b[?2004h".to_vec()), cx);
    cx.write_to_clipboard(ClipboardItem::new_string("a\nb".to_owned()));
    dispatch(&fixture, TerminalPaste, cx);
    assert!(!has_dialog(&fixture, cx));
    assert_eq!(
        sent_bytes(&mut fixture),
        [b"\x1b[200~a\nb\x1b[201~".to_vec()]
    );
}

#[gpui_kit::test]
fn an_oversized_paste_is_refused(cx: &mut TestAppContext) {
    let mut fixture = live_tab(cx);
    cx.write_to_clipboard(ClipboardItem::new_string("x".repeat(MAX_PASTE_BYTES + 1)));
    dispatch(&fixture, TerminalPaste, cx);
    assert!(sent_bytes(&mut fixture).is_empty());
}

#[gpui_kit::test]
fn copy_puts_the_selection_on_the_clipboard_and_nothing_without_one(cx: &mut TestAppContext) {
    let fixture = live_tab(cx);
    apply(&fixture, ShellUpdate::Output(b"token=abc123".to_vec()), cx);
    render(&fixture, cx);
    cx.write_to_clipboard(ClipboardItem::new_string("before".to_owned()));
    // No selection: the chord changes nothing.
    dispatch(&fixture, TerminalCopy, cx);
    let kept = cx.read_from_clipboard().and_then(|item| item.text());
    assert_eq!(kept.as_deref(), Some("before"));
    // Select the line the text is on, then copy.
    fixture.tab.update(cx, |tab, _| {
        tab.session
            .borrow_mut()
            .begin_selection(0., 0., SelectionKind::Lines);
    });
    let selected = fixture
        .tab
        .read_with(cx, |tab, _| tab.session.borrow().selected_text());
    assert!(
        selected
            .as_deref()
            .is_some_and(|text| text.contains("token=abc123"))
    );
    dispatch(&fixture, TerminalCopy, cx);
    let copied = cx.read_from_clipboard().and_then(|item| item.text());
    assert!(
        copied
            .as_deref()
            .is_some_and(|text| text.contains("token=abc123"))
    );
}

#[gpui_kit::test]
fn find_opens_counts_and_closes_with_escape(cx: &mut TestAppContext) {
    let fixture = live_tab(cx);
    apply(
        &fixture,
        ShellUpdate::Output(b"api one\r\nweb\r\nAPI two\r\napi three".to_vec()),
        cx,
    );
    render(&fixture, cx);
    dispatch(&fixture, TerminalFind, cx);
    render(&fixture, cx);
    assert!(fixture.tab.read_with(cx, |tab, _| tab.is_find_open));
    // `set_value` is silent, so the events the field would emit are delivered by hand.
    let find_event = |event: InputEvent, cx: &mut TestAppContext| {
        cx.update_window(fixture.window.into(), |_, window, cx| {
            fixture.tab.update(cx, |tab, cx| {
                let input = tab.find_input.clone();
                tab.on_find_event(&input, &event, window, cx);
            });
        })
        .expect("the window is open");
    };
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.tab.update(cx, |tab, cx| {
            tab.find_input
                .update(cx, |input, cx| input.set_value("api", window, cx));
        });
    })
    .expect("the window is open");
    find_event(InputEvent::Change, cx);
    let status = |cx: &mut TestAppContext| {
        fixture
            .tab
            .read_with(cx, |tab, cx| tab.find_status_text(cx))
    };
    assert_eq!(status(cx).as_deref(), Some("1 of 3"));
    // Enter goes to the next older match and Shift Enter back.
    find_event(
        InputEvent::PressEnter {
            secondary: false,
            shift: false,
        },
        cx,
    );
    assert_eq!(status(cx).as_deref(), Some("2 of 3"));
    find_event(
        InputEvent::PressEnter {
            secondary: false,
            shift: true,
        },
        cx,
    );
    assert_eq!(status(cx).as_deref(), Some("1 of 3"));
    // Escape in the field closes Find and drops the highlights.
    press(&fixture, "escape", cx);
    assert!(!fixture.tab.read_with(cx, |tab, _| tab.is_find_open));
    let matches = fixture
        .tab
        .read_with(cx, |tab, _| tab.session.borrow().find_matches().len());
    assert_eq!(matches, 0);
}

fn enter_event(is_held: bool) -> gpui_kit::KeyDownEvent {
    gpui_kit::KeyDownEvent {
        keystroke: Keystroke::parse("enter").expect("a valid keystroke"),
        is_held,
        prefer_character_input: false,
    }
}

fn send_event(fixture: &Fixture, event: gpui_kit::KeyDownEvent, cx: &mut TestAppContext) {
    use gpui_kit::InputEvent as _;
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.dispatch_event(event.to_platform_input(), cx);
    })
    .expect("the window is open");
    cx.run_until_parked();
}

#[gpui_kit::test]
fn a_held_enter_does_not_confirm_the_multi_line_paste(cx: &mut TestAppContext) {
    let mut fixture = live_tab(cx);
    cx.write_to_clipboard(ClipboardItem::new_string("a\nb".to_owned()));
    dispatch(&fixture, TerminalPaste, cx);
    render(&fixture, cx);
    assert!(has_dialog(&fixture, cx));
    // The key repeat of an Enter that was already down when the dialog opened.
    send_event(&fixture, enter_event(true), cx);
    send_event(&fixture, enter_event(true), cx);
    assert!(
        has_dialog(&fixture, cx),
        "a held Enter leaves the dialog open"
    );
    assert!(sent_bytes(&mut fixture).is_empty(), "and pastes nothing");
    // A fresh press confirms.
    send_event(&fixture, enter_event(false), cx);
    assert!(!has_dialog(&fixture, cx));
    assert_eq!(sent_bytes(&mut fixture), [b"a\rb".to_vec()]);
}

// ---- 0037: debug tabs ----

const DEBUG_IMAGE: &str = "docker.io/library/busybox:1.36.1@sha256:abc";

fn debug_kind() -> ShellKind {
    ShellKind::Debug {
        target_container: "api".to_owned(),
        image: DEBUG_IMAGE.to_owned(),
    }
}

fn node_kind() -> ShellKind {
    ShellKind::NodeShell {
        node: "wk-03".to_owned(),
        image: DEBUG_IMAGE.to_owned(),
    }
}

/// A tab of `kind` over the ephemeral container `k8sboard-debug-x7k2q`.
fn open_debug_tab(kind: ShellKind, cx: &mut TestAppContext) -> Fixture {
    let fixture = open_tab(900., 300., cx);
    fixture.tab.update(cx, |tab, _| {
        tab.target.container = "k8sboard-debug-x7k2q".to_owned();
        tab.kind = kind;
    });
    fixture
}

fn is_drawn(fixture: &Fixture, id: &'static str, cx: &mut TestAppContext) -> bool {
    use gpui_kit::test::TestWindowExt as _;
    cx.update_window(fixture.window.into(), |_, window, _| {
        window.try_find(id).is_some()
    })
    .expect("the window is open")
}

#[gpui_kit::test]
fn debug_tab_label_and_hidden_picker(cx: &mut TestAppContext) {
    let exec = open_tab(900., 300., cx);
    render(&exec, cx);
    render(&exec, cx);
    assert_eq!(
        exec.tab.read_with(cx, |tab, _| tab.label()),
        "shell · api-m8n2p/api"
    );
    assert!(
        is_drawn(&exec, "shell-picker", cx),
        "an exec picks its shell"
    );
    let debug = open_debug_tab(debug_kind(), cx);
    render(&debug, cx);
    render(&debug, cx);
    assert_eq!(
        debug.tab.read_with(cx, |tab, _| tab.label()),
        "debug · api-m8n2p/api"
    );
    assert!(
        !is_drawn(&debug, "shell-picker", cx),
        "a debug shell has no picker"
    );
    let node = open_debug_tab(node_kind(), cx);
    render(&node, cx);
    render(&node, cx);
    assert_eq!(
        node.tab.read_with(cx, |tab, _| tab.label()),
        "node shell · wk-03 (debug pod)"
    );
    assert!(!is_drawn(&node, "shell-picker", cx));
}

#[gpui_kit::test]
fn debug_tab_headers_name_the_container_the_image_and_the_context(cx: &mut TestAppContext) {
    let debug = open_debug_tab(debug_kind(), cx);
    assert_eq!(
        debug.tab.read_with(cx, |tab, _| tab.header_text()),
        format!("›_ debug k8sboard-debug-x7k2q → api · api-7d9f8c-m8n2p · {DEBUG_IMAGE} · stg-b")
    );
    let node = open_debug_tab(node_kind(), cx);
    node.tab.update(cx, |tab, _| {
        tab.target.namespace = "kube-system".to_owned();
        tab.target.pod = "k8sboard-node-shell-wk-03-x7k2q".to_owned();
    });
    assert_eq!(
        node.tab.read_with(cx, |tab, _| tab.header_text()),
        format!(
            "›_ node wk-03 · pod kube-system/k8sboard-node-shell-wk-03-x7k2q · {DEBUG_IMAGE} · stg-b"
        )
    );
}

#[gpui_kit::test]
fn a_started_debug_tab_hints_at_the_prompt_and_an_exec_does_not(cx: &mut TestAppContext) {
    let debug = open_debug_tab(debug_kind(), cx);
    apply(&debug, ShellUpdate::Started, cx);
    let text = screen_text(&debug, cx).join("\n");
    assert!(
        text.contains("If you don't see a prompt, press Enter."),
        "{text}"
    );
    let exec = open_tab(900., 300., cx);
    apply(&exec, ShellUpdate::Started, cx);
    assert!(!screen_text(&exec, cx).join("\n").contains("press Enter"));
}

#[gpui_kit::test]
fn waiting_reasons_reach_the_terminal_as_notes(cx: &mut TestAppContext) {
    let debug = open_debug_tab(debug_kind(), cx);
    apply(
        &debug,
        ShellUpdate::Waiting("ContainerCreating".to_owned()),
        cx,
    );
    apply(&debug, ShellUpdate::Waiting("Pulling".to_owned()), cx);
    let text = screen_text(&debug, cx).join("\n");
    assert!(text.contains("Creating container…"), "{text}");
    assert!(text.contains("Pulling image…"), "{text}");
}

#[test]
fn waiting_words_read_as_plain_text() {
    assert_eq!(
        waiting_text("ContainerCreating", &debug_kind()),
        "Creating container…"
    );
    assert_eq!(waiting_text("PodInitializing", &debug_kind()), "Starting…");
    assert_eq!(waiting_text("Pulling", &debug_kind()), "Pulling image…");
    assert_eq!(waiting_text("Odd", &debug_kind()), "Waiting: Odd");
    assert_eq!(starting_text(&debug_kind()), "Starting debug container…");
    assert_eq!(starting_text(&node_kind()), "Starting node shell pod…");
}

#[gpui_kit::test]
fn no_shell_tab_offers_debug_container(cx: &mut TestAppContext) {
    let fixture = open_tab(900., 300., cx);
    render(&fixture, cx);
    render(&fixture, cx);
    assert!(!is_drawn(&fixture, "shell-debug-container", cx));
    apply(
        &fixture,
        ShellUpdate::Exited(ShellExit {
            code: None,
            message: Some("OCI runtime exec failed: executable file not found in $PATH".into()),
        }),
        cx,
    );
    render(&fixture, cx);
    render(&fixture, cx);
    assert_eq!(
        fixture.tab.read_with(cx, |tab, _| tab.state().clone()),
        ShellState::Ended(ShellEnd::NoShell)
    );
    assert!(is_drawn(&fixture, "shell-debug-container", cx));
    let text = screen_text(&fixture, cx).join("\n");
    assert!(
        text.contains("No shell in this container; try Debug container…"),
        "{text}"
    );
}

#[gpui_kit::test]
fn the_end_of_a_session_is_reported_once_for_the_cleanup(cx: &mut TestAppContext) {
    use std::cell::RefCell;
    let fixture = open_debug_tab(node_kind(), cx);
    let events = std::rc::Rc::new(RefCell::new(Vec::new()));
    let log = std::rc::Rc::clone(&events);
    let _subscription = cx.update(|cx| {
        cx.subscribe(&fixture.tab, move |_, event: &ShellEvent, _| {
            log.borrow_mut().push(event.clone());
        })
    });
    fixture
        .tab
        .update(cx, |tab, _| tab.is_start_unreported = true);
    apply(&fixture, ShellUpdate::Started, cx);
    apply(
        &fixture,
        ShellUpdate::Exited(ShellExit {
            code: Some(0),
            message: None,
        }),
        cx,
    );
    assert_eq!(*events.borrow(), [ShellEvent::Opened, ShellEvent::Ended]);
}

#[gpui_kit::test]
fn node_shell_tab_label_matches_w5(cx: &mut TestAppContext) {
    let node = open_debug_tab(node_kind(), cx);
    assert_eq!(
        node.tab.read_with(cx, |tab, _| tab.label()),
        "node shell · wk-03 (debug pod)"
    );
}

#[gpui_kit::test]
fn shell_tabs_have_no_pop_out(cx: &mut TestAppContext) {
    // A shell holds window-bound state and counts in `leaving_work`, so it stays in the dock.
    let fixture = open_tab(900., 300., cx);
    render(&fixture, cx);
    render(&fixture, cx);
    assert!(
        is_drawn(&fixture, "shell-clear", cx),
        "the toolbar is drawn"
    );
    assert!(!is_drawn(&fixture, "log-pop-out", cx));
}

#[gpui_kit::test]
fn attach_tab_label_and_header(cx: &mut TestAppContext) {
    let attach = open_debug_tab(ShellKind::Attach, cx);
    attach
        .tab
        .update(cx, |tab, _| tab.target.container = "api".to_owned());
    render(&attach, cx);
    render(&attach, cx);
    assert_eq!(
        attach.tab.read_with(cx, |tab, _| tab.label()),
        "attach · api-m8n2p/api"
    );
    assert_eq!(
        attach.tab.read_with(cx, |tab, _| tab.header_text()),
        "›_ attach api-7d9f8c-m8n2p · api · stg-b"
    );
    assert!(
        !is_drawn(&attach, "shell-picker", cx),
        "an attach has no shell picker"
    );
    assert!(
        !is_drawn(&attach, "shell-reconnect", cx),
        "an attach has no Reconnect: A attaches anew"
    );
    // Find and Clear stay, as in every shell tab.
    assert!(is_drawn(&attach, "shell-find", cx));
    assert!(is_drawn(&attach, "shell-clear", cx));
    // Every other kind keeps its Reconnect.
    let exec = open_tab(900., 300., cx);
    render(&exec, cx);
    render(&exec, cx);
    assert!(is_drawn(&exec, "shell-reconnect", cx));
    assert_eq!(starting_text(&ShellKind::Attach), "Attaching…");
}

#[gpui_kit::test]
fn an_ended_attach_tab_offers_no_debug_container(cx: &mut TestAppContext) {
    let attach = open_debug_tab(ShellKind::Attach, cx);
    apply(
        &attach,
        ShellUpdate::Exited(ShellExit {
            code: None,
            message: Some("OCI runtime exec failed: executable file not found in $PATH".into()),
        }),
        cx,
    );
    render(&attach, cx);
    render(&attach, cx);
    assert!(!is_drawn(&attach, "shell-debug-container", cx));
}

#[gpui_kit::test]
fn an_attach_tab_ends_with_the_exit_code(cx: &mut TestAppContext) {
    let attach = open_debug_tab(ShellKind::Attach, cx);
    apply(&attach, ShellUpdate::Started, cx);
    apply(
        &attach,
        ShellUpdate::Exited(ShellExit {
            code: Some(137),
            message: None,
        }),
        cx,
    );
    let text = screen_text(&attach, cx).join("\n");
    assert!(text.contains("[process exited with code 137]"), "{text}");
}

#[gpui_kit::test]
fn a_tab_takes_the_saved_shell_and_scrollback_but_an_attach_has_no_shell_choice(
    cx: &mut TestAppContext,
) {
    use crate::settings::{AppSettings, Settings, TerminalSettings};
    use crate::settings_store::{LoadedSettings, WriteMode};
    cx.update(|cx| {
        gpui_kit::init(cx);
        AppSettings::install(
            LoadedSettings {
                settings: Settings {
                    terminal: TerminalSettings {
                        default_shell: ShellCommand::Bash,
                        ..TerminalSettings::default()
                    },
                    ..Settings::default()
                },
                writes: WriteMode::Disabled,
                notice: None,
            },
            cx,
        );
    });
    cx.update(|cx| {
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            let exec = cx.new(|cx| {
                ShellTab::new(
                    target(),
                    "stg-b".to_owned(),
                    WeakEntity::new_invalid(),
                    window,
                    cx,
                )
            });
            assert_eq!(exec.read(cx).command(), ShellCommand::Bash);
            let attach = cx.new(|cx| {
                ShellTab::new(
                    target(),
                    "stg-b".to_owned(),
                    WeakEntity::new_invalid(),
                    window,
                    cx,
                )
                .with_kind(ShellKind::Attach)
            });
            assert_eq!(attach.read(cx).command(), ShellCommand::Auto);
            cx.new(|_| Host { tab: exec })
        })
        .expect("open the test window");
    });
}

#[gpui_kit::test]
fn the_close_tooltip_says_a_live_shell_ends(cx: &mut TestAppContext) {
    let fixture = open_tab(800., 500., cx);
    let tooltip = |cx: &mut TestAppContext| fixture.tab.read_with(cx, |tab, _| tab.close_tooltip());
    assert_eq!(tooltip(cx), "Close");
    apply(&fixture, ShellUpdate::Started, cx);
    assert_eq!(tooltip(cx), "Close (ends the shell)");
    apply(
        &fixture,
        ShellUpdate::Exited(ShellExit {
            code: Some(0),
            message: None,
        }),
        cx,
    );
    assert_eq!(tooltip(cx), "Close");
}

#[gpui_kit::test]
fn the_exec_header_names_pod_container_shell_namespace_and_cluster(cx: &mut TestAppContext) {
    let fixture = open_tab(800., 500., cx);
    assert_eq!(
        fixture.tab.read_with(cx, |tab, _| tab.header_text()),
        "›_ api-7d9f8c-m8n2p · api · Auto · payments · stg-b"
    );
}

#[gpui_kit::test]
fn the_find_arrows_step_through_the_matches(cx: &mut TestAppContext) {
    let fixture = live_tab(cx);
    apply(
        &fixture,
        ShellUpdate::Output(b"api one\r\nweb\r\nAPI two\r\napi three".to_vec()),
        cx,
    );
    render(&fixture, cx);
    dispatch(&fixture, TerminalFind, cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.tab.update(cx, |tab, cx| {
            tab.find_input
                .update(cx, |input, cx| input.set_value("api", window, cx));
            let input = tab.find_input.clone();
            tab.on_find_event(&input, &InputEvent::Change, window, cx);
        });
    })
    .expect("the window is open");
    render(&fixture, cx);
    let status = |cx: &mut TestAppContext| {
        fixture
            .tab
            .read_with(cx, |tab, cx| tab.find_status_text(cx))
    };
    assert_eq!(status(cx).as_deref(), Some("1 of 3"));
    let click = |id: &'static str, cx: &mut TestAppContext| {
        cx.update_window(fixture.window.into(), |_, window, cx| window.click(id, cx))
            .expect("the window is open");
    };
    click("shell-find-next", cx);
    assert_eq!(status(cx).as_deref(), Some("2 of 3"));
    click("shell-find-previous", cx);
    assert_eq!(status(cx).as_deref(), Some("1 of 3"));
}

#[test]
fn a_node_shell_pod_says_which_image_it_pulls_on_which_node() {
    for reason in ["ContainerCreating", "Pulling"] {
        assert_eq!(
            waiting_text(reason, &node_kind()),
            format!("Pulling {DEBUG_IMAGE} on wk-03…")
        );
    }
    assert_eq!(waiting_text("PodInitializing", &node_kind()), "Starting…");
}

#[test]
fn a_failed_pull_names_the_image_the_node_the_cause_and_where_to_change_it() {
    assert_eq!(
        pull_failed_text(&node_kind(), Some("pull access denied")),
        format!(
            "Image {DEBUG_IMAGE} could not be pulled on wk-03: pull access denied. Set another \
             image in the node shell options or Settings › Clusters"
        )
    );
    // No readable event: the sentence still names the image and where to change it.
    let text = pull_failed_text(&debug_kind(), None);
    assert_eq!(
        text,
        format!(
            "Image {DEBUG_IMAGE} could not be pulled. Set another image in the debug options or \
             Settings › Clusters"
        )
    );
}

#[gpui_kit::test]
fn a_failed_pull_ends_the_node_shell_tab_and_replaces_reconnect(cx: &mut TestAppContext) {
    let fixture = open_debug_tab(node_kind(), cx);
    let events = std::rc::Rc::new(RefCell::new(Vec::new()));
    let seen = std::rc::Rc::clone(&events);
    cx.update(|cx| {
        cx.subscribe(&fixture.tab, move |_, event: &ShellEvent, _| {
            seen.borrow_mut().push(event.clone());
        })
        .detach();
    });
    fixture
        .tab
        .update(cx, |tab, _| tab.is_start_unreported = true);
    assert_eq!(
        fixture.tab.read_with(cx, |tab, _| tab.reconnect_texts().0),
        "Reconnect"
    );
    apply(
        &fixture,
        ShellUpdate::ImagePullFailed {
            detail: Some("not found".to_owned()),
        },
        cx,
    );
    let expected = pull_failed_text(&node_kind(), Some("not found"));
    assert_eq!(
        *events.borrow(),
        [
            ShellEvent::OpenFailed { error: expected },
            ShellEvent::Ended
        ]
    );
    assert_eq!(
        fixture.tab.read_with(cx, |tab, _| tab.reconnect_texts().0),
        "New node shell…"
    );
    let text = screen_text(&fixture, cx).join("\n");
    assert!(text.contains("could not be pulled on wk-03"), "{text}");
}
