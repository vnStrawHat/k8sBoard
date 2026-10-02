//! One pod container's log view: toolbar, stream state, and the line list.

use cluster::{ClusterConnection, ContainerSummary, LogRequest, LogSource, LogUpdate, PodSummary};
use gpui_kit::assets::IconName;
use gpui_kit::component::alert::Alert;
use gpui_kit::component::button::{Button, ButtonVariants as _, Toggle};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::message_scroller::{MessageScroller, MessageScrollerState};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, AppContext as _, ClipboardItem, Context, Entity, HighlightStyle, IntoElement,
    ParentElement as _, Rems, Render, StyleRefinement, Styled as _, StyledText, Subscription,
    WeakEntity, Window, div, prelude::FluentBuilder as _, px, rems,
};

use crate::cluster_runtime::{ClusterRuntime, WatchSubscription};
use crate::cluster_session::error_text;
use crate::log_buffer::{LogBuffer, find_matches, format_log_time};
use crate::pod_drawer::{default_container, kind_tag_text};
use crate::status_tone::StatusTone;

/// 13 characters of the mono `text_xs` font (0.75 rem at about 0.6 em per character).
const TIME_COLUMN_WIDTH: Rems = rems(5.85);

/// What a tab streams: one pod, with the containers the picker offers.
#[derive(Clone)]
pub(crate) struct LogTarget {
    pub(crate) namespace: String,
    pub(crate) pod: String,
    containers: Vec<ContainerSummary>,
    initial_container: String,
}

