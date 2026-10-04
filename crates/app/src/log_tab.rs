//! One log view: a pod container or every pod of a workload. Toolbar, streams, merge, and the
//! line list.

use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use cluster::{ClusterConnection, LogRequest, LogSource, LogUpdate, NamespaceScope, PodSummary};
use gpui_kit::assets::IconName;
use gpui_kit::component::alert::Alert;
use gpui_kit::component::button::{Button, ButtonVariants as _, Toggle};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::message_scroller::{MessageScroller, MessageScrollerState};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, AppContext as _, Bounds, ClipboardItem, Context, Entity, EventEmitter,
    IntoElement, ParentElement as _, Pixels, Render, SharedString, StyleRefinement, Styled as _,
    Subscription, Task, WeakEntity, Window, div, prelude::FluentBuilder as _, px,
};

use crate::cluster_registry::ClusterRef;
use crate::cluster_runtime::{ClusterRuntime, WatchSubscription};
use crate::cluster_session::{ClusterSession, error_text};
use crate::dock::LogOrigin;
use crate::file_export::{ExportState, export_file_name, start_export};
use crate::kind_row::PodOwner;
use crate::line_matcher::{FilterMode, InvalidRegex, LineMatcher};
use crate::log_buffer::{
    LineKind, LineTime, LineView, LogBuffer, SourceId, SourcedLine, TimeWindow,
};
use crate::log_legend::{LegendChip, legend_row, pod_color};
use crate::log_level::{LevelSet, LogLevel};
use crate::log_rows::{RowPrefix, RowStyle, log_row};
use crate::log_target::{LogTarget, PodTarget, WorkloadTarget};
use crate::log_volume::{
    BrushDrag, BrushHandlers, BrushView, FractionHandler, Volume, brush_window, volume,
    volume_chart,
};
use crate::log_workload::{
    MemberChange, RestartBaselines, container_names, join_slots, member_change, pod_short_name,
    ranked_pods, restart_marker, rising_restarts, scope_covers,
};
use crate::pod_drawer::{default_container, kind_tag_text};
use crate::settings::{AppSettings, LogSettings};
use crate::status_tone::{StatusTone, tone_color};

/// The saved defaults of new tabs; a view built without settings (a test) gets the built-in ones.
fn log_defaults(cx: &App) -> LogSettings {
    AppSettings::try_get(cx)
        .map(|settings| settings.logs.clone())
        .unwrap_or_default()
}

/// A pod that joins after the merge flush asks for little history, so it cannot land far out of
/// order.
const LATE_JOIN_TAIL_LINES: u32 = 50;
/// Initial tails arrive pod by pod; waiting this long lets them be sorted by time first.
const MERGE_WINDOW: Duration = Duration::from_secs(2);
/// A pod that left the list is usually closed by the server with its final lines; this is how
/// long its streams may still deliver them.
const LEAVE_GRACE: Duration = Duration::from_secs(10);
const MAX_STAGING_BYTES: usize = 8 * 1024 * 1024;
const PLAIN_PLACEHOLDER: &str = "Filter lines";
const REGEX_PLACEHOLDER: &str = "Regex, e.g. error|timeout";
/// A release closer than this to the press is a click, not a drag.
const MIN_BRUSH_DRAG_PX: f32 = 3.;

/// Compact is the docked tab; Full (the zoomed dock) adds the pod legend and the histogram.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LogLayout {
    Compact,
    Full,
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

    fn is_live(&self) -> bool {
        matches!(self, Self::Connecting | Self::Streaming)
    }
}

/// What the streams of a tab add up to.
#[derive(Clone, Debug, PartialEq, Eq)]
enum TabPhase {
    /// A workload tab with no stream yet.
    Waiting,
    Connecting,
    Streaming,
    Ended,
    Failed {
        message: String,
    },
}

impl TabPhase {
    fn tone(&self) -> StatusTone {
        match self {
            Self::Waiting | Self::Connecting => StatusTone::Info,
            Self::Streaming => StatusTone::Ok,
            Self::Ended => StatusTone::Done,
            Self::Failed { .. } => StatusTone::Bad,
        }
    }
}

fn stream_phase<'a>(states: impl Iterator<Item = &'a LogStreamState>) -> TabPhase {
    let (mut total, mut failed) = (0, 0);
    let (mut is_streaming, mut is_connecting) = (false, false);
    let mut first_failure = None;
    for state in states {
        total += 1;
        match state {
            LogStreamState::Connecting => is_connecting = true,
            LogStreamState::Streaming => is_streaming = true,
            LogStreamState::Ended => {}
            LogStreamState::Failed { message } => {
                failed += 1;
                first_failure.get_or_insert(message);
            }
        }
    }
    if total == 0 {
        return TabPhase::Waiting;
    }
    if is_streaming {
        return TabPhase::Streaming;
    }
    if is_connecting {
        return TabPhase::Connecting;
    }
    match first_failure {
        Some(message) if failed == total => TabPhase::Failed {
            message: message.clone(),
        },
        _ => TabPhase::Ended,
    }
}

/// The tab dot: Info while waiting or opening, Ok while any stream flows, Bad when every stream
/// failed, otherwise Done.
fn workload_tone<'a>(states: impl Iterator<Item = &'a LogStreamState>) -> StatusTone {
    stream_phase(states).tone()
}

/// One pod container's stream inside a tab. Its index in `LogTab::streams` is its `SourceId`.
struct TabStream {
    namespace: String,
    pod: String,
    container: String,
    /// `{short}/{container}`, as shown on screen.
    prefix: SharedString,
    /// `{pod}/{container}`, as written by Export.
    full_prefix: SharedString,
    color_slot: usize,
    /// The pod is still listed; a leaver keeps streaming until its grace ends.
    is_member: bool,
    state: LogStreamState,
    /// Dropping it aborts the stream and closes the HTTP connection.
    _stream: Option<WatchSubscription>,
    _grace: Option<Task<()>>,
}

/// Lines held back during the merge window so the first batches can be sorted by time.
struct Staging {
    lines: Vec<SourcedLine>,
    bytes: usize,
    _timer: Task<()>,
}

enum LogSubject {
    Pod {
        target: PodTarget,
        container: String,
        /// Weak: the tab never keeps the session alive.
        session: WeakEntity<ClusterSession>,
        /// Restart markers follow the session's pod list.
        _pods_observer: Subscription,
    },
    Workload(WorkloadSubject),
}

struct WorkloadSubject {
    target: WorkloadTarget,
    /// Weak: the tab never keeps the session alive.
    session: WeakEntity<ClusterSession>,
    /// Names of the pods whose streams are followed.
    members: Vec<String>,
    /// The containers every member streams; empty until a pod is listed.
    selected: Vec<String>,
    /// The container names the picker offers, over the pods of the workload.
    offered: Vec<String>,
    /// The namespace filter no longer covers the workload, so membership stays as it was.
    is_frozen: bool,
    _pods_observer: Subscription,
}

