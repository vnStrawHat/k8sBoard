use std::path::PathBuf;
use std::sync::Arc;

use cluster::{
    ContextSummary, EventFilter, InvolvedObject, Kubeconfig, KubeconfigError, NamespaceScope,
};
use gpui_kit::component::resizable::ResizableState;
use gpui_kit::component::table::{TableDelegate, TableEvent, TableState};
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::{
    App, AppContext as _, Context, Entity, Focusable as _, IntoElement, ParentElement as _, Point,
    Render, Styled as _, Subscription, Task, Window,
};

#[cfg(feature = "screenshot")]
use crate::cluster_session::SessionPhase;
use crate::cluster_session::{ClusterSession, LiveCluster, error_text};
use crate::drawer::{ContainerTab, DRAWER_SUBJECT_DELAY, DrawerState, DrawerTab};
use crate::kind_table::KindTableDelegate;
use crate::launch_options::{
    LaunchOptions, LaunchScreen, has_ignored_kubeconfig_entries, kubeconfig_path,
};
use crate::log_dock::{DockMode, LogDock};
use crate::log_tab::LogTarget;
use crate::navigation::{NavigationCounts, sidebar};
use crate::node_table::NodeTableDelegate;
use crate::object_events::{SubjectChange, event_subject, subject_change};
use crate::pod_table::PodTableDelegate;
use crate::resource_kind::ResourceKind;
#[cfg(feature = "screenshot")]
use crate::screenshot::{SettleInput, TargetState, is_drawer_ready};
use crate::screenshot::{pick_drawer_pod, pick_logs_pod};
use crate::status_bar::status_bar;
use crate::table_selection::{ResourceKey, SelectionSync, list_row_index, selection_sync};
use crate::title_bar::title_bar;
use crate::yaml_view::{YamlView, yaml_subject};

#[path = "workspace.rs"]
mod workspace;

const IGNORED_KUBECONFIG_NOTE: &str =
    "Only the first KUBECONFIG entry is used; merging kubeconfigs is not supported";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Screen {
    Pods,
    Nodes,
    Kind(ResourceKind),
}

impl Screen {
    /// The explorer kind this screen lists, if it is a kind screen.
    pub(crate) fn kind(self) -> Option<ResourceKind> {
        match self {
            Self::Kind(kind) => Some(kind),
            Self::Pods | Self::Nodes => None,
        }
    }
}

enum KubeconfigState {
    Loading,
    Loaded(Arc<Kubeconfig>),
    Failed(String),
}

/// What the command line asked for, used only by the first session.
struct RequestedStart {
    context: Option<String>,
    namespace: Option<String>,
}

/// The root view: the six regions of the window, the screen choice, and the selection that
/// opens the drawer.
pub(crate) struct AppShell {
    kubeconfig: KubeconfigState,
    /// Set when the kubeconfig loaded but names no usable context; there is no session then.
    context_error: Option<String>,
    session: Option<Entity<ClusterSession>>,
    _session_observer: Option<Subscription>,
    screen: Screen,
    pod_table: Entity<TableState<PodTableDelegate>>,
    node_table: Entity<TableState<NodeTableDelegate>>,
    kind_table: Entity<TableState<KindTableDelegate>>,
    _table_subscriptions: Vec<Subscription>,
    /// The drawer is open exactly while this is set.
    selected: Option<ResourceKey>,
    drawer: DrawerState,
    /// The pending debounced start of the object events watch. Replacing or dropping it cancels it.
    event_subject_task: Option<Task<()>>,
    log_dock: Entity<LogDock>,
    /// Keeps the dock height across zoom and minimize, which unmount the split.
    dock_split: Entity<ResizableState>,
    /// A `--screen` drawer or logs request that waits for its list to load.
    pending_launch_screen: Option<LaunchScreen>,
    requested: RequestedStart,
}

