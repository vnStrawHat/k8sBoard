//! The Topology screen (W11): the toolbar, the canvas with its cards, and the state between them.
//! The graph and its layout are rebuilt at most every `TOPOLOGY_TICK`, and only when something
//! changed; everything runs on the main thread because the input is bounded (decision 29).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::rc::Rc;
use std::time::{Duration, Instant};

use cluster::{
    ClusterConnection, MetricsSource, NamespaceScope, NamespaceSummary, PodSummary,
    TrafficMetricSource, TrafficSourceKind,
};
use gpui_kit::component::alert::Alert;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IconName, Selectable as _, Sizable as _, h_flex, v_flex,
};
use gpui_kit::{
    AnyElement, App, Context, Div, Entity, FocusHandle, InteractiveElement as _, IntoElement,
    KeyDownEvent, MouseButton, MouseDownEvent, ParentElement as _, Pixels, Point, Render,
    ScrollDelta, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Task,
    WeakEntity, Window, div, prelude::FluentBuilder as _, px,
};

use crate::app_shell::AppShell;
use crate::app_shell::workspace::toggle_button;
use crate::cluster_metrics::{SourceState, TrafficSources};
use crate::cluster_runtime::ClusterRuntime;
use crate::cluster_session::{ClusterSession, LiveCluster, scope_includes};
use crate::drawer::{DrawerSize, drawer_width};
use crate::file_export::{ExportState, export_file_name, start_export_with};
use crate::status_tone::{StatusTone, tone_color};
use crate::table_selection::ResourceKey;
use crate::topology_canvas::{
    CanvasPaint, MinimapPaint, TITLE_PILL_HEIGHT, TITLE_PILL_LEFT, TITLE_PILL_PADDING,
    TITLE_PILL_TOP, TITLE_SIZE, graph_canvas, handle_canvas, legend_entries, legend_swatch,
    legend_width, minimap_canvas,
};
use crate::topology_card::{CardFrame, CardState, node_card};
use crate::topology_checks::{ConfigCheck, checks_chip, topology_coverage};
use crate::topology_colors::CanvasColors;
use crate::topology_export::{TopologyExport, export_scale, svg_style, topology_svg, write_export};
use crate::topology_feeds::TopologySubject;
use crate::topology_graph::{
    FeedRows, GroupBy, KindFilter, NODE_LIMIT, NodeId, NodeLook, RAW_LIMIT, TooLarge,
    TopologyBuild, TopologyFilter, TopologyGraph, TopologyInputs, TopologyKind, build_topology,
    resolve_group_by,
};
use crate::topology_layout::{
    GraphPoint, GraphRect, GraphStructure, TopologyLayout, layout as lay_out, structure,
};
use crate::topology_traffic::{
    TrafficLayer, TrafficOverlay, TrafficSample, shown_caption, tooltip_with_traffic,
};
use crate::topology_traffic_labels::{LABEL_HEIGHT, LABEL_TEXT_SIZE, edge_labels};
use crate::topology_viewport::{
    CONTROLS_INSET, MIN_TEXT_ZOOM, MINIMAP_HEIGHT, MINIMAP_WIDTH, OVERLAY_GUTTER, Viewport,
    ZOOM_BUTTON_STEPS, is_drag, visible_nodes, wheel_steps,
};

/// The rebuild runs at most this often, and only when something changed.
const TOPOLOGY_TICK: Duration = Duration::from_millis(500);
/// The most checks the dropdown lists; the rest is a count.
const CHECKS_MENU_LIMIT: usize = 50;
/// The size the first frame assumes for the canvas, before it has painted once.
const DEFAULT_CANVAS: (f32, f32) = (1200., 700.);
/// The font size of a band title in a zoomed-out view, in pixels.
const LOW_ZOOM_TITLE_SIZE: f32 = 11.;
/// The flow of the selected edges repaints at about 30 fps: a repaint of the whole window every
/// display frame is not worth a moving dash.
const FLOW_FRAME: Duration = Duration::from_millis(33);
/// A selected card is kept this far from the edge of the area the drawer leaves free.
const REVEAL_MARGIN: f32 = 24.;
/// The share of its size the minimap keeps while the drawer is open.
const COMPACT_MINIMAP_SCALE: f32 = 0.5;
/// The font size of the legend text, in pixels.
const LEGEND_TEXT_SIZE: f32 = 11.;
/// The legend sits this far left of the minimap.
const LEGEND_RIGHT_GAP: f32 = 28.;
/// The height of that title's pill, in pixels.
const LOW_ZOOM_TITLE_HEIGHT: f32 = 18.;
/// The height of the namespace list the dropdown shows before it scrolls.
const NAMESPACE_MENU_HEIGHT: f32 = 320.;
/// The tooltip of a layer chip that Traffic mode has turned off.
const NOT_IN_TRAFFIC: &str = "Not shown in Traffic";

/// How often the Traffic sample is read again, and the metric list tried again after a failure.
const TRAFFIC_REFRESH: Duration = Duration::from_secs(30);

/// The pointer interaction that is running.
enum Drag {
    None,
    /// An empty-space press: a drag pans, a release without one clears the selection.
    Pan {
        start: Point<Pixels>,
        last: Point<Pixels>,
        moved: bool,
    },
    /// A card press: a drag moves the node and pins it, a release without one is a click.
    Node {
        id: NodeId,
        start: Point<Pixels>,
        origin: GraphPoint,
        moved: bool,
    },
    Minimap,
}

/// What the canvas draws: the Resources layout alone, or the traffic that flows over it (0049).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TopologyMode {
    Resources,
    Traffic,
}

/// The Traffic fetches of the view: dropped as a whole on a mode, namespace, session, or
/// visibility change, which stops the request and the refresh with it.
#[derive(Default)]
struct TrafficRun {
    /// The newest sample with an answer, kept while a refresh fails.
    sample: Option<Rc<TrafficSample>>,
    /// The layer drawn: derived from the sample, the graph, and the layout.
    layer: Option<Rc<TrafficLayer>>,
    /// The request in flight; dropping it aborts the request.
    fetch: Option<Task<()>>,
    last_fetch: Option<Instant>,
    /// When the metric list failed last, so a retry waits for the next refresh.
    last_names_try: Option<Instant>,
    /// Why the last refresh failed; the older sample stays drawn.
    paused: Option<String>,
}

/// The built graph, or why there is none.
type Built = Result<Rc<TopologyGraph>, TooLarge>;

/// What one Traffic request round returns: the reading of each detected source.
type TrafficReadings = Vec<(
    TrafficMetricSource,
    Result<cluster::TrafficReading, cluster::MetricsError>,
)>;

/// What one Traffic request round needs.
struct TrafficFetchPlan {
    sources: Vec<TrafficMetricSource>,
    source: MetricsSource,
    connection: ClusterConnection,
}
/// What `sync_traffic` does next.
#[derive(Debug, PartialEq, Eq)]
enum TrafficStep {
    /// The source cannot serve, or holds no traffic metric: back to Resources.
    Leave,
    LoadNames,
    Wait,
    Fetch,
}

/// The next step, from the source state, the metric list, and the run so far. A source that is
/// being checked again (after a change in Settings) is waited for; only one that is missing,
/// invalid, or unreachable ends Traffic mode.
fn traffic_step(run: &TrafficRun, metrics: &crate::cluster_metrics::ClusterMetrics) -> TrafficStep {
    match &metrics.source {
        SourceState::Ready { .. } => {}
        SourceState::Checking { .. } => return TrafficStep::Wait,
        SourceState::None | SourceState::Invalid | SourceState::Failed { .. } => {
            return TrafficStep::Leave;
        }
    }
    let is_due = |at: Option<Instant>| at.is_none_or(|at| at.elapsed() >= TRAFFIC_REFRESH);
    match &metrics.traffic_sources {
        TrafficSources::NotLoaded => TrafficStep::LoadNames,
        TrafficSources::Loading { .. } => TrafficStep::Wait,
        TrafficSources::Loaded(Err(_)) if is_due(run.last_names_try) => TrafficStep::LoadNames,
        TrafficSources::Loaded(Err(_)) => TrafficStep::Wait,
        TrafficSources::Loaded(Ok(sources)) if sources.is_empty() => TrafficStep::Leave,
        TrafficSources::Loaded(Ok(_)) if run.fetch.is_some() || !is_due(run.last_fetch) => {
            TrafficStep::Wait
        }
        TrafficSources::Loaded(Ok(_)) => TrafficStep::Fetch,
    }
}

fn fetch_plan(live: &LiveCluster) -> Option<TrafficFetchPlan> {
    let (source, _) = live.metrics.source.ready()?;
    let TrafficSources::Loaded(Ok(sources)) = &live.metrics.traffic_sources else {
        return None;
    };
    Some(TrafficFetchPlan {
        sources: sources.clone(),
        source: source.clone(),
        connection: live.connection().clone(),
    })
}
/// The Traffic segment: whether it can be pressed, and what its tooltip says.
pub(crate) struct TrafficButton {
    pub(crate) is_enabled: bool,
    pub(crate) tooltip: String,
}

/// The state table of the Traffic segment (spec 0049): the 0048 source state, then the metric list.
pub(crate) fn traffic_button(metrics: &crate::cluster_metrics::ClusterMetrics) -> TrafficButton {
    let disabled = |tooltip: String| TrafficButton {
        is_enabled: false,
        tooltip,
    };
    let enabled = |tooltip: String| TrafficButton {
        is_enabled: true,
        tooltip,
    };
    let source = match &metrics.source {
        SourceState::None => {
            return disabled("Choose a metrics source in Settings \u{203a} Metrics".to_owned());
        }
        SourceState::Invalid => {
            return disabled("The metrics source in Settings is not valid".to_owned());
        }
        SourceState::Checking { .. } => {
            return disabled("Checking the metrics source\u{2026}".to_owned());
        }
        SourceState::Failed { error, .. } => {
            return disabled(format!("Metrics source unreachable: {error}"));
        }
        SourceState::Ready { source, .. } => source.display(),
    };
    match &metrics.traffic_sources {
        TrafficSources::NotLoaded | TrafficSources::Loading { .. } => {
            enabled(format!("Show traffic from {source}"))
        }
        TrafficSources::Loaded(Ok(sources)) if sources.is_empty() => {
            disabled(format!("No traffic metrics in {source}"))
        }
        TrafficSources::Loaded(Ok(_)) => enabled(format!("Show traffic from {source}")),
        TrafficSources::Loaded(Err(error)) => enabled(format!(
            "Show traffic (the metric list failed: {error}; retrying)"
        )),
    }
}