pub(crate) struct LogTab {
    /// The cluster the streams read from.
    cluster: ClusterRef,
    connection: ClusterConnection,
    subject: LogSubject,
    instance: LogInstance,
    shows_timestamps: bool,
    wraps_lines: bool,
    shows_json: bool,
    filter_mode: FilterMode,
    hidden_levels: LevelSet,
    /// The regex in the input does not compile; the previous matcher stays in force.
    has_invalid_filter: bool,
    buffer: LogBuffer,
    streams: Vec<TabStream>,
    /// The last restart count seen per (pod, container), so only a rise adds a marker.
    restart_seen: RestartBaselines,
    staging: Option<Staging>,
    /// The color slot of each pod name, in join order; a returning pod keeps its slot.
    pod_slots: Vec<String>,
    layout: LogLayout,
    /// The tab lives in a window of its own (`Pop out`), outside the dock.
    is_popped_out: bool,
    /// The histogram of the visible lines, computed for the buffer revision it carries.
    volume_memo: Option<(u64, Option<Rc<Volume>>)>,
    /// The drag across the histogram in progress.
    brush: Option<BrushDrag>,
    /// Where the histogram sits, from its last prepaint (the brush maps the pointer onto it).
    chart_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    export_state: ExportState,
    /// The line count of the last saved export, for the status text.
    exported_lines: usize,
    _export: Option<Task<()>>,
    filter_input: Entity<InputState>,
    scroller: Entity<MessageScrollerState>,
    _filter_events: Subscription,
    /// The "Paused" status and the jump button depend on the scroll position.
    _scroller_observer: Subscription,
}

impl LogTab {
    pub(crate) fn new(
        origin: LogOrigin,
        target: LogTarget,
        session: &Entity<ClusterSession>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let filter_input = cx.new(|cx| InputState::new(window, cx).placeholder(PLAIN_PLACEHOLDER));
        let scroller = cx.new(|cx| MessageScrollerState::new(0, cx));
        let filter_events = cx.subscribe(&filter_input, Self::on_filter_event);
        let scroller_observer = cx.observe(&scroller, |_, _, cx| cx.notify());
        let subject = match target {
            LogTarget::Pod(target) => LogSubject::Pod {
                container: target.initial_container.clone(),
                target,
                session: session.downgrade(),
                _pods_observer: cx.observe(session, |tab, _, cx| tab.note_restarts(cx)),
            },
            LogTarget::Workload(target) => LogSubject::Workload(WorkloadSubject {
                target,
                session: session.downgrade(),
                members: Vec::new(),
                selected: Vec::new(),
                offered: Vec::new(),
                is_frozen: false,
                _pods_observer: cx.observe(session, |tab, _, cx| tab.sync_members(cx)),
            }),
        };
        let defaults = log_defaults(cx);
        let mut tab = Self {
            cluster: origin.cluster,
            connection: origin.connection,
            subject,
            instance: LogInstance::Current,
            shows_timestamps: defaults.show_timestamps,
            wraps_lines: defaults.wrap_lines,
            shows_json: defaults.show_json,
            filter_mode: FilterMode::Plain,
            hidden_levels: LevelSet::default(),
            has_invalid_filter: false,
            buffer: LogBuffer::new(),
            streams: Vec::new(),
            restart_seen: HashMap::new(),
            staging: None,
            pod_slots: Vec::new(),
            layout: LogLayout::Compact,
            is_popped_out: false,
            volume_memo: None,
            brush: None,
            chart_bounds: Rc::new(Cell::new(None)),
            export_state: ExportState::Idle,
            exported_lines: 0,
            _export: None,
            filter_input,
            scroller,
            _filter_events: filter_events,
            _scroller_observer: scroller_observer,
        };
        tab.start_streams(cx);
        tab
    }

    /// The same pod or workload of the same cluster: the same names exist in several clusters.
    pub(crate) fn is_for(&self, cluster: &ClusterRef, target: &LogTarget) -> bool {
        if self.cluster != *cluster {
            return false;
        }
        match (&self.subject, target) {
            (LogSubject::Pod { target: mine, .. }, LogTarget::Pod(other)) => mine.is_same(other),
            (LogSubject::Workload(mine), LogTarget::Workload(other)) => mine.target.is_same(other),
            _ => false,
        }
    }

    /// The tab's own toggles as `(timestamps, wrap, json)`, for the tests of the saved defaults.
    #[cfg(test)]
    pub(crate) fn toggles(&self) -> (bool, bool, bool) {
        (self.shows_timestamps, self.wraps_lines, self.shows_json)
    }

    /// Flips the three toggles as the toolbar does.
    #[cfg(test)]
    pub(crate) fn flip_toggles(&mut self) {
        self.shows_timestamps = !self.shows_timestamps;
        self.wraps_lines = !self.wraps_lines;
        self.shows_json = !self.shows_json;
    }

    /// The tab label: `{pod}/{container}`, or the workload label.
    pub(crate) fn label(&self) -> String {
        match &self.subject {
            LogSubject::Pod {
                target, container, ..
            } => format!("{}/{container}", target.pod),
            LogSubject::Workload(workload) => workload.target.label.clone(),
        }
    }

    pub(crate) fn tone(&self) -> StatusTone {
        workload_tone(self.streams.iter().map(|stream| &stream.state))
    }

    fn phase(&self) -> TabPhase {
        stream_phase(self.streams.iter().map(|stream| &stream.state))
    }

    fn is_workload(&self) -> bool {
        matches!(self.subject, LogSubject::Workload(_))
    }

    /// Whether the tab still waits for its streams to open or its merge window to close.
    #[cfg(feature = "screenshot")]
    pub(crate) fn is_connecting(&self) -> bool {
        self.staging.is_some() || self.phase() == TabPhase::Connecting
    }

    /// Every start is fresh: no resume, so there is nothing to de-duplicate.
    fn restart_stream(&mut self, cx: &mut Context<Self>) {
        self.streams.clear();
        self.restart_seen.clear();
        self.staging = None;
        self.pod_slots.clear();
        // A save in flight finishes: dropping its task would cut the file short and hide the
        // result.
        if self.export_state != ExportState::Saving {
            self.export_state = ExportState::Idle;
            self._export = None;
        }
        if let LogSubject::Workload(workload) = &mut self.subject {
            workload.members.clear();
        }
        self.buffer.clear();
        // A restarted buffer holds other lines, so a window over the old ones means nothing.
        self.brush = None;
        if self.buffer.view().window.is_some() {
            self.apply_view(None, cx);
        }
        self.scroller
            .update(cx, |scroller, cx| scroller.reset(0, cx));
        self.start_streams(cx);
    }

    fn start_streams(&mut self, cx: &mut Context<Self>) {
        match &self.subject {
            LogSubject::Pod {
                target, container, ..
            } => {
                let open = StreamOpen {
                    namespace: target.namespace.clone(),
                    pod: target.pod.clone(),
                    container: container.clone(),
                    prefix: SharedString::from(format!("{}/{container}", target.pod)),
                    full_prefix: SharedString::from(format!("{}/{container}", target.pod)),
                    color_slot: 0,
                    tail_lines: log_defaults(cx).tail_lines(),
                };
                self.open_stream(open, cx);
                self.note_restarts(cx);
            }
            LogSubject::Workload(_) => {
                self.sync_members(cx);
            }
        }
        cx.notify();
    }

