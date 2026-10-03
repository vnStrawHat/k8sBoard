//! The Topology screen (W11): the toolbar, the canvas with its cards, and the state between them.
//! The graph and its layout are rebuilt at most every `TOPOLOGY_TICK`, and only when something
//! changed; everything runs on the main thread because the input is bounded (decision 29).

use std::collections::{BTreeSet, HashMap};
use std::rc::Rc;
use std::time::Duration;

use cluster::{NamespaceScope, NamespaceSummary, PodSummary};
use gpui_kit::component::alert::Alert;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Selectable as _, Sizable as _, h_flex, v_flex,
};
use gpui_kit::{
    AnyElement, App, Context, Div, Entity, FocusHandle, InteractiveElement as _, IntoElement,
    KeyDownEvent, MouseButton, MouseDownEvent, ParentElement as _, Pixels, Point, Render,
    ScrollDelta, SharedString, Styled as _, Subscription, Task, WeakEntity, Window, div,
    prelude::FluentBuilder as _, px,
};

use crate::app_shell::AppShell;
use crate::app_shell::workspace::toggle_button;
use crate::cluster_session::{ClusterSession, LiveCluster, scope_includes};
use crate::file_export::{ExportState, export_file_name, start_export_with};
use crate::status_tone::{StatusTone, tone_color};
use crate::table_selection::ResourceKey;
use crate::topology_canvas::{
    CanvasColors, CanvasPaint, CardState, LEGEND, MinimapPaint, graph_canvas, legend_color,
    minimap_canvas, node_card,
};
use crate::topology_checks::{ConfigCheck, checks_chip, topology_coverage};
use crate::topology_export::{TopologyExport, export_scale, svg_style, topology_svg, write_export};
use crate::topology_feeds::TopologySubject;
use crate::topology_graph::{
    FeedRows, GroupBy, KindFilter, NODE_LIMIT, NodeId, NodeLook, RAW_LIMIT, TooLarge,
    TopologyBuild, TopologyFilter, TopologyGraph, TopologyInputs, TopologyKind, build_topology,
    resolve_group_by,
};
use crate::topology_layout::{
    GraphPoint, GraphStructure, TopologyLayout, layout as lay_out, structure,
};
use crate::topology_viewport::{
    MIN_TEXT_ZOOM, MINIMAP_HEIGHT, MINIMAP_WIDTH, OVERLAY_GUTTER, Viewport, is_drag, visible_nodes,
    wheel_steps,
};

/// The rebuild runs at most this often, and only when something changed.
const TOPOLOGY_TICK: Duration = Duration::from_millis(500);
/// The most checks the dropdown lists; the rest is a count.
const CHECKS_MENU_LIMIT: usize = 50;
/// The size the first frame assumes for the canvas, before it has painted once.
const DEFAULT_CANVAS: (f32, f32) = (1200., 700.);
/// The font size of a band title in a zoomed-out view, in pixels.
const LOW_ZOOM_TITLE_SIZE: f32 = 11.;
/// The height of the namespace list the dropdown shows before it scrolls.
const NAMESPACE_MENU_HEIGHT: f32 = 320.;

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

/// The built graph, or why there is none.
type Built = Result<Rc<TopologyGraph>, TooLarge>;

pub(crate) struct TopologyView {
    session: Option<Entity<ClusterSession>>,
    shell: WeakEntity<AppShell>,
    /// The namespace drawn; `None` until one is picked (scope `All`).
    namespace: Option<String>,
    filter: TopologyFilter,
    /// The pod groups the user opened.
    expanded: BTreeSet<NodeId>,
    /// The dragged positions, kept in memory per (context, namespace) (decision 23).
    pins: HashMap<String, HashMap<String, HashMap<NodeId, GraphPoint>>>,
    build: Option<Built>,
    layout: Option<(GraphStructure, GroupBy, Rc<TopologyLayout>)>,
    viewport: Viewport,
    needs_fit: bool,
    /// The first view was made for `DEFAULT_CANVAS`: it is made again once the real size is known.
    fit_waits_for_size: bool,
    /// A ghost or unchecked node that was clicked.
    highlighted: Option<NodeId>,
    /// A node to center and select after the next build that has it (Show in Topology, a check).
    pending_focus: Option<NodeId>,
    drag: Drag,
    is_dirty: bool,
    is_visible: bool,
    /// The size of the canvas at its last paint, which Fit and the focus need.
    canvas_size: Option<(f32, f32)>,
    wheel_carry: f32,
    focus_handle: FocusHandle,
    export: ExportState,
    /// The scale the last PNG was rendered at.
    export_scale: Option<f32>,
    _export: Option<Task<()>>,
    _tick: Option<Task<()>>,
    _observe: Option<Subscription>,
}

