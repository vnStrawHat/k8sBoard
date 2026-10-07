use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use cluster::{
    ClusterConnection, ContextSummary, EventFilter, HelmReleaseSummary, InvolvedObject, Kubeconfig,
    KubeconfigError, NamespaceScope, NetworkPolicySummary, ObjectKind, SecretSummary,
};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::resizable::{ResizablePanelEvent, ResizableState};
use gpui_kit::component::table::{TableDelegate, TableEvent, TableState};
use gpui_kit::component::{ActiveTheme as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::{
    App, AppContext as _, Context, Div, Entity, FocusHandle, Focusable as _,
    InteractiveElement as _, IntoElement, ParentElement as _, Point, Render, SharedString,
    Styled as _, Subscription, Task, Window, prelude::FluentBuilder as _, px,
};

use crate::active_session::{ActiveConnection, ActiveSession};
use crate::cluster_catalog::{CatalogHandle, ClusterCatalog};
use crate::cluster_form::RowOrigin;
use crate::cluster_health::{ProbeCandidate, ProbeResult, ProbeTarget, RowHealth, probe_stream};
use crate::cluster_registry::{
    ClusterProfile, ClusterRef, ScopeMemory, StartChoice, launch_last_used, remember_scope,
    start_choice, start_scope,
};
use crate::cluster_runtime::ClusterRuntime;
use crate::cluster_session::{
    ClusterSession, CountTrigger, FlowState, LiveCluster, LiveList, RbacState, RelatedList,
    SessionPhase, error_text, is_related_denied,
};
use crate::cluster_switcher::{
    ClusterSwitcherState, OpenClusterSwitcher, SwitchToCluster1, SwitchToCluster2,
    SwitchToCluster3, SwitchToCluster4, SwitchToCluster5, SwitchToCluster6, SwitchToCluster7,
    SwitchToCluster8, SwitchToCluster9, SwitcherContent, SwitcherList,
};
use crate::cluster_switcher_rows::{
    HighlightStep, SwitcherSection, SwitcherSegment, ViewedCluster, connected_count,
    move_highlight, nth_cluster, row_count, switcher_sections, visible_sections,
};
use crate::command_palette::{ActiveCluster, PaletteContext, PaletteSnapshot, open_palette};
use crate::custom_kind::{CustomKind, CustomKindCache};
use crate::dock::{Dock, DockMode, LogOrigin};
use crate::drawer::{
    ContainerTab, DRAWER_SUBJECT_DELAY, DrawerSize, DrawerState, DrawerTab, MonitorCache,
    MonitorKey, MonitorRange, MonitorScope, MonitorState, drawer_tabs, shown_tab,
};
use crate::environment::cluster_environment_label;
use crate::file_export::{ExportState, export_file_name, start_export};
use crate::filter_bar::ToolkitState;
use crate::helm_release_view::{
    HelmReleaseView, HelmSource, HistoryState, ShowLatest, ValuesLayout, earlier_revision,
    helm_subject,
};
use crate::issue_table::IssueTableDelegate;
use crate::keymap::{
    FocusQuickFilter, OpenKindPalette, OpenNamespacePicker, OpenPalette, ShowShortcuts,
};
use crate::kind_row::{KindObject, PodOwner};
use crate::kind_table::KindTableDelegate;
use crate::kubeconfig_folder::FileStamp;
use crate::kubelet_metrics::{KubeletDemand, KubeletSubject};
use crate::last_log::{LastLogKey, last_log_key};
use crate::launch_options::{LaunchOptions, LaunchScreen};
use crate::live_sections::loaded_replica_sets;
use crate::log_target::{LogTarget, NoLogTarget, check_logs_access};
use crate::monitor_data::{MonitorInput, MonitorSubject, monitor_data};
use crate::name_index::IndexSummary;
use crate::namespace_picker::{NamespacePickerState, PickerAnchor};
use crate::navigation::{NavigationCounts, issue_counts, sidebar};
use crate::navigation_history::NavigationHistory;
use crate::node_table::NodeTableDelegate;
use crate::object_events::{SubjectChange, event_subject, subject_change};
use crate::overview::OverviewState;
use crate::overview_report::live_report;
use crate::palette_search::{
    PaletteInput, PaletteQuery, PaletteSession, lists_pairs, lists_resources, live_feed_objects,
    palette_entries, parse_query,
};
use crate::permissions_view::PermissionsView;
use crate::pod_drawer::selected_container_index;
use crate::pod_table::PodTableDelegate;
use crate::port_forwards::{PortForwards, StartReport};
use crate::process_usage::{ProcessUsage, TrafficTotals};
use crate::recent_changes::ChangeWindow;
use crate::related_objects::{RelatedSubject, key_related_subject, related_subject};
use crate::resource_actions::{
    KeyAvailability, RowAction, delete_kind, delete_kind_of, key_availability, view_logs_reason,
    workload_logs_owner,
};
use crate::resource_kind::ResourceKind;
use crate::revision_diff::{RevisionDiffRequest, RevisionDiffView, dialog_body};
use crate::row_context::RowContext;
use crate::row_selection::{UNTICKED_NOTICE_LIFETIME, UntickedNotice};
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
use crate::settings::{AppSettings, TablePrefs, screen_key};
use crate::settings_reset_banner::reset_banner;
use crate::settings_store::SettingsNotice;
use crate::shortcut_sheet::open_shortcut_sheet;
use crate::status_bar::status_bar;
use crate::table_filter::{
    FilterChip, FilterPreset, TableFilter, parse_label_queries, quick_filter_text,
};
use crate::table_selection::{
    ClusterObject, DialogOrigin, ResourceKey, SelectionSync, list_item_index, list_row_index,
    selection_sync, take_row_echo,
};
use crate::table_sort::next_sort;
use crate::table_view::{FilteredTable, RowCheck, TableView};
use crate::title_bar::{scope_label, title_bar};
use crate::topology_graph::{NodeId, TopologyKind};
use crate::topology_view::TopologyView;
use crate::traffic_test_view::{TrafficTestView, traffic_defaults};
use crate::value_popover::ValuePopover;
use crate::who_can_view::WhoCanView;
use crate::write_guard::ClusterGuard;
use crate::yaml_view::{YamlView, yaml_subject};

/// The width of the tool dialogs (Who can, Check permissions, Test traffic).
const DIALOG_WIDTH: f32 = 760.;
/// The revision diff needs room for the long lines of a pod template.
const REVISION_DIFF_WIDTH: f32 = 960.;

#[path = "workspace.rs"]
pub(crate) mod workspace;

#[path = "batch_write.rs"]
pub(crate) mod batch_write;

#[path = "certificate_renewal.rs"]
pub(crate) mod certificate_renewal;

#[path = "app_shell_history.rs"]
mod app_shell_history;
#[path = "app_shell_monitor_source.rs"]
mod app_shell_monitor_source;
#[path = "app_shell_session.rs"]
mod app_shell_session;
#[path = "debug_open.rs"]
mod debug_open;
#[path = "drain_dialog.rs"]
pub(crate) mod drain_dialog;
#[path = "drain_driver.rs"]
pub(crate) mod drain_driver;
#[path = "edit_yaml_flow.rs"]
mod edit_yaml_flow;
#[path = "hpa_watch.rs"]
mod hpa_watch;
#[path = "keyboard_navigation.rs"]
mod keyboard_navigation;
#[path = "leaving_work.rs"]
mod leaving_work;
#[path = "metadata_editor.rs"]
mod metadata_editor;
#[path = "node_editor.rs"]
pub(crate) mod node_editor;
#[path = "node_shell_cleanup.rs"]
mod node_shell_cleanup;
#[path = "node_shell_open.rs"]
mod node_shell_open;
#[path = "node_shell_run_history.rs"]
mod node_shell_run_history;
#[path = "node_shell_sweep.rs"]
mod node_shell_sweep;
#[path = "object_delete.rs"]
pub(crate) mod object_delete;
#[path = "port_forward_dialogs.rs"]
mod port_forward_dialogs;
#[path = "port_forward_open.rs"]
mod port_forward_open;
#[path = "port_forward_page.rs"]
mod port_forward_page;
#[path = "resource_edit_flow.rs"]
mod resource_edit_flow;
#[path = "revision_change_flow.rs"]
mod revision_change_flow;
#[path = "rollout_watch.rs"]
mod rollout_watch;
#[path = "secret_form.rs"]
pub(crate) mod secret_form;
#[path = "shell_open.rs"]
pub(crate) mod shell_open;
#[path = "values_edit_flow.rs"]
mod values_edit_flow;
#[path = "write_flow.rs"]
pub(crate) mod write_flow;
#[path = "write_lock.rs"]
mod write_lock;

use edit_yaml_flow::OpenEdit;
use keyboard_navigation::shell_key_context;

#[cfg(test)]
#[path = "app_shell_tests.rs"]
mod app_shell_tests;

#[cfg(test)]
#[path = "app_shell_history_tests.rs"]
mod app_shell_history_tests;

#[cfg(test)]
#[path = "app_shell_switch_tests.rs"]
mod app_shell_switch_tests;

#[cfg(test)]
#[path = "app_shell_script_tests.rs"]
mod app_shell_script_tests;

#[cfg(test)]
#[path = "app_shell_metrics_tests.rs"]
mod app_shell_metrics_tests;

#[cfg(test)]
#[path = "app_shell_log_defaults_tests.rs"]
mod app_shell_log_defaults_tests;

#[cfg(test)]
#[path = "app_shell_folder_tests.rs"]
mod app_shell_folder_tests;

#[cfg(test)]
#[path = "app_shell_write_tests.rs"]
mod app_shell_write_tests;

#[cfg(test)]
#[path = "app_shell_edit_tests.rs"]
mod app_shell_edit_tests;

#[cfg(test)]
#[path = "app_shell_values_edit_tests.rs"]
mod app_shell_values_edit_tests;

#[cfg(test)]
#[path = "app_shell_create_tests.rs"]
mod app_shell_create_tests;

#[cfg(test)]
#[path = "app_shell_workload_tests.rs"]
mod app_shell_workload_tests;

#[cfg(test)]
#[path = "app_shell_certificate_tests.rs"]
mod app_shell_certificate_tests;

#[cfg(test)]
#[path = "app_shell_delete_tests.rs"]
mod app_shell_delete_tests;

#[cfg(test)]
#[path = "app_shell_kind_count_tests.rs"]
mod app_shell_kind_count_tests;

#[cfg(test)]
#[path = "app_shell_metadata_edit_tests.rs"]
mod app_shell_metadata_edit_tests;

#[cfg(test)]
#[path = "app_shell_secret_form_tests.rs"]
mod app_shell_secret_form_tests;

#[cfg(test)]
#[path = "app_shell_node_edit_tests.rs"]
mod app_shell_node_edit_tests;

#[cfg(test)]
#[path = "app_shell_resource_edit_tests.rs"]
mod app_shell_resource_edit_tests;

#[cfg(test)]
#[path = "app_shell_drain_tests.rs"]
mod app_shell_drain_tests;

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
    /// The resource graph of one namespace; it lists no explorer kind and opens its drawers over the
    /// graph.
    Topology,
    /// The forwards of every cluster; a local list that needs no live session.
    PortForwarding,
    Kind(ResourceKind),
}

impl Screen {
    /// The explorer kind this screen lists, if it is a kind screen.
    pub(crate) fn kind(self) -> Option<ResourceKind> {
        match self {
            Self::Kind(kind) => Some(kind),
            Self::Overview
            | Self::Pods
            | Self::Nodes
            | Self::Issues
            | Self::Topology
            | Self::PortForwarding => None,
        }
    }

    /// The object kind a row of this screen edits and deletes (specs 0031 and 0033), if any: the
    /// lazy `update` and `delete` permissions are asked for it when the screen shows.
    pub(crate) fn access_kind(self) -> Option<ObjectKind> {
        match self {
            Self::Pods => Some(ObjectKind::Pod),
            Self::Nodes => Some(ObjectKind::Node),
            Self::Kind(kind) => delete_kind_of(kind),
            Self::Overview | Self::Issues | Self::Topology | Self::PortForwarding => None,
        }
    }
}

/// How far the catalog is: the shell shows a busy view, the load error, or its screens.
enum KubeconfigState {
    Loading,
    Loaded,
    /// No kubeconfig file or cluster exists yet; carries the reason as a muted detail.
    Empty(String),
    /// A kubeconfig exists but does not load.
    Failed(String),
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

/// The drawer watches that start once the selection has rested, in the cluster of the subject.
/// Dropping it cancels the timer.
#[derive(Default)]
struct PendingSubjects {
    /// The cluster whose session starts the watches.
    cluster: Option<ClusterRef>,
    events: Option<InvolvedObject>,
    related: Option<RelatedSubject>,
    task: Option<Task<()>>,
}

impl PendingSubjects {
    fn is_empty(&self) -> bool {
        self.events.is_none() && self.related.is_none()
    }