impl LogTarget {
    /// `None` when the pod has no containers to read.
    pub(crate) fn of_pod(pod: &PodSummary) -> Option<Self> {
        let index = default_container(&pod.containers)?;
        let initial_container = pod.containers.get(index)?.name.clone();
        Some(Self {
            namespace: pod.namespace.clone(),
            pod: pod.name.clone(),
            containers: pod.containers.clone(),
            initial_container,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LogInstance {
    Current,
    Previous,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum LogStreamState {
    Connecting,
    Streaming,
    Ended,
    Failed { message: String },
}

impl LogStreamState {
    fn start(&mut self) {
        if *self == Self::Connecting {
            *self = Self::Streaming;
        }
    }

    fn fail(&mut self, message: String) {
        *self = Self::Failed { message };
    }

    /// A failure stays a failure: the close that follows it carries no news.
    fn close(&mut self) {
        if matches!(self, Self::Connecting | Self::Streaming) {
            *self = Self::Ended;
        }
    }

    fn tone(&self) -> StatusTone {
        match self {
            Self::Connecting => StatusTone::Info,
            Self::Streaming => StatusTone::Ok,
            Self::Ended => StatusTone::Done,
            Self::Failed { .. } => StatusTone::Bad,
        }
    }
}

pub(crate) struct LogTab {
    connection: ClusterConnection,
    target: LogTarget,
    container: String,
    instance: LogInstance,
    shows_timestamps: bool,
    wraps_lines: bool,
    buffer: LogBuffer,
    stream: LogStreamState,
    filter_input: Entity<InputState>,
    scroller: Entity<MessageScrollerState>,
    /// Dropping it aborts the stream and closes the HTTP connection.
    _stream: Option<WatchSubscription>,
    _filter_events: Subscription,
    /// The "Paused" status and the jump button depend on the scroll position.
    _scroller_observer: Subscription,
}

impl LogTab {
    pub(crate) fn new(
        connection: ClusterConnection,
        target: LogTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let filter_input = cx.new(|cx| InputState::new(window, cx).placeholder("Filter lines"));
        let scroller = cx.new(|cx| MessageScrollerState::new(0, cx));
        let filter_events = cx.subscribe(&filter_input, Self::on_filter_event);
        let scroller_observer = cx.observe(&scroller, |_, _, cx| cx.notify());
        let mut tab = Self {
            connection,
            container: target.initial_container.clone(),
            target,
            instance: LogInstance::Current,
            shows_timestamps: true,
            wraps_lines: false,
            buffer: LogBuffer::new(),
            stream: LogStreamState::Connecting,
            filter_input,
            scroller,
            _stream: None,
            _filter_events: filter_events,
            _scroller_observer: scroller_observer,
        };
        tab.start_stream(cx);
        tab
    }

    pub(crate) fn is_for(&self, namespace: &str, pod: &str) -> bool {
        self.target.namespace == namespace && self.target.pod == pod
    }

    /// `{pod}/{container}`, the tab title.
    pub(crate) fn label(&self) -> String {
        format!("{}/{}", self.target.pod, self.container)
    }

    pub(crate) fn tone(&self) -> StatusTone {
        self.stream.tone()
    }

    #[cfg(feature = "screenshot")]
    pub(crate) fn is_connecting(&self) -> bool {
        self.stream == LogStreamState::Connecting
    }

    /// Every start is fresh: no resume, so there is nothing to de-duplicate.
    fn restart_stream(&mut self, cx: &mut Context<Self>) {
        self._stream = None;
        self.buffer.clear();
        self.scroller
            .update(cx, |scroller, cx| scroller.reset(0, cx));
        self.stream = LogStreamState::Connecting;
        self.start_stream(cx);
    }

    /// Subscribes to the pod logs of the current container and toggle.
    fn start_stream(&mut self, cx: &mut Context<Self>) {
        let source = match self.instance {
            LogInstance::Current => LogSource::Current,
            LogInstance::Previous => LogSource::Previous,
        };
        let updates = self.connection.pod_logs(LogRequest {
            namespace: self.target.namespace.clone(),
            pod: self.target.pod.clone(),
            container: self.container.clone(),
            source,
        });
        let runtime = cx.global::<ClusterRuntime>().clone();
        self._stream =
            Some(runtime.subscribe(updates, cx, Self::apply_update, |tab, _| tab.stream.close()));
        cx.notify();
    }

    fn apply_update(&mut self, update: LogUpdate, cx: &mut Context<Self>) {
        match update {
            LogUpdate::Started => self.stream.start(),
            LogUpdate::Lines(lines) => {
                let change = self.buffer.push(lines);
                self.scroller.update(cx, |scroller, cx| {
                    scroller.splice(0..change.removed_visible, 0, cx);
                    scroller.append(change.added_visible, cx);
                });
            }
            LogUpdate::Failed(error) => self.stream.fail(error_text(&error)),
        }
    }

    fn on_filter_event(
        &mut self,
        input: Entity<InputState>,
        event: &InputEvent,
        cx: &mut Context<Self>,
    ) {
        if !matches!(event, InputEvent::Change) {
            return;
        }
        let value = input.read(cx).value();
        self.buffer.set_filter(&value);
        let visible = self.buffer.visible_len();
        self.scroller
            .update(cx, |scroller, cx| scroller.reset(visible, cx));
        cx.notify();
    }

    fn pick_container(&mut self, name: String, cx: &mut Context<Self>) {
        if name == self.container {
            return;
        }
        self.container = name;
        self.restart_stream(cx);
    }

    fn toggle_previous(&mut self, cx: &mut Context<Self>) {
        self.instance = match self.instance {
            LogInstance::Current => LogInstance::Previous,
            LogInstance::Previous => LogInstance::Current,
        };
        self.restart_stream(cx);
    }

    /// Row heights depend on both toggles, so the list measures its rows again.
    fn remeasure(&mut self, cx: &mut Context<Self>) {
        self.scroller
            .update(cx, |scroller, cx| scroller.remeasure(cx));
        cx.notify();
    }

    fn status_text(&self, is_scrolled_up: bool) -> String {
        let count = count_text(
            self.buffer.visible_len(),
            self.buffer.total_len(),
            self.buffer.needle().is_some(),
            self.buffer.has_dropped(),
        );
        match (&self.stream, self.instance) {
            (LogStreamState::Connecting, _) => "Opening…".to_owned(),
            (LogStreamState::Streaming, _) if is_scrolled_up => format!("Paused · {count}"),
            (LogStreamState::Streaming, _) => format!("Streaming · {count}"),
            (LogStreamState::Ended, LogInstance::Current) => format!("Stream ended · {count}"),
            (LogStreamState::Ended, LogInstance::Previous) => {
                format!("Previous instance · {count}")
            }
            (LogStreamState::Failed { .. }, _) => format!("Failed · {count}"),
        }
    }

    fn render_toolbar(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let is_scrolled_up = self.scroller.read(cx).is_scrolled_up();
        let status = self.status_text(is_scrolled_up);
        let is_connecting = self.stream == LogStreamState::Connecting;
        h_flex()
            .flex_shrink_0()
            .flex_wrap()
            .items_center()
            .gap_2()
            .px_3()
            .py_1p5()
            .border_b_1()
            .border_color(theme.border)
            .child(self.render_container_picker(cx))
            .child(
                div()
                    .flex_1()
                    .min_w(px(160.))
                    .max_w(px(360.))
                    .child(Input::new(&self.filter_input).small().cleanable(true)),
            )
            .child(
                Toggle::new("log-previous")
                    .small()
                    .label("Previous")
                    .tooltip("Logs of the previous container instance")
                    .checked(self.instance == LogInstance::Previous)
                    .on_click(cx.listener(|tab, _: &bool, _, cx| tab.toggle_previous(cx))),
            )
            .child(
                Toggle::new("log-timestamps")
                    .small()
                    .label("Timestamps")
                    .tooltip("Kubelet time, UTC")
                    .checked(self.shows_timestamps)
                    .on_click(cx.listener(|tab, checked: &bool, _, cx| {
                        tab.shows_timestamps = *checked;
                        tab.remeasure(cx);
                    })),
            )
            .child(
                Toggle::new("log-wrap")
                    .small()
                    .label("Wrap")
                    .checked(self.wraps_lines)
                    .on_click(cx.listener(|tab, checked: &bool, _, cx| {
                        tab.wraps_lines = *checked;
                        tab.remeasure(cx);
                    })),
            )
            .child(
                Button::new("log-copy")
                    .ghost()
                    .small()
                    .icon(Icon::new(IconName::Copy))
                    .tooltip("Copy visible lines")
                    .on_click(cx.listener(|tab, _, _, cx| {
                        let text = tab.buffer.visible_text(tab.shows_timestamps);
                        cx.write_to_clipboard(ClipboardItem::new_string(text));
                    })),
            )
            .child(
                div()
                    .ml_auto()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(status),
            )
            .when(!is_connecting, |toolbar| {
                toolbar.child(
                    Button::new("log-reconnect")
                        .ghost()
                        .small()
                        .icon(Icon::new(IconName::RefreshCw))
                        .tooltip("Reconnect")
                        .on_click(cx.listener(|tab, _, _, cx| tab.restart_stream(cx))),
                )
            })
    }

    fn render_container_picker(&self, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let current_kind = self
            .target
            .containers
            .iter()
            .find(|container| container.name == self.container)
            .map(|container| kind_tag_text(container.kind));
        let label = h_flex()
            .gap_1p5()
            .items_center()
            .font_family(theme.mono_font_family.clone())
            .child(self.container.clone())
            .children(current_kind.map(|kind| {
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(kind)
            }));
        // A single container has nothing to pick, so it is plain text, not a disabled button.
        if self.target.containers.len() < 2 {
            return label.px_2().text_sm().into_any_element();
        }
        let button = Button::new("log-container").ghost().small().child(label);
        let tab = cx.weak_entity();
        let containers = self.target.containers.clone();
        let selected = self.container.clone();
        button
            .dropdown_caret(true)
            .dropdown_menu(move |menu, _, _| {
                containers.iter().fold(menu, |menu, container| {
                    let name = container.name.clone();
                    let tab = tab.clone();
                    menu.item(
                        PopupMenuItem::new(format!(
                            "{} · {}",
                            container.name,
                            kind_tag_text(container.kind)
                        ))
                        .checked(container.name == selected)
                        .on_click(move |_, _, cx| {
                            let _ = tab.update(cx, |tab, cx| tab.pick_container(name.clone(), cx));
                        }),
                    )
                })
            })
            .into_any_element()
    }

    fn render_body(&self, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let muted = |text: String| {
            div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child(text)
                .into_any_element()
        };
        if self.buffer.total_len() == 0 {
            return match &self.stream {
                LogStreamState::Connecting => v_flex()
                    .size_full()
                    .items_center()
                    .justify_center()
                    .gap_3()
                    .child(Spinner::new())
                    .child(
                        div()
                            .text_sm()
                            .text_color(theme.muted_foreground)
                            .child(format!(
                                "Opening logs of {}/{}…",
                                self.target.pod, self.container
                            )),
                    )
                    .into_any_element(),
                LogStreamState::Ended => muted("The container wrote no log lines".to_owned()),
                LogStreamState::Streaming => muted("No log lines yet".to_owned()),
                LogStreamState::Failed { .. } => div().into_any_element(),
            };
        }
        if let (0, Some(needle)) = (self.buffer.visible_len(), self.buffer.needle()) {
            return muted(format!("No lines match \"{needle}\""));
        }
        let tab = cx.weak_entity();
        let background = theme.background;
        MessageScroller::new("log-lines", self.scroller.clone(), move |index, _, cx| {
            row_of(&tab, index, cx)
        })
        .with_bottom_fade(background)
        .with_row_style(StyleRefinement::default().pb_0().px_3())
        .with_list_style(StyleRefinement::default().py_1())
        .into_any_element()
    }

    fn render_row(&self, index: usize, cx: &App) -> AnyElement {
        let Some(line) = self.buffer.visible_line(index) else {
            return div().into_any_element();
        };
        let theme = cx.theme();
        let highlights = match self.buffer.needle() {
            Some(needle) => find_matches(&line.text, needle)
                .into_iter()
                .map(|range| {
                    let style = HighlightStyle {
                        background_color: Some(theme.selection),
                        ..Default::default()
                    };
                    (range, style)
                })
                .collect(),
            None => Vec::new(),
        };
        // An empty line still needs a line box, or the row would collapse to nothing.
        let shown = if line.text.is_empty() {
            " ".to_owned()
        } else {
            line.text.clone()
        };
        let time = line.timestamp.map(format_log_time).unwrap_or_default();
        h_flex()
            .items_start()
            .gap_2()
            .font_family(theme.mono_font_family.clone())
            .text_xs()
            .when(self.shows_timestamps, |row| {
                row.child(
                    div()
                        .w(TIME_COLUMN_WIDTH)
                        .flex_shrink_0()
                        .text_color(theme.muted_foreground)
                        .child(time),
                )
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .when(!self.wraps_lines, |text| {
                        text.whitespace_nowrap().truncate()
                    })
                    .child(StyledText::new(shown).with_highlights(highlights)),
            )
            .into_any_element()
    }
}

fn row_of(tab: &WeakEntity<LogTab>, index: usize, cx: &App) -> AnyElement {
    tab.read_with(cx, |tab, cx| tab.render_row(index, cx))
        .unwrap_or_else(|_| div().into_any_element())
}

impl Render for LogTab {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let failure = match &self.stream {
            LogStreamState::Failed { message } => Some(message.clone()),
            _ => None,
        };
        v_flex()
            .size_full()
            .child(self.render_toolbar(cx))
            .children(failure.map(|message| {
                div()
                    .flex_shrink_0()
                    .px_3()
                    .py_2()
                    .child(Alert::error("log-error", message).title("Cannot read the logs"))
            }))
            .child(div().flex_1().min_h_0().child(self.render_body(cx)))
    }
}

/// `N lines`, or `V of N lines` while a filter hides some, then the eviction note.
fn count_text(visible: usize, total: usize, has_filter: bool, has_dropped: bool) -> String {
    let noun = if total == 1 { "line" } else { "lines" };
    let mut text = if has_filter {
        format!("{visible} of {total} {noun}")
    } else {
        format!("{total} {noun}")
    };
    if has_dropped {
        text.push_str(" · older lines dropped");
    }
    text
}

#[cfg(test)]
mod tests {
    use cluster::{ContainerKind, ContainerState, PodStatus, ReadyCount, StatusReason};

    use super::*;

    fn container(name: &str, kind: ContainerKind, is_ready: bool) -> ContainerSummary {
        ContainerSummary {
            name: name.to_owned(),
            image: "img".to_owned(),
            kind,
            state: ContainerState::NotReported,
            is_ready,
            restart_count: 0,
            last_termination: None,
            image_digest: None,
            pull_policy: None,
            is_started: None,
            ports: Vec::new(),
            resources: Vec::new(),
            probes: cluster::ContainerProbes::default(),
            env: Vec::new(),
            env_from: Vec::new(),
            mounts: Vec::new(),
        }
    }

    fn pod(containers: Vec<ContainerSummary>) -> PodSummary {
        PodSummary {
            namespace: "ns".to_owned(),
            name: "pod".to_owned(),
            status: PodStatus::Reason(StatusReason::Running),
            ready: ReadyCount { ready: 0, total: 0 },
            restarts: 0,
            node_name: None,
            created_at: None,
            pod_ip: None,
            qos_class: None,
            service_account: None,
            controller: None,
            conditions: Vec::new(),
            status_message: None,
            labels: Vec::new(),
            host_network: false,
            image_pull_secrets: Vec::new(),
            containers,
        }
    }

    #[test]
    fn stream_state_started_moves_connecting_to_streaming() {
        let mut state = LogStreamState::Connecting;
        state.start();
        assert_eq!(state, LogStreamState::Streaming);
    }

    #[test]
    fn stream_state_close_while_streaming_is_ended() {
        let mut state = LogStreamState::Streaming;
        state.close();
        assert_eq!(state, LogStreamState::Ended);
    }

    #[test]
    fn stream_state_close_while_connecting_is_ended() {
        let mut state = LogStreamState::Connecting;
        state.close();
        assert_eq!(state, LogStreamState::Ended);
    }

    #[test]
    fn stream_state_close_after_failure_stays_failed() {
        let mut state = LogStreamState::Streaming;
        state.fail("boom".to_owned());
        state.close();
        assert_eq!(
            state,
            LogStreamState::Failed {
                message: "boom".to_owned()
            }
        );
    }

    #[test]
    fn log_target_of_pod_picks_default_container() {
        let pod = pod(vec![
            container("init", ContainerKind::Init, true),
            container("ready", ContainerKind::Main, true),
            container("waiting", ContainerKind::Main, false),
        ]);
        let target = LogTarget::of_pod(&pod).expect("a target");
        assert_eq!(target.initial_container, "waiting");
        assert_eq!(target.containers.len(), 3);
        assert_eq!(
            (target.namespace.as_str(), target.pod.as_str()),
            ("ns", "pod")
        );
    }

    #[test]
    fn log_target_of_pod_without_containers_is_none() {
        assert!(LogTarget::of_pod(&pod(Vec::new())).is_none());
    }

    #[test]
    fn count_text_without_filter_counts_lines() {
        assert_eq!(count_text(5, 5, false, false), "5 lines");
    }

    #[test]
    fn count_text_uses_singular_for_one_line() {
        assert_eq!(count_text(1, 1, false, false), "1 line");
    }

    #[test]
    fn count_text_with_filter_shows_visible_of_total() {
        assert_eq!(count_text(2, 10, true, false), "2 of 10 lines");
    }

    #[test]
    fn count_text_notes_dropped_lines() {
        assert_eq!(
            count_text(10_000, 10_000, false, true),
            "10000 lines · older lines dropped"
        );
    }
}