    /// Subscribes to the logs of one container and adds it to `streams`.
    fn open_stream(&mut self, open: StreamOpen, cx: &mut Context<Self>) {
        // A source id is a u16; a tab would need 65,536 stream opens (churn without Reconnect) to
        // reach it, and a full table simply stops opening streams.
        let Ok(index) = u16::try_from(self.streams.len()) else {
            return;
        };
        let id = SourceId(index);
        let source = match self.instance {
            LogInstance::Current => LogSource::Current,
            LogInstance::Previous => LogSource::Previous,
        };
        let updates = self.connection.pod_logs(LogRequest {
            namespace: open.namespace.clone(),
            pod: open.pod.clone(),
            container: open.container.clone(),
            source,
            tail_lines: open.tail_lines,
        });
        let runtime = cx.global::<ClusterRuntime>().clone();
        let subscription = runtime.subscribe(
            updates,
            cx,
            move |tab, update, cx| tab.apply_update(id, update, cx),
            move |tab, cx| tab.close_stream(id, cx),
        );
        self.streams.push(TabStream {
            namespace: open.namespace,
            container: open.container,
            pod: open.pod,
            prefix: open.prefix,
            full_prefix: open.full_prefix,
            color_slot: open.color_slot,
            is_member: true,
            state: LogStreamState::Connecting,
            _stream: Some(subscription),
            _grace: None,
        });
    }

    fn arm_staging(&mut self, cx: &mut Context<Self>) {
        let timer = cx.spawn(async move |tab, cx| {
            cx.background_executor().timer(MERGE_WINDOW).await;
            let _ = tab.update(cx, |tab, cx| tab.flush_staging(cx));
        });
        self.staging = Some(Staging {
            lines: Vec::new(),
            bytes: 0,
            _timer: timer,
        });
    }

    fn apply_update(&mut self, id: SourceId, update: LogUpdate, cx: &mut Context<Self>) {
        match update {
            LogUpdate::Started => {
                if let Some(stream) = self.streams.get_mut(usize::from(id.0)) {
                    stream.state.start();
                }
            }
            LogUpdate::Lines(lines) => {
                let lines: Vec<SourcedLine> = lines
                    .into_iter()
                    .map(|line| SourcedLine {
                        source: id,
                        kind: LineKind::Log,
                        line,
                    })
                    .collect();
                let Some(staging) = &mut self.staging else {
                    self.push_lines(lines, cx);
                    return;
                };
                staging.bytes += lines.iter().map(|line| line.line.text.len()).sum::<usize>();
                staging.lines.extend(lines);
                if staging.bytes > MAX_STAGING_BYTES {
                    self.flush_staging(cx);
                }
            }
            LogUpdate::Failed(error) => {
                if let Some(stream) = self.streams.get_mut(usize::from(id.0)) {
                    stream.state.fail(error_text(&error));
                }
            }
        }
    }

    fn close_stream(&mut self, id: SourceId, cx: &mut Context<Self>) {
        if let Some(stream) = self.streams.get_mut(usize::from(id.0)) {
            stream.state.close();
        }
        // A closed stream frees a slot for a pod that was waiting.
        self.sync_members(cx);
    }

    fn push_lines(&mut self, lines: Vec<SourcedLine>, cx: &mut Context<Self>) {
        let change = self.buffer.push(lines);
        self.scroller.update(cx, |scroller, cx| {
            scroller.splice(0..change.removed_visible, 0, cx);
            scroller.append(change.added_visible, cx);
        });
    }

    /// Adds a `SYS` line for each streamed container whose restart count rose in the session's
    /// pod list. Every streamed (pod, container) pair is checked, whatever its stream state: a
    /// follow stream ends when its container exits, before the kubelet raises `restartCount`.
    /// Only the Current instance marks; Previous is a fixed read of the old container.
    fn note_restarts(&mut self, cx: &mut Context<Self>) {
        if self.instance != LogInstance::Current {
            return;
        }
        let session = match &self.subject {
            LogSubject::Pod { session, .. } => session,
            LogSubject::Workload(workload) => &workload.session,
        };
        let Some(session) = session.upgrade() else {
            return;
        };
        let markers = {
            let session = session.read(cx);
            let Some(pods) = session.live().and_then(|live| live.pods.ready_items()) else {
                return;
            };
            restart_markers(&self.streams, &mut self.restart_seen, pods)
        };
        if markers.is_empty() {
            return;
        }
        match &mut self.staging {
            Some(staging) => {
                staging.bytes += markers
                    .iter()
                    .map(|line| line.line.text.len())
                    .sum::<usize>();
                staging.lines.extend(markers);
            }
            None => self.push_lines(markers, cx),
        }
        cx.notify();
    }

    /// Pushes the staged lines once, sorted by kubelet time. Later lines append on arrival.
    fn flush_staging(&mut self, cx: &mut Context<Self>) {
        let Some(mut staging) = self.staging.take() else {
            return;
        };
        sort_staged(&mut staging.lines);
        self.push_lines(staging.lines, cx);
        cx.notify();
    }

    /// Follows the pods of a workload, then notes the restarts of the containers it streams.
    fn sync_members(&mut self, cx: &mut Context<Self>) {
        self.follow_members(cx);
        self.note_restarts(cx);
    }

    /// Starts streams for pods that joined the list, and lets the streams of pods that left run
    /// out their grace.
    fn follow_members(&mut self, cx: &mut Context<Self>) {
        let LogSubject::Workload(workload) = &self.subject else {
            return;
        };
        let live_streams = self
            .streams
            .iter()
            .filter(|stream| stream.state.is_live())
            .count();
        let Some(plan) = plan_sync(workload, live_streams, cx) else {
            return;
        };
        let LogSubject::Workload(workload) = &mut self.subject else {
            return;
        };
        let owner = workload.target.owner.clone();
        let mut has_changed = workload.is_frozen != plan.is_frozen;
        workload.is_frozen = plan.is_frozen;
        if plan.is_frozen {
            if has_changed {
                cx.notify();
            }
            return;
        }
        workload.selected = plan.selected;
        if workload.offered != plan.offered {
            workload.offered = plan.offered;
            has_changed = true;
        }
        workload
            .members
            .retain(|member| !plan.change.left.contains(member));
        workload.members.extend(plan.change.joined.iter().cloned());
        for name in &plan.change.left {
            has_changed = true;
            self.leave_pod(name, cx);
        }
        if opens_merge_window(
            self.streams.len(),
            self.staging.is_some(),
            plan.admissions.len(),
        ) {
            self.arm_staging(cx);
        }
        for admission in plan.admissions {
            has_changed = true;
            // A pod name that returns (a StatefulSet recreate) starts clean: its old streams
            // stop, and their buffered lines stay.
            self.drop_streams_of(&admission.pod);
            let color_slot = slot_for(&mut self.pod_slots, &admission.pod);
            let tail_lines = if self.staging.is_some() {
                log_defaults(cx).tail_lines()
            } else {
                LATE_JOIN_TAIL_LINES
            };
            let short = pod_short_name(&owner, &admission.pod).to_owned();
            for container in admission.containers {
                let open = StreamOpen {
                    namespace: admission.namespace.clone(),
                    pod: admission.pod.clone(),
                    prefix: SharedString::from(format!("{short}/{container}")),
                    full_prefix: SharedString::from(format!("{}/{container}", admission.pod)),
                    container,
                    color_slot,
                    tail_lines,
                };
                self.open_stream(open, cx);
            }
        }
        if has_changed {
            cx.notify();
        }
    }

