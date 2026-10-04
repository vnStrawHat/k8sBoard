//! One shell tab of the dock (spec 0036): the header, the terminal, and the exec session behind it.
//!
//! The tab never opens a session on its own: `connect` takes the proof (`ExecPermit`) and the
//! connection of the target's own cluster, and the only callers are the guarded open and
//! Reconnect flows of the shell. Nothing from the session is logged or kept after the tab closes.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use cluster::{
    AttachPermit, AttachRequest, AttachWait, ClusterConnection, ExecPermit, GridSize, PodSummary,
    ShellCommand, ShellExit, ShellInput, ShellRequest, ShellUpdate,
};
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _, Toggle};
use gpui_kit::component::dialog::DialogButtonProps;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, InteractiveElement as _,
    IntoElement, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    ParentElement as _, Render, ScrollWheelEvent, SharedString, Styled as _, Subscription,
    WeakEntity, Window, div, px,
};
use oneterm_vt::SelectionKind;
use oneterm_vt::input::{KeyMods, KeySpec, MouseModifiers, NamedKey, encode_wheel_event};

use crate::app_shell::AppShell;
use crate::cluster_registry::ClusterRef;
use crate::cluster_runtime::{ClusterRuntime, WatchSubscription};
use crate::cluster_session::error_text;
use crate::fresh_enter::FreshEnter;
use crate::keymap::{CloseTerminalFind, TerminalCopy, TerminalFind, TerminalPaste};
use crate::log_workload::pod_short_name;
use crate::screenshot::controller_owner_of;
use crate::secret_clipboard::ClipboardWriteError;
use crate::status_tone::StatusTone;
use crate::terminal_element::{SharedMetrics, cell_at, terminal_element};
use crate::terminal_input::{MAX_PASTE_BYTES, PasteAsk, PasteDecision, decide_paste, key_to_vt};
use crate::terminal_session::TerminalSession;

/// The size a terminal starts at, before its element has measured the pane.
const START_SIZE: GridSize = GridSize { cols: 80, rows: 24 };
/// The text the server gives when the container has no such executable.
const NO_EXECUTABLE: &str = "executable file not found";

/// The container a shell tab runs in, with the cluster of its pod.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ShellTarget {
    pub(crate) cluster: ClusterRef,
    pub(crate) namespace: String,
    pub(crate) pod: String,
    /// What the tab label calls the pod: `short_pod_name`.
    pub(crate) short_pod: String,
    pub(crate) container: String,
}