impl TopologyView {
    pub(crate) fn new(shell: WeakEntity<AppShell>, cx: &mut Context<Self>) -> Self {
        Self {
            session: None,
            shell,
            namespace: None,
            filter: TopologyFilter::everything(),
            expanded: BTreeSet::new(),
            pins: HashMap::new(),
            build: None,
            layout: None,
            viewport: Viewport::default(),
            needs_fit: true,
            fit_waits_for_size: false,
            highlighted: None,
            pending_focus: None,
            drag: Drag::None,
            is_dirty: true,
            is_visible: false,
            canvas_size: None,
            wheel_carry: 0.,
            focus_handle: cx.focus_handle(),
            export: ExportState::Idle,
            export_scale: None,
            _export: None,
            _tick: None,
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
        self.namespace = None;
        self.expanded.clear();
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

    /// The count under the title: `ns: {ns} · {n} resources`.
    pub(crate) fn header_count(&self) -> Option<String> {
        Some(count_text(self.namespace.as_deref()?, self.build.as_ref()))
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

    /// Whether the graph exists, for the screenshot hook.
    #[cfg(feature = "screenshot")]
    pub(crate) fn has_build(&self) -> bool {
        self.build.is_some()
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
        let Some(session) = self.session.clone() else {
            return;
        };
        if let Some(live) = session.read(cx).live() {
            let resolved = resolve_namespace(self.namespace.as_deref(), &live.scope);
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
                kinds: self.filter.kinds.clone(),
            });
        if session.read(cx).topology_subject() != subject.as_ref() {
            session.update(cx, |session, cx| session.set_topology_subject(subject, cx));
        }
    }

    /// Another namespace: nothing of the old one stays (decision 1), and the drawer closes if it
    /// shows an object of it.
    fn change_namespace(&mut self, namespace: Option<String>, cx: &mut Context<Self>) {
        self.namespace = namespace;
        self.expanded.clear();
        self.pending_focus = None;
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
        let Some(namespace) = self.namespace.clone() else {
            return;
        };
        let Some(session) = self.session.clone() else {
            return;
        };
        let aspect = self.aspect();
        let selected = self.selected(cx);
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
                filter: &self.filter,
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
        self.build = Some(Ok(Rc::new(graph)));
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
        self.viewport = Viewport::first_view(layout.extent, width, height);
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
        self.viewport = self.viewport.center_on(center, width, height);
        self.pending_focus = None;
        self.highlighted = (key.is_none()).then_some(id);
        self.with_shell(cx, |shell, cx| shell.select_on_topology(key, cx));
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
    }

    // ---- toolbar actions ----

    /// Fit: the whole graph in view, even where the cards are too small to read.
    pub(crate) fn fit(&mut self, cx: &mut Context<Self>) {
        if let Some((_, _, layout)) = &self.layout {
            let (width, height) = self.view_area();
            self.viewport = Viewport::fit(layout.extent, width, height);
        }
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

    fn toggle_kind(&mut self, kind: KindFilter, cx: &mut Context<Self>) {
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
        // The first view was made for a default size.
        if self.fit_waits_for_size {
            self.needs_fit = true;
        }
        self.apply_fit();
        cx.notify();
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
        match key {
            Some(key) if click_count >= 2 => {
                self.with_shell(cx, |shell, cx| shell.reveal_in_primary(key, cx));
            }
            Some(key) => {
                self.highlighted = None;
                self.with_shell(cx, |shell, cx| shell.select_on_topology(Some(key), cx));
            }
            None => self.highlighted = Some(id.clone()),
        }
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
        let svg = topology_svg(graph, layout, &title, &svg_style(cx));
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
        let namespace_label = format!("Namespace: {}", self.namespace.as_deref().unwrap_or("none"));
        let current = self.namespace.clone();
        let namespace_button = Button::new("topology-namespace")
            .outline()
            .small()
            .label(namespace_label)
            .dropdown_caret(true)
            .tooltip("The namespace to draw")
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
            let is_on = self.filter.kinds.contains(&kind);
            toggle_button(chip_id(kind), kind.label(), is_on)
                .tooltip(format!("Show {} nodes", kind.label()))
                .on_click(cx.listener(move |view, _, _, cx| view.toggle_kind(kind, cx)))
        });
        let rbac = Button::new("topology-chip-rbac")
            .label("RBAC")
            .small()
            .outline()
            .disabled(true)
            .tooltip("Not shown in this version");
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
        let segment = h_flex()
            .gap_1()
            .child(
                Button::new("topology-resources")
                    .small()
                    .primary()
                    .label("Resources"),
            )
            .child(
                Button::new("topology-traffic")
                    .small()
                    .outline()
                    .label("Traffic")
                    .disabled(true)
                    .tooltip("Needs a service mesh; not available yet"),
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
            .child(h_flex().gap_1().children(chips).child(rbac))
            .child(problems)
            .child(
                h_flex()
                    .ml_auto()
                    .gap_2()
                    .children(reset)
                    .children(self.render_checks_chip(cx)),
            )
            .into_any_element()
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

    fn render_body(&self, cx: &mut Context<Self>) -> AnyElement {
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
            return centered("Pick a namespace to draw its topology.".into());
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
            Some(Err(too_large)) => centered(too_large_text(*too_large, &namespace).into()),
            Some(Ok(graph)) if graph.nodes.is_empty() => {
                let text = if self.filter.problems_only {
                    format!("No problems in {namespace}.")
                } else {
                    format!("Nothing to draw in {namespace}.")
                };
                centered(text.into())
            }
            Some(Ok(graph)) => self.render_canvas(Rc::clone(graph), cx),
        }
    }

    fn render_canvas(&self, graph: Rc<TopologyGraph>, cx: &mut Context<Self>) -> AnyElement {
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
        let (width, height) = self.canvas_size.unwrap_or(DEFAULT_CANVAS);
        let viewport = self.viewport;
        let paint = CanvasPaint {
            graph: Rc::clone(&graph),
            layout: Rc::clone(&layout),
            viewport,
            selected,
            colors,
            view: cx.weak_entity(),
            is_dragging: !matches!(self.drag, Drag::None | Drag::Minimap),
        };
        let cards: Vec<AnyElement> = visible_nodes(&layout, viewport, width, height)
            .into_iter()
            .map(|index| self.card(index, &graph, &layout, selected, cx))
            .collect();
        let titles: Vec<AnyElement> = layout
            .bands
            .iter()
            .filter_map(|band| {
                let title = band.title.clone()?;
                let (x, y) = viewport.to_screen(band.rect.origin);
                let zoom = viewport.zoom();
                let title = div()
                    .absolute()
                    .font_family(cx.theme().mono_font_family.clone())
                    .whitespace_nowrap()
                    .child(title);
                // Zoomed out, the title keeps a readable size on the band edge, over the cards.
                Some(if zoom >= MIN_TEXT_ZOOM {
                    title
                        .left(px(x + 10. * zoom))
                        .top(px(y + 4. * zoom))
                        .text_size(px(10. * zoom))
                        .text_color(colors.muted_foreground)
                        .into_any_element()
                } else {
                    title
                        .left(px(x + 4.))
                        .top(px(y - 6.))
                        .px_1()
                        .bg(background)
                        .text_size(px(LOW_ZOOM_TITLE_SIZE))
                        .text_color(colors.muted_foreground)
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
                    .child(self.render_legend(colors, cx))
                    .child(self.render_minimap(&graph, &layout, colors, cx)),
            )
            .into_any_element()
    }

    fn card(
        &self,
        index: usize,
        graph: &TopologyGraph,
        layout: &TopologyLayout,
        selected: Option<usize>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let node = &graph.nodes[index];
        let tooltip = (node.look == NodeLook::Ghost)
            .then(|| {
                graph
                    .checks
                    .iter()
                    .find(|check| check.node == node.id)
                    .map(|check| SharedString::from(check.text.clone()))
            })
            .flatten();
        let state = CardState {
            is_selected: selected == Some(index),
            is_highlighted: self.highlighted.as_ref() == Some(&node.id),
            tooltip,
        };
        node_card(index, node, layout.rects[index], self.viewport, &state, cx)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |view, event: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    view.press_node(index, event.position, window, cx);
                }),
            )
            .into_any_element()
    }

    fn render_legend(&self, colors: CanvasColors, cx: &App) -> Div {
        let theme = cx.theme();
        let mono = theme.mono_font_family.clone();
        let entries = LEGEND.map(|(relation, glyph, text)| {
            h_flex()
                .gap_1()
                .child(
                    div()
                        .text_color(legend_color(relation, &colors))
                        .child(glyph),
                )
                .child(div().text_color(theme.muted_foreground).child(text))
        });
        div()
            .absolute()
            .bottom_3()
            .right(px(MINIMAP_WIDTH + 28.))
            .child(
                h_flex()
                    .gap_3()
                    .px_2()
                    .py_1()
                    .rounded(px(6.))
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.background)
                    .font_family(mono)
                    .text_size(px(10.))
                    .children(entries),
            )
    }

    fn render_minimap(
        &self,
        graph: &Rc<TopologyGraph>,
        layout: &Rc<TopologyLayout>,
        colors: CanvasColors,
        cx: &mut Context<Self>,
    ) -> Div {
        let theme = cx.theme();
        let paint = MinimapPaint {
            graph: Rc::clone(graph),
            layout: Rc::clone(layout),
            viewport: self.viewport,
            canvas: self.canvas_size.unwrap_or(DEFAULT_CANVAS),
            colors,
            view: cx.weak_entity(),
            is_dragging: matches!(self.drag, Drag::Minimap),
        };
        div()
            .absolute()
            .bottom_3()
            .right_3()
            .w(px(MINIMAP_WIDTH))
            .h(px(MINIMAP_HEIGHT))
            .rounded(px(6.))
            .border_1()
            .border_color(theme.border)
            .bg(theme.muted)
            .overflow_hidden()
            .child(minimap_canvas(paint))
    }
}

impl Render for TopologyView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .child(self.render_toolbar(cx))
            .children(self.render_note(cx))
            .children(self.render_export_error())
            .child(v_flex().flex_1().min_h_0().child(self.render_body(cx)))
    }
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