    /// The pod left the list: its streams stay for `LEAVE_GRACE`, then end.
    fn leave_pod(&mut self, name: &str, cx: &mut Context<Self>) {
        for (index, stream) in self.streams.iter_mut().enumerate() {
            if stream.pod != name || !stream.is_member {
                continue;
            }
            stream.is_member = false;
            let id = SourceId(u16::try_from(index).unwrap_or(u16::MAX));
            stream._grace = Some(cx.spawn(async move |tab, cx| {
                cx.background_executor().timer(LEAVE_GRACE).await;
                let _ = tab.update(cx, |tab, cx| tab.end_stream(id, cx));
            }));
        }
    }

    /// The grace of a leaver is over: stop its stream and free its slot.
    fn end_stream(&mut self, id: SourceId, cx: &mut Context<Self>) {
        if let Some(stream) = self.streams.get_mut(usize::from(id.0)) {
            stream._stream = None;
            stream.state.close();
        }
        self.sync_members(cx);
        cx.notify();
    }

    fn drop_streams_of(&mut self, pod: &str) {
        for stream in self.streams.iter_mut().filter(|stream| stream.pod == pod) {
            stream._stream = None;
            stream._grace = None;
            stream.is_member = false;
            stream.state.close();
        }
    }

    fn on_filter_event(
        &mut self,
        _: Entity<InputState>,
        event: &InputEvent,
        cx: &mut Context<Self>,
    ) {
        if !matches!(event, InputEvent::Change) {
            return;
        }
        self.refresh_view(cx);
    }

    /// Applies the input text, the filter mode, and the level chips to the buffer; the brush
    /// window stays as it is. An invalid regex keeps the previous matcher, so typing `a|(` never
    /// blanks the view.
    fn refresh_view(&mut self, cx: &mut Context<Self>) {
        self.apply_view(self.buffer.view().window, cx);
    }

    /// `refresh_view` with `window` as the brush window.
    fn apply_view(&mut self, window: Option<TimeWindow>, cx: &mut Context<Self>) {
        let text = self.filter_input.read(cx).value();
        let matcher = match LineMatcher::parse(&text, self.filter_mode) {
            Ok(matcher) => {
                self.has_invalid_filter = false;
                matcher
            }
            Err(_) => {
                self.has_invalid_filter = true;
                self.buffer.view().matcher.clone()
            }
        };
        self.buffer.set_view(LineView {
            matcher,
            hidden_levels: self.hidden_levels,
            window,
        });
        let visible = self.buffer.visible_len();
        self.scroller
            .update(cx, |scroller, cx| scroller.reset(visible, cx));
        cx.notify();
    }

    /// The press on the histogram starts a drag.
    fn begin_brush(&mut self, fraction: f32, cx: &mut Context<Self>) {
        self.brush = Some(BrushDrag {
            anchor: fraction,
            current: fraction,
        });
        cx.notify();
    }

    fn move_brush(&mut self, fraction: f32, cx: &mut Context<Self>) {
        let Some(drag) = &mut self.brush else {
            return;
        };
        drag.current = fraction;
        cx.notify();
    }

    /// The release ends the drag: a drag across the chart sets the window of its buckets, a
    /// click without movement clears the window.
    fn end_brush(&mut self, fraction: f32, cx: &mut Context<Self>) {
        let Some(drag) = self.brush.take() else {
            return;
        };
        let width = self
            .chart_bounds
            .get()
            .map_or(0., |bounds| f32::from(bounds.size.width));
        let has_moved = (fraction - drag.anchor).abs() * width >= MIN_BRUSH_DRAG_PX;
        let window = if has_moved {
            self.current_volume()
                .and_then(|volume| brush_window(&volume, drag.anchor, fraction))
        } else {
            None
        };
        self.apply_view(window, cx);
    }

    fn clear_time_window(&mut self, cx: &mut Context<Self>) {
        self.apply_view(None, cx);
    }

    /// Switching the mode re-reads the input, so the placeholder and the matcher agree.
    fn set_filter_mode(&mut self, is_regex: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.filter_mode = if is_regex {
            FilterMode::Regex
        } else {
            FilterMode::Plain
        };
        let placeholder = match self.filter_mode {
            FilterMode::Plain => PLAIN_PLACEHOLDER,
            FilterMode::Regex => REGEX_PLACEHOLDER,
        };
        self.filter_input.update(cx, |input, cx| {
            input.set_placeholder(placeholder, window, cx);
        });
        self.refresh_view(cx);
    }

    pub(crate) fn pick_container(&mut self, name: String, cx: &mut Context<Self>) {
        let LogSubject::Pod { container, .. } = &mut self.subject else {
            return;
        };
        if name == *container {
            return;
        }
        *container = name;
        self.restart_stream(cx);
    }

    fn toggle_previous(&mut self, cx: &mut Context<Self>) {
        self.instance = match self.instance {
            LogInstance::Current => LogInstance::Previous,
            LogInstance::Previous => LogInstance::Current,
        };
        self.restart_stream(cx);
    }