impl AppShell {
    pub(crate) fn new(options: LaunchOptions, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let kubeconfig_env = std::env::var_os("KUBECONFIG");
        let has_ignored_entries = options.kubeconfig.is_none()
            && has_ignored_kubeconfig_entries(kubeconfig_env.as_deref());
        let kubeconfig = match kubeconfig_path(
            options.kubeconfig,
            kubeconfig_env,
            std::env::home_dir(),
        ) {
            Some(path) => {
                Self::load_kubeconfig(path, has_ignored_entries, cx);
                KubeconfigState::Loading
            }
            None => KubeconfigState::Failed(
                "no kubeconfig found: pass --kubeconfig, set KUBECONFIG, or create ~/.kube/config"
                    .to_owned(),
            ),
        };

        let shell = cx.weak_entity();
        let log_dock = cx.new(|_| LogDock::new());
        let dock_split = cx.new(|_| ResizableState::default());
        let pod_table = cx.new(|cx| {
            configure(TableState::new(
                PodTableDelegate::new(log_dock.downgrade(), shell.clone()),
                window,
                cx,
            ))
        });
        let node_table = cx.new(|cx| {
            configure(TableState::new(
                NodeTableDelegate::new(shell.clone()),
                window,
                cx,
            ))
        });
        let initial_kind = options.screen.screen().kind();
        let kind_table = cx.new(|cx| {
            configure(TableState::new(
                KindTableDelegate::new(initial_kind, shell),
                window,
                cx,
            ))
        });
        // The workspace layout depends on the dock's mode and tab count.
        let table_subscriptions = vec![
            cx.subscribe_in(&pod_table, window, Self::on_pod_table_event),
            cx.subscribe_in(&node_table, window, Self::on_node_table_event),
            cx.subscribe_in(&kind_table, window, Self::on_kind_table_event),
            cx.observe(&log_dock, |_, _, cx| cx.notify()),
        ];

        let mut drawer = DrawerState::new();
        drawer.tab = options.screen.drawer_tab().unwrap_or(DrawerTab::Overview);
        // W4b shows the Containers tab expanded.
        drawer.is_expanded = options.screen == LaunchScreen::PodDrawer(DrawerTab::Containers);
        Self {
            kubeconfig,
            context_error: None,
            session: None,
            _session_observer: None,
            screen: options.screen.screen(),
            pod_table,
            node_table,
            kind_table,
            _table_subscriptions: table_subscriptions,
            selected: None,
            drawer,
            event_subject_task: None,
            log_dock,
            dock_split,
            pending_launch_screen: (options.screen.has_drawer() || options.screen.has_log_dock())
                .then_some(options.screen),
            requested: RequestedStart {
                context: options.context,
                namespace: options.namespace,
            },
        }
    }