    /// The same watches in the same cluster: the same names exist in several clusters.
    fn has_same_subjects(&self, other: &Self) -> bool {
        self.cluster == other.cluster
            && self.events == other.events
            && self.related == other.related
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
    /// The loaded kubeconfigs, shared with the Settings window.
    catalog: Entity<ClusterCatalog>,
    _catalog_observer: Subscription,
    /// Set when the kubeconfig loaded but names no usable context; there is no session then.
    context_error: Option<String>,
    /// Nothing may start on its own (only a file of a watched folder is loaded): the shell shows
    /// "No cluster selected" and the switcher opens. Cleared by the next start.
    needs_pick: bool,
    /// The switcher opens from `render`, which has the window.
    pending_pick_switcher: bool,
    /// The context of the primary cluster, also while its session is connecting or failed, and
    /// between the release of the old sessions and the deferred connect of the new ones.
    active: Option<ContextSummary>,
    /// The cluster the user came from (the one before the last switch), for "Back to".
    previous: Option<ClusterRef>,
    /// The namespace scope of every cluster left during this run.
    scope_memory: ScopeMemory,
    /// The custom kind definitions of the last session, kept until the next one connects, so a
    /// switch that is replaced before its connect runs does not lose them.
    kind_cache: CustomKindCache,
    /// Why the last switch did nothing; shown by the title-bar warning button.
    switch_notice: Option<String>,
    /// Why the last audit line could not be written; shown by the title-bar warning button.
    write_notice: Option<String>,
    /// The text of the banner under the title bar after a settings reset; kept until dismissed.
    reset_banner: Option<String>,
    /// The open value popover (Scale), floating over the bottom of the workspace. It belongs to the
    /// row under the cursor and closes when the cursor leaves it.
    value_popover: Option<Entity<ValuePopover>>,
    /// The clusters with a confirmed batch still committing. A second batch on one of them waits:
    /// two would race over the same objects and interleave their audit lines.
    running_batches: HashSet<ClusterRef>,
    /// The user confirmed leaving with a drain running: the close goes on without asking again.
    is_quit_confirmed: bool,
    /// The delete whose objects are being read before its dialog opens (spec 0033). While it runs,
    /// a second Del (a held key repeats) starts nothing.
    delete_start: Option<Task<()>>,
    /// The open Edit YAML view (spec 0031). It replaces the table and the drawer in the workspace;
    /// the cursor and the drawer flag are kept under it and come back when it closes.
    edit: Option<OpenEdit>,
    /// The ReplicaSet list a click on a Deployment change row waits for (spec 0041); a second click
    /// replaces it.
    revision_lookup: Option<Task<()>>,
    /// The editor an Apply started a write from (`ValuesEditView::open_id`), until its commit ends.
    values_commit_open: Option<u64>,
    /// The taints the taint editor read when it last sent a change to review: a conflict on that
    /// change reopens the editor and tells what changed on the node since. Replaced by the next
    /// review, taken by the retry.
    taint_base: Option<node_editor::TaintBase>,
    /// The change a read-only lock refused last, so the Unlock that follows can say what it was for
    /// (`write_lock::UnlockFor`).
    unlock_for: Option<write_lock::UnlockFor>,
    /// The name in the discard prompt asked last, for the tests that drive it.
    #[cfg(test)]
    last_discard: Option<String>,
    /// The confirm dialog opened last, for the tests that drive it.
    #[cfg(test)]
    last_dialog: Option<gpui_kit::WeakEntity<crate::confirm_dialog::ConfirmDialog>>,
    /// The lines of the "work will close" dialog asked last, for the tests that drive it.
    #[cfg(test)]
    last_leaving: Option<Vec<String>>,
    /// The node editor opened last, for the tests that drive it.
    #[cfg(test)]
    last_node_editor: Option<gpui_kit::WeakEntity<node_editor::NodeEditor>>,
    /// The labels and annotations editor opened last, for the tests that drive it.
    #[cfg(test)]
    last_metadata_editor: Option<gpui_kit::WeakEntity<metadata_editor::MetadataEditor>>,
    /// The Secret form opened last, for the tests that drive it.
    #[cfg(test)]
    last_secret_form: Option<gpui_kit::WeakEntity<secret_form::SecretForm>>,
    /// The bulk label editor opened last, for the tests that drive it.
    #[cfg(test)]
    last_bulk_label_editor: Option<gpui_kit::WeakEntity<node_editor::BulkLabelEditor>>,
    /// The drain dialog opened last, for the tests that drive it.
    #[cfg(test)]
    last_drain_dialog: Option<gpui_kit::WeakEntity<drain_dialog::DrainDialog>>,
    /// The drain tab opened last, for the tests that drive it.
    #[cfg(test)]
    last_drain_tab: Option<gpui_kit::WeakEntity<crate::drain_tab::DrainTab>>,
    /// The window of the shell: a dialog that starts outside an event handler opens in it.
    window: gpui_kit::AnyWindowHandle,
    /// Shell starts that have not reported yet (spec 0036).
    shell_starts: shell_open::ShellStarts,
    /// The cleanups of the open node shells and the deletes in flight (spec 0037).
    node_shell_runs: node_shell_cleanup::NodeShellRuns,
    /// The id of this app run, on every node shell pod it creates: the leftover sweep of another
    /// run tells its pods from ours by it.
    run_id: String,
    /// The leftover notices shown, for the tests that drive the sweep.
    #[cfg(test)]
    sweep_notices: Vec<(ClusterRef, usize)>,
    /// The port forwards of every cluster (spec 0035). They outlive a switch and a released slot.
    port_forwards: Entity<PortForwards>,
    /// Forward starts that have not reported yet.
    forward_starts: port_forward_open::ForwardStarts,
    /// The filter of the Port Forwarding page.
    forward_filter: Entity<InputState>,
    _forward_subscriptions: Vec<Subscription>,
    /// The app's own CPU, memory and network use, at the right end of the status bar (spec 0054).
    usage: Entity<ProcessUsage>,
    /// `--screen port-forwards` and its dialogs: the list holds fixed rows, which neither the
    /// presets nor a cluster may replace.
    #[cfg(feature = "screenshot")]
    forward_fixture: bool,
    /// `--screen pod-monitor-source-fixture`: the Monitor shows synthetic source data and sends no
    /// request to a source.
    #[cfg(feature = "screenshot")]
    is_monitor_source_fixture: bool,
    /// The title-bar switcher popover.
    switcher: ClusterSwitcherState,
    _switcher_filter_events: Subscription,
    /// `--screen switcher`: the popover opens once the session is live.
    pending_switcher_launch: bool,
    /// `--screen namespace-picker`: the picker opens once the session is live.
    pending_picker_launch: bool,
    /// `--palette`: the query the palette opens with once the session is live.
    pending_palette_launch: Option<String>,
    /// Test hook: whether the old session was gone each time a deferred connect started.
    #[cfg(test)]
    old_session: Option<gpui_kit::WeakEntity<ClusterSession>>,
    #[cfg(test)]
    old_session_gone_at_connect: Vec<bool>,
    /// Test hook: the start scope of every session that was created.
    #[cfg(test)]
    connected_scopes: Vec<Option<NamespaceScope>>,
    /// The open cluster and its session. `None` before the first start and between a release and
    /// its deferred connect.
    active_session: Option<ActiveSession>,
    screen: Screen,
    pod_table: Entity<TableState<PodTableDelegate>>,
    node_table: Entity<TableState<NodeTableDelegate>>,
    issue_table: Entity<TableState<IssueTableDelegate>>,
    kind_table: Entity<TableState<KindTableDelegate>>,
    /// The Topology screen: its graph, canvas, and toolbar.
    topology: Entity<TopologyView>,
    _table_subscriptions: Vec<Subscription>,
    /// The row cursor: the row of the table that is highlighted, in the cluster it came from.
    /// The drawer shows it while `drawer.is_open`; closing the drawer keeps it.
    selected: Option<ClusterObject>,
    /// The table row the shell itself just selected. Its `SelectRow` echo moves the cursor but
    /// never opens the drawer, which only a click does (`take_row_echo`).
    row_echo: Option<usize>,
    /// The drawer has the keyboard: it was opened with Enter or clicked. PageUp, PageDown, Home, and
    /// End then scroll its body; a click on a table row or closing the drawer gives the keys back.
    is_drawer_keyed: bool,
    /// Enter asked for the drawer of a row with no cursor yet; the `SelectRow` that opens it hands
    /// the keys to it.
    opens_drawer_keyed: bool,
    drawer: DrawerState,
    /// The debounced start of the drawer watches (object events, related objects) that is waiting
    /// for the selection to rest. Replacing or dropping it cancels it.
    pending_subjects: Option<PendingSubjects>,
    /// A reveal that waits for its list to load before it clears a filter hiding the row.
    pending_reveal: Option<ClusterObject>,
    /// Where `reveal_object` came from, for Alt+Left and Alt+Right.
    navigation: NavigationHistory,
    dock: Entity<Dock>,
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
    /// What Overview keeps between renders: the range of Recent changes.
    overview: OverviewState,
    /// The picker popover: which trigger is open, and the draft.
    namespace_picker: NamespacePickerState,
    /// The picker's type-to-filter input, shared by its two triggers.
    namespace_filter: Entity<InputState>,
    /// Whether Reveal and Copy work: decided once from the launch options, and the only thing the
    /// values view and the menus read (`value_access`).
    secret_value_access: ValueAccess,
    /// The clear of the last copied value. It lives here, not in the drawer, so it survives the
    /// drawer closing and a context switch. A new copy replaces it.
    clipboard_clear: Option<ArmedClear>,
    /// The notice that a filter unticked hidden rows; it clears itself after a few seconds.
    unticked_notice: Option<UntickedNotice>,
    /// Clears an armed copy when the app quits (best effort).
    _clipboard_quit: Subscription,
    /// Re-renders the title bar when a setting or a settings notice changes.
    _settings_observer: Subscription,
    /// Saves the dock height when a resize ends (spec 0044).
    _dock_split_events: Subscription,
}

impl AppShell {
    #[cfg_attr(
        feature = "hotpath-profiling",
        hotpath::measure(impl_type = "AppShell")
    )]
    pub(crate) fn new(options: LaunchOptions, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let secret_value_access = value_access(&options);
        let catalog = CatalogHandle::of(cx);
        let catalog_observer = cx.observe(&catalog, |shell, _, cx| shell.on_catalog_changed(cx));
        // Only an explicit `--kubeconfig` limits which saved `last_used` may pick the start cluster.
        let explicit_files = options
            .kubeconfig
            .is_some()
            .then(|| catalog.read(cx).chain_files().to_vec());

        let shell = cx.weak_entity();
        let dock = cx.new(|_| Dock::new(shell.clone()));
        let dock_split = cx.new(|_| ResizableState::default());
        let dock_split_events =
            cx.subscribe(&dock_split, |_, split, _: &ResizablePanelEvent, cx| {
                Self::save_dock_height(&split, cx);
            });
        let saved_tables = AppSettings::get(cx).tables.clone();
        let pod_table = cx.new(|cx| {
            configure(TableState::new(
                PodTableDelegate::new(
                    dock.downgrade(),
                    shell.clone(),
                    saved_tables.get(screen_key(Screen::Pods)),
                ),
                window,
                cx,
            ))
        });
        let node_table = cx.new(|cx| {
            configure(TableState::new(
                NodeTableDelegate::new(shell.clone(), saved_tables.get(screen_key(Screen::Nodes))),
                window,
                cx,
            ))
        });
        let issue_table = cx.new(|cx| {
            configure(TableState::new(
                IssueTableDelegate::new(
                    dock.downgrade(),
                    shell.clone(),
                    saved_tables.get(screen_key(Screen::Issues)),
                ),
                window,
                cx,
            ))
        });
        let topology = cx.new(|cx| TopologyView::new(shell.clone(), cx));
        let initial_kind = options.screen.screen().kind();
        let kind_table = cx.new(|cx| {
            configure(TableState::new(
                KindTableDelegate::new(initial_kind, shell, saved_tables),
                window,
                cx,
            ))
        });
        // The workspace layout depends on the dock's mode and tab count.
        let table_subscriptions = vec![
            cx.subscribe_in(&pod_table, window, Self::on_pod_table_event),
            cx.subscribe_in(&node_table, window, Self::on_node_table_event),
            cx.subscribe_in(&kind_table, window, Self::on_kind_table_event),
            cx.observe(&dock, |_, _, cx| cx.notify()),
        ];

        let quick_filter = cx.new(|cx| InputState::new(window, cx).placeholder("Filter  /"));
        let quick_filter_events =
            cx.subscribe_in(&quick_filter, window, Self::on_quick_filter_event);
        let switcher_filter =
            cx.new(|cx| InputState::new(window, cx).placeholder("Filter clusters…"));
        let switcher_filter_events =
            cx.subscribe_in(&switcher_filter, window, Self::on_switcher_filter_event);
        let namespace_filter =
            cx.new(|cx| InputState::new(window, cx).placeholder("Filter namespaces…"));
        // Typing redraws the picker; the subscription ends with the two entities.
        cx.subscribe_in(&namespace_filter, window, |_, _, _: &InputEvent, _, cx| {
            cx.notify();
        })
        .detach();
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
        let launch_filter = options.filter;
        let launch_select = options.select;
        let port_forwards = cx.new(|_| PortForwards::new());
        let usage_owner = cx.weak_entity();
        let usage = cx.new(|cx| ProcessUsage::new(usage_owner, cx));
        let forward_filter =
            cx.new(|cx| InputState::new(window, cx).placeholder("Filter targets and clusters"));
        let forward_subscriptions = vec![
            cx.subscribe(&port_forwards, |shell, _, report: &StartReport, cx| {
                shell.audit_forward_start(report, cx);
            }),
            // The shell repaints when the count of running forwards changes (sidebar, status bar),
            // while the page or a drawer is open, but not for every sample of an idle forward.
            {
                let mut last_count = 0;
                cx.observe(&port_forwards, move |shell, forwards, cx| {
                    let count = forwards.read(cx).running_count();
                    let is_shown = shell.screen == Screen::PortForwarding || shell.drawer.is_open;
                    if is_shown || count != last_count {
                        cx.notify();
                    }
                    last_count = count;
                })
            },
            cx.subscribe_in(
                &forward_filter,
                window,
                |_, _, event: &InputEvent, _, cx| {
                    if matches!(event, InputEvent::Change) {
                        cx.notify();
                    }
                },
            ),
        ];
        let mut shell = Self {
            catalog,
            _catalog_observer: catalog_observer,
            context_error: None,
            needs_pick: false,
            pending_pick_switcher: false,
            active: None,
            previous: None,
            scope_memory: ScopeMemory::new(),
            kind_cache: CustomKindCache::default(),
            switch_notice: None,
            write_notice: None,
            reset_banner: AppSettings::notice(cx).and_then(SettingsNotice::banner_text),
            value_popover: None,
            running_batches: HashSet::new(),
            delete_start: None,
            edit: None,
            revision_lookup: None,
            values_commit_open: None,
            taint_base: None,
            unlock_for: None,
            #[cfg(test)]
            last_discard: None,
            #[cfg(test)]
            last_dialog: None,
            #[cfg(test)]
            last_leaving: None,
            #[cfg(test)]
            last_node_editor: None,
            #[cfg(test)]
            last_metadata_editor: None,
            #[cfg(test)]
            last_secret_form: None,
            #[cfg(test)]
            last_bulk_label_editor: None,
            #[cfg(test)]
            last_drain_dialog: None,
            #[cfg(test)]
            last_drain_tab: None,
            is_quit_confirmed: false,
            window: window.window_handle(),
            shell_starts: shell_open::ShellStarts::default(),
            node_shell_runs: node_shell_cleanup::NodeShellRuns::default(),
            run_id: cluster::run_id(),
            #[cfg(test)]
            sweep_notices: Vec::new(),
            port_forwards,
            forward_starts: port_forward_open::ForwardStarts::default(),
            forward_filter,
            _forward_subscriptions: forward_subscriptions,
            usage,
            #[cfg(feature = "screenshot")]
            forward_fixture: options.screen.is_port_forward_fixture(),
            #[cfg(feature = "screenshot")]
            is_monitor_source_fixture: options.screen == LaunchScreen::PodMonitorSourceFixture,
            switcher: ClusterSwitcherState::new(switcher_filter),
            _switcher_filter_events: switcher_filter_events,
            pending_switcher_launch: options.screen == LaunchScreen::Switcher,
            pending_picker_launch: options.screen == LaunchScreen::NamespacePicker,
            pending_palette_launch: options.palette.clone(),
            #[cfg(test)]
            old_session: None,
            #[cfg(test)]
            old_session_gone_at_connect: Vec::new(),
            #[cfg(test)]
            connected_scopes: Vec::new(),
            active_session: None,
            screen: options.screen.screen(),
            pod_table,
            node_table,
            issue_table,
            kind_table,
            topology,
            _table_subscriptions: table_subscriptions,
            selected: None,
            row_echo: None,
            is_drawer_keyed: false,
            opens_drawer_keyed: false,
            drawer,
            pending_subjects: None,
            pending_reveal: None,
            navigation: NavigationHistory::default(),
            dock,
            dock_split,
            _dock_split_events: dock_split_events,
            // A custom launch resolves against the CRD list first, then sets this.
            pending_launch_screen: (options.screen.selects_row()
                || options.screen.has_dock()
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
            namespace_filter,
            secret_value_access,
            overview: OverviewState::default(),
            clipboard_clear: None,
            unticked_notice: None,
            _clipboard_quit: clipboard_quit,
            _settings_observer: cx.observe_global::<AppSettings>(|shell, cx| {
                // A renamed or re-colored cluster shows at once in the Cluster column.
                if shell.refresh_slot_labels(cx) {
                    shell.sync_view_sessions(cx);
                }
                shell.sync_forward_presets(cx);
                shell.sync_metrics_source(cx);
                cx.notify();
            }),
        };
        // The Settings window reads the published connection; it must not outlive the shell.
        cx.on_release(|_, cx| {
            if cx.has_global::<ActiveConnection>() {
                cx.remove_global::<ActiveConnection>();
            }
        })
        .detach();
        // However the main window goes, its pop-outs go with it: a log tab must not outlive it. An open
        // editor goes too: its inputs would outlive the app, which a debug build reports at exit.
        let (main_window, popped_dock, closing) = (
            window.window_handle().window_id(),
            shell.dock.downgrade(),
            cx.weak_entity(),
        );
        cx.on_window_closed(move |cx, closed| {
            if closed == main_window {
                let _ = popped_dock.update(cx, |dock, cx| dock.close_popped(cx));
                let _ = closing.update(cx, |shell, cx| shell.close_edit(cx));
            }
        })
        .detach();
        let is_topology = shell.screen == Screen::Topology;
        let wants_problems = options.screen == LaunchScreen::TopologyProblems;
        let wants_fixture_selection =
            options.screen == LaunchScreen::TopologyTrafficFixtureSelected;
        let wants_selection =
            options.screen == LaunchScreen::TopologySelected || wants_fixture_selection;
        let wants_rbac = options.screen == LaunchScreen::TopologyRbac;
        let wants_traffic = options.screen == LaunchScreen::TopologyTraffic;
        if wants_selection {
            // The flow of the selected edges stands still, so the capture is deterministic.
            cx.set_reduce_motion(true);
        }
        let launch_node = shell.launch_select.clone();
        shell.topology.update(cx, |view, cx| {
            view.set_problems_only(wants_problems, cx);
            view.set_rbac(wants_rbac, cx);
            view.select_first_deployment_once(wants_selection, launch_node);
            view.set_launch_zoom(options.zoom_percent);
            view.start_in_traffic(wants_traffic);
            #[cfg(feature = "screenshot")]
            if options.screen == LaunchScreen::TopologyTrafficFixture || wants_fixture_selection {
                view.show_traffic_fixture(wants_fixture_selection, cx);
            }
            view.set_visible(is_topology, cx);
        });
        if let Some(text) = launch_filter {
            shell.apply_launch_filter(&text, cx);
        }
        // A catalog with nothing to load is already done and will not notify.
        shell.on_catalog_changed(cx);
        #[cfg(feature = "screenshot")]
        if options.screen.is_port_forward_fixture() {
            shell.fill_forward_fixture(options.screen != LaunchScreen::PortForwardsList, cx);
        }
        shell
    }

    /// Starts the first session once the catalog has loaded; later catalog changes only re-render
    /// (the switcher reads the catalog) and refresh the labels of the viewed clusters.
    fn on_catalog_changed(&mut self, cx: &mut Context<Self>) {
        cx.notify();
        self.sync_forward_presets(cx);
        if self.refresh_slot_labels(cx) {
            self.sync_view_sessions(cx);
        }
        // `active` is set by the first start even while its connect is still deferred.
        let is_waiting_to_start = self.active.is_none() && self.context_error.is_none();
        if !is_waiting_to_start {
            return;
        }
        let registry = &AppSettings::get(cx).registry;
        let last_used = launch_last_used(
            registry.last_used.as_ref(),
            self.requested.explicit_files.as_deref(),
        )
        .cloned();
        let saved_stamp = registry.last_used_stamp;
        let (kubeconfigs, listed, current_stamp) = {
            let catalog = self.catalog.read(cx);
            // Wait for the file `last_used` names too: its rows are not there while it loads, and
            // the start would take another cluster (a chain file's) instead.
            let is_picked_file_loading = last_used
                .as_ref()
                .is_some_and(|cluster| catalog.is_file_loading(&cluster.kubeconfig));
            if catalog.is_loading() || is_picked_file_loading {
                return;
            }
            (
                catalog.start_kubeconfigs().cloned().collect::<Vec<_>>(),
                catalog.kubeconfigs().cloned().collect::<Vec<_>>(),
                last_used
                    .as_ref()
                    .and_then(|cluster| catalog.folder_stamp_of(&cluster.kubeconfig)),
            )
        };
        if listed.is_empty() {
            return;
        }
        // A file of a watched folder starts a session only as the exact cluster the user picked
        // last time, and only while the file is as it was then: never through `--context`,
        // `current-context`, or the first-file fallback.
        match folder_start(
            self.requested.context.as_deref(),
            last_used.as_ref(),
            (saved_stamp, current_stamp),
            &kubeconfigs,
            &listed,
        ) {
            FolderStart::NotAFolderFile => {}
            FolderStart::Start(cluster) => {
                let namespace = self.take_launch_request().1;
                self.switch_to(&cluster, namespace, cx);
                return;
            }
            FolderStart::Changed => {
                self.ask_for_a_pick(cx);
                return;
            }
        }
        if kubeconfigs.is_empty() {
            self.ask_for_a_pick(cx);
            return;
        }
        let (requested, namespace) = self.take_launch_request();
        match resolve_start(&kubeconfigs, requested.as_deref(), last_used.as_ref()) {
            Ok((_, summary)) => self.switch_to(&ClusterRef::of(&summary), namespace, cx),
            Err(error) => self.context_error = Some(error_text(&error)),
        }
    }

    /// The launch request of the first start: the `--context` and the `--namespace`. The explicit
    /// files are spent with it.
    fn take_launch_request(&mut self) -> (Option<String>, Option<NamespaceScope>) {
        self.requested.explicit_files = None;
        (
            self.requested.context.take(),
            self.requested.namespace.take(),
        )
    }

    /// Nothing may start on its own: says so, and opens the switcher once.
    fn ask_for_a_pick(&mut self, cx: &mut Context<Self>) {
        if self.needs_pick {
            return;
        }
        self.needs_pick = true;
        self.pending_pick_switcher = true;
        cx.notify();
    }

    /// Opens the switcher after `ask_for_a_pick`. It runs from `render` because opening needs a
    /// window.
    fn open_pending_pick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.pending_pick_switcher {
            return;
        }
        self.pending_pick_switcher = false;
        self.open_cluster_switcher(window, cx);
    }

