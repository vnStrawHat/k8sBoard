use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use cluster::{
    ContextSummary, EventFilter, HelmReleaseSummary, InvolvedObject, Kubeconfig, KubeconfigError,
    NamespaceScope, NetworkPolicySummary, SecretSummary,
};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::resizable::ResizableState;
use gpui_kit::component::table::{TableDelegate, TableEvent, TableState};
use gpui_kit::component::{ActiveTheme as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::{
    App, AppContext as _, Context, Entity, FocusHandle, Focusable as _, InteractiveElement as _,
    IntoElement, KeyBinding, ParentElement as _, Point, Render, SharedString, Styled as _,
    Subscription, Task, Window, px,
};

use crate::FocusQuickFilter;
use crate::cluster_registry::{
    ClusterProfile, ClusterRef, StartChoice, launch_last_used, start_choice, switcher_label,
};
#[cfg(feature = "screenshot")]
use crate::cluster_session::SessionPhase;
use crate::cluster_session::{
    ClusterSession, CountTrigger, FlowState, LiveCluster, LiveList, RbacState, RelatedList,
    denied_related_check, error_text,
};
use crate::custom_kind::CustomKind;
use crate::drawer::{
    ContainerTab, DRAWER_SUBJECT_DELAY, DrawerState, DrawerTab, MonitorCache, MonitorKey,
    MonitorRange, MonitorScope, MonitorState, drawer_tabs, shown_tab,
};
use crate::filter_bar::ToolkitState;
use crate::helm_release_view::{
    HelmReleaseView, HistoryState, ShowLatest, ValuesLayout, earlier_revision, helm_subject,
};
use crate::issue_table::IssueTableDelegate;
use crate::kind_row::{KindObject, PodOwner};
use crate::kind_table::KindTableDelegate;
use crate::kubelet_metrics::{KubeletDemand, KubeletSubject};
use crate::launch_options::{
    LaunchOptions, LaunchScreen, LoadedKubeconfigs, kubeconfig_chain, load_kubeconfigs,
    standalone_files,
};
use crate::log_dock::{DockMode, LogDock};
use crate::log_target::{LogTarget, NoLogTarget, check_logs_access};
use crate::monitor_data::{MonitorInput, MonitorSubject, monitor_data};
use crate::namespace_picker::{NamespacePickerState, PickerAnchor};
use crate::navigation::{NavigationCounts, issue_counts, sidebar};
use crate::node_table::NodeTableDelegate;
use crate::object_events::{SubjectChange, event_subject, subject_change};
use crate::permissions_view::PermissionsView;
use crate::pod_drawer::selected_container_index;
use crate::pod_table::PodTableDelegate;
use crate::related_objects::{RelatedSubject, related_subject};
use crate::resource_actions::{open_shell_reason, view_logs_reason};
use crate::resource_kind::ResourceKind;
#[cfg(feature = "screenshot")]
use crate::screenshot::{FeedProgress, kubelet_progress};
#[cfg(feature = "screenshot")]
use crate::screenshot::{SettleInput, TargetState, is_drawer_ready};
use crate::screenshot::{controller_owner_of, pick_drawer_pod, pick_logs_pod, pick_selected};
use crate::secret_clipboard::{
    CLIPBOARD_CLEAR_DELAY, ClearStep, ClipboardMark, clear_if_unchanged, next_clear_step,
};
use crate::secret_values::{
    PendingAction, SecretAction, SecretCopied, SecretValuesView, ValueAccess, fetcher,
    pending_action, value_access, values_subject,
};
use crate::settings::AppSettings;
use crate::status_bar::status_bar;
use crate::table_filter::{
    FilterChip, FilterPreset, TableFilter, parse_label_queries, quick_filter_text,
};
use crate::table_selection::{
    ResourceKey, SelectionSync, list_item_index, list_row_index, selection_sync,
};
use crate::table_sort::next_sort;
use crate::table_view::{FilteredTable, RowCheck, TableView};
use crate::title_bar::title_bar;
use crate::traffic_test_view::{TrafficTestView, traffic_defaults};
use crate::who_can_view::WhoCanView;
use crate::yaml_view::{YamlView, yaml_subject};

/// The width of the tool dialogs (Who can, Check permissions, Test traffic).
const DIALOG_WIDTH: f32 = 760.;

#[path = "workspace.rs"]
mod workspace;

#[cfg(test)]
#[path = "app_shell_tests.rs"]
mod app_shell_tests;

/// The logical column of the Events table that holds the reason.
const EVENT_REASON_COLUMN: usize = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Screen {
    /// What is broken, how much room is left, what changed; it lists no kind and opens no drawer.
    Overview,
    Pods,
    Nodes,
    /// The problems the engine found; it lists no explorer kind and opens no drawer.
    Issues,
    Kind(ResourceKind),
}

impl Screen {
    /// The explorer kind this screen lists, if it is a kind screen.
    pub(crate) fn kind(self) -> Option<ResourceKind> {
        match self {
            Self::Kind(kind) => Some(kind),
            Self::Overview | Self::Pods | Self::Nodes | Self::Issues => None,
        }
    }
}

enum KubeconfigState {
    Loading,
    /// The launch chain first (when it loaded), then each registry file.
    Loaded(Vec<Arc<Kubeconfig>>),
    Failed(String),
}

/// One switcher row: a context of a loaded kubeconfig.
pub(crate) struct SwitcherItem {
    pub(crate) cluster: ClusterRef,
    pub(crate) label: String,
    pub(crate) is_active: bool,
}

/// A `--screen custom:<crd-name>` request that has not met its CRD list yet.
struct CustomLaunch {
    crd_name: &'static str,
    /// The drawer tab to open on the first row; `None` opens the list only.
    tab: Option<DrawerTab>,
}

/// The kind a `--screen custom:` request names, or the failure text of a request whose CRD is not
/// Established. An interactive run falls back to the CRDs screen; a screenshot run fails with it.
fn resolve_custom_launch(kinds: &[CustomKind], crd_name: &str) -> Result<CustomKind, String> {
    kinds
        .iter()
        .find(|kind| kind.crd_name() == crd_name)
        .copied()
        .ok_or_else(|| format!("no Established CRD named {crd_name}"))
}

/// The screen that replaces a shown custom kind after the CRD list changed: the kind with the
/// same CRD name when its definition changed (a new kind), else the CRDs screen when the CRD is
/// gone. `None` while the shown kind is still served as it is, or when no custom kind is shown.
pub(crate) fn remapped_screen(screen: Screen, kinds: &[CustomKind]) -> Option<Screen> {
    let Screen::Kind(ResourceKind::Custom(shown)) = screen else {
        return None;
    };
    if kinds.contains(&shown) {
        return None;
    }
    let replacement = kinds
        .iter()
        .find(|kind| kind.crd_name() == shown.crd_name());
    Some(Screen::Kind(match replacement {
        Some(kind) => ResourceKind::Custom(*kind),
        None => ResourceKind::Crds,
    }))
}

/// The drawer watches that start once the selection has rested. Dropping it cancels the timer.
#[derive(Default)]
struct PendingSubjects {
    events: Option<InvolvedObject>,
    related: Option<RelatedSubject>,
    task: Option<Task<()>>,
}

impl PendingSubjects {
    fn is_empty(&self) -> bool {
        self.events.is_none() && self.related.is_none()
    }

    fn has_same_subjects(&self, other: &Self) -> bool {
        self.events == other.events && self.related == other.related
    }
}

/// A copied value waiting for its clear: what was written, which attempt this is, and the timer.
struct ArmedClear {
    mark: ClipboardMark,
    /// Counted from 0; a retry after an unreadable clipboard adds one.
    attempt: u32,
    _task: Task<()>,
}

/// Whether a failed clear may be tried again.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Retry {
    Allowed,
    Never,
}

/// What the command line asked for, used only by the first session.
struct RequestedStart {
    context: Option<String>,
    /// With `--kubeconfig`: the files the user named, the only source of a `last_used`.
    explicit_files: Option<Vec<PathBuf>>,
    namespace: Option<NamespaceScope>,
}

/// The root view: the six regions of the window, the screen choice, and the selection that
/// opens the drawer.
pub(crate) struct AppShell {
    kubeconfig: KubeconfigState,
    /// Set when the kubeconfig loaded but names no usable context; there is no session then.
    context_error: Option<String>,
    /// The context the session was started for, also while it is connecting or failed.
    active: Option<ContextSummary>,
    /// One line per kubeconfig file that was skipped; shown by the title-bar warning button.
    notices: Vec<String>,
    /// Whether the current session was already seen Live, so `last_used` is written once.
    has_reported_live: bool,
    session: Option<Entity<ClusterSession>>,
    _session_observer: Option<Subscription>,
    screen: Screen,
    pod_table: Entity<TableState<PodTableDelegate>>,
    node_table: Entity<TableState<NodeTableDelegate>>,
    issue_table: Entity<TableState<IssueTableDelegate>>,
    kind_table: Entity<TableState<KindTableDelegate>>,
    _table_subscriptions: Vec<Subscription>,
    /// The drawer is open exactly while this is set.
    selected: Option<ResourceKey>,
    drawer: DrawerState,
    /// The debounced start of the drawer watches (object events, related objects) that is waiting
    /// for the selection to rest. Replacing or dropping it cancels it.
    pending_subjects: Option<PendingSubjects>,
    /// A reveal that waits for its list to load before it clears a filter hiding the row.
    pending_reveal: Option<ResourceKey>,
    log_dock: Entity<LogDock>,
    /// Keeps the dock height across zoom and minimize, which unmount the split.
    dock_split: Entity<ResizableState>,
    /// A `--screen` drawer or logs request that waits for its list to load.
    pending_launch_screen: Option<LaunchScreen>,
    /// `--select`: the row that request opens instead of the first one.
    launch_select: Option<String>,
    /// A `--screen` tool dialog that opens once the session is live.
    pending_dialog_launch: Option<LaunchScreen>,
    /// The open Who can dialog, which a screenshot waits on.
    #[cfg(feature = "screenshot")]
    who_can: Option<gpui_kit::WeakEntity<WhoCanView>>,
    /// The open Check permissions dialog, which a screenshot waits on.
    #[cfg(feature = "screenshot")]
    permissions: Option<gpui_kit::WeakEntity<PermissionsView>>,
    /// The open Test traffic dialog, which a screenshot waits on.
    #[cfg(feature = "screenshot")]
    traffic: Option<gpui_kit::WeakEntity<TrafficTestView>>,
    /// `--screen custom:<crd-name>`: waits for the CRD list, then opens the kind.
    pending_custom_launch: Option<CustomLaunch>,
    /// Why a `--screen custom:` request found no kind; a screenshot run fails with it.
    #[cfg(feature = "screenshot")]
    launch_failure: Option<String>,
    requested: RequestedStart,
    /// The `/` input. Its text belongs to the screen in `quick_filter_screen`.
    quick_filter: Entity<InputState>,
    /// The screen whose filter text the input shows; `None` makes the next render load it.
    quick_filter_screen: Option<Screen>,
    /// Keeps the keyboard inside the `AppShell` key context, so `/` works before any click.
    focus_handle: FocusHandle,
    _quick_filter_events: Subscription,
    /// Puts the focus back inside the key context when the focused element disappears.
    _focus_lost: Subscription,
    /// The picker popover: which trigger is open, and the draft.
    namespace_picker: NamespacePickerState,
    /// Whether Reveal and Copy work: decided once from the launch options, and the only thing the
    /// values view and the menus read (`value_access`).
    secret_value_access: ValueAccess,
    /// The clear of the last copied value. It lives here, not in the drawer, so it survives the
    /// drawer closing and a context switch. A new copy replaces it.
    clipboard_clear: Option<ArmedClear>,
    /// Clears an armed copy when the app quits (best effort).
    _clipboard_quit: Subscription,
    /// Re-renders the title bar when a setting or a settings notice changes.
    _settings_observer: Subscription,
}