/// The pod name a tab label shows: the Logs tab's rule, the last `-` segment (`m8n2p`), except for
/// a StatefulSet pod, whose ordinal alone would not tell the replicas apart. A pod no controller
/// owns keeps its name.
pub(crate) fn short_pod_name(pod: &PodSummary) -> String {
    match controller_owner_of(pod) {
        Some(owner) => pod_short_name(&owner, &pod.name).to_owned(),
        None => pod.name.clone(),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ShellState {
    Connecting,
    Live,
    Ended(ShellEnd),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ShellEnd {
    Exited { code: Option<i32> },
    Failed { reason: SharedString },
    NoShell,
}

/// What a session needs to start, taken from the target's own cluster at the moment the user
/// confirmed. The permit is proof that both exec verbs were allowed; nothing else builds one.
pub(crate) struct ShellGrant {
    pub(crate) connection: ClusterConnection,
    pub(crate) permit: ExecPermit,
    pub(crate) command: ShellCommand,
}

/// What a shell tab runs (spec 0037). A debug shell attaches to a container k8sBoard created, so
/// its tab has no shell picker and its Reconnect makes a new container, never reusing one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ShellKind {
    /// An exec into an existing container.
    Exec,
    /// A debug container added to a running pod. `ShellTarget.container` is the ephemeral
    /// container's own name; `target_container` is the one it shares a process namespace with.
    Debug {
        target_container: String,
        image: String,
    },
    /// A privileged pod on `node`. `ShellTarget` names the pod and its namespace.
    NodeShell { node: String, image: String },
    /// A running container of the pod's own spec that has a terminal (spec 0040). Its tab has no
    /// Reconnect: the user attaches anew through the same gate.
    Attach,
}

impl ShellKind {
    pub(crate) fn is_exec(&self) -> bool {
        matches!(self, Self::Exec)
    }

    pub(crate) fn is_node_shell(&self) -> bool {
        matches!(self, Self::NodeShell { .. })
    }
}

/// What a debug session needs to start, taken from the target's own cluster after the commit. The
/// permit is proof that both attach verbs were allowed; nothing else builds one.
pub(crate) struct AttachGrant {
    pub(crate) connection: ClusterConnection,
    pub(crate) permit: AttachPermit,
}

/// What the shell records about the start of a session: one audit line per start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ShellEvent {
    /// The exec or attach connection came up.
    Opened,
    /// The session ended before it came up.
    OpenFailed { error: String },
    /// The session ended, by exit, failure, or a closed stream. A node shell pod is deleted now.
    Ended,
}

impl EventEmitter<ShellEvent> for ShellTab {}

/// A left press on the terminal: where it fell. The drag starts on the first move.
#[derive(Clone, Copy)]
struct Press {
    row: f32,
    col: f32,
}

pub(crate) struct ShellTab {
    target: ShellTarget,
    /// What the tab runs; an exec unless a debug start opened it.
    kind: ShellKind,
    /// The switcher text of the cluster, for the banner and the title while several are viewed.
    cluster_label: String,
    command: ShellCommand,
    state: ShellState,
    /// Shared with the element, which sizes and paints it; never held across a frame.
    session: Rc<RefCell<TerminalSession>>,
    /// Replaced by every connect: the receiver belongs to the session that is running.
    input: UnboundedSender<ShellInput>,
    metrics: SharedMetrics,
    /// Dropping it ends the session: the tokio task stops and the WebSocket closes.
    connection: Option<WatchSubscription>,
    /// How many sessions this tab has started; a second one is a reconnect.
    starts: u32,
    /// The audit line of the running start is still owed.
    is_start_unreported: bool,
    focus_handle: FocusHandle,
    needs_focus: bool,
    press: Option<Press>,
    is_selecting: bool,
    is_find_open: bool,
    find_input: Entity<InputState>,
    _find_events: Subscription,
    /// Reconnect and a change of shell run through it, like the first open.
    app: WeakEntity<AppShell>,
}

impl ShellTab {
    pub(crate) fn new(
        target: ShellTarget,
        cluster_label: String,
        app: WeakEntity<AppShell>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        // The receiver is dropped at once: nothing listens until `connect` makes a pair.
        let (input, _) = unbounded();
        let find_input = cx.new(|cx| InputState::new(window, cx).placeholder("Find"));
        let find_events = cx.subscribe_in(&find_input, window, Self::on_find_event);
        Self {
            target,
            kind: ShellKind::Exec,
            cluster_label,
            command: ShellCommand::Auto,
            state: ShellState::Connecting,
            session: Rc::new(RefCell::new(TerminalSession::new(START_SIZE))),
            input,
            metrics: SharedMetrics::default(),
            connection: None,
            starts: 0,
            is_start_unreported: false,
            focus_handle: cx.focus_handle(),
            needs_focus: true,
            press: None,
            is_selecting: false,
            is_find_open: false,
            find_input,
            _find_events: find_events,
            app,
        }
    }

    /// The tab of a debug start: the same terminal, with the kind that names it. The kind is set
    /// before the first connect.
    pub(crate) fn with_kind(mut self, kind: ShellKind) -> Self {
        self.kind = kind;
        self
    }

    /// Starts a session. A running one ends first. The only callers are the guarded open and
    /// Reconnect flows of the shell.
    pub(crate) fn connect(&mut self, grant: ShellGrant, cx: &mut Context<Self>) {
        let ShellGrant {
            connection,
            permit,
            command,
        } = grant;
        self.command = command;
        let banner = banner(&self.target, command, &self.cluster_label);
        let (receiver, size) = self.begin_session(&banner);
        let request = ShellRequest {
            namespace: self.target.namespace.clone(),
            pod: self.target.pod.clone(),
            container: self.target.container.clone(),
            shell: command,
            size,
        };
        let updates = connection.pod_shell(permit, request, receiver);
        self.subscribe_to(updates, cx);
    }

    /// Starts the attach of a debug tab, after the container or pod it attaches to was created. The
    /// only callers are the guarded open flows of the shell.
    pub(crate) fn connect_attach(&mut self, grant: AttachGrant, cx: &mut Context<Self>) {
        let AttachGrant { connection, permit } = grant;
        let banner = attach_banner(&self.target, &self.kind, &self.cluster_label);
        let (receiver, size) = self.begin_session(&banner);
        let wait = match self.kind {
            ShellKind::NodeShell { .. } => AttachWait::NodeShellPod,
            ShellKind::Attach => AttachWait::Container,
            ShellKind::Debug { .. } | ShellKind::Exec => AttachWait::EphemeralContainer,
        };
        let request = AttachRequest {
            namespace: self.target.namespace.clone(),
            pod: self.target.pod.clone(),
            container: self.target.container.clone(),
            wait,
            size,
        };
        let updates = connection.attach_shell(permit, request, receiver);
        self.session.borrow_mut().note(starting_text(&self.kind));
        self.subscribe_to(updates, cx);
    }

    /// The part of a start both kinds share: the state, the fresh input channel, and the banner.
    fn begin_session(&mut self, banner: &str) -> (UnboundedReceiver<ShellInput>, GridSize) {
        self.state = ShellState::Connecting;
        self.is_start_unreported = true;
        let (input, receiver) = unbounded();
        self.input = input;
        let size = {
            let mut session = self.session.borrow_mut();
            if self.starts > 0 {
                session.note("──────── new session ────────");
            }
            session.note(banner);
            session.grid_size()
        };
        self.starts += 1;
        (receiver, size)
    }

    fn subscribe_to(
        &mut self,
        updates: impl futures::Stream<Item = ShellUpdate> + Send + 'static,
        cx: &mut Context<Self>,
    ) {
        let runtime = cx.global::<ClusterRuntime>().clone();
        // Replacing the subscription drops the old one, which ends the old session.
        self.connection = Some(runtime.subscribe(
            updates,
            cx,
            |tab, update, cx| tab.apply(update, cx),
            |tab, cx| tab.on_stream_closed(cx),
        ));
        cx.notify();
    }

    /// One update of the session. Output is fed to the terminal and the replies it owes the
    /// program go back at once.
    pub(crate) fn apply(&mut self, update: ShellUpdate, cx: &mut Context<Self>) {
        match update {
            // Only a debug start waits for its container; an exec never sends it.
            ShellUpdate::Waiting(reason) => {
                self.session.borrow_mut().note(&waiting_text(&reason));
            }
            ShellUpdate::Started => {
                self.state = ShellState::Live;
                if !self.kind.is_exec() {
                    self.session.borrow_mut().note(PROMPT_HINT);
                }
                if std::mem::take(&mut self.is_start_unreported) {
                    cx.emit(ShellEvent::Opened);
                }
            }
            ShellUpdate::Output(bytes) => self.feed(&bytes),
            ShellUpdate::Exited(exit) => {
                let end = exit_end(&exit);
                self.finish(end, cx);
            }
            ShellUpdate::Failed(error) => {
                let reason = error_text(&error);
                if std::mem::take(&mut self.is_start_unreported) {
                    cx.emit(ShellEvent::OpenFailed {
                        error: reason.clone(),
                    });
                }
                self.finish(
                    ShellEnd::Failed {
                        reason: reason.into(),
                    },
                    cx,
                );
            }
        }
        cx.notify();
    }

    /// The stream ended without `Exited` or `Failed`: the owner dropped it, or the runtime stopped.
    fn on_stream_closed(&mut self, cx: &mut Context<Self>) {
        if matches!(self.state, ShellState::Ended(_)) {
            return;
        }
        self.finish(
            ShellEnd::Failed {
                reason: "the connection closed".into(),
            },
            cx,
        );
    }

    fn feed(&mut self, bytes: &[u8]) {
        let owed = {
            let mut session = self.session.borrow_mut();
            session.feed(bytes, Instant::now());
            session.take_outbox()
        };
        self.send(owed);
    }

    fn finish(&mut self, end: ShellEnd, cx: &mut Context<Self>) {
        let was_started = self.state == ShellState::Live;
        let text = end_note(&end, was_started);
        self.session.borrow_mut().note(&text);
        self.state = ShellState::Ended(end);
        cx.emit(ShellEvent::Ended);
        cx.notify();
    }

    /// Queues bytes for the program. A closed receiver means the session ended: nobody to tell.
    fn send(&self, bytes: Vec<u8>) {
        if bytes.is_empty() || matches!(self.state, ShellState::Ended(_)) {
            return;
        }
        let _ = self.input.unbounded_send(ShellInput::Bytes(bytes));
    }

    /// Whether keys and pastes reach the program: not once the session has ended.
    fn accepts_input(&self) -> bool {
        !matches!(self.state, ShellState::Ended(_))
    }

    /// Keys no action consumed. Any key returns the view to the live screen.
    fn on_key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        if !self.accepts_input() {
            return;
        }
        let Some((spec, mods)) = key_to_vt(&event.keystroke) else {
            return;
        };
        let bytes = self.session.borrow().encode_key(&spec, mods);
        let Some(bytes) = bytes else {
            return;
        };
        self.send_typed(bytes, cx);
        cx.stop_propagation();
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus_handle, cx);
        let Some(metrics) = self.metrics.get() else {
            return;
        };
        let (row, col) = cell_at(metrics, event.position);
        let mut session = self.session.borrow_mut();
        session.clear_selection();
        // A plain click selects nothing; a double click selects the word and a triple click the
        // line at once.
        match event.click_count {
            0 | 1 => self.press = Some(Press { row, col }),
            2 => {
                session.begin_selection(row, col, SelectionKind::Semantic);
                self.is_selecting = true;
            }
            _ => {
                session.begin_selection(row, col, SelectionKind::Lines);
                self.is_selecting = true;
            }
        }
        drop(session);
        cx.notify();
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if event.pressed_button != Some(MouseButton::Left) {
            return;
        }
        let Some(metrics) = self.metrics.get() else {
            return;
        };
        let (row, col) = cell_at(metrics, event.position);
        let mut session = self.session.borrow_mut();
        if let Some(press) = self.press.take() {
            session.begin_selection(press.row, press.col, SelectionKind::Simple);
            self.is_selecting = true;
        }
        if self.is_selecting {
            session.extend_selection(row, col);
            drop(session);
            cx.notify();
        }
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.press = None;
        self.is_selecting = false;
    }

    /// The wheel scrolls the history on the main screen. A program that asked for mouse reports
    /// gets the wheel as reports; one on the alternate screen with alternate scroll gets arrow keys.
    fn on_scroll(&mut self, event: &ScrollWheelEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(metrics) = self.metrics.get() else {
            return;
        };
        let delta = event.delta.pixel_delta(metrics.cell.height);
        let mut lines = (delta.y / metrics.cell.height).round() as i32;
        if lines == 0 && delta.y != px(0.) {
            lines = if delta.y > px(0.) { 1 } else { -1 };
        }
        if lines == 0 {
            return;
        }
        let (row, col) = cell_at(metrics, event.position);
        let is_up = lines > 0;
        let count = lines.unsigned_abs() as usize;
        let modes = self.session.borrow().modes();
        if modes.mouse.is_some() {
            let mods = MouseModifiers {
                shift: event.modifiers.shift,
                alt: event.modifiers.alt,
                ctrl: event.modifiers.control,
            };
            let delta_y = if is_up { 1. } else { -1. };
            let report = encode_wheel_event(row as usize, col as usize, delta_y, modes, mods);
            self.send(report.repeat(count));
        } else if modes.alt_screen && modes.alternate_scroll {
            let key = KeySpec::Named(if is_up {
                NamedKey::ArrowUp
            } else {
                NamedKey::ArrowDown
            });
            let arrow = self.session.borrow().encode_key(&key, KeyMods::default());
            if let Some(arrow) = arrow {
                self.send(arrow.repeat(count));
            }
        } else {
            // Wheel up shows older rows, which the engine counts as a negative delta.
            self.session.borrow_mut().scroll_lines(-lines);
        }
        cx.stop_propagation();
        cx.notify();
    }

    /// Copies the selection through the private clipboard write of 0016. Without a selection the
    /// chord does nothing. The text never goes into a notice.
    fn on_copy(&mut self, _: &TerminalCopy, window: &mut Window, cx: &mut Context<Self>) {
        let selected = self.session.borrow().selected_text();
        let Some(text) = selected else {
            return;
        };
        if let Err(error) = copy_private(&text, cx) {
            window.push_notification(Notification::warning(error.to_string()), cx);
        }
    }

    /// Pastes the clipboard text: cleaned, wrapped when the program asked for bracketed paste, and
    /// shown first when several lines would otherwise run as they arrive.
    fn on_paste(&mut self, _: &TerminalPaste, window: &mut Window, cx: &mut Context<Self>) {
        if !self.accepts_input() {
            return;
        }
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return;
        };
        let is_bracketed = self.session.borrow().modes().bracketed_paste;
        match decide_paste(&text, is_bracketed) {
            PasteDecision::Nothing => {}
            PasteDecision::TooLarge => {
                let limit = MAX_PASTE_BYTES / 1024;
                let text = format!("The paste is larger than {limit} KiB; nothing was sent.");
                window.push_notification(Notification::warning(text), cx);
            }
            PasteDecision::Send(bytes) => self.send_typed(bytes, cx),
            PasteDecision::Ask(ask) => self.ask_before_pasting(ask, window, cx),
        }
    }

    /// Sends bytes the user typed or pasted: the view returns to the live screen first.
    fn send_typed(&mut self, bytes: Vec<u8>, cx: &mut Context<Self>) {
        self.session.borrow_mut().scroll_to_bottom();
        self.send(bytes);
        cx.notify();
    }

    fn ask_before_pasting(&mut self, ask: PasteAsk, window: &mut Window, cx: &mut Context<Self>) {
        let tab = cx.weak_entity();
        let PasteAsk {
            lines,
            preview,
            bytes,
        } = ask;
        let more = lines.saturating_sub(preview.len());
        // Held Enter must not paste lines nobody read: the content confirms on a fresh press only.
        let paste: Rc<dyn Fn(&mut App)> = Rc::new(move |cx| {
            let _ = tab.update(cx, |tab, cx| tab.send_typed(bytes.clone(), cx));
        });
        let on_enter = Rc::clone(&paste);
        let body = cx.new(|cx| {
            FreshEnter::new(
                move |cx| paste_preview(&preview, more, cx).into_any_element(),
                move |window, cx| {
                    on_enter(cx);
                    window.close_dialog(cx);
                },
                cx,
            )
        });
        window.open_alert_dialog(cx, move |alert, _, _| {
            let paste = Rc::clone(&paste);
            alert
                .title(format!("Paste {lines} lines?"))
                .child(body.clone())
                .confirm()
                .button_props(
                    DialogButtonProps::default()
                        .ok_text("Paste")
                        .show_cancel(true),
                )
                .on_ok(move |_, _, cx| {
                    paste(cx);
                    true
                })
        });
    }

    /// A new exec in this tab, through the guarded flow of the shell: its gate, lock, tier dialog,
    /// and audit line. Deferred, because the flow updates this tab. A debug tab never reuses its
    /// container or pod: Reconnect opens the options dialog again, prefilled, for a new one.
    fn request_reconnect(
        &mut self,
        command: ShellCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (app, tab) = (self.app.clone(), cx.weak_entity());
        window.defer(cx, move |window, cx| {
            let _ = app.update(cx, |shell, cx| {
                if shell.is_debug_tab(&tab, cx) {
                    shell.reopen_debug_options(&tab, window, cx);
                } else {
                    shell.reconnect_shell(&tab, command, window, cx);
                }
            });
        });
    }

    /// The `Debug container…` button of an exec that found no shell in its container: it opens
    /// the options dialog for that pod with the container prefilled.
    fn render_debug_offer(&self, cx: &Context<Self>) -> Option<impl IntoElement> {
        if self.state != ShellState::Ended(ShellEnd::NoShell) || self.kind == ShellKind::Attach {
            return None;
        }
        Some(
            Button::new("shell-debug-container")
                .small()
                .label("Debug container…")
                .tooltip("Add a debug container with a shell next to this one")
                .on_click(cx.listener(|tab, _, window, cx| {
                    let (app, tab) = (tab.app.clone(), cx.weak_entity());
                    window.defer(cx, move |window, cx| {
                        let _ = app.update(cx, |shell, cx| {
                            shell.offer_debug_container(&tab, window, cx);
                        });
                    });
                })),
        )
    }

    /// The first line of the header: what runs and where.
    fn header_text(&self) -> String {
        let context = &self.target.cluster.context;
        match &self.kind {
            ShellKind::Exec => format!(
                "›_ {} · {} · {}",
                self.target.pod,
                self.target.container,
                self.shell_label()
            ),
            ShellKind::Debug {
                target_container,
                image,
            } => format!(
                "›_ debug {} → {target_container} · {} · {image} · {context}",
                self.target.container, self.target.pod
            ),
            ShellKind::NodeShell { node, image } => format!(
                "›_ node {node} · pod {}/{} · {image} · {context}",
                self.target.namespace, self.target.pod
            ),
            ShellKind::Attach => format!(
                "›_ attach {} · {} · {context}",
                self.target.pod, self.target.container
            ),
        }
    }

    fn on_find(&mut self, _: &TerminalFind, window: &mut Window, cx: &mut Context<Self>) {
        self.open_find(window, cx);
    }

    /// Opens Find in the header and focuses its field.
    fn open_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.is_find_open = true;
        self.find_input
            .update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    fn on_close_find(
        &mut self,
        _: &CloseTerminalFind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_find(window, cx);
    }

    /// Closes Find, drops its highlights, and returns the focus to the terminal.
    fn close_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.is_find_open = false;
        self.session.borrow_mut().clear_find();
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    /// The query changed: search again. Enter goes to the next match, Shift Enter to the previous.
    fn on_find_event(
        &mut self,
        input: &Entity<InputState>,
        event: &InputEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Change => {
                let query = input.read(cx).value().to_string();
                self.session.borrow_mut().find(&query);
            }
            InputEvent::PressEnter { shift, .. } => {
                let mut session = self.session.borrow_mut();
                if *shift {
                    session.find_previous();
                } else {
                    session.find_next();
                }
            }
            InputEvent::Focus | InputEvent::Blur => return,
        }
        cx.notify();
    }

    /// `3 of 12`, or `No matches` for a query nothing matches; `None` without a query.
    fn find_status_text(&self, cx: &App) -> Option<String> {
        if self.find_input.read(cx).value().is_empty() {
            return None;
        }
        Some(match self.session.borrow().find_status() {
            Some((position, total)) => format!("{position} of {total}"),
            None => "No matches".to_owned(),
        })
    }

    /// `--screen shell-fixture`: the banner and a fixed transcript, as if a session had run.
    #[cfg(feature = "screenshot")]
    pub(crate) fn show_fixture(&mut self, transcript: &str, cx: &mut Context<Self>) {
        self.state = ShellState::Live;
        let first_line = if self.kind.is_exec() {
            banner(&self.target, self.command, &self.cluster_label)
        } else {
            attach_banner(&self.target, &self.kind, &self.cluster_label)
        };
        {
            let mut session = self.session.borrow_mut();
            session.note(&first_line);
            session.feed(transcript.as_bytes(), Instant::now());
        }
        cx.notify();
    }

    /// `--screen shell-paste-fixture`: the multi-line paste dialog over the fixture transcript.
    #[cfg(feature = "screenshot")]
    pub(crate) fn show_paste_fixture(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = "kubectl -n payments rollout restart deploy/api\nkubectl -n payments rollout status deploy/api\nkubectl -n payments get pods -l app=api\nkubectl -n payments logs deploy/api --tail=50\nkubectl -n payments top pod -l app=api\nkubectl -n payments describe deploy/api\nkubectl -n payments get events --sort-by=.lastTimestamp";
        if let PasteDecision::Ask(ask) = decide_paste(text, false) {
            self.ask_before_pasting(ask, window, cx);
        }
    }

    /// `--screen shell-find-fixture`: Find open on `query`, with the matches highlighted.
    #[cfg(feature = "screenshot")]
    pub(crate) fn show_find_fixture(
        &mut self,
        query: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.is_find_open = true;
        self.find_input.update(cx, |input, cx| {
            input.set_value(query.to_owned(), window, cx)
        });
        self.session.borrow_mut().find(query);
        cx.notify();
    }

    pub(crate) fn target(&self) -> &ShellTarget {
        &self.target
    }

    pub(crate) fn command(&self) -> ShellCommand {
        self.command
    }

    pub(crate) fn kind(&self) -> &ShellKind {
        &self.kind
    }

    #[cfg(any(test, feature = "screenshot"))]
    pub(crate) fn state(&self) -> &ShellState {
        &self.state
    }

    pub(crate) fn cluster(&self) -> &ClusterRef {
        &self.target.cluster
    }

    #[cfg(test)]
    pub(crate) fn cluster_label(&self) -> &str {
        &self.cluster_label
    }

    /// `shell · m8n2p/api`, the tab label: the pod by the suffix rule of the Logs tab. A debug
    /// container reads `debug · m8n2p/api` (the container it shares), a node shell
    /// `node shell · wk-03 (debug pod)`.
    pub(crate) fn label(&self) -> String {
        match &self.kind {
            ShellKind::Exec => format!(
                "shell · {}/{}",
                self.target.short_pod, self.target.container
            ),
            ShellKind::Debug {
                target_container, ..
            } => format!("debug · {}/{target_container}", self.target.short_pod),
            ShellKind::NodeShell { node, .. } => format!("node shell · {node} (debug pod)"),
            ShellKind::Attach => format!(
                "attach · {}/{}",
                self.target.short_pod, self.target.container
            ),
        }
    }

    /// The dot of the tab: dim once the session has ended.
    pub(crate) fn tone(&self) -> StatusTone {
        match self.state {
            ShellState::Connecting => StatusTone::Info,
            ShellState::Live => StatusTone::Ok,
            ShellState::Ended(_) => StatusTone::Done,
        }
    }

    /// The shell the header names: the pick the `Auto` script reported, once it has.
    fn shell_label(&self) -> &'static str {
        match self.command {
            ShellCommand::Auto => self.session.borrow().shell().label(),
            command => shell_command_label(command),
        }
    }

    fn render_header(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        h_flex()
            .flex_shrink_0()
            .items_center()
            .gap_2()
            .px_3()
            .py_1p5()
            .border_b_1()
            .border_color(theme.border)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_family(theme.mono_font_family.clone())
                    .text_xs()
                    .child(self.header_text()),
            )
            .children(self.kind.is_exec().then(|| self.render_shell_picker(cx)))
            .children(self.render_debug_offer(cx))
            .children(self.is_find_open.then(|| self.render_find(cx)))
            .child(
                Toggle::new("shell-find")
                    .small()
                    .label("Find")
                    .checked(self.is_find_open)
                    .on_click(cx.listener(|tab, checked: &bool, window, cx| {
                        if *checked {
                            tab.open_find(window, cx);
                        } else {
                            tab.close_find(window, cx);
                        }
                    })),
            )
            .child(
                Button::new("shell-clear")
                    .ghost()
                    .small()
                    .label("Clear")
                    .tooltip("Clear the screen and the scrollback of this tab")
                    .on_click(cx.listener(|tab, _, _, cx| tab.clear(cx))),
            )
            .children((self.kind != ShellKind::Attach).then(|| {
                Button::new("shell-reconnect")
                    .ghost()
                    .small()
                    .label("Reconnect")
                    .tooltip("Open a new shell in this container; the screen is kept")
                    .disabled(self.state == ShellState::Connecting)
                    .on_click(cx.listener(|tab, _, window, cx| {
                        tab.request_reconnect(tab.command, window, cx);
                    }))
            }))
    }

    /// `Shell: Auto ▾`: a change reconnects with the new shell.
    fn render_shell_picker(&self, cx: &Context<Self>) -> impl IntoElement {
        let tab = cx.weak_entity();
        let current = self.command;
        Button::new("shell-picker")
            .ghost()
            .small()
            .label(format!("Shell: {}", shell_command_label(current)))
            .dropdown_caret(true)
            .dropdown_menu(move |menu, _, _| {
                [ShellCommand::Auto, ShellCommand::Bash, ShellCommand::Sh]
                    .into_iter()
                    .fold(menu, |menu, command| {
                        let tab = tab.clone();
                        menu.item(
                            PopupMenuItem::new(shell_command_label(command))
                                .checked(command == current)
                                .on_click(move |_, window, cx| {
                                    let _ = tab.update(cx, |tab, cx| {
                                        tab.request_reconnect(command, window, cx);
                                    });
                                }),
                        )
                    })
            })
    }

    /// The Find field and its match count. The `ShellFind` context lets Esc close it.
    fn render_find(&self, cx: &Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        h_flex()
            .key_context("ShellFind")
            .items_center()
            .gap_2()
            .child(
                div()
                    .w(px(220.))
                    .child(Input::new(&self.find_input).small().cleanable(true)),
            )
            .children(
                self.find_status_text(cx)
                    .map(|text| div().text_xs().text_color(muted).child(text)),
            )
    }

    /// Clears the local screen and scrollback; the remote shell is not told.
    fn clear(&mut self, cx: &mut Context<Self>) {
        self.session
            .borrow_mut()
            .feed(b"\x1b[H\x1b[2J\x1b[3J", Instant::now());
        cx.notify();
    }
}