    /// Switches to `target`: the open session is released first. Nothing happens when it already
    /// is the open cluster. Every start goes through here.
    pub(crate) fn switch_cluster(&mut self, target: &ClusterRef, cx: &mut Context<Self>) {
        self.switch_cluster_in_scope(target, None, cx);
    }

    /// `switch_cluster`, starting the target in `scope` when given: for this switch it wins over
    /// the remembered and the saved default scope (0026 decision 3), as `--namespace` does. The
    /// scope rides through the leaving-work dialog, so it applies after Continue.
    pub(crate) fn switch_cluster_in_scope(
        &mut self,
        target: &ClusterRef,
        scope: Option<NamespaceScope>,
        cx: &mut Context<Self>,
    ) {
        // The open cluster leaves; a switch to the open one changes nothing.
        let leaving: Vec<ClusterRef> = self
            .open_cluster()
            .into_iter()
            .filter(|cluster| cluster != target)
            .collect();
        let work = self.leaving_work(&leaving, cx);
        if work.is_empty() {
            self.switch_to(target, scope, cx);
            return;
        }
        let target = target.clone();
        self.confirm_leaving(
            work,
            move |shell, cx| shell.switch_to(&target, scope, cx),
            cx,
        );
    }

    /// `requested` wins over the remembered and the saved default scope. It is the `--namespace`
    /// scope of the first start, or the scope a palette `@` switch carries
    /// (`switch_cluster_in_scope`).
    fn switch_to(
        &mut self,
        target: &ClusterRef,
        requested: Option<NamespaceScope>,
        cx: &mut Context<Self>,
    ) {
        let kubeconfigs: Vec<Arc<Kubeconfig>> =
            self.catalog.read(cx).kubeconfigs().cloned().collect();
        let Some((_, summary)) = find_cluster(&kubeconfigs, target) else {
            self.switch_notice = Some(format!(
                "'{}' is no longer in its kubeconfig",
                target.context
            ));
            cx.notify();
            return;
        };
        let is_active = self
            .active
            .as_ref()
            .is_some_and(|active| target.is_of(active));
        if is_active {
            return;
        }
        self.switch_notice = None;
        self.context_error = None;
        self.needs_pick = false;
        let profile = AppSettings::get(cx).registry.profile(&summary);
        // The first start has nothing to release and keeps the launch filter and screen request.
        let Some(current) = self.active.as_ref().map(ClusterRef::of) else {
            let namespace = requested.or_else(|| start_scope(&self.scope_memory, target, &profile));
            self.active = Some(summary);
            self.connect_active(target.clone(), namespace, cx);
            return;
        };
        // The release writes the scopes the user had, so the memory is read after it.
        self.release_all(cx);
        let namespace = requested.or_else(|| start_scope(&self.scope_memory, target, &profile));
        if current != *target {
            self.previous = Some(current);
        }
        self.active = Some(summary);
        cx.notify();
        // Break before make: the deferred call runs after the old session entities were released,
        // so no two watch sets exist at once.
        let (shell, target) = (cx.weak_entity(), target.clone());
        cx.defer(move |cx| {
            let _ = shell.update(cx, |shell, cx| {
                shell.connect_active(target, namespace, cx);
            });
        });
    }

    /// Releases every session and everything that belongs to the clusters they served. The custom
    /// kind definitions seen so far wait in `kind_cache` for the next session. What the user had
    /// (the scope of each cluster, whether it answered) is kept first.
    fn release_all(&mut self, cx: &mut Context<Self>) {
        // Every session goes, so the edit of one of them cannot be applied any more, and no drain
        // can go on.
        self.edit = None;
        let open = self.open_cluster();
        self.stop_drains_of(open.as_slice(), cx);
        self.clear_selection(cx);
        self.dock.update(cx, |dock, cx| dock.close_all(cx));
        if let Some(open) = &self.active_session {
            self.kind_cache = open
                .session
                .update(cx, |session, _| session.take_custom_kind_cache());
        }
        #[cfg(test)]
        {
            self.old_session = self
                .active_session
                .as_ref()
                .map(|open| open.session.downgrade());
        }
        if let Some(open) = &self.active_session {
            record_leaving(open, &mut self.scope_memory, &mut self.switcher, cx);
        }
        // The delegates and the graph hold the session too: they must let go before the entity
        // can be released, and before the next session connects.
        let released = self.active_session.take();
        self.sync_active_connection(cx);
        self.sync_view_sessions(cx);
        drop(released);
        // A filter, a pending reveal, or a picker draft written for one cluster would surprise in
        // another. The screen, the dock height, and the column prefs stay.
        self.clear_all_filters(cx);
        self.namespace_picker = NamespacePickerState::default();
        self.pending_launch_screen = None;
        self.pending_reveal = None;
        self.pending_custom_launch = None;
        self.pending_dialog_launch = None;
        // The places hold objects of the cluster that left.
        self.navigation.clear();
    }

    /// Creates the only session, for the active target. It runs after `release_all` released the
    /// old ones.
    fn connect_active(
        &mut self,
        target: ClusterRef,
        namespace: Option<NamespaceScope>,
        cx: &mut Context<Self>,
    ) {
        // A switch that came in between wins; its own deferred connect follows.
        if !self
            .active
            .as_ref()
            .is_some_and(|active| target.is_of(active))
        {
            return;
        }
        #[cfg(test)]
        if let Some(old) = &self.old_session {
            self.old_session_gone_at_connect
                .push(old.upgrade().is_none());
        }
        #[cfg(test)]
        self.connected_scopes.push(namespace.clone());
        let Some(open) = self.new_session(&target, namespace, cx) else {
            // The catalog reloaded between the switch and this call.
            self.active = None;
            self.context_error = Some(format!(
                "'{}' is no longer in its kubeconfig",
                target.context
            ));
            cx.notify();
            return;
        };
        let label =
            cluster_environment_label(&open.profile.display_name, &open.profile.environment);
        self.dock
            .update(cx, |dock, cx| dock.set_environment_label(label, cx));
        self.active_session = Some(open);
        self.sync_view_sessions(cx);
    }

    /// "Back to {previous}" after a failed switch.
    pub(crate) fn back_to_previous(&mut self, cx: &mut Context<Self>) {
        if let Some(previous) = self.previous.clone() {
            self.switch_cluster(&previous, cx);
        }
    }

    /// The text the switcher shows for `cluster`; `None` once it left every loaded kubeconfig.
    pub(crate) fn cluster_label(&self, cluster: &ClusterRef, cx: &App) -> Option<String> {
        self.catalog
            .read(cx)
            .groups(cx)
            .into_iter()
            .flat_map(|group| group.rows)
            .find(|row| row.cluster == *cluster)
            .map(|row| row.label)
    }

    /// The active cluster's switcher text, for the failure and busy views.
    pub(crate) fn active_label(&self, cx: &App) -> Option<String> {
        self.cluster_label(&ClusterRef::of(self.active.as_ref()?), cx)
    }

    /// The label of the cluster "Back to" returns to, when it still resolves.
    pub(crate) fn previous_label(&self, cx: &App) -> Option<String> {
        self.cluster_label(self.previous.as_ref()?, cx)
    }

    // ---- cluster switcher ----

    pub(crate) fn switcher(&self) -> &ClusterSwitcherState {
        &self.switcher
    }

    /// The health of the viewed row, which comes from its session and never from a probe.
    fn viewed_health(&self, cx: &App) -> Option<ViewedCluster> {
        self.active_session.as_ref().map(|open| {
            let health = match open.session.read(cx).phase() {
                SessionPhase::Connecting { .. } => RowHealth::Connecting,
                SessionPhase::Failed { .. } => RowHealth::Unreachable,
                SessionPhase::Live(live) if live.has_problem() => RowHealth::Interrupted,
                SessionPhase::Live(live) => RowHealth::Live(live.api_latency),
            };
            ViewedCluster {
                cluster: open.cluster.clone(),
                health,
            }
        })
    }

    /// Every row of the switcher, unfiltered: the `Ctrl n` numbers read this list.
    fn all_switcher_sections(&self, cx: &App) -> Vec<SwitcherSection> {
        let groups = self.catalog.read(cx).groups(cx);
        switcher_sections(
            &groups,
            self.switcher.health(),
            self.viewed_health(cx).as_slice(),
        )
    }

    /// The first row of the filtered list, where the highlight starts.
    fn first_visible_cluster(&self, cx: &App) -> Option<ClusterRef> {
        let filter = self.switcher.filter().read(cx).value();
        let visible = visible_sections(
            &self.all_switcher_sections(cx),
            &filter,
            self.switcher.segment(),
        );
        move_highlight(&visible, None, HighlightStep::Next)
    }

    /// What the popover shows now; the content closure of the popover owns it.
    pub(crate) fn switcher_content(
        &self,
        shell: gpui_kit::WeakEntity<Self>,
        cx: &App,
    ) -> SwitcherContent {
        let all = self.all_switcher_sections(cx);
        let filter_text = self.switcher.filter().read(cx).value().to_string();
        let list = match (all.is_empty(), self.catalog.read(cx).is_loading()) {
            (false, _) => SwitcherList::Clusters,
            (true, true) => SwitcherList::LoadingCatalog,
            (true, false) => SwitcherList::NoClusters,
        };
        SwitcherContent {
            list,
            sections: visible_sections(&all, &filter_text, self.switcher.segment()),
            all_count: row_count(&all),
            connected_count: connected_count(&all),
            segment: self.switcher.segment(),
            highlight: self.switcher.highlight().cloned(),
            filter: self.switcher.filter().clone(),
            filter_text,
            shell,
        }
    }

    /// Opens the popover: clears the filter and probes the clusters whose health is stale. The
    /// kit focuses the filter itself while the popover opens.
    pub(crate) fn open_cluster_switcher(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.switcher.is_open() {
            return;
        }
        self.switcher.open();
        self.switcher
            .filter()
            .update(cx, |input, cx| input.set_value("", window, cx));
        let first = self.first_visible_cluster(cx);
        self.switcher.set_highlight(first);
        let due = self
            .switcher
            .health()
            .due(&self.probe_candidates(cx), Instant::now());
        self.start_probes(&due, cx);
        cx.notify();
    }

    /// Closes the popover and aborts the probes that still run.
    pub(crate) fn close_cluster_switcher(&mut self, cx: &mut Context<Self>) {
        if !self.switcher.is_open() {
            return;
        }
        self.switcher.close();
        cx.notify();
    }

    fn toggle_cluster_switcher(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.switcher.is_open() {
            self.close_cluster_switcher(cx);
        } else {
            self.open_cluster_switcher(window, cx);
        }
    }

    /// Typing in the filter moves the highlight to the first row that still matches.
    fn on_switcher_filter_event(
        &mut self,
        _: &Entity<InputState>,
        event: &InputEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(event, InputEvent::Change) {
            let first = self.first_visible_cluster(cx);
            self.switcher.set_highlight(first);
            cx.notify();
        }
    }

    pub(crate) fn set_switcher_segment(
        &mut self,
        segment: SwitcherSegment,
        cx: &mut Context<Self>,
    ) {
        self.switcher.set_segment(segment);
        let first = self.first_visible_cluster(cx);
        self.switcher.set_highlight(first);
        cx.notify();
    }

    pub(crate) fn move_switcher_highlight(&mut self, step: HighlightStep, cx: &mut Context<Self>) {
        let filter = self.switcher.filter().read(cx).value();
        let visible = visible_sections(
            &self.all_switcher_sections(cx),
            &filter,
            self.switcher.segment(),
        );
        let next = move_highlight(&visible, self.switcher.highlight(), step);
        self.switcher.set_highlight(next);
        cx.notify();
    }

    /// Enter: switches to the highlighted row.
    pub(crate) fn confirm_switcher_highlight(&mut self, cx: &mut Context<Self>) {
        if let Some(target) = self.switcher.highlight().cloned() {
            self.switch_from_switcher(&target, cx);
        }
    }

    /// A row click, Enter, or `Ctrl n`: closes the popover, then switches.
    pub(crate) fn switch_from_switcher(&mut self, target: &ClusterRef, cx: &mut Context<Self>) {
        self.close_cluster_switcher(cx);
        self.switch_cluster(target, cx);
    }

    /// `Ctrl n`: row `shortcut` of the unfiltered list; nothing when there is no such row.
    fn switch_to_nth_cluster(&mut self, shortcut: u8, cx: &mut Context<Self>) {
        let sections = self.all_switcher_sections(cx);
        let Some(target) = nth_cluster(&sections, shortcut).cloned() else {
            return;
        };
        self.switch_from_switcher(&target, cx);
    }

    /// Retry or Check on a row. A viewed row retries its session; any other row is probed,
    /// whatever its auth kind, because the user asked.
    pub(crate) fn probe_cluster(&mut self, target: &ClusterRef, cx: &mut Context<Self>) {
        if let Some(session) = self.session_of(target).cloned() {
            session.update(cx, |session, cx| session.retry(cx));
            return;
        }
        if self.switcher.health().is_running(target) {
            return;
        }
        self.start_probes(std::slice::from_ref(target), cx);
    }

    fn probe_candidates(&self, cx: &App) -> Vec<ProbeCandidate> {
        let viewed = self.open_cluster();
        let viewed = &viewed;
        let catalog = self.catalog.read(cx);
        catalog
            .kubeconfigs()
            .flat_map(|kubeconfig| {
                kubeconfig.contexts().iter().map(move |summary| {
                    let cluster = ClusterRef::of(summary);
                    let origin = if catalog.is_folder_source(&summary.source) {
                        RowOrigin::Folder
                    } else {
                        RowOrigin::Registry
                    };
                    ProbeCandidate {
                        is_active: viewed.as_ref() == Some(&cluster),
                        auth: kubeconfig.connection_info(summary).auth,
                        cluster,
                        origin,
                    }
                })
            })
            .collect()
    }

    /// Probes `clusters` on the tokio runtime. The subscription lives in the switcher state, so
    /// closing the popover aborts what has not answered.
    fn start_probes(&mut self, clusters: &[ClusterRef], cx: &mut Context<Self>) {
        let kubeconfigs: Vec<Arc<Kubeconfig>> =
            self.catalog.read(cx).kubeconfigs().cloned().collect();
        let targets: Vec<ProbeTarget> = clusters
            .iter()
            .filter_map(|cluster| {
                let (kubeconfig, summary) = find_cluster(&kubeconfigs, cluster)?;
                let proxy = AppSettings::get(cx).registry.profile(&summary).proxy;
                Some(ProbeTarget {
                    cluster: cluster.clone(),
                    kubeconfig,
                    context: summary.name,
                    proxy,
                })
            })
            .collect();
        if targets.is_empty() {
            return;
        }
        let started: Vec<ClusterRef> = targets
            .iter()
            .map(|target| target.cluster.clone())
            .collect();
        self.switcher.health_mut().mark_running(&started);
        let subscription = cx.global::<ClusterRuntime>().clone().subscribe(
            probe_stream(targets),
            cx,
            |shell: &mut Self, (cluster, result), _| {
                shell
                    .switcher
                    .health_mut()
                    .record(cluster, result, Instant::now());
            },
            |_, _| {},
        );
        self.switcher.track_probe(subscription);
    }

    /// Clears the notice behind the title-bar warning button.
    pub(crate) fn dismiss_notices(&mut self, cx: &mut Context<Self>) {
        self.switch_notice = None;
        self.write_notice = None;
        self.catalog
            .update(cx, |catalog, cx| catalog.clear_notices(cx));
        AppSettings::dismiss_notice(cx);
    }

    /// The skipped-kubeconfig lines and the last switch notice, for the warning button.
    pub(crate) fn notices(&self, cx: &App) -> Vec<String> {
        let catalog = self.catalog.read(cx);
        catalog
            .notices()
            .iter()
            .map(ToString::to_string)
            .chain(self.switch_notice.clone())
            .chain(self.write_notice.clone())
            .collect()
    }

    fn kubeconfig_state(&self, cx: &App) -> KubeconfigState {
        let catalog = self.catalog.read(cx);
        if catalog.is_loading() {
            return KubeconfigState::Loading;
        }
        match catalog.kubeconfigs().next() {
            Some(_) => KubeconfigState::Loaded,
            None if catalog.has_invalid_start_file() => {
                KubeconfigState::Failed(catalog.failure_text())
            }
            None => KubeconfigState::Empty(catalog.failure_text()),
        }
    }

    /// The status bar items for the app's own CPU, memory and network use.
    pub(crate) fn usage(&self) -> &Entity<ProcessUsage> {
        &self.usage
    }

    /// The session of the open cluster: what the screens read.
    pub(crate) fn session(&self) -> Option<&Entity<ClusterSession>> {
        self.active_session.as_ref().map(|open| &open.session)
    }

    /// The open cluster, with its session.
    pub(crate) fn active_session(&self) -> Option<&ActiveSession> {
        self.active_session.as_ref()
    }

    /// The open cluster, if any.
    fn open_cluster(&self) -> Option<ClusterRef> {
        self.active_session
            .as_ref()
            .map(|open| open.cluster.clone())
    }

    /// The session of `cluster`: the open one, else none. Every `*_of` helper and `guard_for`
    /// route through this, so a cluster that is not the open one is refused in one place.
    fn session_of(&self, cluster: &ClusterRef) -> Option<&Entity<ClusterSession>> {
        self.active_session
            .as_ref()
            .filter(|open| open.cluster == *cluster)
            .map(|open| &open.session)
    }