/// The key bindings of the shell. `!Input` keeps `/` typable in every input, the YAML editor
/// included.
pub(crate) fn bind_keys(cx: &mut App) {
    cx.bind_keys([KeyBinding::new(
        "/",
        FocusQuickFilter,
        Some("AppShell && !Input"),
    )]);
}

impl AppShell {
    pub(crate) fn new(options: LaunchOptions, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let secret_value_access = value_access(&options);
        let is_explicit = options.kubeconfig.is_some();
        let chain = kubeconfig_chain(
            options.kubeconfig,
            std::env::var_os("KUBECONFIG"),
            std::env::home_dir(),
        );
        let explicit_files = is_explicit.then(|| chain.clone());
        let standalone = standalone_files(&AppSettings::get(cx).registry.kubeconfigs, &chain);
        let kubeconfig = if chain.is_empty() && standalone.is_empty() {
            KubeconfigState::Failed(
                "no kubeconfig found: pass --kubeconfig, set KUBECONFIG, or create ~/.kube/config"
                    .to_owned(),
            )
        } else {
            Self::load_kubeconfigs(chain, standalone, cx);
            KubeconfigState::Loading
        };

        let shell = cx.weak_entity();
        let log_dock = cx.new(|_| LogDock::new(shell.clone()));
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
        let issue_table = cx.new(|cx| {
            configure(TableState::new(
                IssueTableDelegate::new(log_dock.downgrade(), shell.clone()),
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

        let quick_filter = cx.new(|cx| InputState::new(window, cx).placeholder("Filter  /"));
        let quick_filter_events =
            cx.subscribe_in(&quick_filter, window, Self::on_quick_filter_event);
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle, cx);
        // A focused input or editor that leaves the tree (a drawer closing, a screen change)
        // would leave nothing focused, and `/` only matches inside the `AppShell` context.
        let focus_lost = cx.on_focus_lost(window, |shell, window, cx| {
            let target = window
                .focus_lost_restore_target(cx)
                .unwrap_or_else(|| shell.focus_handle.clone());
            window.focus(&target, cx);
        });

        let clipboard_quit = cx.on_app_quit(|shell, cx| {
            shell.clear_armed_clipboard(Retry::Never, cx);
            std::future::ready(())
        });
        let mut drawer = DrawerState::new();
        drawer.tab = options.screen.drawer_tab().unwrap_or(DrawerTab::Overview);
        // W4b shows the Containers tab expanded, and W4c the Monitor tab.
        drawer.is_expanded = options.screen.opens_expanded();
        let launch_filter = options.filter;
        let launch_select = options.select;
        let mut shell = Self {
            kubeconfig,
            context_error: None,
            active: None,
            notices: Vec::new(),
            has_reported_live: false,
            session: None,
            _session_observer: None,
            screen: options.screen.screen(),
            pod_table,
            node_table,
            issue_table,
            kind_table,
            _table_subscriptions: table_subscriptions,
            selected: None,
            drawer,
            pending_subjects: None,
            pending_reveal: None,
            log_dock,
            dock_split,
            // A custom launch resolves against the CRD list first, then sets this.
            pending_launch_screen: (options.screen.has_drawer()
                || options.screen.has_log_dock()
                || options.screen.checks_rows())
            .then_some(options.screen)
            .filter(|screen| !matches!(screen, LaunchScreen::Custom { .. })),
            launch_select,
            pending_dialog_launch: options.screen.opens_dialog().then_some(options.screen),
            #[cfg(feature = "screenshot")]
            who_can: None,
            #[cfg(feature = "screenshot")]
            permissions: None,
            #[cfg(feature = "screenshot")]
            traffic: None,
            pending_custom_launch: match options.screen {
                LaunchScreen::Custom { crd_name, tab } => Some(CustomLaunch { crd_name, tab }),
                _ => None,
            },
            #[cfg(feature = "screenshot")]
            launch_failure: None,
            requested: RequestedStart {
                context: options.context,
                explicit_files,
                namespace: options.namespace,
            },
            quick_filter,
            quick_filter_screen: None,
            focus_handle,
            _quick_filter_events: quick_filter_events,
            _focus_lost: focus_lost,
            namespace_picker: NamespacePickerState::default(),
            secret_value_access,
            clipboard_clear: None,
            _clipboard_quit: clipboard_quit,
            _settings_observer: cx.observe_global::<AppSettings>(|_, cx| cx.notify()),
        };
        if let Some(text) = launch_filter {
            shell.apply_launch_filter(&text, cx);
        }
        shell
    }

    /// Reading the files is blocking I/O, so it runs on the background executor, not on the
    /// UI thread and not on tokio.
    fn load_kubeconfigs(chain: Vec<PathBuf>, standalone: Vec<PathBuf>, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let loaded = cx
                .background_executor()
                .spawn(async move { load_kubeconfigs(&chain, &standalone) })
                .await;
            let _ = this.update(cx, |shell, cx| shell.finish_kubeconfig_load(loaded, cx));
        })
        .detach();
    }

    fn finish_kubeconfig_load(
        &mut self,
        loaded: Result<LoadedKubeconfigs, KubeconfigError>,
        cx: &mut Context<Self>,
    ) {
        let loaded = match loaded {
            Ok(loaded) => loaded,
            Err(error) => {
                self.kubeconfig = KubeconfigState::Failed(error_text(&error));
                cx.notify();
                return;
            }
        };
        self.notices = loaded.notices;
        self.kubeconfig = KubeconfigState::Loaded(loaded.kubeconfigs.clone());
        let requested = self.requested.context.take();
        let explicit_files = self.requested.explicit_files.take();
        let saved = AppSettings::get(cx).registry.last_used.as_ref();
        let last_used = launch_last_used(saved, explicit_files.as_deref()).cloned();
        match resolve_start(
            &loaded.kubeconfigs,
            requested.as_deref(),
            last_used.as_ref(),
        ) {
            Ok((kubeconfig, summary)) => {
                let namespace = self.requested.namespace.take();
                self.start_session(kubeconfig, &summary, namespace, cx);
            }
            Err(error) => self.context_error = Some(error_text(&error)),
        }
        cx.notify();
    }

    /// Replaces the session. Dropping the old one cancels every task and watch it owns.
    fn start_session(
        &mut self,
        kubeconfig: Arc<Kubeconfig>,
        summary: &ContextSummary,
        namespace: Option<NamespaceScope>,
        cx: &mut Context<Self>,
    ) {
        let kind = self.screen.kind();
        let is_switch = self.session.is_some();
        self.active = Some(summary.clone());
        self.has_reported_live = false;
        self.close_drawer(cx);
        self.log_dock.update(cx, |dock, cx| dock.close_all(cx));
        // The definitions seen so far move on, so a context switch reuses them (decision 16).
        let cache = self
            .session
            .as_ref()
            .map(|session| session.update(cx, |session, _| session.take_custom_kind_cache()))
            .unwrap_or_default();
        let session =
            cx.new(|cx| ClusterSession::new(kubeconfig, summary, namespace, kind, cache, cx));
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
        self.issue_table.update(cx, |table, cx| {
            table.delegate_mut().set_session(shared.clone());
            cx.notify();
        });
        self.kind_table.update(cx, |table, cx| {
            table.delegate_mut().set_session(shared);
            cx.notify();
        });
        let weak_session = session.downgrade();
        self.log_dock
            .update(cx, |dock, _| dock.set_session(Some(weak_session)));
        self.session = Some(session);
        // A filter set for one cluster would surprise in another. The first session keeps the
        // filter of `--filter`.
        if is_switch {
            self.clear_all_filters(cx);
            self.namespace_picker = NamespacePickerState::default();
        }
        cx.notify();
    }

    /// Clears the notice behind the title-bar warning button.
    pub(crate) fn dismiss_notices(&mut self, cx: &mut Context<Self>) {
        self.notices.clear();
        AppSettings::dismiss_notice(cx);
    }

    /// The skipped-kubeconfig lines for the warning button.
    pub(crate) fn notices(&self) -> &[String] {
        &self.notices
    }

    pub(crate) fn session(&self) -> Option<&Entity<ClusterSession>> {
        self.session.as_ref()
    }

    /// The loaded contexts for the switcher, in load order.
    pub(crate) fn switcher_items(&self, cx: &App) -> Vec<SwitcherItem> {
        let KubeconfigState::Loaded(kubeconfigs) = &self.kubeconfig else {
            return Vec::new();
        };
        let registry = &AppSettings::get(cx).registry;
        let summaries: Vec<&ContextSummary> = kubeconfigs
            .iter()
            .flat_map(|kubeconfig| kubeconfig.contexts())
            .collect();
        summaries
            .iter()
            .map(|summary| {
                let profile = registry.profile(summary);
                let is_duplicate_name = summaries
                    .iter()
                    .any(|other| other.name == summary.name && other.source != summary.source);
                let cluster = ClusterRef::of(summary);
                let is_active = self
                    .active
                    .as_ref()
                    .is_some_and(|active| cluster.is_of(active));
                SwitcherItem {
                    label: switcher_label(&profile, summary, is_duplicate_name),
                    is_active,
                    cluster,
                }
            })
            .collect()
    }

    /// The active context's profile; `None` before a session starts.
    pub(crate) fn active_profile(&self, cx: &App) -> Option<ClusterProfile> {
        let active = self.active.as_ref()?;
        Some(AppSettings::get(cx).registry.profile(active))
    }

    pub(crate) fn switch_cluster(&mut self, cluster: &ClusterRef, cx: &mut Context<Self>) {
        let KubeconfigState::Loaded(kubeconfigs) = &self.kubeconfig else {
            return;
        };
        let Some((kubeconfig, summary)) = find_cluster(kubeconfigs, cluster) else {
            return;
        };
        let is_active = self
            .active
            .as_ref()
            .is_some_and(|active| cluster.is_of(active));
        if is_active {
            return;
        }
        self.context_error = None;
        self.start_session(kubeconfig, &summary, None, cx);
    }

    pub(crate) fn namespace_picker(&self) -> &NamespacePickerState {
        &self.namespace_picker
    }

    /// Opens the picker from `anchor`, with the current scope ticked.
    pub(crate) fn open_namespace_picker(&mut self, anchor: PickerAnchor, cx: &mut Context<Self>) {
        let Some(scope) = self.live(cx).map(|live| live.scope.clone()) else {
            return;
        };
        self.namespace_picker.open(anchor, &scope);
        cx.notify();
    }

    /// `anchor`'s popover closed: only that anchor's own picker is closed.
    pub(crate) fn close_namespace_picker(&mut self, anchor: PickerAnchor, cx: &mut Context<Self>) {
        self.namespace_picker.close(anchor);
        cx.notify();
    }

    pub(crate) fn toggle_picker_namespace(&mut self, name: &str, cx: &mut Context<Self>) {
        self.namespace_picker.toggle(name);
        cx.notify();
    }

    pub(crate) fn clear_picker_draft(&mut self, cx: &mut Context<Self>) {
        self.namespace_picker.clear();
        cx.notify();
    }

    /// Apply, a namespace name, or All: sets the scope and closes the picker.
    pub(crate) fn apply_namespace_scope(&mut self, scope: NamespaceScope, cx: &mut Context<Self>) {
        self.namespace_picker.dismiss();
        self.set_namespace(scope, cx);
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
        self.drawer.monitor = MonitorState::new();
        if let Some(session) = &self.session {
            session.update(cx, |session, cx| {
                session.set_explorer_kind(screen.kind(), cx);
                session.set_issues_visible(screen == Screen::Issues);
                session.refresh_kind_counts(CountTrigger::Navigation, cx);
                if screen == Screen::Kind(ResourceKind::Crds) {
                    session.refresh_custom_counts(cx);
                }
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
        self.rebuild_visible_view(cx, |_| {});
        self.close_drawer(cx);
        self.log_dock.update(cx, |dock, cx| dock.unzoom(cx));
    }

    /// Opens the key's screen with its row selected, replacing the drawer. A filter that hides
    /// the row is cleared, or the drawer would close at once. A list that is still loading keeps
    /// the key, and `on_session_changed` resolves it after the first snapshot; a loaded list
    /// without the row drops it.
    pub(crate) fn reveal(&mut self, key: ResourceKey, cx: &mut Context<Self>) {
        self.reveal_then(key, cx, |_, _| {});
    }

    /// `reveal`, then `then` once the selection stands. The selection is made after the
    /// ClearSelection events that `show_screen` queues, which would erase a selection made now
    /// before a still-loading list could confirm it; so a step that reads or builds on the
    /// selection must run in the same deferred closure, not after this call returns.
    pub(crate) fn reveal_then(
        &mut self,
        key: ResourceKey,
        cx: &mut Context<Self>,
        then: impl FnOnce(&mut Self, &mut Context<Self>) + 'static,
    ) {
        self.show_screen(key.screen(), cx);
        let shell = cx.weak_entity();
        cx.defer(move |cx| {
            let _ = shell.update(cx, |shell, cx| {
                shell.pending_reveal = Some(key.clone());
                shell.change_selection(Some(key), cx);
                shell.apply_pending_reveal(cx);
                shell.sync_selection(cx);
                then(shell, cx);
            });
        });
    }

    /// Runs `step` with `key` selected: at once when it already is, else after a reveal. A row
    /// that vanished clears the selection again, and then `step` does not run.
    fn when_selected(
        &mut self,
        key: ResourceKey,
        cx: &mut Context<Self>,
        step: impl FnOnce(&mut Self, &mut Context<Self>) + 'static,
    ) {
        if self.selected.as_ref() == Some(&key) {
            step(self, cx);
            return;
        }
        let wanted = key.clone();
        self.reveal_then(key, cx, move |shell, cx| {
            if shell.selected.as_ref() == Some(&wanted) {
                step(shell, cx);
            }
        });
    }

    /// Clears the filter of the revealed row's table when it hides the row, once the list has
    /// loaded. Waits while the list loads; forgets a reveal the selection has moved away from.
    fn apply_pending_reveal(&mut self, cx: &mut Context<Self>) {
        let Some(key) = self.pending_reveal.clone() else {
            return;
        };
        if self.selected.as_ref() != Some(&key) {
            self.pending_reveal = None;
            return;
        }
        let Some(live) = self.live(cx) else {
            return;
        };
        let found = match &key {
            ResourceKey::Pod { .. } => list_item_index(&live.pods, |pod| key.is_pod(pod)),
            ResourceKey::Node { .. } => list_item_index(&live.nodes, |node| key.is_node(node)),
            ResourceKey::Kind { kind, .. } => live
                .kind_list(*kind)
                .and_then(|explorer| list_item_index(&explorer.list, |row| key.is_row(*kind, row))),
        };
        // `None`: still loading, or the explorer has not switched to the kind yet.
        let Some(found) = found else {
            return;
        };
        self.pending_reveal = None;
        if let Some(item) = found {
            self.rebuild_visible_view(cx, move |view| view.reveal(item));
        }
    }

    /// Opens the Who can… dialog. `namespace: None` asks about cluster-wide grants. The view is
    /// created here once; the dialog builder only clones the handle on every frame.
    pub(crate) fn open_who_can(
        &mut self,
        query: Option<String>,
        namespace: Option<String>,
        check_now: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.session.clone() else {
            return;
        };
        let shell = cx.weak_entity();
        let view =
            cx.new(|cx| WhoCanView::new(shell, &session, query, namespace, check_now, window, cx));
        #[cfg(feature = "screenshot")]
        {
            self.who_can = Some(view.downgrade());
        }
        window.open_dialog(cx, move |dialog, _, _| {
            dialog
                .title("Who can…")
                .w(px(DIALOG_WIDTH))
                .child(view.clone())
        });
    }

    /// Opens the Check permissions dialog. `subject` is the text to prefill (`None` is You);
    /// `namespace: None` is cluster-wide grants for a subject other than You.
    pub(crate) fn open_permissions(
        &mut self,
        subject: Option<String>,
        namespace: Option<String>,
        check_now: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.session.clone() else {
            return;
        };
        let shell = cx.weak_entity();
        let view = cx.new(|cx| {
            PermissionsView::new(shell, &session, subject, namespace, check_now, window, cx)
        });
        #[cfg(feature = "screenshot")]
        {
            self.permissions = Some(view.downgrade());
        }
        window.open_dialog(cx, move |dialog, _, _| {
            dialog
                .title("Check permissions")
                .w(px(DIALOG_WIDTH))
                .child(view.clone())
        });
    }

    /// Opens the Test traffic dialog with the defaults for `policy` (the pod it selects as the
    /// destination); without one, the first two pods.
    pub(crate) fn open_traffic_test(
        &mut self,
        policy: Option<&NetworkPolicySummary>,
        check_now: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.session.clone() else {
            return;
        };
        let form = match session.read(cx).live() {
            Some(live) => traffic_defaults(live.pods.items(), policy),
            None => return,
        };
        let shell = cx.weak_entity();
        let view = cx.new(|cx| TrafficTestView::new(shell, &session, form, check_now, window, cx));
        #[cfg(feature = "screenshot")]
        {
            self.traffic = Some(view.downgrade());
        }
        window.open_dialog(cx, move |dialog, _, _| {
            dialog
                .title("Test traffic")
                .w(px(DIALOG_WIDTH))
                .child(view.clone())
        });
    }

    /// The service account whose drawer is open, as `(subject text, namespace)`.
    pub(crate) fn drawer_account(&self) -> Option<(String, String)> {
        match &self.selected {
            Some(ResourceKey::Kind {
                kind: ResourceKind::ServiceAccounts,
                namespace: Some(namespace),
                name,
            }) => Some((format!("sa {namespace}/{name}"), namespace.clone())),
            _ => None,
        }
    }

    /// The namespace the top-level tool buttons start in: the first one of the scope, else
    /// cluster-wide.
    pub(crate) fn tool_namespace(&self, cx: &App) -> Option<String> {
        self.live(cx)?.scope.namespaces().first().cloned()
    }

    /// Opens the `--screen` dialog once the session is live. It runs from `render` because a
    /// dialog needs a window.
    fn open_pending_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(launch) = self.pending_dialog_launch else {
            return;
        };
        if self.live(cx).is_none() {
            return;
        }
        let namespace = self.tool_namespace(cx);
        match launch {
            LaunchScreen::WhoCan => {
                self.open_who_can(Some("get secrets".to_owned()), namespace, true, window, cx);
            }
            LaunchScreen::CheckPermissions => {
                self.open_permissions(None, namespace, true, window, cx);
            }
            LaunchScreen::AccountPermissions => {
                let Some(account) = self.launch_account(cx) else {
                    return;
                };
                let (subject, namespace) = match account {
                    Some((subject, namespace)) => (Some(subject), Some(namespace)),
                    None => (None, namespace),
                };
                self.open_permissions(subject, namespace, true, window, cx);
            }
            LaunchScreen::TestTraffic => {
                let Some(policy) = self.launch_policy(cx) else {
                    return;
                };
                self.open_traffic_test(policy.as_ref(), true, window, cx);
            }
            _ => {}
        }
        self.pending_dialog_launch = None;
    }

    /// `--screen test-traffic`: the first policy the table shows (or the one `--select` names),
    /// whose selected pod is the destination. The outer `None` means the pods or the policies
    /// have not loaded yet; the inner `None` means there is no policy, so the defaults are plain.
    fn launch_policy(&self, cx: &App) -> Option<Option<NetworkPolicySummary>> {
        let live = self.live(cx)?;
        if live.pods.is_loading() {
            return None;
        }
        let rows = live
            .kind_list(ResourceKind::NetworkPolicies)?
            .list
            .ready_items()?;
        if rows.is_empty() {
            return Some(None);
        }
        let item = match self.launch_select.as_deref() {
            Some(select) => pick_selected(
                select,
                rows.iter()
                    .map(|row| (row.namespace.as_deref(), row.name.as_str())),
            ),
            None => self.kind_table.read(cx).delegate().view()?.item_index(0),
        };
        let policy = item
            .and_then(|item| rows.get(item))
            .and_then(|row| match &row.object {
                KindObject::NetworkPolicy(policy) => Some(policy.clone()),
                _ => None,
            });
        Some(policy)
    }

    /// `--screen account-permissions`: the first service account the table shows (or the one
    /// `--select` names), as `(subject text, namespace)`. The outer `None` means the list has not
    /// loaded yet; the inner `None` means it holds no such account, so the dialog opens for You.
    fn launch_account(&self, cx: &App) -> Option<Option<(String, String)>> {
        let explorer = self.live(cx)?.kind_list(ResourceKind::ServiceAccounts)?;
        let rows = explorer.list.ready_items()?;
        let item = match self.launch_select.as_deref() {
            Some(select) => pick_selected(
                select,
                rows.iter()
                    .map(|row| (row.namespace.as_deref(), row.name.as_str())),
            ),
            None => {
                let view = self.kind_table.read(cx).delegate().view()?;
                view.item_index(0)
            }
        };
        let account = item.and_then(|item| rows.get(item)).and_then(|row| {
            let namespace = row.namespace.clone()?;
            Some((format!("sa {namespace}/{}", row.name), namespace))
        });
        Some(account)
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
        self.issue_table
            .update(cx, |table, cx| table.clear_selection(cx));
        self.kind_table
            .update(cx, |table, cx| table.clear_selection(cx));
        cx.notify();
    }

    pub(crate) fn set_drawer_tab(&mut self, tab: DrawerTab, cx: &mut Context<Self>) {
        self.drawer.tab = tab;
        if tab != DrawerTab::Overview {
            self.drop_secret_values();
        }
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

    pub(crate) fn set_monitor_range(&mut self, range: MonitorRange, cx: &mut Context<Self>) {
        self.drawer.monitor.range = range;
        cx.notify();
    }

    pub(crate) fn set_monitor_scope(&mut self, scope: MonitorScope, cx: &mut Context<Self>) {
        self.drawer.monitor.scope = scope;
        cx.notify();
    }

    pub(crate) fn toggle_monitor_table(&mut self, cx: &mut Context<Self>) {
        self.drawer.monitor.is_table = !self.drawer.monitor.is_table;
        cx.notify();
    }

    /// Keeps `drawer.monitor.cache` for what the open drawer shows, and frees it while no Monitor
    /// is shown. It runs inside `render`, so it only assigns and never notifies. The series are
    /// rebuilt only when the key changes (a new tick, range, scope, or subject), so a hover repaint
    /// or an unrelated notify reuses them.
    fn refresh_monitor_cache(&mut self, cx: &App) {
        let is_container_tab = self.drawer.tab == DrawerTab::Containers
            && self.drawer.container_tab == ContainerTab::Monitor;
        if !self.shows_monitor() {
            self.drawer.monitor.cache = None;
            return;
        }
        let (Some(subject), Some(live)) = (self.selected.clone(), self.live(cx)) else {
            return;
        };
        let container = self.monitor_container(&subject, live, is_container_tab);
        let ticks = match subject {
            ResourceKey::Node { .. } => live.metrics.nodes.history.tick_count(),
            ResourceKey::Pod { .. } | ResourceKey::Kind { .. } => {
                live.metrics.pods.history.tick_count()
            }
        };
        let key = MonitorKey {
            subject,
            container,
            ticks,
            kubelet_ticks: live.metrics.kubelet.history.tick_count(),
            kubelet_status: live.metrics.kubelet.status.clone(),
            scope: self.drawer.monitor.scope.clone(),
            range: self.drawer.monitor.range,
        };
        if self
            .drawer
            .monitor
            .cache
            .as_ref()
            .is_some_and(|cache| cache.key == key)
        {
            return;
        }
        let Some(monitor_subject) =
            Self::monitor_subject(&key.subject, key.container.as_deref(), live)
        else {
            return;
        };
        let data = monitor_data(&MonitorInput {
            subject: monitor_subject,
            scope: &key.scope,
            range: key.range,
            pods: live.pods.items(),
            pod_history: &live.metrics.pods.history,
            node_history: &live.metrics.nodes.history,
            kubelet: &live.metrics.kubelet,
            nodes: live.nodes.items(),
            is_all_namespaces: live.scope == NamespaceScope::All,
        });
        self.drawer.monitor.cache = Some(MonitorCache { key, data });
    }

    /// The container whose Monitor sub-tab is shown (`is_shown`), else `None`.
    fn monitor_container(
        &self,
        subject: &ResourceKey,
        live: &LiveCluster,
        is_shown: bool,
    ) -> Option<String> {
        if !is_shown {
            return None;
        }
        let pod = live.pods.items().iter().find(|pod| subject.is_pod(pod))?;
        let index = selected_container_index(pod, &self.drawer)?;
        Some(pod.containers[index].name.clone())
    }

    /// The subject of the open drawer, read from the live lists. `container` is the container
    /// sub-tab's container; `None` for a pod's own Monitor tab.
    fn monitor_subject<'a>(
        key: &ResourceKey,
        container: Option<&'a str>,
        live: &'a LiveCluster,
    ) -> Option<MonitorSubject<'a>> {
        match key {
            ResourceKey::Pod { .. } => {
                let pod = live.pods.items().iter().find(|pod| key.is_pod(pod))?;
                Some(match container {
                    Some(container) => MonitorSubject::Container { pod, container },
                    None => MonitorSubject::Pod(pod),
                })
            }
            ResourceKey::Node { .. } => live
                .nodes
                .items()
                .iter()
                .find(|node| key.is_node(node))
                .map(MonitorSubject::Node),
            ResourceKey::Kind { kind, .. } => {
                let row = live
                    .kind_list(*kind)?
                    .list
                    .items()
                    .iter()
                    .find(|row| key.is_row(*kind, row))?;
                row.related_pods.as_ref().map(MonitorSubject::Workload)
            }
        }
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

    /// Opens the drawer of `key` on `tab`. When the key is not the selection it is revealed first; a
    /// vanished row clears the selection again, and then no drawer opens on that tab.
    pub(crate) fn open_drawer_tab(
        &mut self,
        key: ResourceKey,
        tab: DrawerTab,
        cx: &mut Context<Self>,
    ) {
        self.when_selected(key, cx, move |shell, cx| {
            shell.drawer.tab = tab;
            cx.notify();
        });
    }

    /// Opens the drawer of `key` on the Values tab for `revision`, in `layout`. The key is revealed
    /// first when it is not the selection; a vanished row clears the selection, and then nothing
    /// opens. The layout waits for the view of its revision (`sync_helm_view`).
    pub(crate) fn open_helm_values(
        &mut self,
        key: ResourceKey,
        revision: u32,
        layout: ValuesLayout,
        cx: &mut Context<Self>,
    ) {
        let subject = key.clone();
        // After the selection: choosing a subject forgets the revision and the layout.
        self.when_selected(key, cx, move |shell, cx| {
            shell.drawer.helm_revision = Some(revision);
            shell.drawer.tab = DrawerTab::Values;
            shell.drawer.pending_helm_layout = Some((subject, layout));
            cx.notify();
        });
    }

    /// Whether Reveal and Copy work. The values view and the menus read only this.
    pub(crate) fn secret_value_access(&self) -> ValueAccess {
        self.secret_value_access
    }

    /// A menu's Reveal or Copy: opens the drawer of `key` on its Overview tab and hands the action
    /// to the values view as soon as it exists (`sync_secret_values`).
    pub(crate) fn run_secret_action(
        &mut self,
        key: ResourceKey,
        action: SecretAction,
        cx: &mut Context<Self>,
    ) {
        if self.secret_value_access == ValueAccess::Blocked {
            return;
        }
        let subject = key.clone();
        self.when_selected(key, cx, move |shell, cx| {
            shell.drawer.tab = DrawerTab::Overview;
            shell.drawer.pending_secret_action = Some((subject, action));
            cx.notify();
        });
    }

    /// Drops the values view (wiping every revealed value) and any action waiting for it.
    fn drop_secret_values(&mut self) {
        self.drawer.secret_values = None;
        self.drawer.pending_secret_action = None;
    }

    /// Keeps `drawer.secret_values` for the shown Secret only: the view lives exactly while the
    /// Overview tab of its drawer is shown. It runs inside `render`, so it only assigns and never
    /// notifies, and it is the only place that creates the view.
    fn sync_secret_values(&mut self, cx: &mut Context<Self>) {
        let subject = values_subject(self.selected.as_ref(), self.drawer.tab);
        let Some(subject) = subject else {
            self.drop_secret_values();
            return;
        };
        let existing = self
            .drawer
            .secret_values
            .clone()
            .filter(|view| view.read(cx).is_for(&subject));
        // The common frame: the view exists, so only the key list is read (and copied only when
        // it changed); the connection and names are cloned for a new view alone.
        if let Some(view) = existing {
            let keys = self
                .live(cx)
                .and_then(|live| secret_of(live, &subject).map(|secret| secret.keys.clone()));
            let Some(keys) = keys else {
                self.drop_secret_values();
                return;
            };
            view.update(cx, |view, _| view.set_keys(&keys));
        } else {
            let found = self.live(cx).and_then(|live| {
                let secret = secret_of(live, &subject)?;
                Some((
                    live.connection().clone(),
                    secret.keys.clone(),
                    secret.namespace.clone(),
                    secret.name.clone(),
                ))
            });
            let Some((connection, keys, namespace, name)) = found else {
                self.drop_secret_values();
                return;
            };
            let access = self.secret_value_access;
            let fetch = fetcher(connection, namespace, name);
            let view = cx.new(|_| SecretValuesView::new(fetch, subject.clone(), keys, access));
            // The subscription ends with the view, which only this shell holds.
            cx.subscribe(&view, |shell, _, event: &SecretCopied, cx| {
                shell.arm_clipboard_clear(event.0.clone(), cx);
            })
            .detach();
            self.drawer.secret_values = Some(view);
        }
        let Some((pending, _)) = &self.drawer.pending_secret_action else {
            return;
        };
        let verdict = pending_action(pending, Some(&subject));
        let Some((_, action)) = self.drawer.pending_secret_action.take() else {
            return;
        };
        if verdict == PendingAction::Run
            && let Some(view) = &self.drawer.secret_values
        {
            view.update(cx, |view, cx| view.run(action, cx));
        }
    }

    /// Starts the 30 s clear of a copied value, replacing any armed one: the clipboard now holds
    /// the new text, so the old mark no longer matters.
    fn arm_clipboard_clear(&mut self, mark: ClipboardMark, cx: &mut Context<Self>) {
        self.arm_clear_after(mark, CLIPBOARD_CLEAR_DELAY, 0, cx);
    }

    fn arm_clear_after(
        &mut self,
        mark: ClipboardMark,
        delay: Duration,
        attempt: u32,
        cx: &mut Context<Self>,
    ) {
        let task = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            let _ = this.update(cx, |shell, cx| {
                shell.clear_armed_clipboard(Retry::Allowed, cx)
            });
        });
        self.clipboard_clear = Some(ArmedClear {
            mark,
            attempt,
            _task: task,
        });
    }

    /// Clears the clipboard when it still holds the armed copy; anything else is left alone. A
    /// clipboard that cannot be read or emptied just now is tried again soon, a few times, because
    /// the value may still be on it. At quit there is one attempt and no retry.
    fn clear_armed_clipboard(&mut self, retry: Retry, cx: &mut Context<Self>) {
        let Some(armed) = self.clipboard_clear.take() else {
            return;
        };
        // Detached, not dropped: this may run inside the task itself.
        armed._task.detach();
        let outcome = clear_if_unchanged(&armed.mark, cx);
        if retry == Retry::Allowed
            && let ClearStep::RetryIn(delay) = next_clear_step(outcome, armed.attempt)
        {
            self.arm_clear_after(armed.mark, delay, armed.attempt + 1, cx);
        }
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

    /// Keeps `drawer.helm` for the shown release revision only: the view lives exactly while a
    /// release drawer shows a Helm tab or its Overview. It runs inside `render`, so it only assigns
    /// and never notifies, and it is the only place that creates the view. A changed revision
    /// (a History button, or a new latest one) drops the old view, which wipes its texts.
    fn sync_helm_view(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let found = self.selected.clone().and_then(|key| {
            let tab = shown_tab(drawer_tabs(&key), self.drawer.tab);
            let live = self.live(cx)?;
            let summary = helm_release_of(live, &key)?;
            let subject = helm_subject(Some(&key), tab, Some(summary), self.drawer.helm_revision)?;
            let history = helm_history_of(live, &key, subject.0.revision);
            Some((
                key.clone(),
                subject,
                summary.revision,
                live.connection().clone(),
                history,
            ))
        });
        let Some((key, (revision, tab), latest, connection, history)) = found else {
            self.drawer.helm = None;
            return;
        };
        let existing = self
            .drawer
            .helm
            .clone()
            .filter(|view| view.read(cx).is_for(&revision));
        let view = match existing {
            Some(view) => view,
            None => {
                let access = self.secret_value_access;
                let view = cx.new(|cx| {
                    HelmReleaseView::new(connection, revision, latest, access, tab, window, cx)
                });
                // The subscriptions end with the view, which only this shell holds.
                cx.subscribe(&view, |shell, _, _: &ShowLatest, cx| {
                    shell.drawer.helm_revision = None;
                    cx.notify();
                })
                .detach();
                cx.subscribe(&view, |shell, _, event: &SecretCopied, cx| {
                    shell.arm_clipboard_clear(event.0.clone(), cx);
                })
                .detach();
                self.drawer.helm = Some(view.clone());
                view
            }
        };
        let layout = match self.drawer.pending_helm_layout.take() {
            Some((pending, layout)) if pending == key => Some(layout),
            _ => None,
        };
        view.update(cx, |view, cx| {
            view.set_latest(latest);
            view.set_tab(tab, window, cx);
            view.set_history(history, window, cx);
            if let Some(layout) = layout {
                view.set_layout(layout, window, cx);
            }
        });
    }

    /// Tells the session which kubelets the open drawer wants. It runs inside `render`, so it
    /// only assigns and never notifies; the session is touched only when the demand changed.
    fn sync_kubelet_demand(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone() else {
            return;
        };
        let demand = self.kubelet_demand(cx);
        let is_current = session
            .read(cx)
            .live()
            .is_none_or(|live| *live.metrics.kubelet.demand() == demand);
        if !is_current {
            session.update(cx, |session, _| session.set_kubelet_demand(demand));
        }
    }

    /// The subject of the open drawer, from its key alone; only a workload reads its row, for
    /// the pods it owns. Disk I/O is wanted only while a Monitor tab shows: the drawer's own, or the
    /// container Monitor sub-tab of a pod.
    fn kubelet_demand(&self, cx: &App) -> KubeletDemand {
        let subject = self.selected.as_ref().and_then(|key| match key {
            ResourceKey::Pod { namespace, name } => Some(KubeletSubject::Pod {
                namespace: namespace.clone(),
                name: name.clone(),
            }),
            ResourceKey::Node { name } => Some(KubeletSubject::Node(name.clone())),
            ResourceKey::Kind {
                kind: ResourceKind::PersistentVolumeClaims,
                namespace: Some(namespace),
                name,
            } => Some(KubeletSubject::Claim {
                namespace: namespace.clone(),
                claim: name.clone(),
            }),
            ResourceKey::Kind { kind, .. } if kind.has_monitor() => self
                .live(cx)?
                .kind_list(*kind)?
                .list
                .items()
                .iter()
                .find(|row| key.is_row(*kind, row))?
                .related_pods
                .clone()
                .map(KubeletSubject::Workload),
            ResourceKey::Kind { .. } => None,
        });
        let wants_disk_io = subject.is_some() && self.shows_monitor();
        KubeletDemand {
            subject,
            wants_disk_io,
        }
    }

    /// Whether a Monitor tab of the open drawer is visible.
    fn shows_monitor(&self) -> bool {
        let Some(key) = &self.selected else {
            return false;
        };
        let is_container_tab = self.drawer.tab == DrawerTab::Containers
            && self.drawer.container_tab == ContainerTab::Monitor
            && matches!(key, ResourceKey::Pod { .. });
        let is_tab =
            self.drawer.tab == DrawerTab::Monitor && drawer_tabs(key).contains(&DrawerTab::Monitor);
        is_container_tab || is_tab
    }

    /// The Helm view of an open release drawer has not read what it shows yet.
    #[cfg(feature = "screenshot")]
    fn is_helm_loading(&self, cx: &App) -> bool {
        self.drawer
            .helm
            .as_ref()
            .is_some_and(|view| view.read(cx).is_loading())
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
        self.drop_secret_values();
        // A revision belongs to one release.
        self.drawer.helm_revision = None;
        self.drawer.pending_helm_layout = None;
        // A part of one subject (a container, a pod) means nothing for the next.
        self.drawer.monitor.scope = MonitorScope::Total;
        self.follow_drawer_subjects(cx);
        cx.notify();
        true
    }

    /// A service account drawer shows Can do, which needs the RBAC snapshot. Only an idle
    /// snapshot is requested here: a failed one waits for the Retry button instead of looping.
    fn request_rbac_for_account(&self, cx: &mut Context<Self>) {
        if self.drawer_account().is_none() {
            return;
        }
        let Some(session) = self.session.clone() else {
            return;
        };
        let is_idle = session
            .read(cx)
            .live()
            .is_some_and(|live| matches!(live.rbac, RbacState::Idle));
        if is_idle {
            session.update(cx, |session, cx| session.request_rbac(cx));
        }
    }

    /// Points the drawer's watches (object events, related objects) at the selected object.
    /// Stopping is immediate. A start waits for the selection to rest, so arrowing through rows
    /// sends no request per row: one timer starts every pending subject together, and a newer
    /// selection replaces it. A watch whose subject does not change keeps running. It also runs
    /// when the session changes, because a row that was not loaded at selection time (a reveal)
    /// only now tells what to watch; an unchanged pending start keeps its timer.
    fn follow_drawer_subjects(&mut self, cx: &mut Context<Self>) {
        self.request_rbac_for_account(cx);
        let next_events = self.selected.as_ref().and_then(event_subject);
        let next_related = self.selected_related_subject(cx);
        let (running_events, running_related) = self.live(cx).map_or((None, None), |live| {
            (
                live.event_subject().cloned(),
                live.related_subject().cloned(),
            )
        });
        let mut pending = PendingSubjects::default();
        match subject_change(running_events.as_ref(), next_events) {
            SubjectChange::Keep => {}
            SubjectChange::Stop => self.set_event_subject(None, cx),
            SubjectChange::Start(subject) => {
                self.set_event_subject(None, cx);
                pending.events = Some(subject);
            }
        }
        match subject_change(running_related.as_ref(), next_related) {
            SubjectChange::Keep => {}
            SubjectChange::Stop => self.set_related_subject(None, cx),
            SubjectChange::Start(subject) => {
                self.set_related_subject(None, cx);
                pending.related = Some(subject);
            }
        }
        if pending.is_empty() {
            self.pending_subjects = None;
            return;
        }
        if self
            .pending_subjects
            .as_ref()
            .is_some_and(|running| running.has_same_subjects(&pending))
        {
            return;
        }
        let (events, related) = (pending.events.clone(), pending.related.clone());
        pending.task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(DRAWER_SUBJECT_DELAY).await;
            let _ = this.update(cx, |shell, cx| {
                shell.pending_subjects = None;
                if events.is_some() {
                    shell.set_event_subject(events, cx);
                }
                if related.is_some() {
                    shell.set_related_subject(related, cx);
                }
            });
        }));
        self.pending_subjects = Some(pending);
    }

    /// What the selected row needs watched besides its events; `None` while its list has not
    /// loaded the row.
    fn selected_related_subject(&self, cx: &App) -> Option<RelatedSubject> {
        let ResourceKey::Kind { kind, .. } = self.selected.as_ref()? else {
            return None;
        };
        let key = self.selected.as_ref()?;
        let live = self.live(cx)?;
        let row = live
            .kind_list(*kind)?
            .list
            .items()
            .iter()
            .find(|row| key.is_row(*kind, row))?;
        related_subject(*kind, row)
            .filter(|subject| denied_related_check(subject, &live.access).is_none())
    }

    fn set_related_subject(&mut self, subject: Option<RelatedSubject>, cx: &mut Context<Self>) {
        if let Some(session) = &self.session {
            session.update(cx, |session, cx| session.set_related_subject(subject, cx));
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
                    .shown_item(table, *row, cx)
                    .and_then(|item| self.live(cx)?.pods.items().get(item))
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
                    .shown_item(table, *row, cx)
                    .and_then(|item| self.live(cx)?.nodes.items().get(item))
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

    /// A click on the issue at table row `row` opens its object on its own screen, with its
    /// drawer. Arrow keys only move the highlight of the Issues table: they would otherwise jump
    /// to another screen at the first key press.
    pub(crate) fn reveal_issue(&mut self, row: usize, cx: &mut Context<Self>) {
        let target = self
            .shown_item(&self.issue_table, row, cx)
            .and_then(|item| {
                let session = self.session.as_ref()?.read(cx);
                session.issues().issues().get(item)?.target.clone()
            });
        if let Some(target) = target {
            self.reveal(target, cx);
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
                    let item = self.shown_item(table, *row, cx)?;
                    Some(ResourceKey::of_row(kind, explorer.list.items().get(item)?))
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

    /// Opens the logs of every pod of a workload in the dock. Nothing opens without a live
    /// session or for a node.
    pub(crate) fn open_workload_logs(
        &mut self,
        owner: PodOwner,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(target) = LogTarget::of_workload(owner) else {
            return;
        };
        let Some(live) = self.live(cx) else {
            return;
        };
        let connection = live.connection().clone();
        self.log_dock
            .update(cx, |dock, cx| dock.open(connection, target, window, cx));
    }

    /// What "Logs of selected" would open: the selected pod, or the workload of the selected row.
    pub(crate) fn selected_log_target(&self, cx: &App) -> Result<LogTarget, NoLogTarget> {
        let live = self.live(cx).ok_or(NoLogTarget::NotConnected)?;
        check_logs_access(view_logs_reason(Some(live)))?;
        let key = self.selected.as_ref().ok_or(NoLogTarget::NotLoggable)?;
        let target = match key {
            ResourceKey::Pod { .. } => live
                .pods
                .items()
                .iter()
                .find(|pod| key.is_pod(pod))
                .and_then(LogTarget::of_pod),
            ResourceKey::Node { .. } => None,
            ResourceKey::Kind { kind, .. } => live
                .kind_list(*kind)
                .and_then(|explorer| {
                    explorer
                        .list
                        .items()
                        .iter()
                        .find(|row| key.is_row(*kind, row))
                })
                .and_then(|row| row.related_pods.clone())
                .and_then(LogTarget::of_workload),
        };
        target.ok_or(NoLogTarget::NotLoggable)
    }

    /// The "+ ▾" menu entry; an `Err` has no target and the menu item is disabled.
    pub(crate) fn open_logs_of_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Ok(target) = self.selected_log_target(cx) else {
            return;
        };
        let Some(live) = self.live(cx) else {
            return;
        };
        let connection = live.connection().clone();
        self.log_dock
            .update(cx, |dock, cx| dock.open(connection, target, window, cx));
    }

    /// The Logs sub-tab of a container: opens or focuses the dock tab on the container shown
    /// in the drawer. Nothing happens while the logs are not permitted.
    pub(crate) fn open_container_logs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(live) = self.live(cx) else {
            return;
        };
        if view_logs_reason(Some(live)).is_some() {
            return;
        }
        let Some(key) = &self.selected else {
            return;
        };
        let Some(pod) = live.pods.items().iter().find(|pod| key.is_pod(pod)) else {
            return;
        };
        let Some(index) = selected_container_index(pod, &self.drawer) else {
            return;
        };
        let Some(target) = LogTarget::of_container(pod, &pod.containers[index].name) else {
            return;
        };
        let connection = live.connection().clone();
        self.log_dock
            .update(cx, |dock, cx| dock.open(connection, target, window, cx));
    }

    /// Why "Shell into selected" is disabled.
    pub(crate) fn open_shell_unavailable_reason(&self, cx: &App) -> SharedString {
        open_shell_reason(self.live(cx))
    }

    /// Writes `last_used` the first time a session is Live, so a cluster that fails to connect
    /// is not reopened at the next start.
    fn record_last_used(&mut self, is_live: bool, cx: &mut Context<Self>) {
        if !is_live || self.has_reported_live {
            return;
        }
        self.has_reported_live = true;
        let Some(active) = &self.active else {
            return;
        };
        // Cloned before the update, so nothing borrowed from the shell crosses the `cx` borrow.
        let cluster = ClusterRef::of(active);
        AppSettings::update(cx, |settings| settings.registry.last_used = Some(cluster));
    }

    fn on_session_changed(&mut self, cx: &mut Context<Self>) {
        let is_live = self
            .session
            .as_ref()
            .is_some_and(|session| session.read(cx).live().is_some());
        self.record_last_used(is_live, cx);
        self.apply_pending_custom_launch(cx);
        self.follow_custom_kinds(cx);
        self.rebuild_visible_view(cx, |_| {});
        self.apply_pending_launch_screen(cx);
        self.apply_pending_reveal(cx);
        self.sync_selection(cx);
        self.follow_drawer_subjects(cx);
        cx.notify();
    }

    /// Once the CRD list has loaded, opens the kind a `--screen custom:` request names, and hands
    /// the drawer part to the launch machinery of the kind screens. A CRD that is not Established
    /// is logged and the CRDs screen opens instead.
    fn apply_pending_custom_launch(&mut self, cx: &mut Context<Self>) {
        let Some(launch) = &self.pending_custom_launch else {
            return;
        };
        let Some(live) = self.live(cx) else {
            return;
        };
        // Without a CRD watch a denied list can never load, so the request is not found.
        let found = match live.crds.as_ref() {
            Some(crds) if !crds.list.is_loading() => {
                resolve_custom_launch(&crds.kinds, launch.crd_name)
            }
            None if live.is_crds_denied() => resolve_custom_launch(&[], launch.crd_name),
            Some(_) | None => return,
        };
        let tab = launch.tab;
        self.pending_custom_launch = None;
        let kind = match found {
            Ok(kind) => ResourceKind::Custom(kind),
            Err(message) => {
                tracing::error!(%message, "custom launch failed");
                #[cfg(feature = "screenshot")]
                {
                    self.launch_failure = Some(message);
                }
                self.show_screen(Screen::Kind(ResourceKind::Crds), cx);
                return;
            }
        };
        self.show_screen(Screen::Kind(kind), cx);
        if let Some(tab) = tab {
            self.drawer.tab = tab;
            self.pending_launch_screen = Some(LaunchScreen::KindDrawer(kind, tab));
        }
    }

    /// Follows the shown custom kind through a change of the CRD list (`remapped_screen`). Waits for
    /// the first snapshot, so a context switch does not drop the screen before the kinds are known.
    fn follow_custom_kinds(&mut self, cx: &mut Context<Self>) {
        let Some(crds) = self.live(cx).and_then(|live| live.crds.as_ref()) else {
            return;
        };
        if crds.list.ready_count().is_none() {
            return;
        }
        let Some(next) = remapped_screen(self.screen, &crds.kinds) else {
            return;
        };
        self.show_screen(next, cx);
    }

    /// The item shown at `row` of `table`.
    fn shown_item<D: FilteredTable>(
        &self,
        table: &Entity<TableState<D>>,
        row: usize,
        cx: &App,
    ) -> Option<usize> {
        table.read(cx).delegate().view()?.item_index(row)
    }

    /// Keeps the table highlight and the drawer on the selected object after a snapshot has
    /// reordered, added, or removed rows, or a filter hid it. A loading list proves nothing; a
    /// failed one has no rows.
    fn sync_selection(&mut self, cx: &mut Context<Self>) {
        let Some(key) = self.selected.clone() else {
            return;
        };
        let Some(live) = self.live(cx) else {
            return;
        };
        match &key {
            ResourceKey::Pod { .. } => {
                let Some(view) = self.pod_table.read(cx).delegate().view() else {
                    return;
                };
                let Some(found) = list_row_index(&live.pods, view, |pod| key.is_pod(pod)) else {
                    return;
                };
                let table = self.pod_table.clone();
                self.apply_selection_sync(&table, found, cx);
            }
            ResourceKey::Node { .. } => {
                let Some(view) = self.node_table.read(cx).delegate().view() else {
                    return;
                };
                let Some(found) = list_row_index(&live.nodes, view, |node| key.is_node(node))
                else {
                    return;
                };
                let table = self.node_table.clone();
                self.apply_selection_sync(&table, found, cx);
            }
            ResourceKey::Kind { kind, .. } => {
                let Some(explorer) = live.kind_list(*kind) else {
                    return;
                };
                let Some(view) = self.kind_table.read(cx).delegate().view() else {
                    return;
                };
                let Some(found) =
                    list_row_index(&explorer.list, view, |row| key.is_row(*kind, row))
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

    /// `--screen issues-drawer`: once the issues are known, opens the object of the first one.
    fn reveal_first_issue(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.session.as_ref().map(|session| session.read(cx)) else {
            return;
        };
        if session.live().is_none() || session.is_issues_pending() {
            return;
        }
        let target = session
            .issues()
            .issues()
            .iter()
            .find_map(|issue| issue.target.clone());
        self.pending_launch_screen = None;
        if let Some(target) = target {
            self.reveal(target, cx);
        }
    }

    /// Opens the drawer that `--screen` asked for, once its list has loaded. The logs screens
    /// wait for `open_pending_logs`, which needs a window. A filter that hides the first item
    /// opens no drawer.
    fn apply_pending_launch_screen(&mut self, cx: &mut Context<Self>) {
        let Some(launch) = self
            .pending_launch_screen
            .filter(|launch| !launch.has_log_dock())
        else {
            return;
        };
        if launch.checks_rows() {
            self.check_first_rows(launch, cx);
            return;
        }
        if launch == LaunchScreen::IssuesDrawer {
            self.reveal_first_issue(cx);
            return;
        }
        let Some(live) = self.live(cx) else {
            return;
        };
        let select = self.launch_select.as_deref();
        let (is_loading, item) = match launch {
            LaunchScreen::NodeDrawer(_) => {
                let nodes = live.nodes.items();
                let item = match select {
                    Some(select) => {
                        pick_selected(select, nodes.iter().map(|node| (None, node.name.as_str())))
                    }
                    None => (!nodes.is_empty()).then_some(0),
                };
                (live.nodes.is_loading(), item)
            }
            LaunchScreen::KindDrawer(kind, _) => {
                let explorer = live.kind_list(kind);
                let rows = explorer.map_or(&[][..], |explorer| explorer.list.items());
                let item = match select {
                    Some(select) => pick_selected(
                        select,
                        rows.iter()
                            .map(|row| (row.namespace.as_deref(), row.name.as_str())),
                    ),
                    None => (!rows.is_empty()).then_some(0),
                };
                (
                    explorer.is_none_or(|explorer| explorer.list.is_loading()),
                    item,
                )
            }
            _ => {
                let pods = live.pods.items();
                let item = match select {
                    Some(select) => pick_selected(
                        select,
                        pods.iter()
                            .map(|pod| (Some(pod.namespace.as_str()), pod.name.as_str())),
                    ),
                    None => pick_drawer_pod(pods),
                };
                (live.pods.is_loading(), item)
            }
        };
        if is_loading {
            return;
        }
        self.pending_launch_screen = None;
        let Some(item) = item else {
            return;
        };
        match launch {
            LaunchScreen::NodeDrawer(_) => {
                let Some(row) = self.row_of_item(&self.node_table, item, cx) else {
                    return;
                };
                let key = self
                    .live(cx)
                    .and_then(|live| live.nodes.items().get(item))
                    .map(ResourceKey::of_node);
                self.change_selection(key, cx);
                self.node_table
                    .update(cx, |table, cx| table.set_selected_row(row, cx));
            }
            LaunchScreen::KindDrawer(kind, _) => {
                let Some(row) = self.row_of_item(&self.kind_table, item, cx) else {
                    return;
                };
                let key = self
                    .live(cx)
                    .and_then(|live| live.kind_list(kind)?.list.items().get(item))
                    .map(|row| ResourceKey::of_row(kind, row));
                self.change_selection(key, cx);
                self.kind_table
                    .update(cx, |table, cx| table.set_selected_row(row, cx));
            }
            _ => {
                let Some(row) = self.row_of_item(&self.pod_table, item, cx) else {
                    return;
                };
                let key = self
                    .live(cx)
                    .and_then(|live| live.pods.items().get(item))
                    .map(ResourceKey::of_pod);
                self.change_selection(key, cx);
                self.pod_table
                    .update(cx, |table, cx| table.set_selected_row(row, cx));
            }
        }
    }

    /// `--screen pods-selected|nodes-selected`: ticks the first two shown rows once the list has
    /// loaded. The view is rebuilt before this runs.
    fn check_first_rows(&mut self, launch: LaunchScreen, cx: &mut Context<Self>) {
        let Some(live) = self.live(cx) else {
            return;
        };
        let is_loading = match launch {
            LaunchScreen::NodesSelected => live.nodes.is_loading(),
            _ => live.pods.is_loading(),
        };
        if is_loading {
            return;
        }
        self.pending_launch_screen = None;
        for row in 0..2 {
            match launch {
                LaunchScreen::NodesSelected => {
                    check_table(&self.node_table, RowCheck::Toggle(row), cx)
                }
                _ => check_table(&self.pod_table, RowCheck::Toggle(row), cx),
            }
        }
    }

    /// The row of `table` that shows `item`; `None` while a filter hides it.
    fn row_of_item<D: FilteredTable>(
        &self,
        table: &Entity<TableState<D>>,
        item: usize,
        cx: &App,
    ) -> Option<usize> {
        table.read(cx).delegate().view()?.row_of(item)
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
        let pods = live.pods.items();
        // `--select` names the pod, so a run can capture a system workload on purpose.
        let picked = self
            .launch_select
            .as_deref()
            .and_then(|select| {
                pick_selected(
                    select,
                    pods.iter()
                        .map(|pod| (Some(pod.namespace.as_str()), pod.name.as_str())),
                )
            })
            .or_else(|| pick_logs_pod(pods))
            .and_then(|row| pods.get(row));
        // A bare pod has no workload to merge, so it falls back to the pod's own tab.
        let workload = (launch == LaunchScreen::LogsWorkload)
            .then(|| {
                picked
                    .and_then(controller_owner_of)
                    .and_then(LogTarget::of_workload)
            })
            .flatten();
        let opened = workload
            .or_else(|| picked.and_then(LogTarget::of_pod))
            .map(|target| (live.connection().clone(), target));
        self.pending_launch_screen = None;
        let Some((connection, target)) = opened else {
            return;
        };
        let mode = if launch != LaunchScreen::LogsDock {
            DockMode::Zoomed
        } else {
            DockMode::Normal
        };
        self.log_dock.update(cx, |dock, cx| {
            dock.open(connection, target, window, cx);
            dock.set_mode(mode, cx);
        });
    }

    /// The failure of a `--screen custom:` request, which a screenshot run reports instead of
    /// capturing the fallback screen.
    #[cfg(feature = "screenshot")]
    pub(crate) fn launch_failure(&self) -> Option<&str> {
        self.launch_failure.as_deref()
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
                        // The panels read these three lists; a failed one is a state the panel shows.
                        Screen::Overview => (
                            live.pods.is_loading()
                                || live.nodes.is_loading()
                                || live.namespaces.is_loading(),
                            false,
                        ),
                        Screen::Pods => (live.pods.is_loading(), live.pods.failure().is_some()),
                        Screen::Nodes => (live.nodes.is_loading(), live.nodes.failure().is_some()),
                        // The table shows what the pods and nodes lists found; a failed one is a
                        // gap the coverage names.
                        Screen::Issues => {
                            (live.pods.is_loading() || live.nodes.is_loading(), false)
                        }
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
                    } else if is_loading
                        || self.pending_custom_launch.is_some()
                        || (self.screen == Screen::Kind(ResourceKind::Crds)
                            && live.is_counting_instances())
                        || live.kind_counts().is_running()
                        || session.read(cx).is_issues_pending()
                        || (matches!(self.screen, Screen::Kind(_)) && live.is_join_loading())
                    {
                        TargetState::Loading
                    } else {
                        TargetState::Loaded
                    }
                }
            },
        };
        // A drawer waits for the debounce, then for its events, related objects, and YAML.
        let is_content_pending = self.pending_subjects.is_some()
            || self
                .live(cx)
                .is_some_and(|live| live.is_object_events_loading() || live.is_related_loading())
            || self.is_yaml_loading(cx)
            || self.is_helm_loading(cx);
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
            is_dialog_pending: self.pending_dialog_launch.is_some()
                || self.who_can.as_ref().is_some_and(|view| {
                    view.read_with(cx, |view, cx| view.is_pending(cx))
                        .unwrap_or(false)
                })
                || self.permissions.as_ref().is_some_and(|view| {
                    view.read_with(cx, |view, cx| view.is_pending(cx))
                        .unwrap_or(false)
                })
                || self.traffic.as_ref().is_some_and(|view| {
                    view.read_with(cx, |view, _| view.is_pending())
                        .unwrap_or(false)
                }),
            pod_metrics: self
                .live(cx)
                .map_or_else(FeedProgress::unavailable, |live| FeedProgress {
                    status: live.metrics.pods.status.clone(),
                    ticks: live.metrics.pods.history.tick_count(),
                }),
            node_metrics: self
                .live(cx)
                .map_or_else(FeedProgress::unavailable, |live| FeedProgress {
                    status: live.metrics.nodes.status.clone(),
                    ticks: live.metrics.nodes.history.tick_count(),
                }),
            kubelet: self
                .live(cx)
                .map_or_else(FeedProgress::unavailable, |live| {
                    kubelet_progress(
                        live.metrics.kubelet.status.clone(),
                        live.metrics.kubelet.history.tick_count(),
                        // Targets follow the nodes list, so an unloaded list may still produce some.
                        live.nodes.is_loading()
                            || !live.metrics.kubelet.targets().summary_nodes.is_empty(),
                    )
                }),
        }
    }

    // ---- table toolkit ----

    /// Applies `change` to the visible table's view, then rebuilds it. A change of the column
    /// layout refreshes the table.
    fn rebuild_visible_view(
        &mut self,
        cx: &mut Context<Self>,
        change: impl FnOnce(&mut TableView),
    ) {
        match self.screen {
            Screen::Pods => rebuild_table(&self.pod_table, change, cx),
            Screen::Nodes => rebuild_table(&self.node_table, change, cx),
            // Overview has no table.
            Screen::Overview => {}
            Screen::Issues => rebuild_table(&self.issue_table, change, cx),
            Screen::Kind(_) => rebuild_table(&self.kind_table, change, cx),
        }
    }

    /// The one path of every toolkit action: change the view, then keep the selection and the
    /// drawer consistent with the rows that remain.
    fn update_view(&mut self, cx: &mut Context<Self>, change: impl FnOnce(&mut TableView)) {
        self.rebuild_visible_view(cx, change);
        self.sync_selection(cx);
        cx.notify();
    }

    /// A header click: ascending, descending, then the source order.
    pub(crate) fn cycle_sort(&mut self, column: usize, cx: &mut Context<Self>) {
        self.update_view(cx, |view| view.sort = next_sort(view.sort, column));
    }

    pub(crate) fn remove_chip(&mut self, index: usize, cx: &mut Context<Self>) {
        self.update_view(cx, |view| {
            if index < view.filter.chips.len() {
                view.filter.chips.remove(index);
            }
        });
    }

    pub(crate) fn toggle_unhealthy(&mut self, cx: &mut Context<Self>) {
        self.update_view(cx, |view| {
            let chips = &mut view.filter.chips;
            match chips.iter().position(|chip| *chip == FilterChip::Unhealthy) {
                Some(index) => {
                    chips.remove(index);
                }
                None => chips.push(FilterChip::Unhealthy),
            }
        });
    }

    pub(crate) fn toggle_column(&mut self, column: usize, cx: &mut Context<Self>) {
        self.update_view(cx, |view| {
            if !view.hidden.remove(&column) {
                view.hidden.insert(column);
            }
        });
    }

    /// A row checkbox, or Ctrl+click.
    pub(crate) fn toggle_row_checked(&mut self, row: usize, cx: &mut Context<Self>) {
        self.check_rows(RowCheck::Toggle(row), cx);
    }

    /// Shift+click: ticks the rows from the anchor to `row`.
    pub(crate) fn check_row_range(&mut self, row: usize, cx: &mut Context<Self>) {
        self.check_rows(RowCheck::Range(row), cx);
    }

    /// The header checkbox.
    pub(crate) fn set_all_checked(&mut self, checked: bool, cx: &mut Context<Self>) {
        self.check_rows(RowCheck::All(checked), cx);
    }

    /// The selection bar's ✕.
    pub(crate) fn clear_checked(&mut self, cx: &mut Context<Self>) {
        self.update_view(cx, TableView::clear_checked);
    }

    fn check_rows(&mut self, change: RowCheck, cx: &mut Context<Self>) {
        match self.screen {
            Screen::Pods => check_table(&self.pod_table, change, cx),
            Screen::Nodes => check_table(&self.node_table, change, cx),
            Screen::Overview => {}
            Screen::Issues => check_table(&self.issue_table, change, cx),
            Screen::Kind(_) => check_table(&self.kind_table, change, cx),
        }
        cx.notify();
    }

    /// Sets or clears the screen's own switch: a Nodes summary chip or Hide inactive.
    pub(crate) fn set_preset(&mut self, preset: Option<FilterPreset>, cx: &mut Context<Self>) {
        self.update_view(cx, move |view| view.filter.preset = preset);
    }

    /// "View pods on node": the Pods screen with only that node's pods. The other Pods filters
    /// go, so every pod on the node shows.
    pub(crate) fn view_pods_on_node(&mut self, node: &str, cx: &mut Context<Self>) {
        self.show_screen(Screen::Pods, cx);
        let filter = TableFilter::on_node(node);
        self.update_view(cx, move |view| view.filter = filter);
        // The input shows the filter text of its screen, which is empty now.
        self.quick_filter_screen = None;
    }

    /// "Filter similar": the Events list keeps the events with this reason.
    pub(crate) fn filter_similar(&mut self, reason: &str, cx: &mut Context<Self>) {
        let reason = reason.to_owned();
        self.update_view(cx, move |view| {
            view.filter
                .set_equals(EVENT_REASON_COLUMN, "Reason", &reason)
        });
    }

    /// Holds the Events list still, or shows the events that arrived meanwhile.
    pub(crate) fn toggle_explorer_paused(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone() else {
            return;
        };
        session.update(cx, |session, cx| {
            let is_paused = matches!(
                session.live().and_then(LiveCluster::explorer_flow),
                Some(FlowState::Paused { .. })
            );
            session.set_explorer_paused(!is_paused, cx);
        });
    }

    /// Removes the text and the chips of the visible table, and empties the input.
    pub(crate) fn clear_filters(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.update_view(cx, TableView::clear_filter);
        self.quick_filter
            .update(cx, |input, cx| input.set_value("", window, cx));
    }

    /// `+ Filter` > `Label…`: the input starts a label query, which Enter turns into chips.
    pub(crate) fn begin_label_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.update_view(cx, |view| view.filter.text.clear());
        self.quick_filter.update(cx, |input, cx| {
            input.set_value("label:", window, cx);
            input.focus(window, cx);
        });
    }

    fn focus_quick_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.quick_filter
            .update(cx, |input, cx| input.focus(window, cx));
    }

    /// `--filter`: a `label:` text becomes chips, as on Enter; anything else is the quick text.
    fn apply_launch_filter(&mut self, text: &str, cx: &mut Context<Self>) {
        let text = text.to_owned();
        self.update_view(cx, move |view| match parse_label_queries(&text) {
            Some(queries) => view.add_chips(queries.into_iter().map(FilterChip::Label).collect()),
            None => view.filter.text = text,
        });
    }

    /// A context switch: a filter written for one cluster would surprise in another.
    fn clear_all_filters(&mut self, cx: &mut Context<Self>) {
        self.pod_table.update(cx, |table, _| {
            if let Some(view) = table.delegate_mut().view_mut() {
                view.reset_filter();
            }
        });
        self.node_table.update(cx, |table, _| {
            if let Some(view) = table.delegate_mut().view_mut() {
                view.reset_filter();
            }
        });
        self.issue_table.update(cx, |table, _| {
            if let Some(view) = table.delegate_mut().view_mut() {
                view.reset_filter();
            }
        });
        self.kind_table
            .update(cx, |table, _| table.delegate_mut().reset_filters());
        self.quick_filter_screen = None;
        self.rebuild_visible_view(cx, |_| {});
    }

    fn on_quick_filter_event(
        &mut self,
        input: &Entity<InputState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Change => {
                let text = quick_filter_text(&input.read(cx).value()).to_owned();
                self.update_view(cx, move |view| view.filter.text = text);
            }
            InputEvent::PressEnter { .. } => {
                let text = input.read(cx).value();
                let Some(queries) = parse_label_queries(&text) else {
                    return;
                };
                let chips = queries.into_iter().map(FilterChip::Label).collect();
                self.update_view(cx, move |view| {
                    view.add_chips(chips);
                    view.filter.text.clear();
                });
                input.update(cx, |input, cx| input.set_value("", window, cx));
            }
            InputEvent::Focus | InputEvent::Blur => {}
        }
    }

    /// Loads the visible screen's filter text into the input when the screen changed. It runs
    /// in `render` because `set_value` needs a window, and it emits no `Change`.
    fn sync_quick_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.quick_filter_screen == Some(self.screen) {
            return;
        }
        self.quick_filter_screen = Some(self.screen);
        let text = self
            .toolkit_state(cx)
            .map(|state| state.text)
            .unwrap_or_default();
        self.quick_filter
            .update(cx, |input, cx| input.set_value(text, window, cx));
    }

    /// What the filter bar and the screen header read; `None` before the table has a view.
    pub(crate) fn toolkit_state(&self, cx: &App) -> Option<ToolkitState> {
        let mut state = match self.screen {
            Screen::Overview => return None,
            Screen::Pods => ToolkitState::of(self.pod_table.read(cx).delegate(), self.screen)?,
            Screen::Nodes => {
                let mut state = ToolkitState::of(self.node_table.read(cx).delegate(), self.screen)?;
                state.node_counts = self.node_table.read(cx).delegate().counts().cloned();
                state
            }
            Screen::Issues => ToolkitState::of(self.issue_table.read(cx).delegate(), self.screen)?,
            Screen::Kind(_) => ToolkitState::of(self.kind_table.read(cx).delegate(), self.screen)?,
        };
        // Nodes and Namespaces are cluster-scoped: the scope does not apply to them.
        let is_namespaced = match self.screen {
            Screen::Pods | Screen::Issues => true,
            Screen::Overview | Screen::Nodes => false,
            Screen::Kind(kind) => kind.is_namespaced(),
        };
        if is_namespaced {
            state.scope = self.live(cx).map(|live| live.scope.clone());
        }
        Some(state)
    }

    // ---- rendering ----

    fn navigation_counts(&self, cx: &App) -> NavigationCounts {
        let live = self.live(cx);
        let event_filter = self
            .session
            .as_ref()
            .map_or(EventFilter::All, |session| session.read(cx).event_filter());
        // A session that is not live keeps its last board, which says nothing about the cluster.
        let (issue_total, issue_counts) = match (live, self.session.as_ref()) {
            (Some(_), Some(session)) => issue_counts(session.read(cx).issues()),
            _ => (None, Vec::new()),
        };
        NavigationCounts {
            issue_total,
            issue_counts,
            pods: live.and_then(|live| live.pods.ready_count()),
            nodes: live.and_then(|live| live.nodes.ready_count()),
            explorer: live.and_then(LiveCluster::explorer_count),
            kinds: live.map_or_else(HashMap::new, |live| {
                let mut kinds = live.kind_counts().all(event_filter);
                // Custom kinds show the cluster-wide instance count until their screen is shown.
                kinds.extend(
                    live.custom_counts
                        .counts
                        .iter()
                        .filter_map(|(kind, count)| {
                            Some((ResourceKind::Custom(*kind), usize::try_from(*count).ok()?))
                        }),
                );
                kinds
            }),
        }
    }
}