/// The first error of a sample in which no source answered; `None` when any did (or none was
/// asked).
fn sample_failure(sample: &TrafficSample) -> Option<String> {
    if sample.readings.iter().any(|(_, reading)| reading.is_ok()) {
        return None;
    }
    sample
        .readings
        .iter()
        .find_map(|(_, reading)| reading.as_ref().err().map(ToString::to_string))
        .or_else(|| Some("no traffic source answered".to_owned()))
}

/// The chip of Traffic mode. `shown` is the sources of the layer drawn and its clock time.
fn traffic_chip_text(shown: Option<(&[TrafficSourceKind], &str)>, paused: Option<&str>) -> String {
    if let Some(reason) = paused {
        return format!("Traffic \u{b7} paused \u{b7} {reason}");
    }
    let Some((sources, clock)) = shown else {
        return "Traffic \u{b7} loading\u{2026}".to_owned();
    };
    let names = match sources {
        [TrafficSourceKind::PodNetwork] => {
            "pod network bytes (per pod, not per connection)".to_owned()
        }
        other => other
            .iter()
            .map(|kind| kind.label())
            .collect::<Vec<_>>()
            .join(", "),
    };
    format!("Traffic \u{b7} {names} \u{b7} last 5 min \u{b7} {clock}")
}

/// The chip tooltip: the notes of failed or cut readings, then the peers outside the namespace.
fn traffic_chip_tooltip(overlay: &TrafficOverlay) -> Option<String> {
    let mut lines = overlay.notes.clone();
    if !overlay.outside.is_empty() {
        lines.push(format!(
            "{} peers outside this namespace: {}",
            overlay.outside.len(),
            overlay.outside.join(", ")
        ));
    }
    (!lines.is_empty()).then(|| lines.join("\n"))
}

/// `HH:MM:SS` of `at` in `zone`.
fn clock_text(at: jiff::Timestamp, zone: &jiff::tz::TimeZone) -> String {
    at.to_zoned(zone.clone()).strftime("%H:%M:%S").to_string()
}

pub(crate) struct TopologyView {
    session: Option<Entity<ClusterSession>>,
    shell: WeakEntity<AppShell>,
    /// The namespace drawn; `None` until one is picked (scope `All`).
    namespace: Option<String>,
    /// The namespace drawn last, per context, kept while the app runs: scope `All` opens in it.
    last_namespaces: HashMap<String, String>,
    filter: TopologyFilter,
    /// The pod groups the user opened.
    expanded: BTreeSet<NodeId>,
    /// The dragged positions, kept in memory per (context, namespace) (decision 23).
    pins: HashMap<String, HashMap<String, HashMap<NodeId, GraphPoint>>>,
    build: Option<Built>,
    layout: Option<(GraphStructure, GroupBy, Rc<TopologyLayout>)>,
    mode: TopologyMode,
    traffic: TrafficRun,
    /// `--screen topology-traffic`: switch to Traffic once the source is ready.
    wants_traffic: bool,
    /// `--screen topology-traffic-fixture`: the pods of the fixture graph. The view then shows a
    /// fixed graph and sample and reads no feed and no source.
    fixture_pods: Option<Rc<Vec<PodSummary>>>,
    /// `--screen topology-traffic-fixture-selected`: the node drawn as selected, since the fixture
    /// has no session whose drawer could select it.
    fixture_selected: Option<NodeId>,
    viewport: Viewport,
    needs_fit: bool,
    /// The first view shows only part of the graph, because the whole is too small to read; Fit
    /// clears it.
    is_first_view_partial: bool,
    /// Whether the user opened or hid the legend; `None` follows the room there is for it.
    legend_choice: Option<bool>,
    /// The first view was made for `DEFAULT_CANVAS`: it is made again once the real size is known.
    fit_waits_for_size: bool,
    /// A ghost or unchecked node that was clicked.
    highlighted: Option<NodeId>,
    /// A node to center and select after the next build that has it (Show in Topology, a check).
    pending_focus: Option<NodeId>,
    /// `--screen topology-selected`: select the first Deployment once a graph has one.
    wants_first_deployment: bool,
    /// The card under the pointer: its edges stand out, the others fade.
    hovered: Option<NodeId>,
    drag: Drag,
    is_dirty: bool,
    is_visible: bool,
    /// The size of the canvas at its last paint, which Fit and the focus need.
    canvas_size: Option<(f32, f32)>,
    wheel_carry: f32,
    focus_handle: FocusHandle,
    /// The flow animation counts from here.
    created: Instant,
    export: ExportState,
    /// The scale the last PNG was rendered at.
    export_scale: Option<f32>,
    _export: Option<Task<()>>,
    _tick: Option<Task<()>>,
    /// The frame timer of the flow animation: it exists only while an edge flows.
    flow_timer: Option<Task<()>>,
    /// Repaints when the window is activated again, which restarts a stopped flow.
    _activation: Option<Subscription>,
    _observe: Option<Subscription>,
}

impl TopologyView {
    pub(crate) fn new(shell: WeakEntity<AppShell>, cx: &mut Context<Self>) -> Self {
        Self {
            session: None,
            shell,
            namespace: None,
            last_namespaces: HashMap::new(),
            filter: TopologyFilter::initial(),
            expanded: BTreeSet::new(),
            pins: HashMap::new(),
            build: None,
            layout: None,
            mode: TopologyMode::Resources,
            traffic: TrafficRun::default(),
            wants_traffic: false,
            fixture_pods: None,
            fixture_selected: None,
            viewport: Viewport::default(),
            needs_fit: true,
            is_first_view_partial: false,
            legend_choice: None,
            fit_waits_for_size: false,
            highlighted: None,
            pending_focus: None,
            wants_first_deployment: false,
            hovered: None,
            drag: Drag::None,
            is_dirty: true,
            is_visible: false,
            canvas_size: None,
            wheel_carry: 0.,
            focus_handle: cx.focus_handle(),
            created: Instant::now(),
            export: ExportState::Idle,
            export_scale: None,
            _export: None,
            _tick: None,
            flow_timer: None,
            _activation: None,
            _observe: None,
        }
    }

    /// A new session (first start or a context switch): the graph, the layout, the opened groups,
    /// and the namespace belong to the old cluster.
    pub(crate) fn set_session(
        &mut self,
        session: Option<Entity<ClusterSession>>,
        cx: &mut Context<Self>,
    ) {
        // The same session again (a viewed set that changed around it) keeps the graph.
        if self.session.as_ref().map(Entity::entity_id) == session.as_ref().map(Entity::entity_id) {
            return;
        }
        self._observe = session
            .as_ref()
            .map(|session| cx.observe(session, |view, _, cx| view.on_session_changed(cx)));
        self.session = session;
        // A fixture keeps its fixed namespace and graph whatever session arrives.
        if self.fixture_pods.is_some() {
            cx.notify();
            return;
        }
        self.namespace = None;
        self.expanded.clear();
        self.traffic = TrafficRun::default();
        self.clear_graph();
        self.sync_subject(cx);
        cx.notify();
    }