/// How the header and the banner name a shell choice.
fn shell_command_label(command: ShellCommand) -> &'static str {
    match command {
        ShellCommand::Auto => "Auto",
        ShellCommand::Bash => "bash",
        ShellCommand::Sh => "sh",
    }
}

/// The first line of the terminal: what the tab runs and where.
fn banner(target: &ShellTarget, command: ShellCommand, cluster_label: &str) -> String {
    let shell = match command {
        ShellCommand::Auto => "auto",
        ShellCommand::Bash => "bash",
        ShellCommand::Sh => "sh",
    };
    format!(
        "# exec -n {} {} -c {} -- {shell} ({cluster_label})",
        target.namespace, target.pod, target.container
    )
}

/// The first line of a debug tab's terminal: what it attaches to and where.
fn attach_banner(target: &ShellTarget, kind: &ShellKind, cluster_label: &str) -> String {
    match kind {
        ShellKind::NodeShell { node, .. } => format!(
            "# node shell {node}: attach -n {} {} -c {} ({cluster_label})",
            target.namespace, target.pod, target.container
        ),
        ShellKind::Attach => format!(
            "# attach -n {} {} -c {} ({cluster_label})",
            target.namespace, target.pod, target.container
        ),
        ShellKind::Debug { .. } | ShellKind::Exec => format!(
            "# debug: attach -n {} {} -c {} ({cluster_label})",
            target.namespace, target.pod, target.container
        ),
    }
}