/// The element id of a kind chip.
fn chip_id(kind: KindFilter) -> &'static str {
    match kind {
        KindFilter::Ingress => "topology-chip-ingress",
        KindFilter::Service => "topology-chip-service",
        KindFilter::Workload => "topology-chip-workload",
        KindFilter::Config => "topology-chip-config",
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

fn too_large_text(too_large: TooLarge, namespace: &str) -> String {
    match too_large {
        TooLarge::Objects(count) => format!(
            "{namespace} holds {count} objects; Topology draws at most {RAW_LIMIT}. Pick a smaller namespace."
        ),
        TooLarge::Nodes(count) => format!(
            "{namespace} would show {count} nodes; Topology draws at most {NODE_LIMIT}. Turn kind chips off or use Problems only."
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::topology_fixtures::Fixture;

    fn named(name: &str) -> NamespaceScope {
        NamespaceScope::Named(name.to_owned())
    }

    #[test]
    fn default_namespace_follows_the_scope() {
        assert_eq!(default_namespace(&named("shop")), Some("shop".to_owned()));
        let several = NamespaceScope::Several(vec!["a".to_owned(), "b".to_owned()]);
        assert_eq!(default_namespace(&several), Some("a".to_owned()));
        assert_eq!(default_namespace(&NamespaceScope::All), None);
    }

    #[test]
    fn a_namespace_that_left_the_scope_resets_to_the_default() {
        assert_eq!(
            resolve_namespace(Some("shop"), &named("blog")),
            Some("blog".to_owned())
        );
        assert_eq!(
            resolve_namespace(Some("shop"), &NamespaceScope::All),
            Some("shop".to_owned())
        );
        assert_eq!(resolve_namespace(None, &NamespaceScope::All), None);
    }

    #[test]
    fn namespace_choices_list_the_scope_or_every_namespace() {
        assert_eq!(namespace_choices(&named("shop"), None), ["shop"]);
        let summary = |name: &str| NamespaceSummary {
            name: name.to_owned(),
            phase: cluster::NamespacePhase::Active,
            created_at: None,
            labels: Vec::new(),
            deleting_since: None,
            deletion_conditions: Vec::new(),
        };
        let loaded = [summary("zeta"), summary("alpha")];
        assert_eq!(
            namespace_choices(&NamespaceScope::All, Some(&loaded)),
            ["alpha", "zeta"]
        );
        assert!(namespace_choices(&NamespaceScope::All, None).is_empty());
    }

    #[test]
    fn a_cut_png_says_its_percentage() {
        assert_eq!(saved_detail("a.png", None), "Saved to a.png");
        assert_eq!(saved_detail("a.png", Some(1.)), "Saved to a.png");
        assert_eq!(
            saved_detail("a.png", Some(0.5)),
            "Saved to a.png \u{b7} exported at 50%"
        );
        // An SVG is not scaled.
        assert_eq!(saved_detail("a.SVG", Some(0.5)), "Saved to a.SVG");
    }

    #[test]
    fn the_header_says_loading_too_large_or_the_count() {
        assert_eq!(count_text("shop", None), "ns: shop \u{b7} loading\u{2026}");
        assert_eq!(
            count_text("shop", Some(&Err(TooLarge::Nodes(900)))),
            "ns: shop \u{b7} too large"
        );
        let graph = Fixture::default().with_deployment("api", 1, 1).graph();
        assert_eq!(
            count_text("shop", Some(&Ok(Rc::new(graph)))),
            "ns: shop \u{b7} 1 resources"
        );
    }

    fn service_key(name: &str) -> ResourceKey {
        ResourceKey::Kind {
            kind: crate::resource_kind::ResourceKind::Services,
            namespace: Some("shop".to_owned()),
            name: name.to_owned(),
        }
    }

    #[test]
    fn a_selection_is_gone_only_from_a_feed_that_has_loaded() {
        let fixture = Fixture::default().with_service("web", &[]);
        let pods: Vec<PodSummary> = Vec::new();
        fixture.with_rows(|rows| {
            assert!(!is_selection_gone(&service_key("web"), "shop", &pods, rows));
            assert!(is_selection_gone(&service_key("api"), "shop", &pods, rows));
            // Another namespace is not this graph's business.
            assert!(!is_selection_gone(&service_key("api"), "blog", &pods, rows));
        });
    }

    #[test]
    fn a_selection_waits_for_a_feed_that_is_not_ready() {
        let fixture = Fixture::default().loading(TopologyKind::Service);
        fixture.with_rows(|rows| {
            assert!(!is_selection_gone(&service_key("web"), "shop", &[], rows));
        });
    }

    #[test]
    fn a_pod_is_gone_when_the_loaded_pods_lack_it() {
        let pods = vec![crate::topology_fixtures::pod("web-1", &[], None)];
        let key = |name: &str| ResourceKey::Pod {
            namespace: "shop".to_owned(),
            name: name.to_owned(),
        };
        assert!(!is_selection_gone(&key("web-1"), "shop", &pods, &[]));
        assert!(is_selection_gone(&key("web-2"), "shop", &pods, &[]));
    }

    #[test]
    fn too_large_states_name_their_limit() {
        assert!(too_large_text(TooLarge::Objects(6_000), "shop").contains("5000"));
        assert!(too_large_text(TooLarge::Nodes(900), "shop").contains("500"));
    }
}