    /// Whether Topology is the screen shown: its feeds and its tick run only then.
    pub(crate) fn set_visible(&mut self, is_visible: bool, cx: &mut Context<Self>) {
        if self.is_visible == is_visible {
            return;
        }
        self.is_visible = is_visible;
        if is_visible {
            self.is_dirty = true;
            self._tick = Some(cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor().timer(TOPOLOGY_TICK).await;
                    if this.update(cx, |view, cx| view.tick(cx)).is_err() {
                        return;
                    }
                }
            }));
        } else {
            self._tick = None;
            self.drag = Drag::None;
            self.traffic = TrafficRun::default();
        }
        self.sync_subject(cx);
        cx.notify();
    }

    /// Show in Topology: draws `namespace` and centers on the object once it is in the graph.
    pub(crate) fn show_object(&mut self, namespace: &str, id: NodeId, cx: &mut Context<Self>) {
        if self.namespace.as_deref() != Some(namespace) {
            self.change_namespace(Some(namespace.to_owned()), cx);
        }
        self.pending_focus = Some(id);
        self.is_dirty = true;
        // Already shown: `show_screen` will not start the feeds of the new namespace.
        if self.is_visible {
            self.sync_subject(cx);
        }
    }

    /// The count under the title: `ns: {ns} · {n} resources`, and how many of the namespaces of
    /// the title-bar scope this graph is when the scope has several.
    pub(crate) fn header_count(&self, cx: &App) -> Option<String> {
        let count = count_text(self.namespace.as_deref()?, self.build.as_ref());
        let note = self.live(cx).and_then(|live| scope_note(&live.scope));
        Some(match note {
            Some(note) => format!("{count} \u{b7} {note}"),
            None => count,
        })
    }

    /// Whether there is a graph to export.
    pub(crate) fn has_graph(&self) -> bool {
        matches!(&self.build, Some(Ok(graph)) if !graph.nodes.is_empty())
    }

    pub(crate) fn export_state(&self) -> &ExportState {
        &self.export
    }

    /// The `Saved to {file}` detail, with the scale a large graph was cut to.
    pub(crate) fn export_detail(&self) -> Option<String> {
        let ExportState::Saved { file_name } = &self.export else {
            return None;
        };
        Some(saved_detail(file_name, self.export_scale))
    }

    /// The hint beside Fit while the first view shows part of the graph.
    pub(crate) fn partial_view_hint(&self) -> Option<String> {
        self.is_first_view_partial.then(|| {
            format!(
                "Showing part of the graph \u{b7} {}%",
                (self.viewport.zoom() * 100.).round()
            )
        })
    }

    /// Whether the graph exists, for the screenshot hook.
    #[cfg(feature = "screenshot")]
    pub(crate) fn has_build(&self) -> bool {
        self.build.is_some()
    }

    /// `--screen topology-selected`: the first Deployment of the first graph is selected.
    pub(crate) fn select_first_deployment_once(&mut self, is_wanted: bool) {
        self.wants_first_deployment = is_wanted;
    }

    /// `--screen topology-rbac`: the RBAC chip is on.
    pub(crate) fn set_rbac(&mut self, is_on: bool, cx: &mut Context<Self>) {
        if is_on {
            self.filter.kinds.insert(KindFilter::Rbac);
        } else {
            self.filter.kinds.remove(&KindFilter::Rbac);
        }
        self.is_dirty = true;
        cx.notify();
    }

    /// `--screen topology-problems`.
    pub(crate) fn set_problems_only(&mut self, is_on: bool, cx: &mut Context<Self>) {
        self.filter.problems_only = is_on;
        self.is_dirty = true;
        cx.notify();
    }

    fn live<'a>(&self, cx: &'a App) -> Option<&'a LiveCluster> {
        self.session.as_ref()?.read(cx).live()
    }

    fn context<'a>(&self, cx: &'a App) -> &'a str {
        self.session
            .as_ref()
            .map_or("", |session| session.read(cx).context())
    }

    /// The object of the open drawer, which is what the graph highlights.
    fn selected(&self, cx: &App) -> Option<ResourceKey> {
        self.shell
            .read_with(cx, |shell, _| {
                shell.drawer_subject().map(|object| object.key.clone())
            })
            .ok()
            .flatten()
    }

    /// Runs `action` on the shell once the current update ends. The shell may be what called this
    /// view (`show_screen`, `show_in_topology`), and an entity cannot be updated inside its own
    /// update.
    fn with_shell(
        &self,
        cx: &mut Context<Self>,
        action: impl FnOnce(&mut AppShell, &mut Context<AppShell>) + 'static,
    ) {
        let shell = self.shell.clone();
        cx.defer(move |cx| {
            let _ = shell.update(cx, action);
        });
    }

    /// The session changed: rebuild at the next tick, and follow a scope change.
    fn on_session_changed(&mut self, cx: &mut Context<Self>) {
        if !self.is_visible {
            return;
        }
        self.is_dirty = true;
        self.sync_subject(cx);
    }

    /// Keeps the namespace inside the scope and the feeds on the namespace and chips: one that left
    /// the scope resets to the default, and the session starts, keeps, or drops its feeds.
    fn sync_subject(&mut self, cx: &mut Context<Self>) {
        // A fixture shows fixed data: it starts no feed.
        if self.fixture_pods.is_some() {
            return;
        }
        let Some(session) = self.session.clone() else {
            return;
        };
        if let Some(live) = session.read(cx).live() {
            // Scope `All` has no default namespace: open in one instead of an empty picker.
            let resolved =
                resolve_namespace(self.namespace.as_deref(), &live.scope).or_else(|| {
                    let last = self.last_namespaces.get(self.context(cx));
                    preselected_namespace(
                        last.map(String::as_str),
                        live.namespaces.ready_items(),
                        live.pods.ready_items().unwrap_or_default(),
                    )
                });
            if resolved != self.namespace {
                self.change_namespace(resolved, cx);
            }
        }
        let subject = self
            .namespace
            .clone()
            .filter(|_| self.is_visible)
            .map(|namespace| TopologySubject {
                namespace,
                kinds: self.shown_kinds(),
            });
        if session.read(cx).topology_subject() != subject.as_ref() {
            session.update(cx, |session, cx| session.set_topology_subject(subject, cx));
        }
    }

    /// Another namespace: nothing of the old one stays (decision 1), and the drawer closes if it
    /// shows an object of it.
    fn change_namespace(&mut self, namespace: Option<String>, cx: &mut Context<Self>) {
        if let Some(namespace) = &namespace {
            let context = self.context(cx).to_owned();
            self.last_namespaces.insert(context, namespace.clone());
        }
        self.namespace = namespace;
        self.expanded.clear();
        self.pending_focus = None;
        self.hovered = None;
        self.traffic = TrafficRun::default();
        self.clear_graph();
        self.clear_selection(cx);
        cx.notify();
    }

    fn clear_graph(&mut self) {
        self.build = None;
        self.layout = None;
        self.highlighted = None;
        self.needs_fit = true;
        self.is_dirty = true;
    }

    fn tick(&mut self, cx: &mut Context<Self>) {
        if self.is_dirty {
            self.rebuild(cx);
        }
        self.sync_traffic(cx);
    }

    /// The pins of the namespace drawn, in this context.
    fn pins<'a>(&'a self, cx: &'a App) -> Option<&'a HashMap<NodeId, GraphPoint>> {
        let namespace = self.namespace.as_deref()?;
        self.pins.get(self.context(cx))?.get(namespace)
    }

    fn has_pins(&self, cx: &App) -> bool {
        self.pins(cx).is_some_and(|pins| !pins.is_empty())
    }

    /// The width over the height of the canvas without the strip the overlays cover, which is the
    /// shape a fresh layout aims for.
    fn aspect(&self) -> f32 {
        let (width, height) = self.view_area();
        width / height
    }

    /// The canvas size Fit and the first view use: the canvas above the overlay strip, or the
    /// default size before the first paint.
    fn view_area(&self) -> (f32, f32) {
        let (width, height) = self.canvas_size.unwrap_or(DEFAULT_CANVAS);
        (width, (height - OVERLAY_GUTTER).max(height / 2.))
    }

    /// Builds the graph from the live lists and lays it out. It waits while a feed that runs has
    /// not delivered its first snapshot, so the graph does not grow node by node. It repaints only
    /// when the graph or the view changed.
    fn rebuild(&mut self, cx: &mut Context<Self>) {
        if self.fixture_pods.is_some() {
            self.is_dirty = false;
            return;
        }
        let Some(namespace) = self.namespace.clone() else {
            return;
        };
        let Some(session) = self.session.clone() else {
            return;
        };
        let aspect = self.aspect();
        let selected = self.selected(cx);
        let filter = TopologyFilter {
            kinds: self.shown_kinds(),
            problems_only: self.filter.problems_only,
            group_by: self.filter.group_by,
        };
        let (build, group_by, selection_is_gone) = {
            let session = session.read(cx);
            let Some(live) = session.live() else {
                return;
            };
            let Some(feeds) = live.topology() else {
                return;
            };
            let Some(pods) = live.pods.ready_items() else {
                return;
            };
            if feeds.subject.namespace != namespace || feeds.has_pending() {
                return;
            }
            let rows = feeds.feed_rows();
            let inputs = TopologyInputs {
                namespace: &namespace,
                pods: Some(pods),
                rows: &rows,
                nodes: live.nodes.items(),
                filter: &filter,
                expanded: &self.expanded,
                now: jiff::Timestamp::now(),
            };
            let group_by = resolve_group_by(self.filter.group_by, &namespace, pods);
            let is_gone = selected
                .as_ref()
                .is_some_and(|key| is_selection_gone(key, &namespace, pods, &rows));
            (build_topology(&inputs), group_by, is_gone)
        };
        self.is_dirty = false;
        let mut is_changed = match build {
            TopologyBuild::TooLarge(too_large) => {
                let is_same = matches!(&self.build, Some(Err(known)) if *known == too_large);
                self.build = Some(Err(too_large));
                self.layout = None;
                !is_same
            }
            TopologyBuild::Graph(graph) => self.install(graph, group_by, aspect, cx),
        };
        // The object of an open drawer is gone from a feed that has loaded.
        if selection_is_gone {
            self.clear_selection(cx);
        }
        if self.wants_first_deployment
            && let Some(Ok(graph)) = &self.build
            && let Some(id) = first_deployment(graph)
        {
            self.wants_first_deployment = false;
            self.pending_focus = Some(id);
        }
        is_changed |= self.apply_pending_focus(cx);
        if is_changed {
            cx.notify();
        }
    }

    /// Keeps the layout when only tones and captions changed; lays out from the previous one when
    /// the structure changed; from scratch (with the first view) for a first graph or a new
    /// grouping. Returns whether anything changed.
    fn install(&mut self, graph: TopologyGraph, group_by: GroupBy, aspect: f32, cx: &App) -> bool {
        let kept = self
            .layout
            .take()
            .filter(|(_, previous_group, _)| *previous_group == group_by);
        let is_same_graph = matches!(&self.build, Some(Ok(known)) if **known == graph);
        if is_same_graph && kept.is_some() {
            self.layout = kept;
            return false;
        }
        let is_fresh = kept.is_none();
        let shape = structure(&graph);
        let no_pins = HashMap::new();
        let pins = self.pins(cx).unwrap_or(&no_pins);
        let layout = match kept {
            Some((previous, _, layout)) if previous == shape => layout,
            Some((_, _, previous)) => {
                Rc::new(lay_out(&graph, group_by, aspect, pins, Some(&previous)))
            }
            None => Rc::new(lay_out(&graph, group_by, aspect, pins, None)),
        };
        self.needs_fit |= is_fresh;
        self.layout = Some((shape, group_by, layout));
        // A node that left the graph cannot be under the pointer.
        if self
            .hovered
            .as_ref()
            .is_some_and(|id| !graph.nodes.iter().any(|node| node.id == *id))
        {
            self.hovered = None;
        }
        self.build = Some(Ok(Rc::new(graph)));
        self.rebuild_traffic_layer(cx);
        self.apply_fit();
        true
    }

    /// Opens the graph at its first view: the whole graph when it is readable at that zoom, else
    /// the readable zoom at its top-left. Before the first paint the size is a default, and the
    /// view is made again once the real size is known.
    fn apply_fit(&mut self) {
        if !self.needs_fit {
            return;
        }
        let Some((_, _, layout)) = &self.layout else {
            return;
        };
        let (width, height) = self.view_area();
        let width = width - CONTROLS_INSET;
        // The graph starts to the right of the zoom panel.
        self.viewport = Viewport::first_view(layout.extent, width, height).pan(CONTROLS_INSET, 0.);
        self.is_first_view_partial = !self.viewport.shows_whole(layout.extent, width, height);
        self.needs_fit = false;
        self.fit_waits_for_size = self.canvas_size.is_none();
    }

    /// Centers and selects the node of `pending_focus` once a graph has it. Returns whether it did.
    fn apply_pending_focus(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(id) = self.pending_focus.clone() else {
            return false;
        };
        let (Some(Ok(graph)), Some((_, _, layout))) = (&self.build, &self.layout) else {
            return false;
        };
        let Some(index) = graph.nodes.iter().position(|node| node.id == id) else {
            return false;
        };
        let (width, height) = self.canvas_size.unwrap_or(DEFAULT_CANVAS);
        let center = layout.rects[index].center();
        let key = graph.nodes[index].key.clone();
        // A node with an object opens the drawer, which covers the right of the canvas.
        let free_width = match &key {
            Some(key) => width - f32::from(drawer_width(DrawerSize::of(key), px(width))),
            None => width,
        };
        self.viewport = self
            .viewport
            .center_on(center, free_width.max(width / 2.), height);
        self.pending_focus = None;
        self.highlighted = key.is_none().then(|| id.clone());
        let has_object = key.is_some();
        self.with_shell(cx, |shell, cx| shell.select_on_topology(key, cx));
        // Centering puts the node in the middle; its neighbours may still be under the drawer.
        if has_object {
            self.reveal_node(&id);
        }
        true
    }

    /// Lays out again from the previous layout, with the pins as they are now. The graph is the
    /// same, so its shape is kept.
    fn relayout(&mut self, cx: &App) {
        let (Some(Ok(graph)), Some((shape, group_by, previous))) =
            (&self.build, self.layout.take())
        else {
            return;
        };
        let no_pins = HashMap::new();
        let pins = self.pins(cx).unwrap_or(&no_pins);
        let layout = Rc::new(lay_out(
            graph,
            group_by,
            self.aspect(),
            pins,
            Some(&previous),
        ));
        self.layout = Some((shape, group_by, layout));
        self.reroute_traffic_calls();
    }

    /// The first layout aimed at `DEFAULT_CANVAS`: lays out from scratch for the shape the canvas
    /// really has, so a wide window gets more band-columns.
    fn lay_out_for_canvas(&mut self, cx: &App) {
        let (Some(Ok(graph)), Some((shape, group_by, _))) = (&self.build, self.layout.take())
        else {
            return;
        };
        let no_pins = HashMap::new();
        let pins = self.pins(cx).unwrap_or(&no_pins);
        let layout = Rc::new(lay_out(graph, group_by, self.aspect(), pins, None));
        self.layout = Some((shape, group_by, layout));
        self.rebuild_traffic_layer(cx);
    }

    // ---- traffic (spec 0049) ----

    /// `--screen topology-traffic`: Traffic opens as soon as the source is ready.
    pub(crate) fn start_in_traffic(&mut self, is_wanted: bool) {
        self.wants_traffic = is_wanted;
    }

    /// The segment: Resources or Traffic. Any change drops the fetch and the sample.
    pub(crate) fn set_mode(&mut self, mode: TopologyMode, cx: &mut Context<Self>) {
        if self.mode == mode {
            return;
        }
        self.mode = mode;
        self.traffic = TrafficRun::default();
        // The layers Traffic turns off leave the graph and stop their feeds, and come back with
        // the chips the user had.
        if mode == TopologyMode::Traffic
            && self.selected(cx).is_some_and(|key| is_traffic_hidden(&key))
        {
            self.clear_selection(cx);
        }
        self.is_dirty = true;
        self.sync_subject(cx);
        self.sync_traffic(cx);
        cx.notify();
    }

    /// The pods of the namespace drawn, which the traffic names are resolved against.
    fn namespace_pods<'a>(&self, cx: &'a App) -> Vec<&'a PodSummary> {
        let Some(namespace) = self.namespace.as_deref() else {
            return Vec::new();
        };
        self.live(cx)
            .and_then(|live| live.pods.ready_items())
            .unwrap_or_default()
            .iter()
            .filter(|pod| pod.namespace == namespace)
            .collect()
    }

    /// Keeps the Traffic fetches in step with the mode and the source: it runs on every tick and
    /// on a mode switch. It leaves Traffic when the source stops serving, loads the metric list
    /// once, and reads a sample every `TRAFFIC_REFRESH`.
    fn sync_traffic(&mut self, cx: &mut Context<Self>) {
        if self.fixture_pods.is_some() {
            return;
        }
        let Some(session) = self.session.clone() else {
            return;
        };
        if self.wants_traffic && self.mode == TopologyMode::Resources {
            let is_ready = session
                .read(cx)
                .live()
                .is_some_and(|live| traffic_button(&live.metrics).is_enabled);
            if is_ready {
                self.wants_traffic = false;
                self.set_mode(TopologyMode::Traffic, cx);
            }
            return;
        }
        let Some(namespace) = self.namespace.clone() else {
            return;
        };
        if self.mode != TopologyMode::Traffic || !self.is_visible {
            return;
        }
        let step = {
            let Some(live) = session.read(cx).live() else {
                return;
            };
            traffic_step(&self.traffic, &live.metrics)
        };
        match step {
            TrafficStep::Leave => self.set_mode(TopologyMode::Resources, cx),
            TrafficStep::LoadNames => {
                self.traffic.last_names_try = Some(Instant::now());
                session.update(cx, |session, cx| session.load_traffic_sources(cx));
            }
            TrafficStep::Wait => {}
            TrafficStep::Fetch => {
                let plan = {
                    let Some(live) = session.read(cx).live() else {
                        return;
                    };
                    fetch_plan(live)
                };
                if let Some(TrafficFetchPlan {
                    sources,
                    source,
                    connection,
                }) = plan
                {
                    self.start_traffic_fetch(sources, source, connection, namespace, cx);
                }
            }
        }
    }

    fn start_traffic_fetch(
        &mut self,
        sources: Vec<TrafficMetricSource>,
        source: MetricsSource,
        connection: ClusterConnection,
        namespace: String,
        cx: &mut Context<Self>,
    ) {
        let runtime = cx.global::<ClusterRuntime>().clone();
        let fetching = runtime.spawn(async move {
            let mut readings = Vec::with_capacity(sources.len());
            for traffic in sources {
                let reading = connection.traffic_rates(&source, traffic, &namespace).await;
                readings.push((traffic, reading));
            }
            readings
        });
        self.traffic.last_fetch = Some(Instant::now());
        self.traffic.fetch = Some(cx.spawn(async move |this, cx| {
            let result = fetching.await;
            let _ = this.update(cx, |view, cx| view.finish_traffic_fetch(result, cx));
        }));
    }

    fn finish_traffic_fetch(
        &mut self,
        result: Result<TrafficReadings, tokio::task::JoinError>,
        cx: &mut Context<Self>,
    ) {
        self.traffic.fetch = None;
        let readings = result.unwrap_or_default();
        self.apply_traffic_sample(
            TrafficSample {
                at: jiff::Timestamp::now(),
                readings,
            },
            cx,
        );
    }

    /// A refresh landed. A sample in which every source failed keeps the older one drawn and says
    /// why in the chip; any other replaces it.
    fn apply_traffic_sample(&mut self, sample: TrafficSample, cx: &mut Context<Self>) {
        match sample_failure(&sample) {
            Some(reason) => self.traffic.paused = Some(reason),
            None => {
                self.traffic.paused = None;
                self.traffic.sample = Some(Rc::new(sample));
                self.rebuild_traffic_layer(cx);
            }
        }
        cx.notify();
    }

    /// The layer from the sample, the graph, and the layout as they are now. The layout is not
    /// computed again: the `Calls` edges are routed over its cards.
    fn rebuild_traffic_layer(&mut self, cx: &App) {
        self.traffic.layer = None;
        if self.mode != TopologyMode::Traffic {
            return;
        }
        let (Some(sample), Some(Ok(graph)), Some((_, _, layout))) =
            (&self.traffic.sample, &self.build, &self.layout)
        else {
            return;
        };
        // A fixture resolves the names against its own pods.
        let fixture_pods = self.fixture_pods.clone();
        let pods: Vec<&PodSummary> = match &fixture_pods {
            Some(fixture) => fixture.iter().collect(),
            None => self.namespace_pods(cx),
        };
        self.traffic.layer = Some(Rc::new(TrafficLayer::build(
            graph,
            layout,
            &pods,
            Rc::clone(sample),
        )));
    }

    /// The cards moved: only the `Calls` routes are computed again.
    fn reroute_traffic_calls(&mut self) {
        let (Some(layer), Some((_, _, layout))) = (&self.traffic.layer, &self.layout) else {
            return;
        };
        self.traffic.layer = Some(Rc::new(layer.rerouted(layout)));
    }

    /// `--screen topology-traffic-fixture`: the fixed namespace of W11 with its Istio and pod
    /// network readings, in Traffic mode. Nothing is read from the cluster.
    #[cfg(feature = "screenshot")]
    pub(crate) fn show_traffic_fixture(&mut self, is_selected: bool, cx: &mut Context<Self>) {
        use crate::topology_traffic_fixture::{
            NAMESPACE, traffic_fixture_graph, traffic_fixture_pods, traffic_fixture_sample,
        };
        self.fixture_pods = Some(Rc::new(traffic_fixture_pods()));
        self.namespace = Some(NAMESPACE.to_owned());
        self.mode = TopologyMode::Traffic;
        self.traffic.sample = Some(Rc::new(traffic_fixture_sample()));
        let aspect = self.aspect();
        let graph = traffic_fixture_graph();
        self.fixture_selected = is_selected.then(|| first_deployment(&graph)).flatten();
        self.install(graph, GroupBy::Components, aspect, cx);
        cx.notify();
    }

    /// Test seam: pretend a Traffic request is in flight.
    #[cfg(test)]
    pub(crate) fn hold_traffic_fetch_for_test(&mut self) {
        self.traffic.fetch = Some(Task::ready(()));
    }

    #[cfg(test)]
    pub(crate) fn has_traffic_fetch_for_test(&self) -> bool {
        self.traffic.fetch.is_some()
    }

    /// Whether a screenshot of a Traffic screen still waits for its first sample.
    #[cfg(feature = "screenshot")]
    pub(crate) fn is_traffic_pending(&self) -> bool {
        self.wants_traffic
            || (self.mode == TopologyMode::Traffic
                && self.traffic.layer.is_none()
                && self.traffic.paused.is_none())
    }

    // ---- toolbar actions ----

    /// Fit: the whole graph in view, even where the cards are too small to read.
    pub(crate) fn fit(&mut self, cx: &mut Context<Self>) {
        if let Some((_, _, layout)) = &self.layout {
            let (width, height) = self.view_area();
            self.viewport = Viewport::fit(layout.extent, width - CONTROLS_INSET, height)
                .pan(CONTROLS_INSET, 0.);
        }
        self.is_first_view_partial = false;
        self.needs_fit = false;
        cx.notify();
    }

    fn set_namespace(&mut self, namespace: String, cx: &mut Context<Self>) {
        if self.namespace.as_deref() == Some(namespace.as_str()) {
            return;
        }
        self.change_namespace(Some(namespace), cx);
        self.sync_subject(cx);
    }

    fn set_group_by(&mut self, choice: GroupBy, cx: &mut Context<Self>) {
        if self.filter.group_by == Some(choice) {
            return;
        }
        self.filter.group_by = Some(choice);
        // A new grouping lays out from scratch.
        self.layout = None;
        self.needs_fit = true;
        self.rebuild(cx);
    }

    /// The kinds the graph and the feeds use now.
    fn shown_kinds(&self) -> BTreeSet<KindFilter> {
        self.filter.shown_kinds(self.mode == TopologyMode::Traffic)
    }

    fn toggle_kind(&mut self, kind: KindFilter, cx: &mut Context<Self>) {
        if self.mode == TopologyMode::Traffic && kind.is_off_in_traffic() {
            return;
        }
        if !self.filter.kinds.remove(&kind) {
            self.filter.kinds.insert(kind);
        }
        self.sync_subject(cx);
        self.rebuild(cx);
        cx.notify();
    }

    fn toggle_problems_only(&mut self, cx: &mut Context<Self>) {
        self.filter.problems_only = !self.filter.problems_only;
        self.needs_fit = true;
        self.layout = None;
        self.rebuild(cx);
    }

    fn reset_positions(&mut self, cx: &mut Context<Self>) {
        if let Some(namespace) = self.namespace.clone() {
            let context = self.context(cx).to_owned();
            if let Some(namespaces) = self.pins.get_mut(&context) {
                namespaces.remove(&namespace);
            }
        }
        self.layout = None;
        self.needs_fit = true;
        self.rebuild(cx);
    }

    /// A pick of the checks dropdown: center and select the node.
    fn focus_node(&mut self, id: NodeId, cx: &mut Context<Self>) {
        self.pending_focus = Some(id);
        self.apply_pending_focus(cx);
        cx.notify();
    }

    // ---- pointer ----

    pub(crate) fn set_canvas_size(&mut self, width: f32, height: f32, cx: &mut Context<Self>) {
        if self.canvas_size == Some((width, height)) {
            return;
        }
        self.canvas_size = Some((width, height));
        // The first layout and view were made for a default size.
        if self.fit_waits_for_size {
            self.lay_out_for_canvas(cx);
            self.needs_fit = true;
        }
        self.apply_fit();
        self.reveal_selected(cx);
        cx.notify();
    }

    /// The first view is made again once the canvas size is known, which may hide the selected
    /// node behind the drawer: bring it into the free part.
    fn reveal_selected(&mut self, cx: &App) {
        let Some(key) = self.selected(cx) else {
            return;
        };
        let Some(Ok(graph)) = &self.build else {
            return;
        };
        let Some(id) = graph
            .nodes
            .iter()
            .find(|node| node.key.as_ref() == Some(&key))
            .map(|node| node.id.clone())
        else {
            return;
        };
        self.reveal_node(&id);
    }

    pub(crate) fn zoom_by_wheel(
        &mut self,
        delta: ScrollDelta,
        x: f32,
        y: f32,
        cx: &mut Context<Self>,
    ) {
        let steps = wheel_steps(delta, &mut self.wheel_carry);
        if steps == 0 {
            return;
        }
        self.viewport = self.viewport.zoom_at(x, y, steps);
        cx.notify();
    }

    /// The + and - buttons: zoom around the center of the canvas.
    fn zoom_by_button(&mut self, steps: i32, cx: &mut Context<Self>) {
        let (width, height) = self.canvas_size.unwrap_or(DEFAULT_CANVAS);
        self.viewport = self.viewport.zoom_at(width / 2., height / 2., steps);
        cx.notify();
    }

    /// The pointer entered or left the card of `id`. It repaints only when the hovered card
    /// changes. During a drag the cards move under the pointer, so the change is recorded but not
    /// painted: `finish_drag` repaints once.
    fn set_hover(&mut self, id: NodeId, is_hovered: bool, cx: &mut Context<Self>) {
        if is_hovered {
            if self.hovered.as_ref() == Some(&id) {
                return;
            }
            self.hovered = Some(id);
        } else if self.hovered.as_ref() == Some(&id) {
            // A leave that follows the enter of the next card finds a different one: ignored.
            self.hovered = None;
        } else {
            return;
        }
        if matches!(self.drag, Drag::None) {
            cx.notify();
        }
    }

    fn clear_hover(&mut self, cx: &mut Context<Self>) {
        if self.hovered.take().is_some() {
            cx.notify();
        }
    }

    fn press_canvas(
        &mut self,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus_handle, cx);
        self.drag = Drag::Pan {
            start: position,
            last: position,
            moved: false,
        };
        cx.notify();
    }

    fn press_node(
        &mut self,
        index: usize,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus_handle, cx);
        let (Some(Ok(graph)), Some((_, _, layout))) = (&self.build, &self.layout) else {
            return;
        };
        let (Some(node), Some(rect)) = (graph.nodes.get(index), layout.rects.get(index)) else {
            return;
        };
        self.drag = Drag::Node {
            id: node.id.clone(),
            start: position,
            origin: rect.origin,
            moved: false,
        };
        cx.notify();
    }

    pub(crate) fn drag_to(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        match &mut self.drag {
            Drag::None | Drag::Minimap => {}
            Drag::Pan { start, last, moved } => {
                let from_start = position - *start;
                if !*moved && !is_drag(f32::from(from_start.x), f32::from(from_start.y)) {
                    return;
                }
                *moved = true;
                let step = position - *last;
                *last = position;
                self.viewport = self.viewport.pan(f32::from(step.x), f32::from(step.y));
                cx.notify();
            }
            Drag::Node {
                id,
                start,
                origin,
                moved,
            } => {
                let from_start = position - *start;
                let (dx, dy) = (f32::from(from_start.x), f32::from(from_start.y));
                if !*moved && !is_drag(dx, dy) {
                    return;
                }
                *moved = true;
                let zoom = self.viewport.zoom();
                let pinned = GraphPoint {
                    x: origin.x + dx / zoom,
                    y: origin.y + dy / zoom,
                };
                let id = id.clone();
                if let Some(namespace) = self.namespace.clone() {
                    let context = self.context(cx).to_owned();
                    self.pins
                        .entry(context)
                        .or_default()
                        .entry(namespace)
                        .or_default()
                        .insert(id, pinned);
                }
                self.relayout(cx);
                cx.notify();
            }
        }
    }

    pub(crate) fn finish_drag(
        &mut self,
        _position: Point<Pixels>,
        click_count: usize,
        cx: &mut Context<Self>,
    ) {
        match std::mem::replace(&mut self.drag, Drag::None) {
            Drag::None | Drag::Minimap => {}
            Drag::Pan { moved, .. } => {
                if !moved {
                    self.highlighted = None;
                    self.with_shell(cx, |shell, cx| shell.select_on_topology(None, cx));
                }
            }
            Drag::Node { id, moved, .. } => {
                if !moved {
                    self.click_node(&id, click_count, cx);
                }
            }
        }
        cx.notify();
    }

    /// A click on a card: a pod group opens, an object selects (a double click reveals it), and a
    /// ghost or unchecked node is only highlighted.
    fn click_node(&mut self, id: &NodeId, click_count: usize, cx: &mut Context<Self>) {
        let Some(Ok(graph)) = &self.build else {
            return;
        };
        let Some(node) = graph.nodes.iter().find(|node| node.id == *id) else {
            return;
        };
        let key = node.key.clone();
        if matches!(node.id, NodeId::PodGroup { .. }) {
            self.expanded.insert(id.clone());
            self.rebuild(cx);
            return;
        }
        let has_row = key.as_ref().is_none_or(|key| self.has_row(key, cx));
        match card_click(key, has_row, click_count) {
            CardClick::Reveal(key) => {
                self.with_shell(cx, |shell, cx| shell.reveal(key, cx));
            }
            CardClick::Select(key) => {
                self.highlighted = None;
                self.reveal_node(id);
                self.with_shell(cx, |shell, cx| shell.select_on_topology(Some(key), cx));
            }
            CardClick::Highlight => self.highlighted = Some(id.clone()),
        }
    }

    /// Whether the drawer could show the object of `key`: a kind row lives in the explorer or a
    /// feed. A pod is always in the session.
    fn has_row(&self, key: &ResourceKey, cx: &App) -> bool {
        !matches!(key, ResourceKey::Kind { .. })
            || self.live(cx).is_some_and(|live| live.row_of(key).is_some())
    }

    /// Brings the node and its direct neighbours into the part of the canvas the drawer leaves
    /// free, if they are not in it: the drawer would hide what the node connects to.
    fn reveal_node(&mut self, id: &NodeId) {
        let (Some(Ok(graph)), Some((_, _, layout))) = (&self.build, &self.layout) else {
            return;
        };
        let Some(index) = graph.nodes.iter().position(|node| node.id == *id) else {
            return;
        };
        let (width, height) = self.view_area();
        let size = graph.nodes[index]
            .key
            .as_ref()
            .map_or(DrawerSize::Standard, DrawerSize::of);
        let free_width = (width - f32::from(drawer_width(size, px(width)))).max(width / 2.);
        self.viewport = self.viewport.reveal_group(
            layout.rects[index],
            &neighbour_rects(graph, layout, index),
            free_width,
            height,
            REVEAL_MARGIN,
        );
    }

    /// Starts or stops the frame timer of the flow animation, as the canvas paints: the timer
    /// runs only while an edge flows.
    pub(crate) fn sync_flow(&mut self, wants_frames: bool, cx: &mut Context<Self>) {
        if !wants_frames {
            self.flow_timer = None;
            return;
        }
        if self.flow_timer.is_some() {
            return;
        }
        self.flow_timer = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(FLOW_FRAME).await;
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    return;
                }
            }
        }));
    }

    pub(crate) fn minimap_press(&mut self, at: GraphPoint, cx: &mut Context<Self>) {
        self.drag = Drag::Minimap;
        self.minimap_drag(at, cx);
    }

    pub(crate) fn minimap_drag(&mut self, at: GraphPoint, cx: &mut Context<Self>) {
        let (width, height) = self.canvas_size.unwrap_or(DEFAULT_CANVAS);
        self.viewport = self.viewport.center_on(at, width, height);
        cx.notify();
    }

    /// Esc: the drawer closes and the row cursor stays.
    fn close_drawer(&mut self, cx: &mut Context<Self>) {
        self.with_shell(cx, |shell, cx| shell.close_drawer(cx));
    }

    /// The object is gone or the namespace changed: the cursor goes with the drawer.
    fn clear_selection(&mut self, cx: &mut Context<Self>) {
        self.with_shell(cx, |shell, cx| shell.clear_selection(cx));
    }

    // ---- export (step 3) ----

    /// `Export PNG`: opens the save dialog; the file is written only once a path is confirmed.
    pub(crate) fn export(&mut self, cx: &mut Context<Self>) {
        if self.export.is_busy() || !self.has_graph() {
            return;
        }
        let Some(namespace) = self.namespace.clone() else {
            return;
        };
        let context = self.context(cx);
        let name = export_file_name(
            &format!("topology-{context}-{namespace}"),
            "png",
            jiff::Timestamp::now(),
        );
        self.export = ExportState::Choosing;
        self._export = Some(start_export_with(
            name,
            "topology",
            |view: &mut Self, cx| view.export_snapshot(cx),
            write_export,
            Self::set_export_state,
            |view, scale| view.export_scale = Some(scale),
            cx,
        ));
        cx.notify();
    }

    fn set_export_state(&mut self, state: ExportState, cx: &mut Context<Self>) {
        self.export = state;
        cx.notify();
    }

    /// The SVG of the graph as arranged now, and the scale a PNG of it gets. Only a string is
    /// built here; the rendering runs on the background executor.
    fn export_snapshot(&mut self, cx: &mut Context<Self>) -> Result<(TopologyExport, f32), String> {
        let (Some(Ok(graph)), Some((_, _, layout))) = (&self.build, &self.layout) else {
            return Err("Could not save the topology: there is no graph".to_owned());
        };
        let title = format!(
            "Topology \u{b7} {} \u{b7} ns: {} \u{b7} {} resources \u{b7} {}",
            self.context(cx),
            self.namespace.as_deref().unwrap_or_default(),
            graph.resources,
            jiff::Timestamp::now()
        );
        let svg = topology_svg(
            graph,
            layout,
            self.traffic.layer.as_deref(),
            &title,
            &svg_style(cx),
        );
        let scale = export_scale(
            layout.extent.width,
            layout.extent.height + crate::topology_export::TITLE_STRIP,
        );
        Ok((TopologyExport { svg }, scale))
    }

    // ---- rendering ----

    fn render_toolbar(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(live) = self.live(cx) else {
            return div().into_any_element();
        };
        let namespaces = namespace_choices(&live.scope, live.namespaces.ready_items());
        let view = cx.weak_entity();
        // Without a namespace the picker is the one thing to do: it is the primary button.
        let namespace_label = match &self.namespace {
            Some(namespace) => format!("Namespace: {namespace}"),
            None => "Pick a namespace".to_owned(),
        };
        let namespace_tooltip = match scope_note(&live.scope) {
            Some(note) => format!("The namespace to draw \u{b7} {note}"),
            None => "The namespace to draw".to_owned(),
        };
        let has_namespace = self.namespace.is_some();
        let current = self.namespace.clone();
        let namespace_button = Button::new("topology-namespace")
            .map(|button| {
                if has_namespace {
                    button.outline()
                } else {
                    button.primary()
                }
            })
            .small()
            .label(namespace_label)
            .dropdown_caret(true)
            .tooltip(namespace_tooltip)
            .dropdown_menu({
                let view = view.clone();
                move |menu, _, _| {
                    let items = namespaces.iter().map(|namespace| {
                        let view = view.clone();
                        let picked = namespace.clone();
                        PopupMenuItem::new(namespace.clone())
                            .checked(current.as_deref() == Some(namespace.as_str()))
                            .on_click(move |_, _, cx| {
                                let picked = picked.clone();
                                let _ = view.update(cx, |view, cx| view.set_namespace(picked, cx));
                            })
                    });
                    items
                        .fold(menu, |menu, item| menu.item(item))
                        .scrollable(true)
                        .max_h(px(NAMESPACE_MENU_HEIGHT))
                }
            });
        let resolved = self.resolved_group_by(live);
        let group_button = Button::new("topology-group-by")
            .outline()
            .small()
            .label(format!("Group by: {}", resolved.label()))
            .dropdown_caret(true)
            .tooltip("How the graph is split into bands")
            .dropdown_menu({
                let view = view.clone();
                move |menu, _, _| {
                    [GroupBy::App, GroupBy::Components]
                        .into_iter()
                        .fold(menu, |menu, option| {
                            let view = view.clone();
                            menu.item(
                                PopupMenuItem::new(option.label())
                                    .checked(option == resolved)
                                    .on_click(move |_, _, cx| {
                                        let _ = view
                                            .update(cx, |view, cx| view.set_group_by(option, cx));
                                    }),
                            )
                        })
                }
            });
        let chips = KindFilter::ALL.into_iter().map(|kind| {
            let is_off = self.mode == TopologyMode::Traffic && kind.is_off_in_traffic();
            let is_on = self.filter.kinds.contains(&kind) && !is_off;
            let tooltip = if is_off {
                NOT_IN_TRAFFIC.to_owned()
            } else {
                kind.tooltip()
            };
            toggle_button(chip_id(kind), kind.label(), is_on)
                .disabled(is_off)
                .tooltip(tooltip)
                .on_click(cx.listener(move |view, _, _, cx| view.toggle_kind(kind, cx)))
        });
        // Amber while on, so it does not read like a kind chip: the graph shows the problems and what
        // touches them, not only the problems.
        let warn = tone_color(StatusTone::Warn, cx);
        let problems = Button::new("topology-problems")
            .label("Problems only")
            .small()
            .outline()
            .map(|button| {
                if self.filter.problems_only {
                    button.text_color(warn).border_color(warn).selected(true)
                } else {
                    button
                }
            })
            .tooltip("Keep the problems and the objects next to them")
            .on_click(cx.listener(|view, _, _, cx| view.toggle_problems_only(cx)));
        let reset = self.has_pins(cx).then(|| {
            Button::new("topology-reset")
                .ghost()
                .small()
                .label("Reset positions")
                .tooltip("Lay the graph out again")
                .on_click(cx.listener(|view, _, _, cx| view.reset_positions(cx)))
        });
        let traffic_button = traffic_button(&live.metrics);
        let is_traffic = self.mode == TopologyMode::Traffic;
        let segment = h_flex()
            .gap_1()
            .child(
                Button::new("topology-resources")
                    .small()
                    .map(|button| {
                        if is_traffic {
                            button.outline()
                        } else {
                            button.primary()
                        }
                    })
                    .label("Resources")
                    .tooltip("The objects of the namespace and how they relate")
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.set_mode(TopologyMode::Resources, cx);
                    })),
            )
            .child(
                Button::new("topology-traffic")
                    .small()
                    .map(|button| {
                        if is_traffic {
                            button.primary()
                        } else {
                            button.outline()
                        }
                    })
                    .label("Traffic")
                    .disabled(!traffic_button.is_enabled)
                    .tooltip(traffic_button.tooltip)
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.set_mode(TopologyMode::Traffic, cx);
                    })),
            );
        h_flex()
            .flex_shrink_0()
            .flex_wrap()
            .items_center()
            .gap_2()
            .px_4()
            .py_2()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(segment)
            .child(namespace_button)
            .child(group_button)
            .child(h_flex().gap_1().children(chips))
            .child(problems)
            .child(
                h_flex()
                    .ml_auto()
                    .gap_2()
                    .children(reset)
                    .children(self.render_checks_chip(cx))
                    .children(self.render_traffic_chip(cx)),
            )
            .into_any_element()
    }

    /// The chip of Traffic mode: the sources, the window, the time of the sample, or why the
    /// refresh paused. Its tooltip lists the notes and the peers outside the namespace.
    fn render_traffic_chip(&self, cx: &App) -> Option<AnyElement> {
        if self.mode != TopologyMode::Traffic {
            return None;
        }
        let layer = self.traffic.layer.as_ref();
        let clock = layer.map(|layer| clock_text(layer.sample.at, &jiff::tz::TimeZone::system()));
        let shown = layer
            .zip(clock.as_deref())
            .map(|(layer, clock)| (layer.overlay.sources.as_slice(), clock));
        let text = traffic_chip_text(shown, self.traffic.paused.as_deref());
        let tooltip = layer.and_then(|layer| traffic_chip_tooltip(&layer.overlay));
        let theme = cx.theme();
        let chip = div()
            .id("topology-traffic-chip")
            .px_2()
            .py_1()
            .rounded(px(6.))
            .border_1()
            .border_color(theme.border)
            .text_xs()
            .text_color(theme.muted_foreground)
            .child(text);
        Some(match tooltip {
            Some(tooltip) => chip
                .tooltip(move |window, cx| {
                    gpui_kit::component::tooltip::Tooltip::new(tooltip.clone()).build(window, cx)
                })
                .into_any_element(),
            None => chip.into_any_element(),
        })
    }

    /// The grouping the graph was laid out with; before a layout, what it would resolve to.
    fn resolved_group_by(&self, live: &LiveCluster) -> GroupBy {
        match &self.layout {
            Some((_, group_by, _)) => *group_by,
            None => resolve_group_by(
                self.filter.group_by,
                self.namespace.as_deref().unwrap_or_default(),
                live.pods.items(),
            ),
        }
    }

    /// The checks chip: its text and tone, and a dropdown that lists every check.
    fn render_checks_chip(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let Some(Ok(graph)) = &self.build else {
            return None;
        };
        let (text, tone) = checks_chip(&graph.checks)?;
        let graph = Rc::clone(graph);
        let view = cx.weak_entity();
        let color = tone_color(tone, cx);
        Some(
            Button::new("topology-checks")
                .outline()
                .small()
                .text_color(color)
                .label(text)
                .dropdown_caret(true)
                .tooltip("Config problems found in this namespace")
                .dropdown_menu(move |menu, _, cx| {
                    let checks = &graph.checks;
                    let hidden = checks.len().saturating_sub(CHECKS_MENU_LIMIT);
                    let menu = checks
                        .iter()
                        .take(CHECKS_MENU_LIMIT)
                        .fold(menu, |menu, check| menu.item(check_item(check, &view, cx)));
                    if hidden == 0 {
                        menu
                    } else {
                        menu.item(PopupMenuItem::label(format!("+{hidden} more")))
                    }
                })
                .into_any_element(),
        )
    }

    fn render_note(&self, cx: &App) -> Option<AnyElement> {
        let feeds = self.live(cx)?.topology()?;
        let note = topology_coverage(&feeds.feed_rows())?;
        Some(
            div()
                .flex_shrink_0()
                .px_4()
                .py_1()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(note)
                .into_any_element(),
        )
    }

    fn render_export_error(&self) -> Option<AnyElement> {
        let ExportState::Failed { message } = &self.export else {
            return None;
        };
        Some(
            div()
                .flex_shrink_0()
                .px_4()
                .py_1()
                .child(Alert::error("topology-export-error", message.clone()))
                .into_any_element(),
        )
    }

    fn render_body(&self, scale_factor: f32, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let centered = |text: SharedString| {
            v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap_2()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child(text)
                .into_any_element()
        };
        let Some(namespace) = self.namespace.clone() else {
            return centered("Pick a namespace above to draw its topology.".into());
        };
        match &self.build {
            None => v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap_3()
                .child(Spinner::new())
                .child(
                    div()
                        .text_sm()
                        .text_color(theme.muted_foreground)
                        .child(format!("Loading the topology of {namespace}\u{2026}")),
                )
                .into_any_element(),
            Some(Err(too_large)) => centered(
                too_large_text(
                    *too_large,
                    &namespace,
                    self.shown_kinds().contains(&KindFilter::Rbac),
                )
                .into(),
            ),
            Some(Ok(graph)) if graph.nodes.is_empty() => {
                let text = if self.filter.problems_only {
                    format!("No problems in {namespace}.")
                } else {
                    format!("Nothing to draw in {namespace}.")
                };
                centered(text.into())
            }
            Some(Ok(graph)) => self.render_canvas(Rc::clone(graph), scale_factor, cx),
        }
    }

    #[cfg_attr(
        feature = "hotpath-profiling",
        hotpath::measure(impl_type = "TopologyView")
    )]
    fn render_canvas(
        &self,
        graph: Rc<TopologyGraph>,
        scale_factor: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some((_, _, layout)) = &self.layout else {
            return div().into_any_element();
        };
        let layout = Rc::clone(layout);
        let colors = CanvasColors::of(cx);
        let theme = cx.theme();
        let (background, border, radius) = (theme.background, theme.border, theme.radius);
        let selected_key = self.selected(cx);
        let selected = selected_key.as_ref().and_then(|key| {
            graph
                .nodes
                .iter()
                .position(|node| node.key.as_ref() == Some(key))
        });
        let selected = selected.or_else(|| {
            let id = self.fixture_selected.as_ref()?;
            graph.nodes.iter().position(|node| node.id == *id)
        });
        let (width, height) = self.canvas_size.unwrap_or(DEFAULT_CANVAS);
        let viewport = self.viewport;
        let visible = visible_nodes(&layout, viewport, width, height);
        // A card the pointer left while it was scrolled out of view is not hovered any more.
        let hovered = self
            .hovered
            .as_ref()
            .and_then(|id| graph.nodes.iter().position(|node| node.id == *id))
            .filter(|index| visible.contains(index));
        // The drawer covers the right of the canvas: the minimap and the legend move left of it,
        // and the minimap shrinks so it covers fewer cards.
        let drawer = selected_key.as_ref().map_or(0., |key| {
            f32::from(drawer_width(DrawerSize::of(key), px(width)))
        });
        let minimap_size = minimap_size(drawer > 0.);
        let frame = CardFrame {
            viewport: self.viewport,
            scale_factor,
            colors,
        };
        let paint = CanvasPaint {
            graph: Rc::clone(&graph),
            layout: Rc::clone(&layout),
            viewport,
            focus: hovered.or(selected),
            selected,
            elapsed: self.created.elapsed(),
            colors,
            view: cx.weak_entity(),
            is_dragging: !matches!(self.drag, Drag::None | Drag::Minimap),
            traffic: self.traffic.layer.clone(),
        };
        let cards: Vec<AnyElement> = visible
            .into_iter()
            .map(|index| self.card(index, &graph, &layout, &frame, selected, cx))
            .collect();
        let titles: Vec<AnyElement> = layout
            .bands
            .iter()
            .filter_map(|band| {
                let title = band.title.clone()?;
                let (x, y) = viewport.to_screen(band.rect.origin);
                let zoom = viewport.zoom();
                let pill = div()
                    .absolute()
                    .flex()
                    .items_center()
                    .rounded_full()
                    .border_1()
                    .border_color(colors.card_border)
                    .bg(colors.card)
                    .text_color(colors.foreground)
                    .font_family(cx.theme().mono_font_family.clone())
                    .whitespace_nowrap()
                    .child(title);
                // Zoomed out, the title keeps a readable size on the band edge, over the cards.
                Some(if zoom >= MIN_TEXT_ZOOM {
                    pill.left(px(x + TITLE_PILL_LEFT * zoom))
                        .top(px(y + TITLE_PILL_TOP * zoom))
                        .h(px(TITLE_PILL_HEIGHT * zoom))
                        .px(px(TITLE_PILL_PADDING * zoom))
                        .text_size(px(TITLE_SIZE * zoom))
                        .into_any_element()
                } else {
                    pill.left(px(x + 4.))
                        .top(px(y - 9.))
                        .h(px(LOW_ZOOM_TITLE_HEIGHT))
                        .px_2()
                        .text_size(px(LOW_ZOOM_TITLE_SIZE))
                        .into_any_element()
                })
            })
            .collect();
        div()
            .flex_1()
            .min_h_0()
            .px_4()
            .pb_4()
            .pt_2()
            .child(
                div()
                    .id("topology-canvas")
                    .relative()
                    .size_full()
                    .overflow_hidden()
                    .rounded(radius)
                    .border_1()
                    .border_color(border)
                    .bg(background)
                    .track_focus(&self.focus_handle)
                    .on_key_down(cx.listener(|view, event: &KeyDownEvent, _, cx| {
                        if event.keystroke.key == "escape" {
                            view.clear_hover(cx);
                            view.close_drawer(cx);
                        }
                    }))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|view, event: &MouseDownEvent, window, cx| {
                            view.press_canvas(event.position, window, cx);
                        }),
                    )
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|view, event: &gpui_kit::MouseUpEvent, _, cx| {
                            view.finish_drag(event.position, event.click_count, cx);
                        }),
                    )
                    .child(graph_canvas(paint))
                    .children(titles)
                    .children(cards)
                    .child(handle_canvas(
                        Rc::clone(&graph),
                        Rc::clone(&layout),
                        self.traffic.layer.clone(),
                        viewport,
                        colors,
                    ))
                    .children(self.render_edge_labels(&graph, &layout, colors, cx))
                    .child(self.render_controls(cx))
                    .child(self.render_legend(colors, minimap_size.0 + drawer, width, cx))
                    .child(self.render_minimap(&graph, &layout, colors, minimap_size, drawer, cx)),
            )
            .into_any_element()
    }

    fn card(
        &self,
        index: usize,
        graph: &TopologyGraph,
        layout: &TopologyLayout,
        frame: &CardFrame,
        selected: Option<usize>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let node = &graph.nodes[index];
        // A ghost says what its check found; every other card says its full name, which the card
        // may have cut.
        let tooltip = (node.look == NodeLook::Ghost)
            .then(|| {
                graph
                    .checks
                    .iter()
                    .find(|check| check.node == node.id)
                    .map(|check| SharedString::from(check.text.clone()))
            })
            .flatten()
            .unwrap_or_else(|| node.name.clone());
        // In Traffic mode a node shows what flows into it, unless it is a problem; the tooltip
        // always ends with it.
        let traffic = self
            .traffic
            .layer
            .as_ref()
            .and_then(|layer| layer.overlay.nodes[index].as_ref());
        let caption = shown_caption(node, traffic);
        let tooltip = tooltip_with_traffic(&tooltip, traffic);
        let state = CardState {
            is_selected: selected == Some(index),
            is_hovered: self.hovered.as_ref() == Some(&node.id),
            is_highlighted: self.highlighted.as_ref() == Some(&node.id),
            tooltip,
            caption,
        };
        let id = node.id.clone();
        node_card(index, node, layout.rects[index], frame, &state, cx)
            .on_hover(cx.listener(move |view, is_hovered: &bool, _, cx| {
                view.set_hover(id.clone(), *is_hovered, cx);
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |view, event: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    view.press_node(index, event.position, window, cx);
                }),
            )
            .into_any_element()
    }

    /// The zoom panel at the bottom left: zoom in, zoom out, and Fit (React Flow `Controls`).
    fn render_controls(&self, cx: &mut Context<Self>) -> Div {
        let theme = cx.theme();
        let button = |id: &'static str, icon: IconName, tip: &'static str| {
            Button::new(id).ghost().xsmall().icon(icon).tooltip(tip)
        };
        div()
            .absolute()
            .bottom_3()
            .left_3()
            // A press on the panel must not start a pan of the canvas under it.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                v_flex()
                    .p_0p5()
                    .rounded(px(6.))
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.background)
                    .shadow_sm()
                    .child(
                        button("topology-zoom-in", IconName::Plus, "Zoom in").on_click(
                            cx.listener(|view, _, _, cx| {
                                view.zoom_by_button(ZOOM_BUTTON_STEPS, cx);
                            }),
                        ),
                    )
                    .child(
                        button("topology-zoom-out", IconName::Minus, "Zoom out").on_click(
                            cx.listener(|view, _, _, cx| {
                                view.zoom_by_button(-ZOOM_BUTTON_STEPS, cx);
                            }),
                        ),
                    )
                    .child(
                        button("topology-fit", IconName::Maximize, "Fit")
                            .on_click(cx.listener(|view, _, _, cx| view.fit(cx))),
                    ),
            )
    }

    /// The labels of the flow edges (Traffic mode): the rate, and the 5xx share when there is one.
    fn render_edge_labels(
        &self,
        graph: &TopologyGraph,
        layout: &TopologyLayout,
        colors: CanvasColors,
        cx: &App,
    ) -> Vec<AnyElement> {
        let Some(layer) = &self.traffic.layer else {
            return Vec::new();
        };
        let canvas = self.canvas_size.unwrap_or(DEFAULT_CANVAS);
        let mono = cx.theme().mono_font_family.clone();
        edge_labels(graph, layout, layer, self.viewport, canvas)
            .into_iter()
            .map(|label| {
                div()
                    .absolute()
                    .left(px(label.left))
                    .top(px(label.top))
                    .w(px(label.width))
                    .h(px(LABEL_HEIGHT))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(4.))
                    .border_1()
                    .border_color(colors.card_border)
                    .bg(colors.background)
                    .text_color(colors.foreground)
                    .font_family(mono.clone())
                    .text_size(px(LABEL_TEXT_SIZE))
                    .whitespace_nowrap()
                    .child(label.text)
                    .into_any_element()
            })
            .collect()
    }

    /// The legend, in the strip at the bottom that Fit keeps clear: a Legend toggle and, while it
    /// is open, a swatch drawn like a real edge and its meaning for each relation (in Traffic mode,
    /// for each flow). Without a choice of the user it is open only where it fits beside the zoom
    /// panel and the minimap; opened by hand where it does not, it wraps instead of covering them.
    fn render_legend(
        &self,
        colors: CanvasColors,
        inset: f32,
        canvas_width: f32,
        cx: &mut Context<Self>,
    ) -> Div {
        let theme = cx.theme();
        let (mono, background, muted) = (
            theme.mono_font_family.clone(),
            theme.background,
            theme.muted_foreground,
        );
        let sources = self
            .traffic
            .layer
            .as_ref()
            .map(|layer| layer.overlay.sources.as_slice());
        let entries = legend_entries(sources);
        let room = legend_room(canvas_width, inset);
        let is_open = self
            .legend_choice
            .unwrap_or_else(|| legend_width(&entries) <= room);
        let toggle = Button::new("topology-legend")
            .ghost()
            .xsmall()
            .label("Legend")
            .icon(if is_open {
                IconName::ChevronDown
            } else {
                IconName::ChevronRight
            })
            .tooltip(if is_open {
                "Hide the legend"
            } else {
                "Show the legend"
            })
            .on_click(cx.listener(move |view, _, _, cx| view.set_legend_open(!is_open, cx)));
        let entries = entries.into_iter().map(|(swatch, text)| {
            h_flex()
                .gap_2()
                .items_center()
                .child(legend_swatch(swatch, colors))
                .child(div().text_color(muted).child(text))
        });
        div()
            .absolute()
            .bottom_3()
            .right(px(inset + LEGEND_RIGHT_GAP))
            // A press on the legend must not start a pan of the canvas under it.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                h_flex()
                    .max_w(px(room.max(0.)))
                    .flex_wrap()
                    .items_center()
                    .gap_x_4()
                    .px_2()
                    .py_0p5()
                    .rounded(px(6.))
                    .border_1()
                    .border_color(colors.card_border)
                    .bg(background)
                    .font_family(mono)
                    .text_size(px(LEGEND_TEXT_SIZE))
                    .child(toggle)
                    .when(is_open, |row| row.children(entries)),
            )
    }

    /// Shows or hides the legend; the choice is kept while the app runs.
    fn set_legend_open(&mut self, is_open: bool, cx: &mut Context<Self>) {
        self.legend_choice = Some(is_open);
        cx.notify();
    }

    fn render_minimap(
        &self,
        graph: &Rc<TopologyGraph>,
        layout: &Rc<TopologyLayout>,
        colors: CanvasColors,
        size: (f32, f32),
        drawer: f32,
        cx: &mut Context<Self>,
    ) -> Div {
        let theme = cx.theme();
        let paint = MinimapPaint {
            graph: Rc::clone(graph),
            layout: Rc::clone(layout),
            viewport: self.viewport,
            canvas: self.canvas_size.unwrap_or(DEFAULT_CANVAS),
            size,
            colors,
            view: cx.weak_entity(),
            is_dragging: matches!(self.drag, Drag::Minimap),
        };
        div()
            .absolute()
            .bottom_3()
            .right(px(12. + drawer))
            .w(px(size.0))
            .h(px(size.1))
            .rounded(px(6.))
            .border_1()
            .border_color(theme.border)
            .bg(theme.muted)
            .overflow_hidden()
            .child(minimap_canvas(paint))
    }
}