    /// The tab moves to a window of its own. The kit input ties focus, blur, and activation to the
    /// window that created it, so the filter input is created again here with the same text and
    /// placeholder, and focused. The buffer, streams, scroller, staging, export, and brush hold no
    /// window and move as they are.
    pub(crate) fn move_to_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.filter_input.read(cx).value();
        let placeholder = match self.filter_mode {
            FilterMode::Plain => PLAIN_PLACEHOLDER,
            FilterMode::Regex => REGEX_PLACEHOLDER,
        };
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(placeholder)
                .default_value(text)
        });
        self._filter_events = cx.subscribe(&input, Self::on_filter_event);
        input.update(cx, |input, cx| input.focus(window, cx));
        self.filter_input = input;
        self.is_popped_out = true;
        cx.notify();
    }

    #[cfg(test)]
    pub(crate) fn is_popped_out(&self) -> bool {
        self.is_popped_out
    }

    pub(crate) fn set_layout(&mut self, layout: LogLayout, cx: &mut Context<Self>) {
        if self.layout == layout {
            return;
        }
        self.layout = layout;
        // The histogram is not drawn outside Full, so a drag in progress would never see its release.
        if layout != LogLayout::Full {
            self.brush = None;
        }
        cx.notify();
    }

    /// The visible lines for the file: full RFC 3339 times and full `{pod}/{container}`
    /// prefixes (a saved file must name its pods without the legend), and their count.
    pub(crate) fn export_snapshot(&self) -> (String, usize) {
        // A pod tab has one source and no prefix column.
        let prefixes: Vec<SharedString> = if self.is_workload() {
            self.streams
                .iter()
                .map(|stream| stream.full_prefix.clone())
                .collect()
        } else {
            Vec::new()
        };
        let mut text = self.buffer.visible_text(LineTime::Rfc3339, &prefixes);
        text.push('\n');
        (text, self.buffer.visible_len())
    }

    pub(crate) fn set_export_state(&mut self, state: ExportState, cx: &mut Context<Self>) {
        self.export_state = state;
        cx.notify();
    }

    pub(crate) fn set_exported_lines(&mut self, lines: usize) {
        self.exported_lines = lines;
    }

    /// Opens the save dialog; nothing is written unless the user confirms a path.
    fn export(&mut self, cx: &mut Context<Self>) {
        if self.export_state.is_busy() || self.buffer.visible_len() == 0 {
            return;
        }
        let label = match &self.subject {
            LogSubject::Pod {
                target, container, ..
            } => format!("{}-{container}", target.pod),
            LogSubject::Workload(workload) => workload.target.label.clone(),
        };
        let name = export_file_name(&label, "log", jiff::Timestamp::now());
        self.export_state = ExportState::Choosing;
        self._export = Some(start_export(
            name,
            "logs",
            |tab: &mut Self, _| Ok(tab.export_snapshot()),
            Self::set_export_state,
            Self::set_exported_lines,
            cx,
        ));
        cx.notify();
    }

    /// The histogram of the visible lines; recomputed only when the buffer changed.
    fn current_volume(&mut self) -> Option<Rc<Volume>> {
        let revision = self.buffer.revision();
        if let Some((memo_revision, memo)) = &self.volume_memo
            && *memo_revision == revision
        {
            return memo.clone();
        }
        let computed = volume(
            self.buffer
                .volume_lines()
                .filter_map(|line| Some((line.line.timestamp?, line.level))),
        )
        .map(Rc::new);
        self.volume_memo = Some((revision, computed.clone()));
        computed
    }

    /// The brush as the histogram draws it, with the handlers that drive this tab.
    fn brush_view(&self, cx: &Context<Self>) -> BrushView {
        let tab = cx.weak_entity();
        let handler = |act: fn(&mut Self, f32, &mut Context<Self>)| {
            let tab = tab.clone();
            Rc::new(move |fraction: f32, cx: &mut App| {
                let _ = tab.update(cx, |tab, cx| act(tab, fraction, cx));
            }) as FractionHandler
        };
        BrushView {
            window: self.buffer.view().window,
            drag: self.brush,
            bounds: Rc::clone(&self.chart_bounds),
            handlers: BrushHandlers {
                press: handler(Self::begin_brush),
                moved: handler(Self::move_brush),
                release: handler(Self::end_brush),
                clear: Rc::new({
                    let tab = tab.clone();
                    move |cx: &mut App| {
                        let _ = tab.update(cx, |tab, cx| tab.clear_time_window(cx));
                    }
                }),
            },
        }
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
            self.buffer.view().is_filtering(),
            self.buffer.has_dropped(),
        );
        let phase = self.phase();
        let LogSubject::Workload(workload) = &self.subject else {
            return match (&phase, self.instance) {
                (TabPhase::Waiting | TabPhase::Connecting, _) => "Opening…".to_owned(),
                (TabPhase::Streaming, _) if is_scrolled_up => format!("Paused · {count}"),
                (TabPhase::Streaming, _) => format!("Streaming · {count}"),
                (TabPhase::Ended, LogInstance::Current) => format!("Stream ended · {count}"),
                (TabPhase::Ended, LogInstance::Previous) => {
                    format!("Previous instance · {count}")
                }
                (TabPhase::Failed { .. }, _) => format!("Failed · {count}"),
            };
        };
        match phase {
            TabPhase::Waiting => format!("Waiting for pods of {}", workload.target.label),
            TabPhase::Connecting => {
                let opening = self
                    .streams
                    .iter()
                    .filter(|stream| stream.state == LogStreamState::Connecting)
                    .count();
                format!("Opening {opening} streams…")
            }
            TabPhase::Streaming => {
                let pods = self.streaming_pod_count();
                let noun = if pods == 1 { "pod" } else { "pods" };
                let lead = if is_scrolled_up {
                    "Paused"
                } else {
                    "Streaming"
                };
                format!("{lead} · {pods} {noun} · {count}")
            }
            TabPhase::Ended => format!("Streams ended · {count}"),
            TabPhase::Failed { .. } => format!("Failed · {count}"),
        }
    }

    fn streaming_pod_count(&self) -> usize {
        let mut pods: Vec<&str> = self
            .streams
            .iter()
            .filter(|stream| stream.state == LogStreamState::Streaming)
            .map(|stream| stream.pod.as_str())
            .collect();
        pods.sort_unstable();
        pods.dedup();
        pods.len()
    }

    fn render_toolbar(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let is_scrolled_up = self.scroller.read(cx).is_scrolled_up();
        let status = self.status_text(is_scrolled_up);
        let status = match &self.export_state {
            ExportState::Saved { file_name } => {
                format!(
                    "Saved {} lines to {file_name} · {status}",
                    self.exported_lines
                )
            }
            _ => status,
        };
        let is_connecting = self.phase() == TabPhase::Connecting;
        h_flex()
            .flex_shrink_0()
            .flex_wrap()
            .items_center()
            .gap_2()
            .px_3()
            .py_1p5()
            .border_b_1()
            .border_color(theme.border)
            .children(self.render_container_picker(cx))
            .child(
                div()
                    .flex_1()
                    .min_w(px(140.))
                    .max_w(px(240.))
                    .child(Input::new(&self.filter_input).small().cleanable(true)),
            )
            .child(
                Toggle::new("log-regex")
                    .small()
                    .icon(Icon::new(IconName::Regex))
                    .tooltip("Regular expression")
                    .checked(self.filter_mode == FilterMode::Regex)
                    .on_click(cx.listener(|tab, checked: &bool, window, cx| {
                        tab.set_filter_mode(*checked, window, cx);
                    })),
            )
            .children(LogLevel::ALL.map(|level| {
                Toggle::new(("log-level", level as usize))
                    .small()
                    .label(level.label())
                    .checked(!self.hidden_levels.is_hidden(level))
                    .on_click(cx.listener(move |tab, _: &bool, _, cx| {
                        tab.hidden_levels = tab.hidden_levels.toggled(level);
                        tab.refresh_view(cx);
                    }))
            }))
            .when(!self.is_workload(), |toolbar| {
                toolbar.child(
                    Toggle::new("log-previous")
                        .small()
                        .label("Previous")
                        .tooltip("Logs of the previous container instance")
                        .checked(self.instance == LogInstance::Previous)
                        .on_click(cx.listener(|tab, _: &bool, _, cx| tab.toggle_previous(cx))),
                )
            })
            .child(
                Toggle::new("log-json")
                    .small()
                    .label("JSON")
                    .tooltip("Show JSON lines as a message and fields")
                    .checked(self.shows_json)
                    .on_click(cx.listener(|tab, checked: &bool, _, cx| {
                        tab.shows_json = *checked;
                        tab.remeasure(cx);
                    })),
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
                        let time = if tab.shows_timestamps {
                            LineTime::Clock
                        } else {
                            LineTime::Hidden
                        };
                        // A pod tab has one source and no prefix column.
                        let prefixes: Vec<SharedString> = if tab.is_workload() {
                            tab.streams
                                .iter()
                                .map(|stream| stream.prefix.clone())
                                .collect()
                        } else {
                            Vec::new()
                        };
                        let text = tab.buffer.visible_text(time, &prefixes);
                        cx.write_to_clipboard(ClipboardItem::new_string(text));
                    })),
            )
            .child(
                Button::new("log-export")
                    .ghost()
                    .small()
                    .icon(Icon::new(IconName::Download))
                    .tooltip("Export visible lines…")
                    .disabled(self.export_state.is_busy() || self.buffer.visible_len() == 0)
                    .on_click(cx.listener(|tab, _, _, cx| tab.export(cx))),
            )
            .when(!self.is_popped_out, |toolbar| {
                toolbar.child(
                    Button::new("log-pop-out")
                        .ghost()
                        .small()
                        .icon(Icon::new(IconName::ExternalLink))
                        .when(self.layout == LogLayout::Full, |button| {
                            button.label("Pop out")
                        })
                        .tooltip("Open in a new window")
                        .on_click(cx.listener(|_, _, _, cx| cx.emit(LogTabEvent::PopOut))),
                )
            })
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
            .child(
                h_flex()
                    .ml_auto()
                    .gap_1p5()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .when(self.has_invalid_filter, |status| {
                        status.child(
                            div()
                                .text_color(tone_color(StatusTone::Bad, cx))
                                .child(InvalidRegex.to_string()),
                        )
                    })
                    .child(status),
            )
    }

    /// `Containers: api, worker ▾`: every member streams the checked containers.
    fn render_workload_picker(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let LogSubject::Workload(workload) = &self.subject else {
            return None;
        };
        if workload.selected.is_empty() {
            return None;
        }
        let label = format!("Containers: {}", workload.selected.join(", "));
        let tab = cx.weak_entity();
        let offered = workload.offered.clone();
        let selected = workload.selected.clone();
        Some(
            Button::new("log-containers")
                .ghost()
                .small()
                .child(label)
                .dropdown_caret(true)
                .dropdown_menu(move |menu, _, _| {
                    offered.iter().fold(menu, |menu, name| {
                        let toggled = name.clone();
                        let tab = tab.clone();
                        menu.item(
                            PopupMenuItem::new(name.clone())
                                .checked(selected.contains(name))
                                .on_click(move |_, _, cx| {
                                    let _ = tab.update(cx, |tab, cx| {
                                        tab.toggle_container(&toggled, cx);
                                    });
                                }),
                        )
                    })
                })
                .into_any_element(),
        )
    }

    /// Checks or unchecks a container for every member and reopens the streams. The last
    /// checked container stays.
    fn toggle_container(&mut self, name: &str, cx: &mut Context<Self>) {
        let LogSubject::Workload(workload) = &mut self.subject else {
            return;
        };
        let Some(selected) = toggled_selection(&workload.selected, &workload.offered, name) else {
            return;
        };
        workload.selected = selected;
        self.restart_stream(cx);
    }

    /// One chip per pod name, in join order.
    fn legend_chips(&self) -> Vec<LegendChip> {
        let LogSubject::Workload(workload) = &self.subject else {
            return Vec::new();
        };
        self.pod_slots
            .iter()
            .enumerate()
            .map(|(slot, pod)| {
                let streams: Vec<&TabStream> = self
                    .streams
                    .iter()
                    .filter(|stream| stream.pod == *pod)
                    .collect();
                let failure = streams.iter().find_map(|stream| match &stream.state {
                    LogStreamState::Failed { message } => Some(SharedString::from(message.clone())),
                    _ => None,
                });
                LegendChip {
                    short_name: pod_short_name(&workload.target.owner, pod)
                        .to_owned()
                        .into(),
                    color_slot: slot,
                    tone: workload_tone(streams.iter().map(|stream| &stream.state)),
                    is_deleted: !streams.is_empty() && streams.iter().all(|s| !s.is_member),
                    failure,
                }
            })
            .collect()
    }

    /// The pod container picker, or the workload container picker.
    fn render_container_picker(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let LogSubject::Pod {
            target, container, ..
        } = &self.subject
        else {
            return self.render_workload_picker(cx);
        };
        let theme = cx.theme();
        let current_kind = target
            .containers
            .iter()
            .find(|candidate| candidate.name == *container)
            .map(|candidate| kind_tag_text(candidate.kind));
        let label = h_flex()
            .gap_1p5()
            .items_center()
            .font_family(theme.mono_font_family.clone())
            .child(container.clone())
            .children(current_kind.map(|kind| {
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(kind)
            }));
        // A single container has nothing to pick, so it is plain text, not a disabled button.
        if target.containers.len() < 2 {
            return Some(label.px_2().text_sm().into_any_element());
        }
        let button = Button::new("log-container").ghost().small().child(label);
        let tab = cx.weak_entity();
        let containers = target.containers.clone();
        let selected = container.clone();
        Some(
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
                                let _ =
                                    tab.update(cx, |tab, cx| tab.pick_container(name.clone(), cx));
                            }),
                        )
                    })
                })
                .into_any_element(),
        )
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
            return match self.phase() {
                TabPhase::Waiting => match &self.subject {
                    LogSubject::Workload(workload) => {
                        muted(format!("Waiting for pods of {}", workload.target.label))
                    }
                    LogSubject::Pod { .. } => div().into_any_element(),
                },
                TabPhase::Connecting => v_flex()
                    .size_full()
                    .items_center()
                    .justify_center()
                    .gap_3()
                    .child(Spinner::new())
                    .child(
                        div()
                            .text_sm()
                            .text_color(theme.muted_foreground)
                            .child(format!("Opening logs of {}…", self.label())),
                    )
                    .into_any_element(),
                TabPhase::Ended => muted("The containers wrote no log lines".to_owned()),
                TabPhase::Streaming => muted("No log lines yet".to_owned()),
                TabPhase::Failed { .. } => div().into_any_element(),
            };
        }
        if self.buffer.visible_len() == 0 {
            return match &self.buffer.view().matcher {
                Some(matcher) => muted(format!("No lines match \"{}\"", matcher.pattern())),
                None if self.buffer.view().window.is_some() => {
                    muted("No lines in the selected time window".to_owned())
                }
                None => muted("No lines at the selected levels".to_owned()),
            };
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
        let prefix = self
            .is_workload()
            .then(|| self.streams.get(usize::from(line.source.0)))
            .flatten()
            .map(|stream| RowPrefix {
                text: stream.prefix.clone(),
                color: pod_color(stream.color_slot, cx),
            });
        let style = RowStyle {
            shows_timestamps: self.shows_timestamps,
            wraps_lines: self.wraps_lines,
            shows_json: self.shows_json,
            matcher: self.buffer.view().matcher.as_ref(),
            prefix,
        };
        log_row(line, &style, cx)
    }
}