    /// Reading the file is blocking I/O, so it runs on the background executor, not on the
    /// UI thread and not on tokio.
    fn load_kubeconfig(path: PathBuf, has_ignored_entries: bool, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let loaded = cx
                .background_executor()
                .spawn(async move { Kubeconfig::load(&path) })
                .await;
            let _ = this.update(cx, |shell, cx| {
                shell.finish_kubeconfig_load(loaded, has_ignored_entries, cx)
            });
        })
        .detach();
    }

    fn finish_kubeconfig_load(
        &mut self,
        loaded: Result<Kubeconfig, KubeconfigError>,
        has_ignored_entries: bool,
        cx: &mut Context<Self>,
    ) {
        match loaded {
            Ok(kubeconfig) => {
                let kubeconfig = Arc::new(kubeconfig);
                self.kubeconfig = KubeconfigState::Loaded(Arc::clone(&kubeconfig));
                let requested = self.requested.context.take();
                match kubeconfig.resolve_context(requested.as_deref()) {
                    Ok(summary) => {
                        let summary = summary.clone();
                        let namespace = self.requested.namespace.take();
                        self.start_session(kubeconfig, &summary, namespace, cx);
                    }
                    Err(error) => {
                        self.context_error = Some(kubeconfig_error_message(
                            error_text(&error),
                            has_ignored_entries,
                        ));
                    }
                }
            }
            Err(error) => {
                self.kubeconfig = KubeconfigState::Failed(kubeconfig_error_message(
                    error_text(&error),
                    has_ignored_entries,
                ));
            }
        }
        cx.notify();
    }

    /// Replaces the session. Dropping the old one cancels every task and watch it owns.
    fn start_session(
        &mut self,
        kubeconfig: Arc<Kubeconfig>,
        summary: &ContextSummary,
        namespace: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let kind = self.screen.kind();
        self.close_drawer(cx);
        self.log_dock.update(cx, |dock, cx| dock.close_all(cx));
        let session = cx.new(|cx| ClusterSession::new(kubeconfig, summary, namespace, kind, cx));
        self._session_observer = Some(cx.observe(&session, |shell, _, cx| {
            shell.on_session_changed(cx);
        }));
        let shared = Some(session.clone());
        self.pod_table.update(cx, |table, cx| {
            table.delegate_mut().set_session(shared.clone());
            cx.notify();
        });
        self.node_table.update(cx, |table, cx| {
            table.delegate_mut().set_session(shared.clone());
            cx.notify();
        });
        self.kind_table.update(cx, |table, cx| {
            table.delegate_mut().set_session(shared);
            cx.notify();
        });
        self.session = Some(session);
        cx.notify();
    }

    pub(crate) fn session(&self) -> Option<&Entity<ClusterSession>> {
        self.session.as_ref()
    }

    pub(crate) fn context_names(&self) -> Vec<String> {
        match &self.kubeconfig {
            KubeconfigState::Loaded(kubeconfig) => kubeconfig
                .contexts()
                .iter()
                .map(|context| context.name.clone())
                .collect(),
            KubeconfigState::Loading | KubeconfigState::Failed(_) => Vec::new(),
        }
    }

    pub(crate) fn switch_context(&mut self, name: &str, cx: &mut Context<Self>) {
        let KubeconfigState::Loaded(kubeconfig) = &self.kubeconfig else {
            return;
        };
        let kubeconfig = Arc::clone(kubeconfig);
        let Ok(summary) = kubeconfig.resolve_context(Some(name)) else {
            return;
        };
        let summary = summary.clone();
        let is_active = self
            .session
            .as_ref()
            .is_some_and(|session| session.read(cx).context() == summary.name);
        if is_active {
            return;
        }
        self.context_error = None;
        self.start_session(kubeconfig, &summary, None, cx);
    }

    pub(crate) fn set_namespace(&mut self, scope: NamespaceScope, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone() else {
            return;
        };
        self.close_drawer(cx);
        session.update(cx, |session, cx| session.set_scope(scope, cx));
    }

    /// Opens `screen`. The explorer watch follows it: it starts for a kind screen, is replaced on
    /// a kind switch, and is dropped when leaving to Pods or Nodes.
    pub(crate) fn show_screen(&mut self, screen: Screen, cx: &mut Context<Self>) {
        self.screen = screen;
        self.drawer.tab = DrawerTab::Overview;
        self.drawer.container_tab = ContainerTab::Info;
        if let Some(session) = &self.session {
            session.update(cx, |session, cx| {
                session.set_explorer_kind(screen.kind(), cx)
            });
        }
        self.kind_table.update(cx, |table, cx| {
            let is_switch = table.delegate_mut().set_kind(screen.kind());
            if is_switch {
                table.refresh(cx);
                // The new kind starts at the top-left. `scroll_to_row(0)` is not used: GPUI drops
                // `deferred_scroll_to_item` on an empty list, and the list is empty while it switches.
                table.scroll_to_col(0, cx);
                table
                    .vertical_scroll_handle
                    .0
                    .borrow()
                    .base_handle
                    .set_offset(Point::default());
            }
        });
        self.close_drawer(cx);
        self.log_dock.update(cx, |dock, cx| dock.unzoom(cx));
    }

    /// Opens the key's screen with its row selected, replacing the drawer. A list that is still
    /// loading keeps the key, and `on_session_changed` resolves it after the first snapshot; a
    /// loaded list without the row drops it.
    pub(crate) fn reveal(&mut self, key: ResourceKey, cx: &mut Context<Self>) {
        self.show_screen(key.screen(), cx);
        self.change_selection(Some(key), cx);
        self.sync_selection(cx);
    }

    fn retry(&mut self, cx: &mut Context<Self>) {
        if let Some(session) = &self.session {
            session.update(cx, |session, cx| session.retry(cx));
        }
    }

    // ---- drawer ----

    pub(crate) fn close_drawer(&mut self, cx: &mut Context<Self>) {
        self.change_selection(None, cx);
        self.pod_table
            .update(cx, |table, cx| table.clear_selection(cx));
        self.node_table
            .update(cx, |table, cx| table.clear_selection(cx));
        self.kind_table
            .update(cx, |table, cx| table.clear_selection(cx));
        cx.notify();
    }

    pub(crate) fn set_drawer_tab(&mut self, tab: DrawerTab, cx: &mut Context<Self>) {
        self.drawer.tab = tab;
        cx.notify();
    }

    pub(crate) fn toggle_drawer_expanded(&mut self, cx: &mut Context<Self>) {
        self.drawer.is_expanded = !self.drawer.is_expanded;
        cx.notify();
    }

    pub(crate) fn set_container_tab(&mut self, tab: ContainerTab, cx: &mut Context<Self>) {
        self.drawer.container_tab = tab;
        cx.notify();
    }

    pub(crate) fn select_container(&mut self, name: String, cx: &mut Context<Self>) {
        self.drawer.selected_container = Some(name);
        cx.notify();
    }

    /// A container row of the Overview tab opens the Containers tab on that container.
    pub(crate) fn open_container(&mut self, name: String, cx: &mut Context<Self>) {
        self.drawer.tab = DrawerTab::Containers;
        self.select_container(name, cx);
    }

    /// Opens the drawer of `key` on its YAML tab. When the key is not the selection it is revealed
    /// first; a vanished row clears the selection again, and then no drawer opens on YAML.
    pub(crate) fn open_yaml(&mut self, key: ResourceKey, cx: &mut Context<Self>) {
        if self.selected.as_ref() != Some(&key) {
            self.reveal(key.clone(), cx);
        }
        if self.selected.as_ref() != Some(&key) {
            return;
        }
        self.drawer.tab = DrawerTab::Yaml;
        cx.notify();
    }

    /// Keeps `drawer.yaml` for the shown subject only: the view lives exactly while the YAML tab of
    /// an open drawer is shown. It runs inside `render`, so it only assigns and never notifies, and
    /// it is the only place that creates a `YamlView`. Comparing by object alone is enough because
    /// every context or namespace switch closes the drawer first.
    fn sync_yaml_view(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(subject) = yaml_subject(self.selected.as_ref(), self.drawer.tab) else {
            self.drawer.yaml = None;
            return;
        };
        if let Some(view) = &self.drawer.yaml
            && view.read(cx).is_for(&subject)
        {
            return;
        }
        let Some(connection) = self.live(cx).map(|live| live.connection().clone()) else {
            self.drawer.yaml = None;
            return;
        };
        self.drawer.yaml = Some(cx.new(|cx| YamlView::new(connection, subject, window, cx)));
    }

    /// The YAML tab is shown and its first fetch has not finished. A failed fetch is settled.
    #[cfg(feature = "screenshot")]
    fn is_yaml_loading(&self, cx: &App) -> bool {
        yaml_subject(self.selected.as_ref(), self.drawer.tab).is_some()
            && self
                .drawer
                .yaml
                .as_ref()
                .is_none_or(|view| view.read(cx).is_loading())
    }

    // ---- selection ----

    /// Returns whether the subject changed. The same key again is a no-op: it is the
    /// `SelectRow` that a programmatic re-selection emits, and a snapshot must not reset the
    /// drawer. A new subject keeps the tab and width but forgets the selected container.
    fn change_selection(&mut self, key: Option<ResourceKey>, cx: &mut Context<Self>) -> bool {
        if key == self.selected {
            return false;
        }
        self.selected = key;
        self.drawer.selected_container = None;
        self.follow_event_subject(cx);
        cx.notify();
        true
    }

    /// Points the object events watch at the selected object. Stopping is immediate; a start
    /// waits for the selection to rest, and a newer selection cancels the pending start.
    fn follow_event_subject(&mut self, cx: &mut Context<Self>) {
        let next = self.selected.as_ref().and_then(event_subject);
        if next.is_none() {
            // A pending start must not outlive a selection that has no events, such as an event row.
            self.event_subject_task = None;
        }
        let running = self.live(cx).and_then(|live| live.event_subject()).cloned();
        match subject_change(running.as_ref(), next) {
            SubjectChange::Keep => {}
            SubjectChange::Stop => {
                self.event_subject_task = None;
                self.set_event_subject(None, cx);
            }
            SubjectChange::Start(subject) => {
                self.set_event_subject(None, cx);
                self.event_subject_task = Some(cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(DRAWER_SUBJECT_DELAY).await;
                    let _ = this.update(cx, |shell, cx| {
                        shell.event_subject_task = None;
                        shell.set_event_subject(Some(subject), cx);
                    });
                }));
            }
        }
    }

    fn set_event_subject(&mut self, subject: Option<InvolvedObject>, cx: &mut Context<Self>) {
        if let Some(session) = &self.session {
            session.update(cx, |session, cx| session.set_event_subject(subject, cx));
        }
    }

    /// Switches the Events screen between all events and warnings only. The drawer stays;
    /// `sync_selection` closes it when its row is filtered out.
    pub(crate) fn toggle_warnings_only(&mut self, cx: &mut Context<Self>) {
        let Some(session) = &self.session else {
            return;
        };
        session.update(cx, |session, cx| {
            session.set_event_filter(toggled(session.event_filter()), cx);
        });
    }

    fn on_pod_table_event(
        &mut self,
        table: &Entity<TableState<PodTableDelegate>>,
        event: &TableEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            TableEvent::SelectRow(row) => {
                let key = self
                    .live(cx)
                    .and_then(|live| live.pods.items().get(*row))
                    .map(ResourceKey::of_pod);
                if self.change_selection(key, cx) {
                    focus_table(table, window, cx);
                }
            }
            TableEvent::ClearSelection => {
                self.change_selection(None, cx);
            }
            _ => {}
        }
    }

    fn on_node_table_event(
        &mut self,
        table: &Entity<TableState<NodeTableDelegate>>,
        event: &TableEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            TableEvent::SelectRow(row) => {
                let key = self
                    .live(cx)
                    .and_then(|live| live.nodes.items().get(*row))
                    .map(ResourceKey::of_node);
                if self.change_selection(key, cx) {
                    focus_table(table, window, cx);
                }
            }
            TableEvent::ClearSelection => {
                self.change_selection(None, cx);
            }
            _ => {}
        }
    }

    fn on_kind_table_event(
        &mut self,
        table: &Entity<TableState<KindTableDelegate>>,
        event: &TableEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            TableEvent::SelectRow(row) => {
                let key = self.screen.kind().and_then(|kind| {
                    let explorer = self.live(cx)?.kind_list(kind)?;
                    let row = explorer.list.items().get(*row)?;
                    Some(ResourceKey::of_row(kind, row))
                });
                if self.change_selection(key, cx) {
                    focus_table(table, window, cx);
                }
            }
            TableEvent::ClearSelection => {
                self.change_selection(None, cx);
            }
            _ => {}
        }
    }

    fn live<'a>(&self, cx: &'a App) -> Option<&'a LiveCluster> {
        self.session.as_ref()?.read(cx).live()
    }

    fn on_session_changed(&mut self, cx: &mut Context<Self>) {
        self.apply_pending_launch_screen(cx);
        self.sync_selection(cx);
        cx.notify();
    }

    /// Keeps the table highlight and the drawer on the selected object after a snapshot has
    /// reordered, added, or removed rows. A loading list proves nothing; a failed one has no rows.
    fn sync_selection(&mut self, cx: &mut Context<Self>) {
        let Some(key) = self.selected.clone() else {
            return;
        };
        let Some(live) = self.live(cx) else {
            return;
        };
        match &key {
            ResourceKey::Pod { .. } => {
                let Some(found) = list_row_index(&live.pods, |pod| key.is_pod(pod)) else {
                    return;
                };
                let table = self.pod_table.clone();
                self.apply_selection_sync(&table, found, cx);
            }
            ResourceKey::Node { .. } => {
                let Some(found) = list_row_index(&live.nodes, |node| key.is_node(node)) else {
                    return;
                };
                let table = self.node_table.clone();
                self.apply_selection_sync(&table, found, cx);
            }
            ResourceKey::Kind { kind, .. } => {
                let Some(explorer) = live.kind_list(*kind) else {
                    return;
                };
                let Some(found) = list_row_index(&explorer.list, |row| key.is_row(*kind, row))
                else {
                    return;
                };
                let table = self.kind_table.clone();
                self.apply_selection_sync(&table, found, cx);
            }
        }
    }

    fn apply_selection_sync<D: TableDelegate>(
        &mut self,
        table: &Entity<TableState<D>>,
        found: Option<usize>,
        cx: &mut Context<Self>,
    ) {
        let table_row = table.read(cx).selected_row();
        match selection_sync(table_row, found) {
            SelectionSync::Keep => {}
            // Never reached with an unchanged index: `set_selected_row` scrolls and re-emits.
            SelectionSync::Move(row) => {
                table.update(cx, |table, cx| table.set_selected_row(row, cx))
            }
            SelectionSync::Clear => {
                self.change_selection(None, cx);
                table.update(cx, |table, cx| table.clear_selection(cx));
            }
        }
    }

    /// Opens the drawer that `--screen` asked for, once its list has loaded. The logs screens
    /// wait for `open_pending_logs`, which needs a window.
    fn apply_pending_launch_screen(&mut self, cx: &mut Context<Self>) {
        let Some(launch) = self
            .pending_launch_screen
            .filter(|launch| !launch.has_log_dock())
        else {
            return;
        };
        let Some(live) = self.live(cx) else {
            return;
        };
        let (is_loading, row) = match launch {
            LaunchScreen::NodeDrawer(_) => (
                live.nodes.is_loading(),
                (!live.nodes.items().is_empty()).then_some(0),
            ),
            LaunchScreen::KindDrawer(kind, _) => {
                let explorer = live.kind_list(kind);
                (
                    explorer.is_none_or(|explorer| explorer.list.is_loading()),
                    explorer
                        .is_some_and(|explorer| !explorer.list.items().is_empty())
                        .then_some(0),
                )
            }
            _ => (live.pods.is_loading(), pick_drawer_pod(live.pods.items())),
        };
        if is_loading {
            return;
        }
        self.pending_launch_screen = None;
        let Some(row) = row else {
            return;
        };
        match launch {
            LaunchScreen::NodeDrawer(_) => {
                let key = self
                    .live(cx)
                    .and_then(|live| live.nodes.items().get(row))
                    .map(ResourceKey::of_node);
                self.change_selection(key, cx);
                self.node_table
                    .update(cx, |table, cx| table.set_selected_row(row, cx));
            }
            LaunchScreen::KindDrawer(kind, _) => {
                let key = self
                    .live(cx)
                    .and_then(|live| live.kind_list(kind)?.list.items().get(row))
                    .map(|row| ResourceKey::of_row(kind, row));
                self.change_selection(key, cx);
                self.kind_table
                    .update(cx, |table, cx| table.set_selected_row(row, cx));
            }
            _ => {
                let key = self
                    .live(cx)
                    .and_then(|live| live.pods.items().get(row))
                    .map(ResourceKey::of_pod);
                self.change_selection(key, cx);
                self.pod_table
                    .update(cx, |table, cx| table.set_selected_row(row, cx));
            }
        }
    }

    /// Opens the log dock that `--screen` asked for, once the pod list has loaded. It runs
    /// from `render` because a new tab needs a window. The RBAC gate is skipped on purpose:
    /// on a denied cluster the error state is what a screenshot should show.
    fn open_pending_logs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(launch) = self
            .pending_launch_screen
            .filter(|launch| launch.has_log_dock())
        else {
            return;
        };
        let Some(live) = self.live(cx) else {
            return;
        };
        if live.pods.is_loading() {
            return;
        }
        let opened = pick_logs_pod(live.pods.items())
            .and_then(|row| live.pods.items().get(row))
            .and_then(LogTarget::of_pod)
            .map(|target| (live.connection().clone(), target));
        self.pending_launch_screen = None;
        let Some((connection, target)) = opened else {
            return;
        };
        let mode = if launch == LaunchScreen::LogsZoomed {
            DockMode::Zoomed
        } else {
            DockMode::Normal
        };
        self.log_dock.update(cx, |dock, cx| {
            dock.open(connection, target, window, cx);
            dock.set_mode(mode, cx);
        });
    }

    /// What the screenshot hook inspects to know when the screen shows its target.
    #[cfg(feature = "screenshot")]
    pub(crate) fn settle_input(&self, cx: &App) -> SettleInput {
        let target = match (&self.kubeconfig, &self.session) {
            (KubeconfigState::Loading, _) => TargetState::Loading,
            (KubeconfigState::Failed(_), _) | (KubeconfigState::Loaded(_), None) => {
                TargetState::Unavailable
            }
            (KubeconfigState::Loaded(_), Some(session)) => match session.read(cx).phase() {
                SessionPhase::Connecting { .. } => TargetState::Loading,
                SessionPhase::Failed { .. } => TargetState::Unavailable,
                SessionPhase::Live(live) => {
                    let (is_loading, has_failed) = match self.screen {
                        Screen::Pods => (live.pods.is_loading(), live.pods.failure().is_some()),
                        Screen::Nodes => (live.nodes.is_loading(), live.nodes.failure().is_some()),
                        // A missing explorer is the moment between a switch and its first watch.
                        Screen::Kind(kind) => {
                            live.kind_list(kind).map_or((true, false), |explorer| {
                                (
                                    explorer.list.is_loading(),
                                    explorer.list.failure().is_some(),
                                )
                            })
                        }
                    };
                    // A failed list shows an error screen, which is the target to capture.
                    if has_failed {
                        TargetState::Unavailable
                    } else if is_loading {
                        TargetState::Loading
                    } else {
                        TargetState::Loaded
                    }
                }
            },
        };
        // A drawer waits for the debounce, then for its events and its YAML.
        let is_content_pending = self.event_subject_task.is_some()
            || self
                .live(cx)
                .is_some_and(LiveCluster::is_object_events_loading)
            || self.is_yaml_loading(cx);
        // A logs screen is pending until its tab exists and has opened its stream.
        let is_log_pending = self
            .pending_launch_screen
            .is_some_and(LaunchScreen::has_log_dock)
            || self.log_dock.read(cx).is_connecting(cx);
        SettleInput {
            target,
            // An empty list opens no drawer, but the launch request is resolved then, so it settles.
            is_drawer_ready: is_drawer_ready(
                self.selected.is_some(),
                self.pending_launch_screen.is_some(),
                is_content_pending,
            ),
            is_log_pending,
        }
    }

    // ---- rendering ----

    fn navigation_counts(&self, cx: &App) -> NavigationCounts {
        let live = self.live(cx);
        NavigationCounts {
            pods: live.and_then(|live| live.pods.ready_count()),
            nodes: live.and_then(|live| live.nodes.ready_count()),
            explorer: live.and_then(LiveCluster::explorer_count),
        }
    }
}