impl Render for TopologyView {
    #[cfg_attr(
        feature = "hotpath-profiling",
        hotpath::measure(impl_type = "TopologyView")
    )]
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let scale_factor = window.scale_factor();
        if self._activation.is_none() {
            self._activation = Some(cx.observe_window_activation(window, |_, _, cx| cx.notify()));
        }
        v_flex()
            .size_full()
            .child(self.render_toolbar(cx))
            .children(self.render_note(cx))
            .children(self.render_export_error())
            .child(
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .child(self.render_body(scale_factor, cx)),
            )
    }
}

/// The width the legend may take: the canvas without the zoom panel on the left and the minimap
/// (and the drawer) on the right.
fn legend_room(canvas_width: f32, inset: f32) -> f32 {
    canvas_width - inset - LEGEND_RIGHT_GAP - CONTROLS_INSET
}

/// The size of the minimap: full, or `COMPACT_MINIMAP_SCALE` of it while the drawer is open.
fn minimap_size(is_drawer_open: bool) -> (f32, f32) {
    let scale = if is_drawer_open {
        COMPACT_MINIMAP_SCALE
    } else {
        1.
    };
    (MINIMAP_WIDTH * scale, MINIMAP_HEIGHT * scale)
}

/// `Saved to {file}`, plus `exported at {pct}%` when a PNG was cut below full size. An SVG has no
/// scale.
fn saved_detail(file_name: &str, scale: Option<f32>) -> String {
    let is_svg = file_name.to_ascii_lowercase().ends_with(".svg");
    match scale.filter(|scale| *scale < 1. && !is_svg) {
        Some(scale) => format!(
            "Saved to {file_name} \u{b7} exported at {}%",
            (scale * 100.).round()
        ),
        None => format!("Saved to {file_name}"),
    }
}