impl Render for AppShell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.fit_table_widths(window, cx);
        self.refresh_monitor_cache(cx);
        self.open_pending_logs(window, cx);
        self.open_pending_dialog(window, cx);
        self.sync_yaml_view(window, cx);
        self.sync_helm_view(window, cx);
        self.sync_secret_values(cx);
        self.sync_kubelet_demand(cx);
        self.sync_quick_filter(window, cx);
        let theme = cx.theme();
        let counts = self.navigation_counts(cx);
        let session = self.session.as_ref().map(|session| session.read(cx));
        let is_kubeconfig_loading = matches!(self.kubeconfig, KubeconfigState::Loading);
        v_flex()
            .size_full()
            .track_focus(&self.focus_handle)
            .key_context("AppShell")
            .on_action(cx.listener(|shell, _: &FocusQuickFilter, window, cx| {
                shell.focus_quick_filter(window, cx);
            }))
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

/// Ticks or unticks rows of the view of `table`.
fn check_table<D: FilteredTable>(table: &Entity<TableState<D>>, change: RowCheck, cx: &mut App) {
    table.update(cx, |table, cx| {
        table.delegate_mut().check_rows(change, cx);
        cx.notify();
    });
}

/// Applies `change` to the view of `table`, then rebuilds it from the session. A new column
/// layout needs a table refresh; otherwise the rows are read again at the next render.
fn rebuild_table<D: FilteredTable>(
    table: &Entity<TableState<D>>,
    change: impl FnOnce(&mut TableView),
    cx: &mut App,
) {
    table.update(cx, |table, cx| {
        if let Some(view) = table.delegate_mut().view_mut() {
            change(view);
        }
        if table.delegate_mut().rebuild_view(cx) {
            table.refresh(cx);
        } else {
            cx.notify();
        }
    });
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

/// The loaded kubeconfig that defines `cluster`, with its context.
fn find_cluster(
    kubeconfigs: &[Arc<Kubeconfig>],
    cluster: &ClusterRef,
) -> Option<(Arc<Kubeconfig>, ContextSummary)> {
    kubeconfigs.iter().find_map(|kubeconfig| {
        let summary = kubeconfig
            .contexts()
            .iter()
            .find(|summary| cluster.is_of(summary))?;
        Some((Arc::clone(kubeconfig), summary.clone()))
    })
}

/// The kubeconfig and context to open first (`start_choice`). The error is the one of the first
/// loaded kubeconfig: a missing requested context, or no current-context.
fn resolve_start(
    kubeconfigs: &[Arc<Kubeconfig>],
    requested: Option<&str>,
    last_used: Option<&ClusterRef>,
) -> Result<(Arc<Kubeconfig>, ContextSummary), KubeconfigError> {
    let contexts: Vec<&ContextSummary> = kubeconfigs
        .iter()
        .flat_map(|kubeconfig| kubeconfig.contexts())
        .collect();
    let choice = start_choice(requested, last_used, &contexts);
    if let StartChoice::Cluster(cluster) = &choice
        && let Some(found) = find_cluster(kubeconfigs, cluster)
    {
        return Ok(found);
    }
    // `load_kubeconfigs` fails on an empty list, so there is a first one in practice.
    let first = kubeconfigs.first().ok_or(KubeconfigError::NoFiles)?;
    let wanted = match choice {
        StartChoice::RequestedMissing => requested,
        StartChoice::Cluster(_) | StartChoice::CurrentContext => None,
    };
    let summary = first.resolve_context(wanted)?.clone();
    Ok((Arc::clone(first), summary))
}

/// The release row of the shown subject.
fn helm_release_of<'a>(
    live: &'a LiveCluster,
    subject: &ResourceKey,
) -> Option<&'a HelmReleaseSummary> {
    let row = live
        .kind_list(ResourceKind::HelmReleases)?
        .list
        .items()
        .iter()
        .find(|row| subject.is_row(ResourceKind::HelmReleases, row))?;
    match &row.object {
        KindObject::HelmRelease(release) => Some(release),
        _ => None,
    }
}