fn row_of(tab: &WeakEntity<LogTab>, index: usize, cx: &App) -> AnyElement {
    tab.read_with(cx, |tab, cx| tab.render_row(index, cx))
        .unwrap_or_else(|_| div().into_any_element())
}

/// What a tab asks of whoever holds it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LogTabEvent {
    /// The Pop out button: move the tab to a window of its own.
    PopOut,
}

impl EventEmitter<LogTabEvent> for LogTab {}

impl Render for LogTab {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let stream_failure = match self.phase() {
            TabPhase::Failed { message } => Some(("log-error", "Cannot read the logs", message)),
            _ => None,
        };
        let export_failure = match &self.export_state {
            ExportState::Failed { message } => {
                Some(("log-export-error", "Export failed", message.clone()))
            }
            _ => None,
        };
        let is_full = self.layout == LogLayout::Full;
        let legend = match &self.subject {
            LogSubject::Workload(workload)
                if is_full && (!self.pod_slots.is_empty() || workload.is_frozen) =>
            {
                Some(legend_row(self.legend_chips(), workload.is_frozen, cx))
            }
            _ => None,
        };
        let histogram = if is_full {
            let brush = self.brush_view(cx);
            self.current_volume()
                .map(|volume| volume_chart(&volume, brush, cx))
        } else {
            None
        };
        v_flex()
            .size_full()
            .children(legend)
            .child(self.render_toolbar(cx))
            .children(histogram)
            .children([stream_failure, export_failure].into_iter().flatten().map(
                |(id, title, message)| {
                    div()
                        .flex_shrink_0()
                        .px_3()
                        .py_2()
                        .child(Alert::error(id, message).title(title))
                },
            ))
            .child(div().flex_1().min_h_0().child(self.render_body(cx)))
    }
}