/// The first Deployment node with an object behind it, in graph order.
fn first_deployment(graph: &TopologyGraph) -> Option<NodeId> {
    graph
        .nodes
        .iter()
        .find(|node| node.kind == TopologyKind::Deployment && node.key.is_some())
        .map(|node| node.id.clone())
}

/// Whether the object of an open drawer, if it is in `namespace`, is gone from a list that has
/// loaded: the pods, or the feed of its kind. A feed that is not ready proves nothing.
fn is_selection_gone(
    key: &ResourceKey,
    namespace: &str,
    pods: &[PodSummary],
    rows: &[(TopologyKind, FeedRows)],
) -> bool {
    match key {
        ResourceKey::Pod {
            namespace: owner, ..
        } => owner == namespace && !pods.iter().any(|pod| key.is_pod(pod)),
        ResourceKey::Kind {
            kind,
            namespace: Some(owner),
            ..
        } if owner == namespace => {
            let Some(topology_kind) = TopologyKind::of_resource_kind(*kind) else {
                return false;
            };
            rows.iter().any(|(listed, feed)| {
                *listed == topology_kind
                    && matches!(feed, FeedRows::Ready(rows) if !rows.iter().any(|row| key.is_row(*kind, row)))
            })
        }
        ResourceKey::Kind { .. } | ResourceKey::Node { .. } => false,
    }
}