    /// The live data of `cluster` when it is the open one; a drawer, YAML, logs, or Monitor read
    /// always names the cluster of its subject.
    fn live_of<'a>(&self, cluster: &ClusterRef, cx: &'a App) -> Option<&'a LiveCluster> {
        self.session_of(cluster)?.read(cx).live()
    }

    /// The connection of `cluster` when it is the open one: the one every action on a row or the
    /// cursor must use. `None` while its session is not live.
    fn connection_of(&self, cluster: &ClusterRef, cx: &App) -> Option<ClusterConnection> {
        Some(self.live_of(cluster, cx)?.connection().clone())
    }

    /// The live data of the cluster that holds the object of the open drawer.
    fn subject_live<'a>(&self, cx: &'a App) -> Option<&'a LiveCluster> {
        self.live_of(&self.drawer_subject()?.cluster, cx)
    }

    /// `key` in the open cluster: a bare key means the one cluster there is, and a link inside a
    /// drawer stays in it (`release_all` clears the drawer and the cursor with the old session).
    fn in_context(&self, key: ResourceKey) -> Option<ClusterObject> {
        Some(ClusterObject::new(self.active_cluster()?, key))
    }

    /// What a row menu keeps of the open cluster.
    fn row_context_of(&self, cluster: &ClusterRef, cx: &App) -> Option<RowContext> {
        let open = self
            .active_session
            .as_ref()
            .filter(|open| open.cluster == *cluster)?;
        Some(open.table_session().row_context(cx))
    }

    /// The saved default namespace of `cluster`, for the Namespaces menu.
    pub(crate) fn default_namespace(&self, cluster: &ClusterRef, cx: &App) -> Option<String> {
        let open = self
            .active_session
            .as_ref()
            .filter(|open| open.cluster == *cluster)?;
        AppSettings::get(cx)
            .registry
            .profile(&open.summary)
            .default_namespace
    }

    /// The open context's profile; `None` before a session starts.
    pub(crate) fn active_profile(&self, cx: &App) -> Option<ClusterProfile> {
        let active = self.active.as_ref()?;
        Some(AppSettings::get(cx).registry.profile(active))
    }

    /// The switcher text of the open cluster `cluster`, for notices; `None` for any other.
    pub(super) fn label_of(&self, cluster: &ClusterRef) -> Option<String> {
        self.active_session
            .as_ref()
            .filter(|open| open.cluster == *cluster)
            .map(|open| open.label.clone())
    }

    /// The write-guard inputs of `cluster`: the permissions, lock, and profile of its own session.
    /// `None` when `cluster` is not the open one or its session is not live. A caller always names
    /// the cluster of the row or action it acts on, so no guardrail reads the open one by default.
    pub(crate) fn guard_for<'a>(
        &'a self,
        cluster: &ClusterRef,
        cx: &'a App,
    ) -> Option<ClusterGuard<'a>> {
        self.session_of(cluster)?
            .read(cx)
            .guard(cx)
            .filter(|guard| guard.cluster == *cluster)
    }

    /// "Set as default namespace": stores `name` as the default of `cluster`, or clears it when it
    /// already is. Local only: the scope of the running session does not change; the default
    /// applies at the next start or switch.
    pub(crate) fn toggle_default_namespace(
        &mut self,
        cluster: &ClusterRef,
        name: &str,
        cx: &mut Context<Self>,
    ) {
        let cluster = cluster.clone();
        let name = name.to_owned();
        AppSettings::update(cx, |settings| {
            let entry = settings.registry.entry_mut(&cluster);
            let is_default = entry.default_namespace.as_deref() == Some(name.as_str());
            entry.default_namespace = (!is_default).then_some(name);
        });
    }

    pub(crate) fn namespace_picker(&self) -> &NamespacePickerState {
        &self.namespace_picker
    }

    pub(crate) fn namespace_filter(&self) -> &Entity<InputState> {
        &self.namespace_filter
    }

    /// Opens the picker from `anchor`, with the current scope ticked and an empty filter.
    pub(crate) fn open_namespace_picker(
        &mut self,
        anchor: PickerAnchor,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(scope) = self.live(cx).map(|live| live.scope.clone()) else {
            return;
        };
        self.namespace_picker.open(anchor, &scope);
        self.namespace_filter
            .update(cx, |input, cx| input.set_value("", window, cx));
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

    /// Sets the scope of every viewed cluster (decision 12).
    pub(crate) fn set_namespace(&mut self, scope: NamespaceScope, cx: &mut Context<Self>) {
        if self.active_session.is_none() {
            return;
        }
        // A new scope leaves the editor, so unsaved text is asked about first.
        let wanted = scope.clone();
        if self.parks_for_discard(move |shell, cx| shell.set_namespace(wanted, cx), cx) {
            return;
        }
        self.clear_selection(cx);
        // A place outside the new scope could not be shown again.
        if self.live(cx).is_none_or(|live| live.scope != scope) {
            self.navigation.clear();
        }
        if let Some(session) = self.session().cloned() {
            session.update(cx, |session, cx| session.set_scope(scope, cx));
        }
    }

    /// Opens `screen`. The explorer watch follows it: it starts for a kind screen, is replaced on
    /// a kind switch, and is dropped when leaving to Pods or Nodes.
    pub(crate) fn show_screen(&mut self, screen: Screen, cx: &mut Context<Self>) {
        // Leaving the editor asks first when it holds unsaved text; a clean one just closes.
        if self.parks_for_discard(move |shell, cx| shell.show_screen(screen, cx), cx) {
            return;
        }
        self.screen = screen;
        self.unticked_notice = None;
        self.close_value_popover(cx);
        self.drawer.tab = DrawerTab::Overview;
        self.drawer.container_tab = ContainerTab::Info;
        self.drawer.monitor = MonitorState::new();
        if let Some(session) = self.session() {
            session.update(cx, |session, cx| {
                session.set_explorer_kind(screen.kind(), cx);
                session.request_kind_access(screen.access_kind(), cx);
                session.set_issues_visible(screen == Screen::Issues);
                session.set_overview_visible(screen == Screen::Overview, cx);
                session.refresh_kind_counts(CountTrigger::Navigation, cx);
                if screen == Screen::Kind(ResourceKind::Crds) {
                    session.refresh_custom_counts(cx);
                }
            });
        }
        self.topology.update(cx, |view, cx| {
            view.set_visible(screen == Screen::Topology, cx)
        });
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
        self.clear_selection(cx);
        self.dock.update(cx, |dock, cx| dock.unzoom(cx));
    }

    /// `reveal_object` for a bare key: the object in the open cluster.
    pub(crate) fn reveal(&mut self, key: ResourceKey, cx: &mut Context<Self>) {
        if let Some(object) = self.in_context(key) {
            self.reveal_object(object, cx);
        }
    }

    /// Opens the object's screen with its row selected, replacing the drawer. A filter that hides
    /// the row is cleared, or the drawer would close at once. A list that is still loading keeps
    /// the object, and `on_session_changed` resolves it after the first snapshot; a loaded list
    /// without the row drops it.
    ///
    /// This is the one reveal that records a place for Back (spec 0056): a link, a palette Go to,
    /// an Issues row. `reveal_then` callers (key actions on another row) do not.
    pub(crate) fn reveal_object(&mut self, object: ClusterObject, cx: &mut Context<Self>) {
        // Ask before recording, so a refused discard leaves no phantom entry.
        if self.has_unsaved_edit(cx) {
            let wanted = object.clone();
            self.ask_discard(move |shell, cx| shell.reveal_object(wanted, cx), cx);
            return;
        }
        self.record_place_before_reveal(&object, cx);
        self.reveal_then(object, cx, |_, _| {});
    }

    /// `reveal`, then `then` once the selection stands. The selection is made after the
    /// ClearSelection events that `show_screen` queues, which would erase a selection made now
    /// before a still-loading list could confirm it; so a step that reads or builds on the
    /// selection must run in the same deferred closure, not after this call returns.
    pub(crate) fn reveal_then(
        &mut self,
        object: ClusterObject,
        cx: &mut Context<Self>,
        then: impl FnOnce(&mut Self, &mut Context<Self>) + 'static,
    ) {
        // The reveal leaves the editor: ask about unsaved text before anything is selected.
        if self.has_unsaved_edit(cx) {
            let wanted = object.clone();
            self.ask_discard(move |shell, cx| shell.reveal_then(wanted, cx, then), cx);
            return;
        }
        self.close_edit(cx);
        self.show_screen(object.key.screen(), cx);
        let shell = cx.weak_entity();
        cx.defer(move |cx| {
            let _ = shell.update(cx, |shell, cx| {
                shell.pending_reveal = Some(object.clone());
                shell.change_selection(Some(object), cx);
                shell.set_drawer_open(true, cx);
                shell.apply_pending_reveal(cx);
                shell.sync_selection(cx);
                then(shell, cx);
            });
        });
    }

    /// The open cluster, also while its session is still to connect.
    pub(crate) fn active_cluster(&self) -> Option<ClusterRef> {
        self.active.as_ref().map(ClusterRef::of)
    }

    /// A click on a Topology node: the drawer opens (or closes with `None`) over the graph, in the
    /// cluster the graph draws. A click is a pointer selection, so it opens the drawer, like a
    /// table row click.
    pub(crate) fn select_on_topology(&mut self, key: Option<ResourceKey>, cx: &mut Context<Self>) {
        let object = key.and_then(|key| self.in_context(key));
        let is_selected = object.is_some();
        self.change_selection(object, cx);
        self.set_drawer_open(is_selected, cx);
    }

    /// Show in Topology: the graph of the object's namespace, centered on the object. `None` for
    /// an object that is not a Service or Ingress, or has no namespace.
    pub(crate) fn show_in_topology(&mut self, key: &ResourceKey, cx: &mut Context<Self>) {
        let Some((namespace, id)) = topology_target(key) else {
            return;
        };
        self.topology
            .update(cx, |view, cx| view.show_object(&namespace, id, cx));
        self.show_screen(Screen::Topology, cx);
    }

    /// Runs `step` with `key` selected: at once when it already is, else after a reveal. A row
    /// that vanished clears the selection again, and then `step` does not run.
    fn when_selected(
        &mut self,
        object: ClusterObject,
        cx: &mut Context<Self>,
        step: impl FnOnce(&mut Self, &mut Context<Self>) + 'static,
    ) {
        if self.selected.as_ref() == Some(&object) {
            self.set_drawer_open(true, cx);
            step(self, cx);
            return;
        }
        let wanted = object.clone();
        self.reveal_then(object, cx, move |shell, cx| {
            if shell.selected.as_ref() == Some(&wanted) {
                step(shell, cx);
            }
        });
    }

    /// Clears the filter of the revealed row's table when it hides the row, once the list has
    /// loaded. Waits while the list loads; forgets a reveal the selection has moved away from.
    fn apply_pending_reveal(&mut self, cx: &mut Context<Self>) {
        let Some(object) = self.pending_reveal.clone() else {
            return;
        };
        if self.selected.as_ref() != Some(&object) {
            self.pending_reveal = None;
            return;
        }
        let key = &object.key;
        let Some(live) = self.live_of(&object.cluster, cx) else {
            return;
        };
        let found = match key {
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
        cluster: &ClusterRef,
        query: Option<String>,
        namespace: Option<String>,
        check_now: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.session_of(cluster).cloned() else {
            return;
        };
        let origin = DialogOrigin {
            shell: cx.weak_entity(),
            cluster: cluster.clone(),
        };
        let view =
            cx.new(|cx| WhoCanView::new(origin, &session, query, namespace, check_now, window, cx));
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
        cluster: &ClusterRef,
        subject: Option<String>,
        namespace: Option<String>,
        check_now: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.session_of(cluster).cloned() else {
            return;
        };
        let origin = DialogOrigin {
            shell: cx.weak_entity(),
            cluster: cluster.clone(),
        };
        let view = cx.new(|cx| {
            PermissionsView::new(origin, &session, subject, namespace, check_now, window, cx)
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

    /// The Diff button of a Deployment revision: a dialog with the line diff of the two pod templates,
    /// read through the connection of the drawer's own cluster. Nothing opens while that session is
    /// not live.
    pub(crate) fn open_revision_diff(
        &mut self,
        request: RevisionDiffRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(live) = self.subject_live(cx) else {
            return;
        };
        let connection = live.connection().clone();
        let title = request.title();
        let offer = self
            .drawer_subject()
            .map(|subject| self.roll_back_offer(cx.weak_entity(), subject.clone(), cx));
        let view = cx.new(|cx| {
            let view = RevisionDiffView::new(request, connection, cx);
            match offer {
                Some(offer) => view.with_roll_back(offer),
                None => view,
            }
        });
        window.open_dialog(cx, move |dialog, window, _| {
            dialog
                .title(title.clone())
                .w(px(REVISION_DIFF_WIDTH))
                .child(dialog_body(&view, window))
        });
    }

    /// `--screen revision-diff`: the dialog over two fixed templates, with no connection behind it.
    #[cfg(feature = "screenshot")]
    fn open_revision_diff_fixture(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        use crate::revision_diff::{RevisionSide, diff_request};
        use crate::screenshot::{REVISION_FIXTURE_NEWER, REVISION_FIXTURE_OLDER};
        let side = |replica_set: &str, revision: u64, tag: &str, is_current: bool| RevisionSide {
            replica_set: replica_set.to_owned(),
            revision: Some(revision),
            tag: Some(tag.to_owned()),
            is_current,
            created_at: None,
            change_cause: None,
        };
        let request = diff_request(
            ResourceKey::Kind {
                kind: ResourceKind::Deployments,
                namespace: Some("payments".to_owned()),
                name: "api".to_owned(),
            },
            side("api-6c8d9f", 38, "2.13.0", false),
            side("api-7d9f8c", 39, "2.14.0", true),
        );
        let title = request.title();
        let (deployment, shell) = (request.deployment.clone(), cx.weak_entity());
        // The fixture shows the enabled button: the gate of a live cluster is not asked.
        let offer = crate::revision_diff::RollBackOffer::Enabled {
            shell: shell.clone(),
            subject: ClusterObject::new(
                crate::screenshot::shell_fixture_target().cluster,
                deployment.clone(),
            ),
        };
        let view = cx.new(|_| {
            RevisionDiffView::fixture(request, REVISION_FIXTURE_OLDER, REVISION_FIXTURE_NEWER, 2)
                .with_go_to(deployment, shell)
                .with_roll_back(offer)
        });
        window.open_dialog(cx, move |dialog, window, _| {
            dialog
                .title(title.clone())
                .w(px(REVISION_DIFF_WIDTH))
                .child(dialog_body(&view, window))
        });
    }

    /// Opens the Test traffic dialog with the defaults for `policy` (the pod it selects as the
    /// destination); without one, the first two pods.
    pub(crate) fn open_traffic_test(
        &mut self,
        cluster: &ClusterRef,
        policy: Option<&NetworkPolicySummary>,
        check_now: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.session_of(cluster).cloned() else {
            return;
        };
        let form = match session.read(cx).live() {
            Some(live) => traffic_defaults(live.pods.items(), policy),
            None => return,
        };
        let origin = DialogOrigin {
            shell: cx.weak_entity(),
            cluster: cluster.clone(),
        };
        let view = cx.new(|cx| TrafficTestView::new(origin, &session, form, check_now, window, cx));
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
        match self.drawer_subject().map(|object| &object.key) {
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

    /// `--screen switcher`: opens the popover once the session is live. It runs from `render`
    /// because opening needs a window.
    fn open_pending_switcher(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.pending_switcher_launch || self.live(cx).is_none() {
            return;
        }
        self.pending_switcher_launch = false;
        self.open_cluster_switcher(window, cx);
    }

    /// `--screen namespace-picker`: opens the picker once the session is live.
    fn open_pending_namespace_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.pending_picker_launch || self.live(cx).is_none() {
            return;
        }
        self.pending_picker_launch = false;
        self.open_namespace_picker(PickerAnchor::TitleBar, window, cx);
    }

    /// Opens the `--screen` dialog once the session is live. It runs from `render` because a
    /// dialog needs a window.
    fn open_pending_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(launch) = self.pending_dialog_launch else {
            return;
        };
        // A fixture dialog is drawn from fixed data, so it does not wait for a cluster.
        #[cfg(feature = "screenshot")]
        if matches!(
            launch,
            LaunchScreen::NodeShellOptions | LaunchScreen::DebugContainerOptions
        ) {
            self.open_options_fixture(launch, window, cx);
            self.pending_dialog_launch = None;
            return;
        }
        #[cfg(feature = "screenshot")]
        if matches!(
            launch,
            LaunchScreen::NodeShellConfirm | LaunchScreen::NodeShellConfirmStaging
        ) {
            let environment = if launch == LaunchScreen::NodeShellConfirm {
                crate::environment::Environment::PRODUCTION
            } else {
                crate::environment::Environment::STAGING
            };
            self.open_node_shell_confirm_fixture(environment, window, cx);
            self.pending_dialog_launch = None;
            return;
        }
        #[cfg(feature = "screenshot")]
        if matches!(
            launch,
            LaunchScreen::NodeTaintsEditor
                | LaunchScreen::NodeTaintsEditorInvalid
                | LaunchScreen::NodeLabelsEditor
        ) {
            let kind = if launch != LaunchScreen::NodeLabelsEditor {
                node_editor::NodeEditKind::Taints
            } else {
                node_editor::NodeEditKind::Labels
            };
            let extra_taints: &[(&str, &str, &str)] =
                if launch == LaunchScreen::NodeTaintsEditorInvalid {
                    &[
                        ("bad key!", "x", "NoSchedule"),
                        ("maintenance", "", "NoExecute"),
                    ]
                } else {
                    &[]
                };
            self.open_node_editor_fixture(kind, extra_taints, window, cx);
            self.pending_dialog_launch = None;
            return;
        }
        #[cfg(feature = "screenshot")]
        if matches!(
            launch,
            LaunchScreen::DrainDialog | LaunchScreen::DrainDialogSkipPdbs
        ) {
            self.open_drain_fixture(launch, window, cx);
            self.pending_dialog_launch = None;
            return;
        }
        #[cfg(feature = "screenshot")]
        if launch == LaunchScreen::NodeLabelsBulkEditor {
            self.open_bulk_label_fixture(window, cx);
            self.pending_dialog_launch = None;
            return;
        }
        #[cfg(feature = "screenshot")]
        if launch == LaunchScreen::LeftoverSweepFixture {
            self.open_leftover_fixture(window, cx);
            self.pending_dialog_launch = None;
            return;
        }
        #[cfg(feature = "screenshot")]
        if matches!(
            launch,
            LaunchScreen::ShellConfirmFixture | LaunchScreen::AttachConfirm
        ) {
            self.open_shell_confirm_fixture(launch, window, cx);
            self.pending_dialog_launch = None;
            return;
        }
        #[cfg(feature = "screenshot")]
        if launch.is_port_forward_fixture() {
            match launch {
                LaunchScreen::PortForwardNewFixture => self.open_new_forward_fixture(window, cx),
                LaunchScreen::PortForwardConfirmFixture => {
                    self.open_forward_confirm_fixture(window, cx);
                }
                LaunchScreen::PortForwardRemoveFixture => {
                    self.open_remove_preset_fixture(window, cx)
                }
                _ => {}
            }
            self.pending_dialog_launch = None;
            return;
        }
        #[cfg(feature = "screenshot")]
        if matches!(
            launch,
            LaunchScreen::DeleteConfirm
                | LaunchScreen::DeleteBulkConfirm
                | LaunchScreen::RestartPodConfirm
                | LaunchScreen::EvictConfirm
        ) {
            self.open_delete_fixture(launch, window, cx);
            self.pending_dialog_launch = None;
            return;
        }
        #[cfg(feature = "screenshot")]
        if launch == LaunchScreen::DefaultClassConfirm {
            self.open_default_class_fixture(window, cx);
            self.pending_dialog_launch = None;
            return;
        }
        #[cfg(feature = "screenshot")]
        if launch == LaunchScreen::RenewConfirm {
            self.open_renew_fixture(window, cx);
            self.pending_dialog_launch = None;
            return;
        }
        #[cfg(feature = "screenshot")]
        if launch == LaunchScreen::ExpandConfirm {
            self.open_expand_fixture(window, cx);
            self.pending_dialog_launch = None;
            return;
        }
        #[cfg(feature = "screenshot")]
        if launch == LaunchScreen::HpaRangePopover {
            self.open_hpa_range_fixture(window, cx);
            self.pending_dialog_launch = None;
            return;
        }
        #[cfg(feature = "screenshot")]
        if launch == LaunchScreen::RevisionDiff {
            self.open_revision_diff_fixture(window, cx);
            self.pending_dialog_launch = None;
            return;
        }
        #[cfg(feature = "screenshot")]
        if launch == LaunchScreen::EditYamlDiff {
            self.open_edit_fixture(crate::yaml_edit::EditTab::Diff, window, cx);
            self.pending_dialog_launch = None;
            return;
        }
        #[cfg(feature = "screenshot")]
        if launch == LaunchScreen::EditYamlHistory {
            self.open_edit_fixture(crate::yaml_edit::EditTab::History, window, cx);
            self.pending_dialog_launch = None;
            return;
        }
        #[cfg(feature = "screenshot")]
        if launch == LaunchScreen::ValuesEdit {
            self.open_values_fixture(window, cx);
            self.pending_dialog_launch = None;
            return;
        }
        #[cfg(feature = "screenshot")]
        if launch == LaunchScreen::NewConfigMap {
            self.open_create_fixture(window, cx);
            self.pending_dialog_launch = None;
            return;
        }
        if self.live(cx).is_none() {
            return;
        }
        let namespace = self.tool_namespace(cx);
        let Some(cluster) = self.active_cluster() else {
            return;
        };
        match launch {
            LaunchScreen::WhoCan => {
                let query = Some("get secrets".to_owned());
                self.open_who_can(&cluster, query, namespace, true, window, cx);
            }
            LaunchScreen::CheckPermissions => {
                self.open_permissions(&cluster, None, namespace, true, window, cx);
            }
            LaunchScreen::AccountPermissions => {
                let Some(account) = self.launch_account(cx) else {
                    return;
                };
                let (subject, namespace) = match account {
                    Some((subject, namespace)) => (Some(subject), Some(namespace)),
                    None => (None, namespace),
                };
                self.open_permissions(&cluster, subject, namespace, true, window, cx);
            }
            LaunchScreen::TestTraffic => {
                let Some(policy) = self.launch_policy(cx) else {
                    return;
                };
                self.open_traffic_test(&cluster, policy.as_ref(), true, window, cx);
            }
            LaunchScreen::Shortcuts => open_shortcut_sheet(window, cx),
            // The cursor row comes from the launch row pick; until then the screen keeps waiting.
            LaunchScreen::ScalePopover => {
                let Some(subject) = self.selected.clone() else {
                    return;
                };
                self.open_scale_popover(&subject, window, cx);
            }
            // The ticks come from the launch row pick; until then the screen keeps waiting.
            #[cfg(feature = "screenshot")]
            LaunchScreen::RestartBulkConfirm => {
                if self.pending_launch_screen.is_some() {
                    return;
                }
                self.open_restart_bulk_fixture(window, cx);
            }
            #[cfg(feature = "screenshot")]
            LaunchScreen::ScaleConfirm => {
                let Some(subject) = self.selected.clone() else {
                    return;
                };
                if !self.open_scale_fixture(&subject, window, cx) {
                    return;
                }
            }
            #[cfg(feature = "screenshot")]
            LaunchScreen::UnlockConfirm => self.begin_unlock(&cluster, window, cx),
            #[cfg(feature = "screenshot")]
            LaunchScreen::CordonConfirm => {
                let is_open = self.open_cordon_fixture(&cluster, window, cx);
                if !is_open {
                    return;
                }
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

    /// Connects the session again.
    fn retry(&mut self, cx: &mut Context<Self>) {
        if let Some(session) = self.session().cloned() {
            session.update(cx, |session, cx| session.retry(cx));
        }
    }

    // ---- drawer ----

    /// The object the drawer shows: the cursor row while the drawer is open.
    pub(crate) fn drawer_subject(&self) -> Option<&ClusterObject> {
        self.selected.as_ref().filter(|_| self.drawer.is_open)
    }

    /// The width of the open drawer, zero while none is open: the bars at the bottom of the
    /// workspace stop left of it.
    pub(crate) fn open_drawer_width(&self) -> gpui_kit::Pixels {
        self.drawer_subject().map_or(px(0.), |subject| {
            self.drawer.width(DrawerSize::of(&subject.key))
        })
    }

    /// Opens or closes the drawer on the cursor row; opening without a cursor does nothing. A
    /// closing drawer stops its watches and its pending debounce, and wipes revealed Secret values.
    fn set_drawer_open(&mut self, is_open: bool, cx: &mut Context<Self>) {
        self.drawer.is_open = is_open && self.selected.is_some();
        if !self.drawer.is_open {
            self.is_drawer_keyed = false;
        }
        if !self.drawer.is_open {
            self.drop_secret_values();
        }
        self.follow_drawer_subjects(cx);
        cx.notify();
    }

    /// Folds or opens the Annotations section of the drawer's Overview.
    pub(crate) fn toggle_annotations(&mut self, cx: &mut Context<Self>) {
        self.drawer.are_annotations_open = !self.drawer.are_annotations_open;
        cx.notify();
    }

    /// Closes the drawer (the ✕ button, Esc); the row stays highlighted.
    pub(crate) fn close_drawer(&mut self, cx: &mut Context<Self>) {
        self.set_drawer_open(false, cx);
    }

    /// Drops the row cursor of every table, which closes the drawer too.
    pub(crate) fn clear_selection(&mut self, cx: &mut Context<Self>) {
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
        // 7d and 30d are read from the source alone: nothing builds sampler data for them.
        if self.drawer.monitor.range.is_long() {
            self.drawer.monitor.cache = None;
            return;
        }
        let (Some(subject), Some(live)) = (self.drawer_subject().cloned(), self.subject_live(cx))
        else {
            return;
        };
        let container = self.monitor_container(&subject.key, live, is_container_tab);
        let ticks = match subject.key {
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
            Self::monitor_subject(&key.subject.key, key.container.as_deref(), live)
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

    /// Opens the drawer of `object` on `tab`. When it is not the selection it is revealed first; a
    /// vanished row clears the selection again, and then no drawer opens on that tab.
    pub(crate) fn open_drawer_tab(
        &mut self,
        object: ClusterObject,
        tab: DrawerTab,
        cx: &mut Context<Self>,
    ) {
        self.when_selected(object, cx, move |shell, cx| {
            shell.drawer.tab = tab;
            cx.notify();
        });
    }

    /// Opens the drawer of `object` on its Overview, scrolled to the section titled `title`
    /// (Show remaining resources, Show selected pods). The scroll happens on the next paint of the
    /// drawer; a section the row lacks leaves the scroll as it is.
    pub(crate) fn open_drawer_section(
        &mut self,
        object: ClusterObject,
        title: &'static str,
        cx: &mut Context<Self>,
    ) {
        self.when_selected(object, cx, move |shell, cx| {
            shell.drawer.tab = DrawerTab::Overview;
            shell.drawer.reveal_section.set(Some(title));
            cx.notify();
        });
    }

    /// A link inside the open drawer (the node drawer's `Pods (N)`): shows its Overview scrolled to
    /// the section titled `title`, on the next paint.
    pub(crate) fn scroll_drawer_to_section(&mut self, title: &'static str, cx: &mut Context<Self>) {
        self.drawer.tab = DrawerTab::Overview;
        self.drawer.reveal_section.set(Some(title));
        cx.notify();
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
        // The release link of a drawer: its cluster is the drawer's.
        let Some(object) = self.in_context(key.clone()) else {
            return;
        };
        let subject = key;
        // After the selection: choosing a subject forgets the revision and the layout.
        self.when_selected(object, cx, move |shell, cx| {
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
        object: ClusterObject,
        action: SecretAction,
        cx: &mut Context<Self>,
    ) {
        if self.secret_value_access == ValueAccess::Blocked {
            return;
        }
        let subject = object.key.clone();
        self.when_selected(object, cx, move |shell, cx| {
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
        let subject = values_subject(
            self.drawer_subject().map(|object| &object.key),
            self.drawer.tab,
        );
        let (Some(subject), Some(cluster)) = (
            subject,
            self.drawer_subject().map(|object| object.cluster.clone()),
        ) else {
            self.drop_secret_values();
            return;
        };
        let object = ClusterObject::new(cluster, subject.clone());
        let existing = self
            .drawer
            .secret_values
            .clone()
            .filter(|view| view.read(cx).is_for(&object));
        // The common frame: the view exists, so only the key list is read (and copied only when
        // it changed); the connection and names are cloned for a new view alone.
        if let Some(view) = existing {
            let keys = self
                .subject_live(cx)
                .and_then(|live| secret_of(live, &subject).map(|secret| secret.keys.clone()));
            let Some(keys) = keys else {
                self.drop_secret_values();
                return;
            };
            view.update(cx, |view, _| view.set_keys(&keys));
        } else {
            let found = self.subject_live(cx).and_then(|live| {
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
            let view = cx.new(|_| SecretValuesView::new(fetch, object, keys, access));
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
    /// it is the only place that creates a `YamlView`. The view is compared by cluster and object:
    /// the same name exists in several clusters, and the connection is the cluster's own.
    fn sync_yaml_view(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(object) = self.drawer_subject().cloned() else {
            self.drawer.yaml = None;
            return;
        };
        let Some(subject) = yaml_subject(Some(&object.key), self.drawer.tab) else {
            self.drawer.yaml = None;
            return;
        };
        if let Some(view) = &self.drawer.yaml
            && view.read(cx).is_for(&object.cluster, &subject)
        {
            return;
        }
        let Some(connection) = self
            .live_of(&object.cluster, cx)
            .map(|live| live.connection().clone())
        else {
            self.drawer.yaml = None;
            return;
        };
        let cluster = object.cluster;
        self.drawer.yaml =
            Some(cx.new(|cx| YamlView::new(connection, cluster, subject, window, cx)));
    }

    /// Keeps `drawer.helm` for the shown release revision only: the view lives exactly while a
    /// release drawer shows a Helm tab or its Overview. It runs inside `render`, so it only assigns
    /// and never notifies, and it is the only place that creates the view. A changed revision
    /// (a History button, or a new latest one) drops the old view, which wipes its texts.
    fn sync_helm_view(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let found = self.drawer_subject().cloned().and_then(|object| {
            let cluster = object.cluster;
            let key = object.key;
            let tab = shown_tab(drawer_tabs(&key), self.drawer.tab);
            let live = self.live_of(&cluster, cx)?;
            let summary = helm_release_of(live, &key)?;
            let subject = helm_subject(Some(&key), tab, Some(summary), self.drawer.helm_revision)?;
            let history = helm_history_of(live, &key, subject.0.revision);
            let source = HelmSource {
                cluster,
                connection: live.connection().clone(),
            };
            Some((key.clone(), subject, summary.revision, source, history))
        });
        let Some((key, (revision, tab), latest, source, history)) = found else {
            self.drawer.helm = None;
            return;
        };
        let existing = self
            .drawer
            .helm
            .clone()
            .filter(|view| view.read(cx).is_for(&source.cluster, &revision));
        let view = match existing {
            Some(view) => view,
            None => {
                let access = self.secret_value_access;
                let view = cx.new(|cx| {
                    HelmReleaseView::new(source, revision, latest, access, tab, window, cx)
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
        let wanted = self.kubelet_demand(cx);
        let Some(session) = self.session() else {
            return;
        };
        let is_current = session
            .read(cx)
            .live()
            .is_none_or(|live| *live.metrics.kubelet.demand() == wanted);
        if !is_current {
            session.update(cx, |session, _| session.set_kubelet_demand(wanted));
        }
    }

    /// The subject of the open drawer, from its key alone; only a workload reads its row, for
    /// the pods it owns. Disk I/O is wanted only while a Monitor tab shows: the drawer's own, or the
    /// container Monitor sub-tab of a pod.
    fn kubelet_demand(&self, cx: &App) -> KubeletDemand {
        let subject = self
            .selected
            .as_ref()
            .map(|object| &object.key)
            .and_then(|key| match key {
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
                    .subject_live(cx)?
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
        let Some(key) = self.drawer_subject().map(|object| &object.key) else {
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
        yaml_subject(
            self.drawer_subject().map(|object| &object.key),
            self.drawer.tab,
        )
        .is_some()
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
    fn change_selection(&mut self, object: Option<ClusterObject>, cx: &mut Context<Self>) -> bool {
        if object == self.selected {
            return false;
        }
        // A view of one cluster's object must not outlive a move to another cluster's object of
        // the same name.
        let moves_cluster = self.selected.as_ref().map(|old| &old.cluster)
            != object.as_ref().map(|new| &new.cluster);
        self.selected = object;
        self.request_row_access(cx);
        self.close_value_popover(cx);
        // Without a row there is nothing to show.
        if self.selected.is_none() {
            self.drawer.is_open = false;
        }
        if moves_cluster {
            self.drawer.yaml = None;
            self.drawer.helm = None;
        }
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

    /// Asks the session of the cursor row for the `update` and `delete` permissions of the row's
    /// kind, so that Edit YAML and Delete know their answer by the time a menu, a key, or the
    /// palette needs it (a drawer opened from Topology or Issues shows kinds no list screen asked
    /// for).
    fn request_row_access(&self, cx: &mut Context<Self>) {
        let Some(object) = &self.selected else {
            return;
        };
        let Some(kind) = delete_kind(&object.key) else {
            return;
        };
        if let Some(session) = self.session_of(&object.cluster) {
            session.update(cx, |session, cx| {
                session.request_kind_access(Some(kind), cx)
            });
        }
    }

    /// A service account drawer shows Can do, which needs the RBAC snapshot. Only an idle
    /// snapshot is requested here: a failed one waits for the Retry button instead of looping.
    fn request_rbac_for_account(&self, cx: &mut Context<Self>) {
        if self.drawer_account().is_none() {
            return;
        }
        let Some(session) = self
            .selected
            .as_ref()
            .and_then(|object| self.session_of(&object.cluster))
            .cloned()
        else {
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

    /// Points the drawer's watches (object events, related objects) at the selected object, in the
    /// session of its cluster. Stopping is immediate, also for the watches of the cluster the
    /// subject left. A start waits for the selection to rest, so arrowing through rows sends no
    /// request per row: one timer starts every pending subject together, and a newer selection
    /// replaces it. A watch whose subject does not change keeps running. It also runs when a
    /// session changes, because a row that was not loaded at selection time (a reveal) only now
    /// tells what to watch; an unchanged pending start keeps its timer.
    fn follow_drawer_subjects(&mut self, cx: &mut Context<Self>) {
        self.request_rbac_for_account(cx);
        let Some(cluster) = self.drawer_subject().map(|object| object.cluster.clone()) else {
            // No subject any more: the watches of the one open cluster stop.
            self.pending_subjects = None;
            if let Some(open) = self.active_cluster() {
                self.set_event_subject(&open, None, cx);
                self.set_related_subject(&open, None, cx);
                self.set_last_log(&open, None, cx);
            }
            return;
        };
        let next_events = self
            .drawer_subject()
            .and_then(|object| event_subject(&object.key));
        let next_related = self.selected_related_subject(cx);
        // The previous log line of a crash-looping pod is one cheap request, so it starts at once and
        // is cached per restart count: no timer is needed to keep arrowing cheap.
        let next_last_log = self.selected_last_log_key(cx);
        self.set_last_log(&cluster, next_last_log, cx);
        let (running_events, running_related) =
            self.subject_live(cx).map_or((None, None), |live| {
                (
                    live.event_subject().cloned(),
                    live.related_subject().cloned(),
                )
            });
        let mut pending = PendingSubjects {
            cluster: Some(cluster.clone()),
            ..PendingSubjects::default()
        };
        match subject_change(running_events.as_ref(), next_events) {
            SubjectChange::Keep => {}
            SubjectChange::Stop => self.set_event_subject(&cluster, None, cx),
            SubjectChange::Start(subject) => {
                self.set_event_subject(&cluster, None, cx);
                pending.events = Some(subject);
            }
        }
        match subject_change(running_related.as_ref(), next_related) {
            SubjectChange::Keep => {}
            SubjectChange::Stop => self.set_related_subject(&cluster, None, cx),
            SubjectChange::Start(subject) => {
                self.set_related_subject(&cluster, None, cx);
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
                    shell.set_event_subject(&cluster, events, cx);
                }
                if related.is_some() {
                    shell.set_related_subject(&cluster, related, cx);
                }
            });
        }));
        self.pending_subjects = Some(pending);
    }

    /// The crash-looping container of the open Pod drawer whose previous log line the WHY box quotes.
    fn selected_last_log_key(&self, cx: &App) -> Option<LastLogKey> {
        let key = &self.drawer_subject()?.key;
        let live = self.subject_live(cx)?;
        let pod = live.pods.items().iter().find(|pod| key.is_pod(pod))?;
        last_log_key(pod)
    }

    fn set_last_log(
        &mut self,
        cluster: &ClusterRef,
        key: Option<LastLogKey>,
        cx: &mut Context<Self>,
    ) {
        if let Some(session) = self.session_of(cluster).cloned() {
            session.update(cx, |session, cx| session.set_last_log(key, cx));
        }
    }

    /// What the selected row needs watched besides its events; `None` while its list has not
    /// loaded the row.
    fn selected_related_subject(&self, cx: &App) -> Option<RelatedSubject> {
        let key = &self.drawer_subject()?.key;
        let live = self.subject_live(cx)?;
        if let Some(subject) = key_related_subject(key) {
            return Some(subject).filter(|subject| !is_related_denied(subject, &live.access));
        }
        let ResourceKey::Kind { kind, .. } = key else {
            return None;
        };
        let row = live
            .kind_list(*kind)?
            .list
            .items()
            .iter()
            .find(|row| key.is_row(*kind, row))?;
        related_subject(*kind, row).filter(|subject| !is_related_denied(subject, &live.access))
    }

    fn set_related_subject(
        &mut self,
        cluster: &ClusterRef,
        subject: Option<RelatedSubject>,
        cx: &mut Context<Self>,
    ) {
        if let Some(session) = self.session_of(cluster).cloned() {
            session.update(cx, |session, cx| session.set_related_subject(subject, cx));
        }
    }

    fn set_event_subject(
        &mut self,
        cluster: &ClusterRef,
        subject: Option<InvolvedObject>,
        cx: &mut Context<Self>,
    ) {
        if let Some(session) = self.session_of(cluster).cloned() {
            session.update(cx, |session, cx| session.set_event_subject(subject, cx));
        }
    }

    /// `Export report`: opens the save dialog, then builds the report from the snapshots of that
    /// moment and writes it to the chosen path. Nothing is written unless the user confirms a path
    /// (C9), and no path or file name is traced.
    pub(crate) fn export_overview_report(&mut self, cx: &mut Context<Self>) {
        if self.overview.export.is_busy() {
            return;
        }
        let Some(session) = self.session() else {
            return;
        };
        if session.read(cx).live().is_none() {
            return;
        }
        // The report must describe the cluster the file name says, so a context switch while the
        // dialog is open cancels it.
        let session_id = session.entity_id();
        let label = format!("overview-{}", session.read(cx).context());
        let name = export_file_name(&label, "md", jiff::Timestamp::now());
        self.overview.export = ExportState::Choosing;
        self.overview._export = Some(start_export(
            name,
            "report",
            move |shell: &mut Self, cx| {
                if shell.session().map(|session| session.entity_id()) != Some(session_id) {
                    return Err("the cluster changed while the dialog was open".to_owned());
                }
                shell
                    .overview_report_text(cx)
                    .map(|report| (report, ()))
                    .ok_or_else(|| {
                        "Could not save the report: the cluster is not connected".to_owned()
                    })
            },
            Self::set_overview_export,
            |_, ()| {},
            cx,
        ));
        cx.notify();
    }

    fn set_overview_export(&mut self, state: ExportState, cx: &mut Context<Self>) {
        self.overview.export = state;
        cx.notify();
    }

    /// The report of the live snapshots now; `None` when the session is not live.
    fn overview_report_text(&self, cx: &App) -> Option<String> {
        let session = self.session()?.read(cx);
        let live = session.live()?;
        Some(live_report(
            live,
            session.issues(),
            session.context(),
            self.overview.window,
            jiff::Timestamp::now(),
        ))
    }

    /// The range of Recent changes; the panel reads it at the next render.
    pub(crate) fn set_change_window(&mut self, window: ChangeWindow, cx: &mut Context<Self>) {
        if self.overview.window == window {
            return;
        }
        self.overview.window = window;
        if let Some(session) = self.session() {
            session.update(cx, |session, cx| {
                session.set_rollout_history(window.reads_replica_sets(), cx)
            });
        }
        cx.notify();
    }

    /// `View all →` of Recent changes: the Events screen with the Changes chip on. Changes are
    /// Normal events that a Warnings-only list would hide, so every event is fetched; the chip keeps
    /// the ones that mark a change, newest first (a list of Normal events sorts by last seen).
    pub(crate) fn view_all_events(&mut self, cx: &mut Context<Self>) {
        self.set_event_filter(EventFilter::All, cx);
        self.show_screen(Screen::Kind(ResourceKind::Events), cx);
        let filter = TableFilter {
            preset: Some(FilterPreset::Changes),
            ..TableFilter::default()
        };
        self.update_view(cx, move |view| view.filter = filter);
        // The input shows the filter text of its screen, which is empty now.
        self.quick_filter_screen = None;
    }

    /// The filter of the Events list of the open cluster.
    fn set_event_filter(&mut self, filter: EventFilter, cx: &mut Context<Self>) {
        if let Some(session) = self.session().cloned() {
            session.update(cx, |session, cx| session.set_event_filter(filter, cx));
        }
    }

    /// Switches the Events screen between all events and warnings only. The drawer stays;
    /// `sync_selection` closes it when its row is filtered out.
    pub(crate) fn toggle_warnings_only(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.session() else {
            return;
        };
        let next = toggled(session.read(cx).event_filter());
        self.set_event_filter(next, cx);
    }

    /// A `SelectRow` moved the cursor to `object`. A click (not the echo of a move the shell made
    /// itself) also opens the drawer and focuses the table, even on the row that is already
    /// selected. An echo never takes the focus: a snapshot that re-selects the row must not pull
    /// it out of the filter.
    fn on_row_selected<D: TableDelegate>(
        &mut self,
        object: Option<ClusterObject>,
        is_echo: bool,
        table: &Entity<TableState<D>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.change_selection(object, cx);
        if is_echo {
            return;
        }
        // A click on a row puts the keyboard back on the table; Enter on a row without a cursor
        // opens the drawer with it.
        self.is_drawer_keyed = std::mem::take(&mut self.opens_drawer_keyed);
        self.set_drawer_open(true, cx);
        focus_table(table, window, cx);
    }

    /// A right click moves the cursor to its row before the menu opens. The key actions of a menu
    /// item run on the cursor (a kit menu dispatches them when a disabled item is confirmed from
    /// the keyboard), so without this they would act on an earlier row, possibly of another cluster.
    fn move_cursor_to_clicked_row<D: TableDelegate>(
        &mut self,
        table: &Entity<TableState<D>>,
        row: usize,
        cx: &mut Context<Self>,
    ) {
        if table.read(cx).selected_row() != Some(row) {
            self.select_table_row(table, row, cx);
        }
    }

    /// Selects `row` on behalf of the shell: the cursor moves, the drawer stays as it is.
    fn select_table_row<D: TableDelegate>(
        &mut self,
        table: &Entity<TableState<D>>,
        row: usize,
        cx: &mut Context<Self>,
    ) {
        self.row_echo = Some(row);
        table.update(cx, |table, cx| table.set_selected_row(row, cx));
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
                let is_echo = take_row_echo(&mut self.row_echo, *row);
                let object = self.row_object(table, *row, cx, |live, item| {
                    live.pods.items().get(item).map(ResourceKey::of_pod)
                });
                self.on_row_selected(object, is_echo, table, window, cx);
            }
            TableEvent::RightClickedRow(Some(row)) => {
                self.move_cursor_to_clicked_row(table, *row, cx)
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
                let is_echo = take_row_echo(&mut self.row_echo, *row);
                let object = self.row_object(table, *row, cx, |live, item| {
                    live.nodes.items().get(item).map(ResourceKey::of_node)
                });
                self.on_row_selected(object, is_echo, table, window, cx);
            }
            TableEvent::RightClickedRow(Some(row)) => {
                self.move_cursor_to_clicked_row(table, *row, cx)
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
                let session = self.session()?.read(cx);
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
                let is_echo = take_row_echo(&mut self.row_echo, *row);
                let kind = self.screen.kind();
                let object = self.row_object(table, *row, cx, |live, item| {
                    let kind = kind?;
                    let explorer = live.kind_list(kind)?;
                    Some(ResourceKey::of_row(kind, explorer.list.items().get(item)?))
                });
                self.on_row_selected(object, is_echo, table, window, cx);
            }
            TableEvent::RightClickedRow(Some(row)) => {
                self.move_cursor_to_clicked_row(table, *row, cx)
            }
            TableEvent::ClearSelection => {
                self.change_selection(None, cx);
            }
            _ => {}
        }
    }

    /// The bytes the open cluster's client has sent and received; `None` without a live cluster.
    pub(crate) fn traffic(&self, cx: &App) -> Option<TrafficTotals> {
        let counter = self.live(cx)?.traffic();
        Some(TrafficTotals {
            received: counter.received(),
            sent: counter.sent(),
        })
    }

    /// The live data of the primary cluster.
    pub(crate) fn live<'a>(&self, cx: &'a App) -> Option<&'a LiveCluster> {
        self.session()?.read(cx).live()
    }

    /// The object shown at table row `row` of `table`, in the open cluster. `key_of` names the
    /// object from the live data of that cluster and the row's item index in its list.
    fn row_object<D: FilteredTable>(
        &self,
        table: &Entity<TableState<D>>,
        row: usize,
        cx: &App,
        key_of: impl FnOnce(&LiveCluster, usize) -> Option<ResourceKey>,
    ) -> Option<ClusterObject> {
        let item = table.read(cx).delegate().view()?.item_index(row)?;
        let open = self.active_session.as_ref()?;
        let live = open.session.read(cx).live()?;
        let key = key_of(live, item)?;
        Some(ClusterObject::new(open.cluster.clone(), key))
    }

    /// Opens the logs of every pod of a workload in the dock. Nothing opens without a live
    /// session or for a node.
    pub(crate) fn open_workload_logs(
        &mut self,
        cluster: &ClusterRef,
        owner: PodOwner,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(target) = LogTarget::of_workload(owner) else {
            return;
        };
        self.open_log_tab(cluster, target, window, cx);
    }

    /// Opens `target` in the dock, reading from `cluster`. Nothing opens for a cluster that is not
    /// live.
    fn open_log_tab(
        &mut self,
        cluster: &ClusterRef,
        target: LogTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(live) = self.live_of(cluster, cx) else {
            return;
        };
        let connection = live.connection().clone();
        let Some(row) = self.row_context_of(cluster, cx) else {
            return;
        };
        self.dock.update(cx, |dock, cx| {
            dock.open(LogOrigin::new(&row, connection), target, window, cx)
        });
    }

    /// What "Logs of selected" would open: the selected pod, or the workload of the selected row.
    pub(crate) fn selected_log_target(&self, cx: &App) -> Result<LogTarget, NoLogTarget> {
        // The cursor row: its keys work with the drawer closed.
        let live = self
            .selected
            .as_ref()
            .and_then(|object| self.live_of(&object.cluster, cx))
            .or_else(|| self.live(cx))
            .ok_or(NoLogTarget::NotConnected)?;
        check_logs_access(view_logs_reason(Some(live)))?;
        let key = &self.selected.as_ref().ok_or(NoLogTarget::NotLoggable)?.key;
        let target = match key {
            ResourceKey::Pod { .. } => live
                .pods
                .items()
                .iter()
                .find(|pod| key.is_pod(pod))
                .and_then(LogTarget::of_pod),
            ResourceKey::Node { .. } => None,
            ResourceKey::Kind { kind, .. } => {
                let row = live.kind_list(*kind).and_then(|explorer| {
                    explorer
                        .list
                        .items()
                        .iter()
                        .find(|row| key.is_row(*kind, row))
                });
                match row.and_then(|row| workload_logs_owner(row, live.pods.items())) {
                    // A CronJob whose last run left no pods says why.
                    Some(Err(reason)) => return Err(NoLogTarget::Unavailable(reason)),
                    Some(Ok(owner)) => LogTarget::of_workload(owner),
                    None => None,
                }
            }
        };
        target.ok_or(NoLogTarget::NotLoggable)
    }

    /// The "+ ▾" menu entry; an `Err` has no target and the menu item is disabled.
    pub(crate) fn open_logs_of_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Ok(target) = self.selected_log_target(cx) else {
            return;
        };
        let Some(cluster) = self.selected.as_ref().map(|object| object.cluster.clone()) else {
            return;
        };
        self.open_log_tab(&cluster, target, window, cx);
    }

    /// The Logs sub-tab of a container: opens or focuses the dock tab on the container shown
    /// in the drawer. Nothing happens while the logs are not permitted.
    pub(crate) fn open_container_logs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(live) = self.subject_live(cx) else {
            return;
        };
        if view_logs_reason(Some(live)).is_some() {
            return;
        }
        let Some(object) = self.drawer_subject() else {
            return;
        };
        let key = &object.key;
        let Some(pod) = live.pods.items().iter().find(|pod| key.is_pod(pod)) else {
            return;
        };
        let Some(index) = selected_container_index(pod, &self.drawer) else {
            return;
        };
        let Some(target) = LogTarget::of_container(pod, &pod.containers[index].name) else {
            return;
        };
        let cluster = object.cluster.clone();
        self.open_log_tab(&cluster, target, window, cx);
    }

    /// The logs tab of `pod`, a pod a WHY box names (a failed Job's). Nothing happens while the
    /// logs are not permitted or the pod is gone.
    pub(crate) fn open_pod_logs(
        &mut self,
        pod: &ResourceKey,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(live) = self.subject_live(cx) else {
            return;
        };
        if view_logs_reason(Some(live)).is_some() {
            return;
        }
        let Some(cluster) = self.drawer_subject().map(|object| object.cluster.clone()) else {
            return;
        };
        let Some(target) = live
            .pods
            .items()
            .iter()
            .find(|candidate| pod.is_pod(candidate))
            .and_then(LogTarget::of_pod)
        else {
            return;
        };
        self.open_log_tab(&cluster, target, window, cx);
    }

    /// Why "Shell into selected" is disabled, `None` when S would open a shell: the same answer
    /// the key reads, for the cursor row in its own cluster.
    pub(crate) fn selected_shell_reason(&self, cx: &App) -> Option<SharedString> {
        let Some(subject) = &self.selected else {
            return Some("Select a pod first".into());
        };
        let (Some(live), Some(guard)) = (
            self.live_of(&subject.cluster, cx),
            self.guard_for(&subject.cluster, cx),
        ) else {
            return Some("Not connected".into());
        };
        match key_availability(RowAction::OpenShell, &subject.key, live, &guard) {
            KeyAvailability::Run(_) => None,
            KeyAvailability::Disabled { reason } => Some(reason),
            KeyAvailability::NotOffered => Some("Select a pod first".into()),
        }
    }

    /// Publishes the live connection of the open cluster for the Settings window, or removes it
    /// when there is none. The same connection is not set again: every watch update lands here.
    fn sync_active_connection(&self, cx: &mut Context<Self>) {
        let published = self.active_session.as_ref().and_then(|open| {
            let session = open.session.read(cx);
            let live = session.live()?;
            Some(ActiveConnection {
                cluster: open.cluster.clone(),
                label: open.label.clone(),
                connection: live.connection().clone(),
                session: open.session.downgrade(),
                generation: session.generation(),
            })
        });
        let Some(published) = published else {
            if cx.has_global::<ActiveConnection>() {
                cx.remove_global::<ActiveConnection>();
            }
            return;
        };
        let is_current = cx.try_global::<ActiveConnection>().is_some_and(|current| {
            current.cluster == published.cluster
                && current.label == published.label
                && current.generation == published.generation
        });
        if !is_current {
            cx.set_global(published);
        }
    }

    /// Hands the stored metrics source of the open cluster to its session, which rebuilds the
    /// state when the entry changed.
    fn sync_metrics_source(&mut self, cx: &mut Context<Self>) {
        let Some(open) = &self.active_session else {
            return;
        };
        // The fixture sends no request to a source, whatever the settings hold.
        #[cfg(feature = "screenshot")]
        if self.is_monitor_source_fixture {
            return;
        }
        let wanted = AppSettings::get(cx).registry.profile(&open.summary).metrics;
        open.session
            .update(cx, |session, cx| session.set_metrics_source(wanted, cx));
    }

    /// The session changed. Its first Live writes `last_used` (so a cluster that fails to connect
    /// is not reopened at the next start); everything that shows rows or the drawer is then brought
    /// up to date.
    fn on_session_changed(&mut self, cluster: &ClusterRef, cx: &mut Context<Self>) {
        self.sync_forward_lock(cluster, cx);
        let Some(open) = self
            .active_session
            .as_mut()
            .filter(|open| open.cluster == *cluster)
        else {
            return;
        };
        let is_live = open.session.read(cx).live().is_some();
        if is_live && !open.has_reported_live {
            open.has_reported_live = true;
            self.on_first_live(cluster, cx);
        }
        self.apply_pending_custom_launch(cx);
        self.follow_custom_kinds(cx);
        self.rebuild_visible_view(cx, |_| {});
        self.apply_pending_launch_screen(cx);
        self.apply_pending_reveal(cx);
        self.sync_selection(cx);
        self.follow_drawer_subjects(cx);
        self.sync_active_connection(cx);
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
        // The graph has no table that could hold or lose the selection.
        if self.screen == Screen::Topology {
            return;
        }
        let Some(object) = self.selected.clone() else {
            return;
        };
        let Some(live) = self.live_of(&object.cluster, cx) else {
            return;
        };
        let key = &object.key;
        match key {
            ResourceKey::Pod { .. } => {
                let delegate = self.pod_table.read(cx).delegate();
                let Some(view) = delegate.view() else {
                    return;
                };
                let Some(found) = list_row_index(&live.pods, view, |pod| key.is_pod(pod)) else {
                    return;
                };
                let table = self.pod_table.clone();
                self.apply_selection_sync(&table, found, cx);
            }
            ResourceKey::Node { .. } => {
                let delegate = self.node_table.read(cx).delegate();
                let Some(view) = delegate.view() else {
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
                let delegate = self.kind_table.read(cx).delegate();
                let Some(view) = delegate.view() else {
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
            SelectionSync::Move(row) => self.select_table_row(table, row, cx),
            SelectionSync::Clear => {
                self.change_selection(None, cx);
                table.update(cx, |table, cx| table.clear_selection(cx));
            }
        }
    }

    /// `--screen issues-drawer`: once the issues are known, opens the object of the first one.
    fn reveal_first_issue(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.session().map(|session| session.read(cx)) else {
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
            .filter(|launch| !launch.has_dock())
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
            LaunchScreen::KindDrawer(..)
            | LaunchScreen::KindMenu(_)
            | LaunchScreen::ScalePopover
            | LaunchScreen::ScaleConfirm => {
                let Some(kind) = launch.row_kind() else {
                    return;
                };
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
                    // The cursor screen shows the first row the table does.
                    None if launch == LaunchScreen::PodsCursor => {
                        self.shown_item(&self.pod_table, 0, cx)
                    }
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
                let Some(row) = self
                    .active_cluster()
                    .and_then(|primary| self.row_of_item(&self.node_table, &primary, item, cx))
                else {
                    return;
                };
                let key = self
                    .live(cx)
                    .and_then(|live| live.nodes.items().get(item))
                    .map(ResourceKey::of_node);
                let object = key.and_then(|key| self.in_context(key));
                self.change_selection(object, cx);
                let table = self.node_table.clone();
                self.select_table_row(&table, row, cx);
                self.open_launch_drawer(launch, cx);
            }
            LaunchScreen::KindDrawer(..)
            | LaunchScreen::KindMenu(_)
            | LaunchScreen::ScalePopover
            | LaunchScreen::ScaleConfirm => {
                let Some(kind) = launch.row_kind() else {
                    return;
                };
                let Some(row) = self
                    .active_cluster()
                    .and_then(|primary| self.row_of_item(&self.kind_table, &primary, item, cx))
                else {
                    return;
                };
                let key = self
                    .live(cx)
                    .and_then(|live| live.kind_list(kind)?.list.items().get(item))
                    .map(|row| ResourceKey::of_row(kind, row));
                let object = key.and_then(|key| self.in_context(key));
                self.change_selection(object, cx);
                let table = self.kind_table.clone();
                self.select_table_row(&table, row, cx);
                self.open_launch_drawer(launch, cx);
            }
            _ => {
                let Some(row) = self
                    .active_cluster()
                    .and_then(|primary| self.row_of_item(&self.pod_table, &primary, item, cx))
                else {
                    return;
                };
                let key = self
                    .live(cx)
                    .and_then(|live| live.pods.items().get(item))
                    .map(ResourceKey::of_pod);
                let object = key.and_then(|key| self.in_context(key));
                self.change_selection(object, cx);
                let table = self.pod_table.clone();
                self.select_table_row(&table, row, cx);
                self.open_launch_drawer(launch, cx);
            }
        }
    }

    /// The drawer screens open the drawer on the row the launch selected; `pods-cursor` leaves it
    /// closed.
    fn open_launch_drawer(&mut self, launch: LaunchScreen, cx: &mut Context<Self>) {
        if launch.has_drawer() {
            self.set_drawer_open(true, cx);
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
            LaunchScreen::RestartBulkConfirm => live
                .kind_list(ResourceKind::Deployments)
                .is_none_or(|explorer| explorer.list.is_loading()),
            _ => live.pods.is_loading(),
        };
        if is_loading {
            return;
        }
        self.pending_launch_screen = None;
        for row in 0..launch.checked_count() {
            match launch {
                LaunchScreen::NodesSelected => {
                    check_table(&self.node_table, RowCheck::Toggle(row), cx)
                }
                LaunchScreen::RestartBulkConfirm => {
                    check_table(&self.kind_table, RowCheck::Toggle(row), cx)
                }
                _ => check_table(&self.pod_table, RowCheck::Toggle(row), cx),
            }
        }
    }

    /// The row of `table` that shows `item` of the list of `cluster`, which must be the open one;
    /// `None` while a filter hides it.
    fn row_of_item<D: FilteredTable>(
        &self,
        table: &Entity<TableState<D>>,
        cluster: &ClusterRef,
        item: usize,
        cx: &App,
    ) -> Option<usize> {
        self.session_of(cluster)?;
        table.read(cx).delegate().view()?.row_of(item)
    }

    /// Opens the log dock that `--screen` asked for, once the pod list has loaded. It runs
    /// from `render` because a new tab needs a window. The RBAC gate is skipped on purpose:
    /// on a denied cluster the error state is what a screenshot should show.
    fn open_pending_logs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(launch) = self
            .pending_launch_screen
            .filter(|launch| launch.has_dock())
        else {
            return;
        };
        #[cfg(feature = "screenshot")]
        if matches!(
            launch,
            LaunchScreen::DrainProgress
                | LaunchScreen::DrainProgressStuck
                | LaunchScreen::DrainProgressPending
        ) {
            self.open_drain_progress_fixture(launch, window, cx);
            return;
        }
        #[cfg(feature = "screenshot")]
        if matches!(
            launch,
            LaunchScreen::ShellFixture
                | LaunchScreen::ShellPasteFixture
                | LaunchScreen::ShellPickerFixture
                | LaunchScreen::ShellFindFixture
                | LaunchScreen::NodeShellTabFixture
                | LaunchScreen::DebugShellTabFixture
        ) {
            self.open_shell_fixture(launch, window, cx);
            return;
        }
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
        let Some(row) = self
            .active_cluster()
            .and_then(|cluster| self.row_context_of(&cluster, cx))
        else {
            return;
        };
        let mode = if matches!(
            launch,
            LaunchScreen::LogsDock | LaunchScreen::ShellDockFixture
        ) {
            DockMode::Normal
        } else {
            DockMode::Zoomed
        };
        self.dock.update(cx, |dock, cx| {
            dock.open(LogOrigin::new(&row, connection), target, window, cx);
            dock.set_mode(mode, cx);
        });
        // The dock screenshot shows both kinds of tab, with the shell active.
        #[cfg(feature = "screenshot")]
        if launch == LaunchScreen::ShellDockFixture {
            self.open_shell_fixture(launch, window, cx);
        }
    }

    /// The `--screen shell-*-fixture` screens: a shell tab that never connects, on a fixed pod of
    /// a fixed cluster, fed the transcript of the W8b pane. The dock is zoomed unless the screen
    /// shows the split. It is drawn from fixed data and waits for no cluster.
    #[cfg(feature = "screenshot")]
    fn open_shell_fixture(
        &mut self,
        launch: LaunchScreen,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.pending_launch_screen = None;
        let fixture = crate::screenshot::shell_tab_fixture(launch);
        let dock = self.dock.clone();
        let tab = dock.update(cx, |dock, cx| {
            let tab = dock.open_shell_fixture(fixture, window, cx);
            if launch != LaunchScreen::ShellDockFixture {
                dock.set_mode(DockMode::Zoomed, cx);
            }
            tab
        });
        match launch {
            LaunchScreen::ShellPasteFixture => {
                tab.update(cx, |tab, cx| tab.show_paste_fixture(window, cx));
            }
            LaunchScreen::ShellFindFixture => {
                let query = crate::screenshot::SHELL_FIXTURE_FIND;
                tab.update(cx, |tab, cx| tab.show_find_fixture(query, window, cx));
            }
            LaunchScreen::ShellPickerFixture => {
                crate::resource_actions::open_shell_picker_fixture(window, cx);
            }
            _ => {}
        }
    }

    /// `--screen logs-popout`: moves the active log tab of the dock to a window of its own.
    #[cfg(feature = "screenshot")]
    pub(crate) fn pop_out_active_log_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.dock
            .update(cx, |dock, cx| dock.pop_out_active(window, cx));
    }

    /// The window a popped-out log tab opened in.
    #[cfg(feature = "screenshot")]
    pub(crate) fn popped_log_window(&self, cx: &App) -> Option<gpui_kit::AnyWindowHandle> {
        self.dock.read(cx).popped_window()
    }

    /// The failure of a `--screen custom:` request, which a screenshot run reports instead of
    /// capturing the fallback screen.
    #[cfg(feature = "screenshot")]
    pub(crate) fn launch_failure(&self) -> Option<&str> {
        self.launch_failure.as_deref()
    }

    /// The texts a script `expect` step checks: the screen, the cursor row, the drawer subject, and
    /// the notices (see `screenshot_script.rs`).
    #[cfg(any(feature = "screenshot", test))]
    pub(crate) fn reported_texts(&self, cx: &App) -> Vec<String> {
        let mut texts = vec![format!("screen {:?}", self.screen)];
        if let Some(object) = &self.selected {
            texts.push(format!("cursor {:?}", object.key));
        }
        if let Some(subject) = self.drawer_subject() {
            texts.push(format!("drawer {:?}", subject.key));
        }
        texts.extend(self.notices(cx));
        texts
    }

    /// How far the data of one viewed cluster is, for the screen shown.
    #[cfg(feature = "screenshot")]
    fn slot_target(&self, session: &ClusterSession, _: &App) -> TargetState {
        let live = match session.phase() {
            SessionPhase::Connecting { .. } => return TargetState::Loading,
            SessionPhase::Failed { .. } => return TargetState::Unavailable,
            SessionPhase::Live(live) => live,
        };
        let (is_loading, has_failed) = match self.screen {
            // The panels read these three lists; a failed one is a state the panel shows.
            Screen::Overview => (
                live.pods.is_loading() || live.nodes.is_loading() || live.namespaces.is_loading(),
                false,
            ),
            // The graph is built from the pods and the feeds; a feed that failed draws as a gap.
            Screen::Topology => (live.pods.is_loading(), false),
            // A local list: there is no cluster data to wait for.
            Screen::PortForwarding => (false, false),
            Screen::Pods => (live.pods.is_loading(), live.pods.failure().is_some()),
            Screen::Nodes => (live.nodes.is_loading(), live.nodes.failure().is_some()),
            // The table shows what the pods and nodes lists found; a failed one is a gap the
            // coverage names.
            Screen::Issues => (live.pods.is_loading() || live.nodes.is_loading(), false),
            // A missing explorer is the moment between a switch and its first watch.
            Screen::Kind(kind) => live.kind_list(kind).map_or((true, false), |explorer| {
                (
                    explorer.list.is_loading(),
                    explorer.list.failure().is_some(),
                )
            }),
        };
        // A failed list shows an error screen, which is the target to capture.
        if has_failed {
            TargetState::Unavailable
        } else if is_loading
            || self.pending_custom_launch.is_some()
            || (self.screen == Screen::Kind(ResourceKind::Crds) && live.is_counting_instances())
            || live.kind_counts().is_running()
            || live.name_index.is_running()
            || session.is_issues_pending()
            || (matches!(self.screen, Screen::Kind(_)) && live.is_join_loading())
        {
            TargetState::Loading
        } else {
            TargetState::Loaded
        }
    }

    /// What the screenshot hook inspects to know when the screen shows its target.
    #[cfg(feature = "screenshot")]
    pub(crate) fn settle_input(&self, cx: &App) -> SettleInput {
        let target = match self.kubeconfig_state(cx) {
            KubeconfigState::Loading => TargetState::Loading,
            KubeconfigState::Empty(_) | KubeconfigState::Failed(_) => TargetState::Unavailable,
            KubeconfigState::Loaded => match self.session() {
                Some(session) => self.slot_target(session.read(cx), cx),
                // Between the release of the old session and the deferred connect of the new one,
                // a started cluster has no session yet.
                None if self.active.is_some() => TargetState::Loading,
                None => TargetState::Unavailable,
            },
        };
        // The forwards are a local list: there is no cluster data to wait for.
        let target = if self.screen == Screen::PortForwarding {
            TargetState::Loaded
        } else {
            target
        };
        // A drawer waits for the debounce, then for its events, related objects, and YAML.
        let is_content_pending = self.pending_subjects.is_some()
            || self
                .subject_live(cx)
                .is_some_and(|live| live.is_object_events_loading() || live.is_related_loading())
            || self.is_yaml_loading(cx)
            || self.is_helm_loading(cx);
        // A logs screen is pending until its tab exists and has opened its stream.
        let is_log_pending = self
            .pending_launch_screen
            .is_some_and(LaunchScreen::has_dock)
            || self.dock.read(cx).is_connecting(cx);
        SettleInput {
            target,
            is_catalog_loading: self.catalog.read(cx).is_loading(),
            is_metrics_page_pending: target != TargetState::Unavailable
                && crate::settings_window::is_metrics_page_pending(cx),
            // An empty list opens no drawer, but the launch request is resolved then, so it settles.
            is_drawer_ready: is_drawer_ready(
                self.drawer_subject().is_some(),
                self.pending_launch_screen.is_some(),
                is_content_pending,
            ),
            is_log_pending,
            is_switcher_pending: self.pending_switcher_launch
                || self.pending_picker_launch
                || self.switcher.health().is_probing(),
            is_change_feed_pending: self
                .session()
                .is_some_and(|session| session.read(cx).is_change_feed_pending()),
            is_topology_pending: self.screen == Screen::Topology
                && (self
                    .session()
                    .is_some_and(|session| session.read(cx).is_topology_pending())
                    || !self.topology.read(cx).has_build()
                    || self.topology.read(cx).is_traffic_pending()),
            is_dialog_pending: self.pending_dialog_launch.is_some()
                || self.pending_palette_launch.is_some()
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
            pod_metrics: self.feed_progress(cx, |live| FeedProgress {
                status: live.metrics.pods.status.clone(),
                ticks: live.metrics.pods.history.tick_count(),
            }),
            node_metrics: self.feed_progress(cx, |live| FeedProgress {
                status: live.metrics.nodes.status.clone(),
                ticks: live.metrics.nodes.history.tick_count(),
            }),
            kubelet: self.feed_progress(cx, |live| {
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

    // ---- command palette ----

    /// Opens the palette with `initial` typed. The snapshot is taken here because the palette
    /// cannot read the shell while this handler holds it.
    pub(crate) fn open_palette(
        &mut self,
        initial: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // The popovers share the window with the dialog: a popover left open would keep its input
        // and its keys.
        self.close_cluster_switcher(cx);
        self.close_value_popover(cx);
        self.namespace_picker.dismiss();
        let snapshot = self.palette_snapshot(&parse_query(initial), cx);
        let focus = self.focus_handle.clone();
        open_palette(initial, &cx.entity(), snapshot, focus, window, cx);
    }

    /// `--palette`: opens once the session is live, the launch row is selected, and the list the
    /// screen shows has loaded. It runs from `render` because a dialog needs a window.
    fn open_pending_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending_palette_launch.is_none()
            || self.pending_launch_screen.is_some()
            || self.is_shown_list_loading(cx)
        {
            return;
        }
        let Some(query) = self.pending_palette_launch.take() else {
            return;
        };
        self.open_palette(&query, window, cx);
    }

    /// Whether there is no session yet or the list of the open screen has not loaded.
    fn is_shown_list_loading(&self, cx: &App) -> bool {
        let Some(live) = self.live(cx) else {
            return true;
        };
        match self.screen {
            Screen::Pods => live.pods.is_loading(),
            Screen::Nodes => live.nodes.is_loading(),
            Screen::Kind(kind) => live
                .kind_list(kind)
                .is_none_or(|explorer| explorer.list.is_loading()),
            Screen::Overview | Screen::Issues | Screen::Topology | Screen::PortForwarding => false,
        }
    }

    /// Asks the open session to list the names of the palette-only kinds (spec 0056). The palette
    /// calls it from a query of two or more characters; the session decides whether a run starts.
    pub(crate) fn request_name_index(&self, cx: &mut Context<Self>) {
        let Some(open) = &self.active_session else {
            return;
        };
        open.session
            .update(cx, |session, cx| session.request_name_index(cx));
    }

    /// What the palette lists now, read from memory only: no list, watch, or request starts here.
    pub(crate) fn palette_snapshot(&self, query: &PaletteQuery<'_>, cx: &App) -> PaletteSnapshot {
        let sections = self.all_switcher_sections(cx);
        let live = self.live(cx);
        // The guard is built before the session borrows it.
        let guard = self
            .active_session
            .as_ref()
            .and_then(|open| open.session.read(cx).guard(cx));
        // Only feeds that are live and loaded: a loading or off feed is not searched.
        let feeds = live.map_or_else(Vec::new, |live| live_feed_objects(&live.issue_feeds));
        let session = self.active_session.as_ref().and_then(|open| {
            let guard = guard.as_ref()?;
            let live = open.session.read(cx).live()?;
            Some(PaletteSession {
                cluster: open.cluster.clone(),
                scope: &live.scope,
                guard,
                namespaces: live.namespaces.items(),
                pods: live.pods.items(),
                nodes: live.nodes.items(),
                kind_rows: self
                    .screen
                    .kind()
                    .and_then(|kind| Some((kind, live.kind_list(kind)?.list.items()))),
                // Only the cursor Deployment has revisions to offer, and only once its drawer
                // has loaded them.
                feeds: &feeds,
                name_index: &live.name_index,
                replica_sets: self
                    .selected
                    .as_ref()
                    .filter(|cursor| cursor.cluster == open.cluster)
                    .and_then(|cursor| match &cursor.key {
                        ResourceKey::Kind {
                            kind: ResourceKind::Deployments,
                            ..
                        } => loaded_replica_sets(
                            ResourceKind::Deployments,
                            live.row_of(&cursor.key)?,
                            live,
                        ),
                        ResourceKey::Pod { .. }
                        | ResourceKey::Node { .. }
                        | ResourceKey::Kind { .. } => None,
                    }),
            })
        });
        let input = PaletteInput {
            screen: self.screen,
            // The cursor is hidden under the Edit YAML view, so the palette offers no row action: an
            // entry would act on a row the user cannot see.
            cursor: self.selected.as_ref().filter(|_| !self.is_editing()),
            has_dock_tabs: self.dock.read(cx).has_tabs(),
            include_resources: lists_resources(query),
            query_text: query.text,
            // Like the cursor row actions, pairs would act on rows the Edit YAML view hides.
            pair_text: lists_pairs(query)
                .then_some(query.text)
                .filter(|_| !self.is_editing()),
            session,
            clusters: &sections,
        };
        PaletteSnapshot {
            entries: palette_entries(&input),
            context: PaletteContext {
                screen: self.screen,
                has_session: live.is_some(),
                has_cursor: input.cursor.is_some(),
                searched_feeds: feeds.iter().map(|feed| feed.kind).collect(),
                name_index: live
                    .map_or_else(IndexSummary::default, |live| live.name_index.summary()),
                cluster: self.active_profile(cx).map(|profile| ActiveCluster {
                    environment: profile.environment.clone(),
                    name: profile.display_name.into(),
                }),
                scope_label: live.map(|live| scope_label(&live.scope).into()),
            },
        }
    }

    /// The table row that shows `object` on the open screen; `None` for another screen, a row
    /// its cluster's list lacks, or a row the filter hides.
    fn shown_row_of(&self, object: &ClusterObject, cx: &App) -> Option<usize> {
        let key = &object.key;
        if key.screen() != self.screen {
            return None;
        }
        let cluster = &object.cluster;
        let live = self.live_of(cluster, cx)?;
        match key {
            ResourceKey::Pod { .. } => {
                let item = live.pods.items().iter().position(|pod| key.is_pod(pod))?;
                self.row_of_item(&self.pod_table, cluster, item, cx)
            }
            ResourceKey::Node { .. } => {
                let item = live
                    .nodes
                    .items()
                    .iter()
                    .position(|node| key.is_node(node))?;
                self.row_of_item(&self.node_table, cluster, item, cx)
            }
            ResourceKey::Kind { kind, .. } => {
                let rows = live.kind_list(*kind)?.list.items();
                let item = rows.iter().position(|row| key.is_row(*kind, row))?;
                self.row_of_item(&self.kind_table, cluster, item, cx)
            }
        }
    }

    /// Whether the Tab preview applies to `object`: it is a row of the open screen's filtered
    /// table, and no drawer is open (an open drawer would follow the cursor and open on the new
    /// row).
    pub(crate) fn can_preview_row(&self, object: &ClusterObject, cx: &App) -> bool {
        !self.drawer.is_open && self.shown_row_of(object, cx).is_some()
    }

    /// Tab in the palette: the table cursor moves to `object`. The screen does not change and
    /// nothing starts; with a drawer open nothing happens at all.
    pub(crate) fn preview_resource(&mut self, object: &ClusterObject, cx: &mut Context<Self>) {
        if !self.can_preview_row(object, cx) {
            return;
        }
        let Some(row) = self.shown_row_of(object, cx) else {
            return;
        };
        self.change_selection(Some(object.clone()), cx);
        match &object.key {
            ResourceKey::Pod { .. } => {
                let table = self.pod_table.clone();
                self.select_table_row(&table, row, cx);
            }
            ResourceKey::Node { .. } => {
                let table = self.node_table.clone();
                self.select_table_row(&table, row, cx);
            }
            ResourceKey::Kind { .. } => {
                let table = self.kind_table.clone();
                self.select_table_row(&table, row, cx);
            }
        }
    }

    /// The progress of a feed of the live session, or an unavailable one without it.
    #[cfg(feature = "screenshot")]
    fn feed_progress(
        &self,
        cx: &App,
        feed_of: impl Fn(&LiveCluster) -> FeedProgress,
    ) -> FeedProgress {
        self.live(cx)
            .map_or_else(FeedProgress::unavailable, feed_of)
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
            // Overview and Topology have no table.
            Screen::Overview | Screen::Topology | Screen::PortForwarding => {}
            Screen::Issues => rebuild_table(&self.issue_table, change, cx),
            Screen::Kind(_) => rebuild_table(&self.kind_table, change, cx),
        }
    }

    /// The one path of every toolkit action: change the view, then keep the selection and the
    /// drawer consistent with the rows that remain.
    fn update_view(&mut self, cx: &mut Context<Self>, change: impl FnOnce(&mut TableView)) {
        // Unticks that data refreshes made since the last change are not this change's.
        self.take_unticked_hidden(cx);
        self.rebuild_visible_view(cx, change);
        let unticked = self.take_unticked_hidden(cx);
        if unticked > 0 {
            self.announce_unticked(unticked, cx);
        }
        self.sync_selection(cx);
        cx.notify();
    }

    fn take_unticked_hidden(&self, cx: &mut Context<Self>) -> usize {
        match self.screen {
            Screen::Pods => take_unticked_hidden(&self.pod_table, cx),
            Screen::Nodes => take_unticked_hidden(&self.node_table, cx),
            Screen::Overview | Screen::Topology | Screen::PortForwarding => 0,
            Screen::Issues => take_unticked_hidden(&self.issue_table, cx),
            Screen::Kind(_) => take_unticked_hidden(&self.kind_table, cx),
        }
    }

    /// Shows how many ticked rows a filter unticked, replacing an older notice, until the
    /// timer ends or the notice is dismissed.
    fn announce_unticked(&mut self, count: usize, cx: &mut Context<Self>) {
        let expiry = cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(UNTICKED_NOTICE_LIFETIME)
                .await;
            let _ = this.update(cx, |shell, cx| shell.dismiss_unticked_notice(cx));
        });
        self.unticked_notice = Some(UntickedNotice {
            count,
            _expiry: expiry,
        });
    }

    /// The notice's ✕, and its timer.
    pub(crate) fn dismiss_unticked_notice(&mut self, cx: &mut Context<Self>) {
        self.unticked_notice = None;
        cx.notify();
    }

    /// A header click: ascending, descending, then the source order.
    pub(crate) fn cycle_sort(&mut self, column: usize, cx: &mut Context<Self>) {
        self.update_view(cx, |view| view.sort = next_sort(view.sort, column));
        self.persist_table_prefs(cx);
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

    /// Adds the `Equals` chip of `column`, or removes it when it is already that one.
    pub(crate) fn toggle_equals(
        &mut self,
        column: usize,
        title: &'static str,
        value: &'static str,
        cx: &mut Context<Self>,
    ) {
        self.update_view(cx, |view| {
            let is_on = view.filter.chips.iter().any(|chip| {
                matches!(chip, FilterChip::Equals { column: other, value: shown, .. }
                    if *other == column && shown.as_ref() == value)
            });
            if is_on {
                view.filter.chips.retain(
                    |chip| !matches!(chip, FilterChip::Equals { column: other, .. } if *other == column),
                );
            } else {
                view.filter.set_equals(column, title, value);
            }
        });
    }

    pub(crate) fn toggle_column(&mut self, column: usize, cx: &mut Context<Self>) {
        self.update_view(cx, |view| {
            if !view.hidden.remove(&column) {
                view.hidden.insert(column);
            }
        });
        self.persist_table_prefs(cx);
    }

    /// Saves the visible table's sort and hidden columns. Only a sort or a column toggle calls
    /// it, so nothing else writes table prefs.
    fn persist_table_prefs(&self, cx: &mut Context<Self>) {
        let screen = self.screen;
        let prefs = match screen {
            Screen::Pods => table_prefs(&self.pod_table, cx),
            Screen::Nodes => table_prefs(&self.node_table, cx),
            // Overview and Topology have no table.
            Screen::Overview | Screen::Topology | Screen::PortForwarding => None,
            Screen::Issues => table_prefs(&self.issue_table, cx),
            Screen::Kind(_) => table_prefs(&self.kind_table, cx),
        };
        let Some(prefs) = prefs else {
            return;
        };
        AppSettings::update(cx, |settings| {
            settings.tables.insert(screen_key(screen).to_owned(), prefs);
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
            Screen::Overview | Screen::Topology | Screen::PortForwarding => {}
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
        let Some(primary) = self.session() else {
            return;
        };
        let is_paused = matches!(
            primary.read(cx).live().and_then(LiveCluster::explorer_flow),
            Some(FlowState::Paused { .. })
        );
        primary.update(cx, |session, cx| {
            session.set_explorer_paused(!is_paused, cx)
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
                    // Plain text already filters as it is typed: Enter hands the keyboard back to
                    // the table. A `label:` text keeps its chip behavior.
                    self.focus_visible_table(window, cx);
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
            Screen::Overview | Screen::Topology | Screen::PortForwarding => return None,
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
            Screen::Overview | Screen::Topology | Screen::PortForwarding | Screen::Nodes => false,
            Screen::Kind(kind) => kind.is_namespaced(),
        };
        if is_namespaced {
            state.scope = self.live(cx).map(|live| live.scope.clone());
        }
        Some(state)
    }

    // ---- rendering ----

    /// The sidebar numbers of the open cluster.
    fn navigation_counts(&self, cx: &App) -> NavigationCounts {
        let event_filter = self
            .session()
            .map_or(EventFilter::All, |session| session.read(cx).event_filter());
        // A session that is not live keeps its last board, which says nothing about the cluster.
        let (issue_total, issue_counts) = match (self.live(cx), self.session()) {
            (Some(_), Some(session)) => issue_counts(session.read(cx).issues()),
            _ => (None, Vec::new()),
        };
        let live = self.live(cx);
        let kinds = live.map_or_else(Default::default, |live| {
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
        });
        NavigationCounts {
            issue_total,
            issue_counts,
            pods: live.and_then(|live| live.pods.ready_count()),
            nodes: live.and_then(|live| live.nodes.ready_count()),
            explorer: live.and_then(LiveCluster::explorer_count),
            kinds,
            port_forwards: self.port_forwards.read(cx).running_count(),
        }
    }
}

impl Render for AppShell {
    #[cfg_attr(
        feature = "hotpath-profiling",
        hotpath::measure(impl_type = "AppShell")
    )]
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.fit_table_widths(window, cx);
        self.sync_monitor_source(cx);
        self.refresh_monitor_cache(cx);
        self.open_pending_logs(window, cx);
        self.open_pending_dialog(window, cx);
        self.sync_yaml_view(window, cx);
        self.sync_helm_view(window, cx);
        self.sync_secret_values(cx);
        self.sync_kubelet_demand(cx);
        self.sync_quick_filter(window, cx);
        self.open_pending_switcher(window, cx);
        self.open_pending_namespace_picker(window, cx);
        self.open_pending_pick(window, cx);
        self.open_pending_palette(window, cx);
        let theme = cx.theme();
        let counts = self.navigation_counts(cx);
        let is_kubeconfig_loading = self.catalog.read(cx).is_loading();
        // A 2 px danger edge on Production only: the title bar can scroll or be covered, this cannot.
        let is_production = self
            .active_profile(cx)
            .is_some_and(|profile| profile.environment.is_production());
        let root = v_flex()
            .size_full()
            .track_focus(&self.focus_handle)
            .key_context(shell_key_context(self.screen))
            .on_action(cx.listener(|shell, _: &FocusQuickFilter, window, cx| {
                shell.focus_quick_filter(window, cx);
            }))
            .on_action(cx.listener(|shell, _: &OpenClusterSwitcher, window, cx| {
                shell.toggle_cluster_switcher(window, cx);
            }))
            .on_action(cx.listener(|_, _: &ShowShortcuts, window, cx| {
                open_shortcut_sheet(window, cx);
            }))
            .on_action(cx.listener(|shell, _: &OpenNamespacePicker, window, cx| {
                shell.open_namespace_picker(PickerAnchor::TitleBar, window, cx);
            }))
            .on_action(cx.listener(|shell, _: &OpenPalette, window, cx| {
                shell.open_palette("", window, cx);
            }))
            .on_action(cx.listener(|shell, _: &OpenKindPalette, window, cx| {
                shell.open_palette(":", window, cx);
            }));
        let root = keyboard_navigation::register_key_handlers(root, cx);
        let root = on_switch_to::<SwitchToCluster1>(root, 1, cx);
        let root = on_switch_to::<SwitchToCluster2>(root, 2, cx);
        let root = on_switch_to::<SwitchToCluster3>(root, 3, cx);
        let root = on_switch_to::<SwitchToCluster4>(root, 4, cx);
        let root = on_switch_to::<SwitchToCluster5>(root, 5, cx);
        let root = on_switch_to::<SwitchToCluster6>(root, 6, cx);
        let root = on_switch_to::<SwitchToCluster7>(root, 7, cx);
        let root = on_switch_to::<SwitchToCluster8>(root, 8, cx);
        let root = on_switch_to::<SwitchToCluster9>(root, 9, cx);
        root.bg(theme.background)
            .when(is_production, |root| {
                root.border_t_2().border_color(theme.danger)
            })
            .text_color(theme.foreground)
            .child(title_bar(self, window.viewport_size().width, cx))
            .children(self.reset_banner.as_deref().map(|text| {
                reset_banner(
                    text,
                    cx,
                    cx.listener(|shell, _, _, cx| {
                        shell.reset_banner = None;
                        cx.notify();
                    }),
                )
            }))
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .child(sidebar(self.screen, &counts, self.live(cx), cx))
                    .child(self.render_workspace(window, cx)),
            )
            .child(status_bar(
                self,
                is_kubeconfig_loading,
                cx.weak_entity(),
                cx,
            ))
    }
}

/// `Ctrl n`: switches to row `shortcut` of the switcher list.
fn on_switch_to<A: gpui_kit::Action>(root: Div, shortcut: u8, cx: &Context<AppShell>) -> Div {
    root.on_action(cx.listener(move |shell, _: &A, _, cx| {
        shell.switch_to_nth_cluster(shortcut, cx);
    }))
}

/// The sort and hidden columns of the view of `table`, by column name.
fn table_prefs<D: FilteredTable>(table: &Entity<TableState<D>>, cx: &App) -> Option<TablePrefs> {
    let delegate = table.read(cx).delegate();
    Some(delegate.view()?.prefs(delegate.column_plan()?))
}

/// How many ticked rows of `table` its filter unticked since the last call.
fn take_unticked_hidden<D: FilteredTable>(table: &Entity<TableState<D>>, cx: &mut App) -> usize {
    table.update(cx, |table, _| {
        table
            .delegate_mut()
            .view_mut()
            .map_or(0, TableView::take_unticked_hidden)
    })
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

/// The namespace and graph node of an object Show in Topology can focus: a Service or an Ingress.
fn topology_target(key: &ResourceKey) -> Option<(String, NodeId)> {
    let ResourceKey::Kind {
        kind,
        namespace: Some(namespace),
        name,
    } = key
    else {
        return None;
    };
    let kind = TopologyKind::of_resource_kind(*kind)
        .filter(|kind| matches!(kind, TopologyKind::Service | TopologyKind::Ingress))?;
    Some((
        namespace.clone(),
        NodeId::Object {
            kind,
            name: name.clone(),
        },
    ))
}

/// What the user had in the open cluster, kept when it is left: the scope, so coming back lands
/// there, and whether the cluster answered, so its switcher row is right at once.
fn record_leaving(
    open: &ActiveSession,
    scope_memory: &mut ScopeMemory,
    switcher: &mut ClusterSwitcherState,
    cx: &App,
) {
    let result = match open.session.read(cx).phase() {
        SessionPhase::Live(live) => {
            remember_scope(scope_memory, open.cluster.clone(), live.scope.clone());
            ProbeResult::Reachable {
                latency: live.api_latency,
            }
        }
        SessionPhase::Failed { message } => ProbeResult::Unreachable {
            reason: message.clone(),
        },
        SessionPhase::Connecting { .. } => return,
    };
    switcher
        .health_mut()
        .record(open.cluster.clone(), result, Instant::now());
}

/// The loaded kubeconfig that defines `cluster`, with its context.
pub(crate) fn find_cluster(
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

/// What the start does about a watched-folder file.
#[derive(Debug, PartialEq, Eq)]
enum FolderStart {
    /// `last_used` is not a file of a watched folder (or `--context` decides): the usual start.
    NotAFolderFile,
    /// The user picked this file and it is unchanged since: it starts.
    Start(ClusterRef),
    /// The user picked this file but it changed since (or no stamp was saved): nothing starts, and
    /// the user picks again. A file anyone can write must not run on yesterday's trust.
    Changed,
}

/// The cluster of a watched-folder file that may start on its own: only the `last_used` the user
/// picked, found in no chain or registry file, never with `--context`, and only while its saved
/// stamp `(saved, current)` still matches the file.
fn folder_start(
    requested: Option<&str>,
    last_used: Option<&ClusterRef>,
    (saved, current): (Option<FileStamp>, Option<FileStamp>),
    start: &[Arc<Kubeconfig>],
    listed: &[Arc<Kubeconfig>],
) -> FolderStart {
    let Some(cluster) = last_used.filter(|_| requested.is_none()) else {
        return FolderStart::NotAFolderFile;
    };
    if find_cluster(listed, cluster).is_none() || find_cluster(start, cluster).is_some() {
        return FolderStart::NotAFolderFile;
    }
    if saved.is_some() && saved == current {
        FolderStart::Start(cluster.clone())
    } else {
        FolderStart::Changed
    }
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
    // The caller passes at least one loaded kubeconfig.
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