/// A pod container stream to open.
struct StreamOpen {
    namespace: String,
    pod: String,
    container: String,
    prefix: SharedString,
    full_prefix: SharedString,
    color_slot: usize,
    tail_lines: u32,
}

/// A pod admitted to a workload tab, with the selected containers it has.
struct Admission {
    namespace: String,
    pod: String,
    containers: Vec<String>,
}

struct SyncPlan {
    is_frozen: bool,
    selected: Vec<String>,
    offered: Vec<String>,
    change: MemberChange,
    admissions: Vec<Admission>,
}

/// Reads the session and decides what changes. `None` while the session is gone, not live, or
/// has no pods snapshot yet: membership then stays as it was.
fn plan_sync(workload: &WorkloadSubject, live_streams: usize, cx: &App) -> Option<SyncPlan> {
    let session = workload.session.upgrade()?;
    let session = session.read(cx);
    let live = session.live()?;
    let pods = live.pods.ready_items()?;
    Some(plan_membership(
        &workload.target.owner,
        &workload.members,
        &workload.selected,
        pods,
        &live.scope,
        live_streams,
    ))
}

/// The pure part of `plan_sync`. `selected` is empty until a pod has been listed; the first
/// ranked pod then decides the container.
fn plan_membership(
    owner: &PodOwner,
    members: &[String],
    selected: &[String],
    pods: &[PodSummary],
    scope: &NamespaceScope,
    live_streams: usize,
) -> SyncPlan {
    let covers = owner
        .namespace()
        .is_some_and(|namespace| scope_covers(scope, namespace));
    if !covers {
        // A scope change must not end streams the user opened.
        return SyncPlan {
            is_frozen: true,
            selected: Vec::new(),
            offered: Vec::new(),
            change: MemberChange {
                joined: Vec::new(),
                left: Vec::new(),
            },
            admissions: Vec::new(),
        };
    }
    let ranked = ranked_pods(owner, pods);
    let selected: Vec<String> = if selected.is_empty() {
        ranked
            .first()
            .and_then(|pod| {
                let index = default_container(&pod.containers)?;
                Some(pod.containers.get(index)?.name.clone())
            })
            .into_iter()
            .collect()
    } else {
        selected.to_vec()
    };
    let slots = join_slots(members.len(), live_streams, selected.len());
    let change = member_change(members, &ranked, slots);
    let admissions = change
        .joined
        .iter()
        .filter_map(|name| ranked.iter().find(|pod| pod.name == *name))
        .map(|pod| Admission {
            namespace: pod.namespace.clone(),
            pod: pod.name.clone(),
            containers: pod
                .containers
                .iter()
                .filter(|container| selected.contains(&container.name))
                .map(|container| container.name.clone())
                .collect(),
        })
        .collect();
    SyncPlan {
        is_frozen: false,
        selected,
        offered: container_names(&ranked),
        change,
        admissions,
    }
}

/// The marker lines for the restarts of the streamed containers, each from the latest stream of
/// its pod and container (so a reopened pod keeps its newest prefix and color). A pod that is no
/// longer listed is skipped.
fn restart_markers(
    streams: &[TabStream],
    seen: &mut RestartBaselines,
    pods: &[PodSummary],
) -> Vec<SourcedLine> {
    let now = jiff::Timestamp::now();
    let listed: HashMap<(&str, &str), &PodSummary> = pods
        .iter()
        .map(|pod| ((pod.namespace.as_str(), pod.name.as_str()), pod))
        .collect();
    // The streams of each pod, as indexes (the index is the source id), in the order they opened.
    let mut by_pod: HashMap<(&str, &str), Vec<usize>> = HashMap::new();
    for (index, stream) in streams.iter().enumerate() {
        by_pod
            .entry((stream.namespace.as_str(), stream.pod.as_str()))
            .or_default()
            .push(index);
    }
    // A pod that is gone and has no live stream left has nothing to compare with.
    seen.retain(|(namespace, pod, _), _| {
        let owner = (namespace.as_str(), pod.as_str());
        listed.contains_key(&owner)
            || by_pod
                .get(&owner)
                .is_some_and(|own| own.iter().any(|index| streams[*index].state.is_live()))
    });
    let mut markers = Vec::new();
    // Pods in the order their first stream opened, so the markers come out in a stable order.
    let mut owners: Vec<_> = by_pod.iter().collect();
    owners.sort_by_key(|(_, own)| own[0]);
    for (owner, own) in owners {
        // A pod that is no longer listed is skipped.
        let Some(pod) = listed.get(owner) else {
            continue;
        };
        let streamed: Vec<&str> = own
            .iter()
            .map(|index| streams[*index].container.as_str())
            .collect();
        for (container, _) in rising_restarts(seen, pod, &streamed) {
            let source = own
                .iter()
                .rfind(|index| streams[**index].container == container)
                .and_then(|index| u16::try_from(*index).ok());
            let summary = pod
                .containers
                .iter()
                .find(|summary| summary.name == container);
            if let (Some(source), Some(summary)) = (source, summary) {
                markers.push(SourcedLine {
                    source: SourceId(source),
                    kind: LineKind::Marker,
                    line: restart_marker(summary, now),
                });
            }
        }
    }
    markers
}

/// The merge window opens with the first pod that gets a stream, so the initial tails of the
/// pods admitted together are sorted. Later pods join without one and ask for a short tail.
fn opens_merge_window(stream_count: usize, has_staging: bool, admitted: usize) -> bool {
    admitted > 0 && stream_count == 0 && !has_staging
}

/// The color slot of `pod`; a name seen before keeps its slot.
fn slot_for(pod_slots: &mut Vec<String>, pod: &str) -> usize {
    if let Some(slot) = pod_slots.iter().position(|name| name == pod) {
        return slot;
    }
    pod_slots.push(pod.to_owned());
    pod_slots.len() - 1
}

/// The selection after toggling `name`, kept in the order the picker offers. `None` when the
/// last checked container would be unchecked.
fn toggled_selection(selected: &[String], offered: &[String], name: &str) -> Option<Vec<String>> {
    if selected.iter().any(|checked| checked == name) {
        if selected.len() == 1 {
            return None;
        }
        return Some(
            selected
                .iter()
                .filter(|checked| *checked != name)
                .cloned()
                .collect(),
        );
    }
    let mut toggled = selected.to_vec();
    toggled.push(name.to_owned());
    toggled.sort_by_key(|checked| {
        offered
            .iter()
            .position(|candidate| candidate == checked)
            .unwrap_or(usize::MAX)
    });
    Some(toggled)
}