/// `ns: {ns} · {n} resources`, or why there are no numbers: the graph is too large, or loading.
fn count_text(namespace: &str, build: Option<&Built>) -> String {
    match build {
        Some(Ok(graph)) => format!("ns: {namespace} \u{b7} {} resources", graph.resources),
        Some(Err(_)) => format!("ns: {namespace} \u{b7} too large"),
        None => format!("ns: {namespace} \u{b7} loading\u{2026}"),
    }
}

/// The cards at the other end of the edges of the node `index`.
fn neighbour_rects(graph: &TopologyGraph, layout: &TopologyLayout, index: usize) -> Vec<GraphRect> {
    graph
        .edges
        .iter()
        .filter_map(|edge| {
            if edge.from == index {
                Some(edge.to)
            } else if edge.to == index {
                Some(edge.from)
            } else {
                None
            }
        })
        .map(|other| layout.rects[other])
        .collect()
}

/// `1 of {n} namespaces in scope` when the title-bar scope holds several: Topology draws one.
fn scope_note(scope: &NamespaceScope) -> Option<String> {
    match scope {
        NamespaceScope::Several(namespaces) => {
            Some(format!("1 of {} namespaces in scope", namespaces.len()))
        }
        NamespaceScope::All | NamespaceScope::Named(_) => None,
    }
}