/// What a debug tab says while its container is created and started.
fn starting_text(kind: &ShellKind) -> &'static str {
    match kind {
        ShellKind::NodeShell { .. } => "Starting node shell pod…",
        ShellKind::Attach => "Attaching…",
        ShellKind::Debug { .. } | ShellKind::Exec => "Starting debug container…",
    }
}

/// The dim line for a waiting reason the pod reports. Anything unknown is shown as the word itself:
/// the reason is a fixed CamelCase word (`cluster::debug_shell` drops the server's messages).
fn waiting_text(reason: &str) -> String {
    match reason {
        "ContainerCreating" => "Creating container…".to_owned(),
        "PodInitializing" => "Starting…".to_owned(),
        "Pulling" | "ErrImagePull" | "ImagePullBackOff" => "Pulling image…".to_owned(),
        other => format!("Waiting: {other}"),
    }
}

/// Shown once the attach is up: a shell that printed no prompt yet looks stuck.
const PROMPT_HINT: &str = "If you don't see a prompt, press Enter.";

fn exit_end(exit: &ShellExit) -> ShellEnd {
    match (&exit.code, &exit.message) {
        (_, Some(message)) if message.contains(NO_EXECUTABLE) => ShellEnd::NoShell,
        (Some(code), _) => ShellEnd::Exited { code: Some(*code) },
        (None, Some(message)) => ShellEnd::Failed {
            reason: message.clone().into(),
        },
        (None, None) => ShellEnd::Exited { code: None },
    }
}