impl Render for AppShell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.fit_table_widths(window, cx);
        self.open_pending_logs(window, cx);
        self.sync_yaml_view(window, cx);
        let theme = cx.theme();
        let counts = self.navigation_counts(cx);
        let session = self.session.as_ref().map(|session| session.read(cx));
        let is_kubeconfig_loading = matches!(self.kubeconfig, KubeconfigState::Loading);
        v_flex()
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .child(title_bar(self, cx))
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .child(sidebar(self.screen, &counts, self.live(cx), cx))
                    .child(self.render_workspace(cx)),
            )
            .child(status_bar(session, is_kubeconfig_loading, cx))
    }
}

/// The other filter.
fn toggled(filter: EventFilter) -> EventFilter {
    match filter {
        EventFilter::All => EventFilter::WarningsOnly,
        EventFilter::WarningsOnly => EventFilter::All,
    }
}

/// Row selection with a movable, non-sortable, fixed-order layout; a click on a header
/// selects nothing.
fn configure<D: TableDelegate>(table: TableState<D>) -> TableState<D> {
    table
        .row_selectable(true)
        .col_selectable(false)
        .col_resizable(true)
        .col_movable(false)
        .sortable(false)
        .cell_selectable(false)
}

/// Keeps the keyboard on the table (arrow keys move the selection, Esc closes the drawer).
fn focus_table<D: TableDelegate>(
    table: &Entity<TableState<D>>,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    let handle = table.read(cx).focus_handle(cx);
    window.focus(&handle, cx);
}

/// A kubeconfig or context error, plus a note when more `KUBECONFIG` entries were ignored:
/// the missing context may well be in one of them.
fn kubeconfig_error_message(message: String, has_ignored_entries: bool) -> String {
    if has_ignored_entries {
        format!("{message}. {IGNORED_KUBECONFIG_NOTE}")
    } else {
        message
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggled_event_filter_switches_between_all_and_warnings_only() {
        assert_eq!(toggled(EventFilter::All), EventFilter::WarningsOnly);
        assert_eq!(toggled(EventFilter::WarningsOnly), EventFilter::All);
    }

    #[test]
    fn kubeconfig_error_message_adds_note_only_for_ignored_entries() {
        assert_eq!(kubeconfig_error_message("boom".to_owned(), false), "boom");
        assert_eq!(
            kubeconfig_error_message("boom".to_owned(), true),
            "boom. Only the first KUBECONFIG entry is used; merging kubeconfigs is not supported"
        );
    }
}