/// The namespace to open in when the scope is `All`: the one Topology drew last in this context,
/// else the one with the most pods among the pods already loaded. `known` is the loaded
/// namespace list, when there is one: a remembered name that is not in it is gone.
fn preselected_namespace(
    last: Option<&str>,
    known: Option<&[NamespaceSummary]>,
    pods: &[PodSummary],
) -> Option<String> {
    let last = last.filter(|last| {
        known.is_none_or(|known| known.iter().any(|namespace| namespace.name == *last))
    });
    if let Some(last) = last {
        return Some(last.to_owned());
    }
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for pod in pods {
        *counts.entry(pod.namespace.as_str()).or_default() += 1;
    }
    // A tie goes to the first name, since the map is sorted and `max_by_key` keeps the last.
    counts
        .into_iter()
        .rev()
        .max_by_key(|(_, count)| *count)
        .map(|(namespace, _)| namespace.to_owned())
}

/// What a click on a card does.
#[derive(Debug, PartialEq, Eq)]
enum CardClick {
    /// Show the object on its own screen, not in the drawer over the graph.
    Reveal(ResourceKey),
    Select(ResourceKey),
    Highlight,
}

/// A double click reveals the object; so does a click on one with no row to show in the drawer
/// (decision 51).
fn card_click(key: Option<ResourceKey>, has_row: bool, click_count: usize) -> CardClick {
    match key {
        Some(key) if click_count >= 2 || !has_row => CardClick::Reveal(key),
        Some(key) => CardClick::Select(key),
        None => CardClick::Highlight,
    }
}