/// The dim line a finished session appends to the terminal.
fn end_note(end: &ShellEnd, was_started: bool) -> String {
    match end {
        ShellEnd::Exited { code: Some(code) } => format!("[process exited with code {code}]"),
        ShellEnd::Exited { code: None } => "[process exited]".to_owned(),
        ShellEnd::Failed { reason } if was_started => format!("[connection lost: {reason}]"),
        ShellEnd::Failed { reason } => format!("[could not open the shell: {reason}]"),
        ShellEnd::NoShell => "No shell in this container; try Debug container…".to_owned(),
    }
}

/// The first lines of a multi-line paste, in the mono font, and how many more there are.
fn paste_preview(preview: &[String], more: usize, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    v_flex()
        .gap_0p5()
        .p_2()
        .rounded_md()
        .bg(theme.muted)
        .font_family(theme.mono_font_family.clone())
        .text_xs()
        .children(
            preview
                .iter()
                .map(|line| div().truncate().child(line.clone())),
        )
        .children((more > 0).then(|| {
            div()
                .text_color(theme.muted_foreground)
                .child(format!("… and {more} more"))
        }))
}

/// The private clipboard write of 0016. Tests write to the test clipboard of the app instead: the
/// real one is the user's.
fn copy_private(text: &str, cx: &mut App) -> Result<(), ClipboardWriteError> {
    #[cfg(test)]
    {
        cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(text.to_owned()));
        Ok(())
    }
    #[cfg(not(test))]
    {
        crate::secret_clipboard::write_private_text(text, cx).map(|_| ())
    }
}