/// What the release's History says about `revision`: the watch is loading (or not started), has
/// failed before any data, or has loaded; the earlier revision is found without copying the list.
fn helm_history_of(live: &LiveCluster, subject: &ResourceKey, revision: u32) -> HistoryState {
    let list = live
        .kind_list(ResourceKind::HelmReleases)
        .and_then(|explorer| {
            explorer
                .list
                .items()
                .iter()
                .find(|row| subject.is_row(ResourceKind::HelmReleases, row))
        })
        .and_then(|row| related_subject(ResourceKind::HelmReleases, row))
        .and_then(|related| live.related_of(&related))
        .and_then(RelatedList::helm_history);
    match list {
        None | Some(LiveList::Loading) => HistoryState::Loading,
        Some(LiveList::Failed { .. }) => HistoryState::Failed,
        Some(LiveList::Ready { items, .. }) => HistoryState::Loaded {
            earlier: earlier_revision(items.iter().map(|item| item.revision), revision),
        },
    }
}

/// The Secret row of the shown subject.
fn secret_of<'a>(live: &'a LiveCluster, subject: &ResourceKey) -> Option<&'a SecretSummary> {
    let row = live
        .kind_list(ResourceKind::Secrets)?
        .list
        .items()
        .iter()
        .find(|row| subject.is_row(ResourceKind::Secrets, row))?;
    match &row.object {
        KindObject::Secret(secret) => Some(secret),
        _ => None,
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
}