/// Whether the object of `key` is one of the layers Traffic mode turns off, so its drawer cannot
/// stay open over a graph that no longer draws it.
fn is_traffic_hidden(key: &ResourceKey) -> bool {
    let ResourceKey::Kind { kind, .. } = key else {
        return false;
    };
    TopologyKind::of_resource_kind(*kind).is_some_and(|kind| kind.filter().is_off_in_traffic())
}

/// The element id of a kind chip.
fn chip_id(kind: KindFilter) -> &'static str {
    match kind {
        KindFilter::Ingress => "topology-chip-ingress",
        KindFilter::Service => "topology-chip-service",
        KindFilter::Workload => "topology-chip-workload",
        KindFilter::Config => "topology-chip-config",
        KindFilter::Rbac => "topology-chip-rbac",
    }
}

/// A dropdown row of one check: a tone dot and the full sentence. A pick centers the node.
fn check_item(check: &ConfigCheck, view: &WeakEntity<TopologyView>, cx: &App) -> PopupMenuItem {
    let color = tone_color(check.tone, cx);
    let text = check.text.clone();
    let node = check.node.clone();
    let view = view.clone();
    PopupMenuItem::element(move |_, _| {
        h_flex()
            .gap_2()
            .child(div().flex_shrink_0().size(px(8.)).rounded_full().bg(color))
            .child(text.clone())
    })
    .on_click(move |_, _, cx| {
        let node = node.clone();
        let _ = view.update(cx, |view, cx| view.focus_node(node, cx));
    })
}

/// The namespace the scope starts Topology in: the one of `Named`, the first of `Several`, none
/// for `All`.
fn default_namespace(scope: &NamespaceScope) -> Option<String> {
    scope.namespaces().first().cloned()
}

/// `current` while it is inside the scope, else the scope's default (decision 1).
fn resolve_namespace(current: Option<&str>, scope: &NamespaceScope) -> Option<String> {
    match current {
        Some(namespace) if scope_includes(scope, namespace) => Some(namespace.to_owned()),
        _ => default_namespace(scope),
    }
}

/// The namespaces the dropdown lists: the scope's own, or for `All` every namespace loaded, sorted.
fn namespace_choices(
    scope: &NamespaceScope,
    namespaces: Option<&[NamespaceSummary]>,
) -> Vec<String> {
    match scope {
        NamespaceScope::All => {
            let mut names: Vec<String> = namespaces
                .unwrap_or_default()
                .iter()
                .map(|namespace| namespace.name.clone())
                .collect();
            names.sort();
            names
        }
        NamespaceScope::Named(_) | NamespaceScope::Several(_) => scope.namespaces().to_vec(),
    }
}

/// The too-large state. With the RBAC chip on, a graph over the node limit says so: the layer adds
/// accounts, bindings, and roles, and turning it off is the quickest way back.
fn too_large_text(too_large: TooLarge, namespace: &str, is_rbac_on: bool) -> String {
    match too_large {
        TooLarge::Nodes(count) if is_rbac_on => format!(
            "{namespace} would show {count} nodes; Topology draws at most {NODE_LIMIT}. Too many nodes with the RBAC layer on; turn RBAC off or pick a smaller namespace."
        ),
        TooLarge::Objects(count) => format!(
            "{namespace} holds {count} objects; Topology draws at most {RAW_LIMIT}. Pick a smaller namespace."
        ),
        TooLarge::Nodes(count) => format!(
            "{namespace} would show {count} nodes; Topology draws at most {NODE_LIMIT}. Turn kind chips off or use Problems only."
        ),
    }
}

#[cfg(test)]
#[path = "topology_view_tests.rs"]
mod topology_view_tests;