impl Render for ShellTab {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.needs_focus {
            self.needs_focus = false;
            window.focus(&self.focus_handle, cx);
        }
        v_flex()
            .size_full()
            .on_action(cx.listener(Self::on_close_find))
            .child(self.render_header(cx))
            .child(
                div()
                    .id("shell-terminal")
                    .key_context("Terminal")
                    .flex_1()
                    .min_h(px(0.))
                    .w_full()
                    .track_focus(&self.focus_handle)
                    .on_key_down(cx.listener(Self::on_key_down))
                    .on_action(cx.listener(Self::on_copy))
                    .on_action(cx.listener(Self::on_paste))
                    .on_action(cx.listener(Self::on_find))
                    .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
                    .on_mouse_move(cx.listener(Self::on_mouse_move))
                    .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
                    .on_scroll_wheel(cx.listener(Self::on_scroll))
                    .child(terminal_element(
                        Rc::clone(&self.session),
                        self.input.clone(),
                        Rc::clone(&self.metrics),
                    )),
            )
    }
}

/// What the shell tests do to a tab that no event would.
#[cfg(test)]
impl ShellTab {
    pub(crate) fn mark_start_unreported_for_test(&mut self) {
        self.is_start_unreported = true;
    }
}

#[cfg(test)]
#[path = "shell_tab_tests.rs"]
mod shell_tab_tests;