/// Stable by kubelet time, so equal times keep arrival order; lines without a time come first.
fn sort_staged(lines: &mut [SourcedLine]) {
    lines.sort_by_key(|line| line.line.timestamp);
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
    use cluster::{
        ContainerKind, ContainerState, ContainerSummary, ControllerRef, LogLine, PodStatus,
        ReadyCount, StatusReason,
    };

    use super::*;
    use crate::kind_row::JOB_KIND;

    fn tone_of(list: &[LogStreamState]) -> StatusTone {
        workload_tone(list.iter())
    }

    fn failed() -> LogStreamState {
        LogStreamState::Failed {
            message: "boom".to_owned(),
        }
    }

    fn staged(source: u16, time: Option<&str>, text: &str) -> SourcedLine {
        SourcedLine {
            source: SourceId(source),
            kind: LineKind::Log,
            line: LogLine {
                timestamp: time.map(|time| time.parse().expect("valid time")),
                text: text.to_owned(),
            },
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
        assert_eq!(state, failed());
    }

    #[test]
    fn workload_tone_follows_stream_states() {
        use LogStreamState::{Connecting, Ended, Streaming};
        assert_eq!(tone_of(&[]), StatusTone::Info);
        assert_eq!(tone_of(&[Connecting, Connecting]), StatusTone::Info);
        assert_eq!(tone_of(&[Connecting, Streaming]), StatusTone::Ok);
        assert_eq!(tone_of(&[Ended, Streaming, failed()]), StatusTone::Ok);
        assert_eq!(tone_of(&[failed(), failed()]), StatusTone::Bad);
        assert_eq!(tone_of(&[Ended, Ended]), StatusTone::Done);
        assert_eq!(tone_of(&[Ended, failed()]), StatusTone::Done);
    }

    #[test]
    fn stream_phase_reports_a_failure_only_when_every_stream_failed() {
        let mixed = stream_phase([failed(), LogStreamState::Ended].iter());
        assert_eq!(mixed, TabPhase::Ended);
        let all = stream_phase([failed()].iter());
        assert_eq!(
            all,
            TabPhase::Failed {
                message: "boom".to_owned()
            }
        );
        assert_eq!(stream_phase([].iter()), TabPhase::Waiting);
    }

    #[test]
    fn staged_lines_sort_by_timestamp_stably() {
        let mut lines = vec![
            staged(0, Some("2024-05-01T10:00:02Z"), "late"),
            staged(1, Some("2024-05-01T10:00:01Z"), "first of two"),
            staged(2, None, "untimed"),
            staged(0, Some("2024-05-01T10:00:01Z"), "second of two"),
        ];
        sort_staged(&mut lines);
        let order: Vec<&str> = lines.iter().map(|line| line.line.text.as_str()).collect();
        assert_eq!(order, ["untimed", "first of two", "second of two", "late"]);
    }

    fn container(name: &str, kind: ContainerKind, is_ready: bool) -> ContainerSummary {
        ContainerSummary {
            terminal: cluster::ContainerTerminal::None,
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

    fn job_pod(name: &str, containers: Vec<ContainerSummary>) -> PodSummary {
        PodSummary {
            is_finished: false,
            namespace: "ns".to_owned(),
            name: name.to_owned(),
            status: PodStatus::Reason(StatusReason::Running),
            ready: ReadyCount { ready: 1, total: 1 },
            restarts: 0,
            node_name: None,
            created_at: None,
            pod_ip: None,
            qos_class: None,
            service_account: None,
            controller: Some(ControllerRef {
                kind: JOB_KIND.to_owned(),
                name: "batch".to_owned(),
            }),
            conditions: Vec::new(),
            status_message: None,
            labels: Vec::new(),
            host_network: false,
            image_pull_secrets: Vec::new(),
            containers,
        }
    }

    fn job_owner() -> PodOwner {
        PodOwner::Controller {
            namespace: "ns".to_owned(),
            kind: JOB_KIND,
            name: "batch".to_owned(),
        }
    }

    fn two_container_pod(name: &str) -> PodSummary {
        job_pod(
            name,
            vec![
                container("app", ContainerKind::Main, true),
                container("sidecar", ContainerKind::Main, true),
            ],
        )
    }

    #[test]
    fn first_sync_selects_default_container_of_first_ranked_pod() {
        let pods = [two_container_pod("a")];
        let plan = plan_membership(&job_owner(), &[], &[], &pods, &NamespaceScope::All, 0);
        assert!(!plan.is_frozen);
        assert_eq!(plan.selected, ["app"]);
        assert_eq!(plan.change.joined, ["a"]);
    }

    #[test]
    fn plan_admits_only_selected_containers_the_pod_has() {
        let pods = [
            two_container_pod("a"),
            job_pod("b", vec![container("other", ContainerKind::Main, true)]),
        ];
        let selected = ["sidecar".to_owned()];
        let plan = plan_membership(&job_owner(), &[], &selected, &pods, &NamespaceScope::All, 0);
        assert_eq!(plan.selected, ["sidecar"]);
        let admitted: Vec<_> = plan
            .admissions
            .iter()
            .map(|admission| (admission.pod.as_str(), admission.containers.clone()))
            .collect();
        assert_eq!(
            admitted,
            [("a", vec!["sidecar".to_owned()]), ("b", Vec::new())]
        );
    }

    #[test]
    fn plan_without_pods_selects_nothing_yet() {
        let plan = plan_membership(&job_owner(), &[], &[], &[], &NamespaceScope::All, 0);
        assert!(plan.selected.is_empty());
        assert!(plan.admissions.is_empty());
    }

    #[test]
    fn plan_is_frozen_when_scope_excludes_the_workload() {
        let pods = [two_container_pod("a")];
        let members = ["a".to_owned()];
        let scope = NamespaceScope::Named("elsewhere".to_owned());
        let plan = plan_membership(&job_owner(), &members, &[], &pods, &scope, 0);
        assert!(plan.is_frozen);
        assert!(plan.change.joined.is_empty());
        assert!(plan.change.left.is_empty());
        assert!(plan.admissions.is_empty());
    }

    #[test]
    fn merge_window_opens_with_the_first_admission_only() {
        assert!(opens_merge_window(0, false, 1));
        // Nothing admitted yet, so a late pods snapshot still gets the full tail.
        assert!(!opens_merge_window(0, false, 0));
        assert!(!opens_merge_window(0, true, 2));
        // Streams exist: a later joiner asks for a short tail.
        assert!(!opens_merge_window(3, false, 1));
    }

    #[test]
    fn toggled_selection_keeps_offer_order_and_the_last_container() {
        let offered = ["api".to_owned(), "worker".to_owned(), "setup".to_owned()];
        let selected = ["worker".to_owned()];
        assert_eq!(
            toggled_selection(&selected, &offered, "api"),
            Some(vec!["api".to_owned(), "worker".to_owned()])
        );
        assert_eq!(toggled_selection(&selected, &offered, "worker"), None);
        let both = ["api".to_owned(), "worker".to_owned()];
        assert_eq!(
            toggled_selection(&both, &offered, "api"),
            Some(vec!["worker".to_owned()])
        );
    }
    #[test]
    fn slot_for_reuses_slot_of_returning_pod() {
        let mut slots = Vec::new();
        assert_eq!(slot_for(&mut slots, "db-0"), 0);
        assert_eq!(slot_for(&mut slots, "db-1"), 1);
        assert_eq!(slot_for(&mut slots, "db-0"), 0);
        assert_eq!(slots.len(), 2);
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

#[cfg(test)]
#[path = "log_tab_tests.rs"]
mod log_tab_tests;
