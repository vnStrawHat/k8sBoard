use std::ffi::{OsStr, OsString};
use std::path::PathBuf;

use cluster::NamespaceScope;

use crate::app_shell::Screen;
use crate::cluster_catalog::{PathStyle, same_path_text};
use crate::color_theme::ColorTheme;
use crate::drawer::DrawerTab;
use crate::namespace_picker::MAX_NAMESPACES;
use crate::resource_kind::ResourceKind;
use crate::settings::ThemePreference;
use crate::settings_window::{SettingsPage, SettingsSize};

pub(crate) const USAGE: &str = "\
Usage: k8sboard [options]

Options:
  --kubeconfig <path>    kubeconfig file (default: first KUBECONFIG entry, else ~/.kube/config)
  --context <name>       context to open (default: the kubeconfig current-context)
  --namespace <a[,b]>    namespaces to show, at most 5 (default: all namespaces if allowed)
  --filter <text>        quick filter of the start screen; label:k=v,k2!=v2 becomes label chips
  --select <name>       with a drawer screen, open the row named <name> or <namespace>/<name>
                         (default: the first row)
  --theme system|light|dark
                         colour theme (default: the saved theme, else follow the system)
  --color-theme default|zed-one
                         colour family (default: the saved one, else zed-one)
  --config-dir <path>    settings folder (default: K8SBOARD_CONFIG_DIR, else the OS config folder)
  --screen overview|switcher|cordon-confirm|unlock-confirm|scale-popover|scale-confirm|restart-bulk-confirm|delete-confirm|delete-bulk-confirm|restart-pod-confirm|evict-confirm|edit-yaml-diff|edit-yaml-history|values-edit|new-config-map|revision-diff|hpa-range-popover|expand-confirm|default-class-confirm|renew-confirm|pods|nodes|issues|issues-drawer|topology|topology-problems|topology-rbac|topology-selected|topology-curves|topology-traffic|topology-traffic-curves|topology-traffic-fixture|topology-traffic-fixture-curves|pod-drawer|pod-containers|pod-events|pod-monitor|pod-monitor-source-fixture|node-drawer|node-events|node-monitor|pod-yaml|node-yaml|logs-dock|logs-zoomed|logs-popout|logs-workload|shell-fixture|shell-dock-fixture|shell-paste-fixture|shell-picker-fixture|shell-confirm-fixture|attach-confirm|node-shell-confirm|node-shell-options|debug-container-options|node-shell-confirm-staging|leftover-sweep-fixture|node-shell-tab-fixture|debug-shell-tab-fixture|shell-find-fixture|port-forwards|port-forwards-list|port-forward-new-fixture|port-forward-confirm-fixture|port-forward-remove-fixture|pods-selected|nodes-selected|shortcuts|pods-cursor|
           node-taints-editor|node-taints-editor-invalid|node-labels-editor|node-labels-bulk-editor|drain-dialog|drain-dialog-skip-pdbs|drain-progress|drain-progress-stuck|
           namespaces|events|deployments|statefulsets|daemonsets|replicasets|jobs|cronjobs|
           services|ingresses|configmaps|<kind>-drawer|<kind>-events|<kind>-monitor|<kind>-yaml|releases-values|releases-manifest|
           customresourcedefinitions|custom:<crd-name>[-drawer|-events|-yaml]|who-can|check-permissions|account-permissions|test-traffic|settings|settings-tall|settings-general|settings-environments|settings-appearance|settings-terminal|settings-logs|settings-metrics|settings-metrics-fixture|settings-shortcuts
                         screen to open (default: overview)
  --palette <text>       open the command palette with <text> typed (for example :po or > rest)
  --window-width <px>    window width, 800 to 3840 (default: 1320)
  --screenshot <path>    write a PNG and exit (needs a build with --features screenshot)
  --help                 print this help
";

/// The screen to open. The drawer values open Pods, Nodes, or a kind with a row already
/// selected, and the logs values open Pods with the log dock on a pod.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LaunchScreen {
    /// `--screen overview`.
    Overview,
    Pods,
    Nodes,
    /// `--screen issues`.
    Issues,
    /// `--screen issues-drawer`: Issues, then the first issue revealed with its drawer.
    IssuesDrawer,
    /// `--screen topology`.
    Topology,
    /// `--screen topology-problems`: the same screen with Problems only on.
    TopologyProblems,
    /// `--screen topology-rbac`: the same screen with the RBAC chip on.
    TopologyRbac,
    /// `--screen topology-selected`: the first Deployment selected, its drawer open, and motion
    /// reduced so the capture does not depend on the clock.
    TopologySelected,
    /// `--screen topology-curves`: the same screen with the edges drawn as curves, in memory
    /// only (the setting is not written).
    TopologyCurves,
    /// `--screen topology-traffic` (spec 0049): Topology in Traffic mode, live, once the source of
    /// the cluster is ready.
    TopologyTraffic,
    /// `--screen topology-traffic-curves`: the same with the edges drawn as curves, in memory.
    TopologyTrafficCurves,
    /// `--screen topology-traffic-fixture`: the fixed namespace of W11 with Istio and pod network
    /// readings. Screenshot builds only; no source is read.
    TopologyTrafficFixture,
    /// `--screen topology-traffic-fixture-curves`: the same with the edges drawn as curves.
    TopologyTrafficFixtureCurves,
    /// `--screen pod-drawer|pod-containers|pod-events|pod-yaml`: a pod drawer on that tab.
    PodDrawer(DrawerTab),
    /// `--screen pod-monitor-source-fixture` (spec 0048): the first pod's Monitor on 30d with
    /// synthetic source data. Screenshot builds only; the pod comes from the connected cluster.
    PodMonitorSourceFixture,
    /// `--screen node-drawer|node-events|node-yaml`.
    NodeDrawer(DrawerTab),
    LogsDock,
    LogsZoomed,
    /// `--screen logs-popout`: the zoomed dock, then its active tab moved to a window of its own;
    /// a screenshot captures that window.
    LogsPopout,
    /// `--screen logs-workload`: the dock zoomed on the workload that owns the logs pod.
    LogsWorkload,
    /// `--screen shell-fixture`: Pods with the dock zoomed on a shell tab fed a fixed transcript
    /// (the W8b pane). Screenshot builds only; it never opens a session.
    ShellFixture,
    /// `--screen shell-dock-fixture`: the dock at its split with a Logs tab and the shell tab.
    ShellDockFixture,
    /// `--screen shell-paste-fixture`: the shell tab with the multi-line paste dialog open.
    ShellPasteFixture,
    /// `--screen shell-picker-fixture`: the container list of the Open shell submenu.
    ShellPickerFixture,
    /// `--screen shell-find-fixture`: the shell tab with Find open and its matches highlighted.
    ShellFindFixture,
    /// `--screen shell-confirm-fixture`: Pods with the Open shell confirm dialog open on a fixed
    /// pod; its confirm button and Enter do nothing.
    ShellConfirmFixture,
    /// `--screen attach-confirm` (spec 0040): Pods with the Attach confirm dialog open on a fixed
    /// pod of a fixed Production cluster; its confirm button and Enter do nothing.
    AttachConfirm,
    /// `--screen node-shell-confirm`: Nodes with the Open node shell confirm dialog open on a fixed
    /// node of a fixed Production cluster; its confirm button and Enter do nothing. Screenshot builds
    /// only; it needs no cluster.
    NodeShellConfirm,
    /// `--screen node-shell-options` and `debug-container-options`: the options dialogs of the two
    /// debug starts over fixed data. Screenshot builds only; they need no cluster and start nothing.
    NodeShellOptions,
    DebugContainerOptions,
    /// `--screen node-shell-confirm-staging`: the node shell confirm on a Staging cluster, which asks
    /// for the node name like any Privileged action. Offline like the Production one.
    NodeShellConfirmStaging,
    /// `--screen node-taints-editor` and `node-labels-editor`: the editors of a fixed node over the
    /// Nodes list. Screenshot builds only; they need no cluster and send nothing.
    NodeTaintsEditor,
    NodeLabelsEditor,
    /// `--screen node-labels-bulk-editor` (spec 0040): the bulk label editor of three ticked nodes of
    /// a fixed cluster, with a Set and a Remove typed. Screenshot builds only; it sends nothing.
    NodeLabelsBulkEditor,
    /// `--screen node-taints-editor-invalid`: the taints editor with one invalid key and one
    /// NoExecute row added, so the validation message and the warning show. Offline like the rest.
    NodeTaintsEditorInvalid,
    /// `--screen drain-dialog`: the W6 drain dialog over fixed pods of a fixed Production cluster.
    /// Screenshot builds only; it needs no cluster and can never send.
    DrainDialog,
    /// `--screen drain-dialog-skip-pdbs` (spec 0040): the same dialog on a fixed Staging cluster with
    /// Skip PodDisruptionBudgets ticked. Same rules.
    DrainDialogSkipPdbs,
    /// `--screen drain-progress`: the dock zoomed on the tab of a fixed drain of one node. Screenshot
    /// builds only; it needs no cluster and starts no run.
    DrainProgress,
    /// `--screen drain-progress-stuck`: the same tab after the node ended Stuck, with its reason.
    DrainProgressStuck,
    /// `--screen leftover-sweep-fixture`: the leftover review dialog over fixed rows.
    LeftoverSweepFixture,
    /// `--screen node-shell-tab-fixture` and `debug-shell-tab-fixture`: the dock zoomed on a node
    /// shell tab and on a debug shell tab over fixed data; they never open a session.
    NodeShellTabFixture,
    DebugShellTabFixture,
    /// `--screen port-forwards`: the Port Forwarding page with five fixed rows and the drawer of the
    /// first. Screenshot builds only; it needs no cluster and starts nothing.
    PortForwards,
    /// `--screen port-forwards-list`: the same rows with no drawer, so every column shows.
    PortForwardsList,
    /// `--screen port-forward-new-fixture`: the same page with the New forward dialog open.
    PortForwardNewFixture,
    /// `--screen port-forward-confirm-fixture`: the confirm dialog of a forward start on a fixed
    /// Production cluster; its confirm button and Enter do nothing.
    PortForwardConfirmFixture,
    /// `--screen port-forward-remove-fixture`: the Remove preset dialog of the fixed preset row.
    PortForwardRemoveFixture,
    /// `--screen pods-selected|nodes-selected`: the first two rows are ticked.
    PodsSelected,
    NodesSelected,
    /// `--screen shortcuts`: Pods with the `?` shortcut sheet open.
    Shortcuts,
    /// `--screen pods-cursor`: Pods with the first row selected and the drawer closed.
    PodsCursor,
    /// `--screen who-can`: ClusterRoles with the Who can dialog open on `get secrets`.
    WhoCan,
    /// `--screen check-permissions`: ServiceAccounts with the Check permissions dialog open for You.
    CheckPermissions,
    /// `--screen account-permissions`: the same dialog for the first service account shown (after
    /// `--filter`), or the one `--select` names.
    AccountPermissions,
    /// `--screen test-traffic`: NetworkPolicies with the Test traffic dialog open on the defaults
    /// of the first policy shown (after `--filter`), or the one `--select` names.
    TestTraffic,
    /// `--screen switcher`: Overview with the cluster switcher popover open once the session is live
    /// and the probes of the other clusters have answered.
    Switcher,
    /// `--screen cordon-confirm`: Nodes with the Cordon dialog of the first node open, in a fixed
    /// state (the dry-run passed in 412 ms). Screenshot builds only; it never reaches a cluster.
    CordonConfirm,
    /// `--screen unlock-confirm`: Nodes with the dialog that unlocks the primary cluster open.
    UnlockConfirm,
    /// `--screen scale-popover`: Deployments with the cursor on the first row and its Scale popover
    /// open.
    ScalePopover,
    /// `--screen scale-confirm`: the same cursor row with the Scale dialog open in a fixed state (the
    /// dry-run passed in 412 ms, the count two above the current one). Screenshot builds only; it
    /// never reaches a cluster.
    ScaleConfirm,
    /// `--screen restart-bulk-confirm`: Deployments with the first four rows ticked and the batch
    /// dialog of Restart open in a fixed state (every dry-run passed). Screenshot builds only; it
    /// never reaches a cluster.
    RestartBulkConfirm,
    /// `--screen delete-confirm`: Deployments with the Delete dialog of a fixed Deployment of a fixed
    /// Production cluster open, drawn from fixed data (W10b). Screenshot builds only; it waits for no
    /// cluster and can never send.
    DeleteConfirm,
    /// `--screen delete-bulk-confirm`: Pods with the Delete dialog of twelve fixed pods of a fixed
    /// Staging cluster open. Screenshot builds only; it waits for no cluster and can never send.
    DeleteBulkConfirm,
    /// `--screen restart-pod-confirm` (spec 0040): Pods with the Restart pod dialog of a fixed
    /// StatefulSet pod of a fixed Production cluster open. Screenshot builds only; it waits for no
    /// cluster and can never send.
    RestartPodConfirm,
    /// `--screen evict-confirm` (spec 0040): Pods with the Evict dialog of a fixed pod of a fixed
    /// Staging cluster open, its dry-run refused by a PodDisruptionBudget. Same rules.
    EvictConfirm,
    /// `--screen edit-yaml-diff`: the Edit YAML view on its Diff tab, drawn from fixed data (W10). It
    /// waits for no cluster and can never send. Screenshot builds only.
    EditYamlDiff,
    /// `--screen edit-yaml-history`: the Edit YAML view of a Deployment on its Revision history tab,
    /// drawn from fixed data (spec 0041, W10). It waits for no cluster and can never send.
    /// Screenshot builds only.
    EditYamlHistory,
    /// `--screen values-edit`: the Edit values view of a fixed Secret, drawn from fixed data (spec 0047).
    /// Every value is fixture text and every Secret field is masked. It waits for no cluster and can
    /// never send. Screenshot builds only.
    ValuesEdit,
    /// `--screen new-config-map`: the New view of a ConfigMap with its template and a passed dry-run,
    /// drawn from fixed data (spec 0042). It waits for no cluster and can never send. Screenshot
    /// builds only.
    NewConfigMap,
    /// `--screen revision-diff`: the Deployment revision diff dialog over the Deployments screen, drawn
    /// from two fixed pod templates (spec 0039). It waits for no cluster and makes no request.
    /// Screenshot builds only.
    RevisionDiff,
    /// `--screen hpa-range-popover`: HPAs with the Edit min / max popover of a fixed HPA open, drawn
    /// from fixed data (0032b). Screenshot builds only; it waits for no cluster and can never send.
    HpaRangePopover,
    /// `--screen expand-confirm`: PVCs with the Expand dialog of a fixed claim (100Gi to 150Gi) of a
    /// fixed Production cluster open, drawn from fixed data (0032b). Screenshot builds only; it waits
    /// for no cluster and can never send.
    ExpandConfirm,
    /// `--screen default-class-confirm`: StorageClasses with the Set default dialog of two fixed
    /// classes of a fixed Staging cluster open, drawn from fixed data (0032b). Screenshot builds
    /// only; it waits for no cluster and can never send.
    DefaultClassConfirm,
    /// `--screen renew-confirm` (spec 0018 step 6): the CRDs screen with the Renew now dialog of a fixed
    /// Certificate of a fixed Production cluster open, drawn from fixed data. Screenshot builds only;
    /// it waits for no cluster and can never send.
    RenewConfirm,
    /// `--screen settings|settings-appearance|settings-shortcuts`: the main window opens as usual,
    /// then the Settings window on that page, which is what the screenshot captures.
    Settings(SettingsPage, SettingsSize),
    /// `--screen settings-metrics` (spec 0048): the Settings window on the Metrics page of the
    /// connected cluster, captured once detection settled.
    SettingsMetrics,
    /// `--screen settings-metrics-fixture`: the same page drawn from fixed data, for no cluster.
    /// Screenshot builds only.
    SettingsMetricsFixture,
    /// `--screen <plural>`, e.g. `deployments`.
    Kind(ResourceKind),
    /// `--screen <plural>-drawer|<plural>-events|<plural>-yaml`: the kind's first row selected, on that tab.
    KindDrawer(ResourceKind, DrawerTab),
    /// `--screen <plural>-menu`: the kind's first row selected with its drawer open and the ⋯ menu of
    /// the drawer open on top (screenshot builds click it).
    KindMenu(ResourceKind),
    /// `--screen custom:<crd-name>[-drawer|-events|-yaml]`: a custom kind, known only once the CRD
    /// list has loaded. The shell resolves it to `Kind` or `KindDrawer`, and the first row is
    /// selected when a tab is given.
    Custom {
        // ponytail: leaked so the request stays `Copy`; one command-line argument per run.
        crd_name: &'static str,
        tab: Option<DrawerTab>,
    },
}

impl LaunchScreen {
    /// The list screen this request opens on.
    pub(crate) fn screen(self) -> Screen {
        match self {
            Self::Overview | Self::Switcher => Screen::Overview,
            Self::CordonConfirm
            | Self::UnlockConfirm
            | Self::NodeShellConfirm
            | Self::NodeShellConfirmStaging
            | Self::NodeTaintsEditor
            | Self::NodeTaintsEditorInvalid
            | Self::NodeLabelsEditor
            | Self::NodeLabelsBulkEditor
            | Self::DrainDialog
            | Self::DrainDialogSkipPdbs
            | Self::NodeShellOptions => Screen::Nodes,
            Self::DebugContainerOptions
            | Self::LeftoverSweepFixture
            | Self::NodeShellTabFixture
            | Self::DebugShellTabFixture => Screen::Pods,
            Self::DrainProgress | Self::DrainProgressStuck => Screen::Nodes,
            Self::ShellConfirmFixture
            | Self::AttachConfirm
            | Self::DeleteBulkConfirm
            | Self::RestartPodConfirm
            | Self::EvictConfirm => Screen::Pods,
            Self::EditYamlDiff | Self::EditYamlHistory | Self::RevisionDiff => {
                Screen::Kind(ResourceKind::Deployments)
            }
            Self::ValuesEdit => Screen::Kind(ResourceKind::Secrets),
            Self::NewConfigMap => Screen::Kind(ResourceKind::ConfigMaps),
            Self::HpaRangePopover => Screen::Kind(ResourceKind::HorizontalPodAutoscalers),
            Self::ExpandConfirm => Screen::Kind(ResourceKind::PersistentVolumeClaims),
            Self::DefaultClassConfirm => Screen::Kind(ResourceKind::StorageClasses),
            Self::RenewConfirm => Screen::Kind(ResourceKind::Crds),
            Self::ScalePopover
            | Self::ScaleConfirm
            | Self::RestartBulkConfirm
            | Self::DeleteConfirm => Screen::Kind(ResourceKind::Deployments),
            Self::PortForwards
            | Self::PortForwardsList
            | Self::PortForwardNewFixture
            | Self::PortForwardConfirmFixture
            | Self::PortForwardRemoveFixture => Screen::PortForwarding,
            Self::Pods
            | Self::PodDrawer(_)
            | Self::PodMonitorSourceFixture
            | Self::LogsDock
            | Self::LogsZoomed
            | Self::LogsPopout
            | Self::LogsWorkload
            | Self::ShellFixture
            | Self::ShellDockFixture
            | Self::ShellPasteFixture
            | Self::ShellPickerFixture
            | Self::ShellFindFixture
            | Self::PodsSelected
            | Self::Shortcuts
            | Self::PodsCursor => Screen::Pods,
            // The sidebar group of Custom Resources starts open; the shell then resolves the kind
            // against the CRD list.
            Self::Custom { .. } => Screen::Kind(ResourceKind::Crds),
            Self::Nodes | Self::NodeDrawer(_) | Self::NodesSelected => Screen::Nodes,
            Self::Issues | Self::IssuesDrawer => Screen::Issues,
            Self::Settings(..) | Self::SettingsMetrics | Self::SettingsMetricsFixture => {
                Screen::Overview
            }
            Self::Topology
            | Self::TopologyProblems
            | Self::TopologyRbac
            | Self::TopologySelected
            | Self::TopologyCurves
            | Self::TopologyTraffic
            | Self::TopologyTrafficCurves
            | Self::TopologyTrafficFixture
            | Self::TopologyTrafficFixtureCurves => Screen::Topology,
            Self::Kind(kind) | Self::KindDrawer(kind, _) | Self::KindMenu(kind) => {
                Screen::Kind(kind)
            }
            Self::WhoCan => Screen::Kind(ResourceKind::ClusterRoles),
            Self::TestTraffic => Screen::Kind(ResourceKind::NetworkPolicies),
            Self::CheckPermissions | Self::AccountPermissions => {
                Screen::Kind(ResourceKind::ServiceAccounts)
            }
        }
    }

    /// The Settings page and window size of the settings screens.
    pub(crate) fn settings_screen(self) -> Option<(SettingsPage, SettingsSize)> {
        match self {
            Self::Settings(page, size) => Some((page, size)),
            Self::SettingsMetrics | Self::SettingsMetricsFixture => {
                Some((SettingsPage::Metrics, SettingsSize::Standard))
            }
            _ => None,
        }
    }

    /// Whether a screenshot opens the ⋯ menu of the drawer before it captures.
    #[cfg(any(feature = "screenshot", test))]
    pub(crate) fn opens_menu(self) -> bool {
        matches!(self, Self::KindMenu(_))
    }

    /// Whether a row must be selected so the drawer is open.
    pub(crate) fn has_drawer(self) -> bool {
        matches!(
            self,
            Self::PodDrawer(_)
                | Self::PodMonitorSourceFixture
                | Self::NodeDrawer(_)
                | Self::KindDrawer(..)
                | Self::KindMenu(_)
                | Self::IssuesDrawer
                | Self::Custom { tab: Some(_), .. }
        )
    }

    /// Whether a row must be selected: the drawer screens, and the one that shows only the cursor.
    pub(crate) fn selects_row(self) -> bool {
        self.has_drawer()
            || matches!(
                self,
                Self::PodsCursor | Self::ScalePopover | Self::ScaleConfirm
            )
    }

    /// The kind whose first row the request puts the cursor on, when it names one: the drawer
    /// screens and the Scale screens.
    pub(crate) fn row_kind(self) -> Option<ResourceKind> {
        match self {
            Self::KindDrawer(kind, _) | Self::KindMenu(kind) => Some(kind),
            Self::ScalePopover | Self::ScaleConfirm => Some(ResourceKind::Deployments),
            _ => None,
        }
    }

    /// The drawer tab this request opens on; `None` for screens without a drawer.
    pub(crate) fn drawer_tab(self) -> Option<DrawerTab> {
        match self {
            Self::PodDrawer(tab) | Self::NodeDrawer(tab) | Self::KindDrawer(_, tab) => Some(tab),
            Self::PodMonitorSourceFixture => Some(DrawerTab::Monitor),
            Self::KindMenu(_) => Some(DrawerTab::Overview),
            Self::Custom { tab, .. } => tab,
            _ => None,
        }
    }

    /// Whether the drawer opens expanded: W4b shows the Containers tab so, and W4c the Monitor tab.
    pub(crate) fn opens_expanded(self) -> bool {
        matches!(
            self,
            Self::PodDrawer(DrawerTab::Containers | DrawerTab::Monitor)
                | Self::PodMonitorSourceFixture
                | Self::NodeDrawer(DrawerTab::Monitor)
                | Self::KindDrawer(_, DrawerTab::Monitor)
        )
    }

    /// How many ticks of its metrics feed the screen waits for: a chart needs two points.
    #[cfg(any(feature = "screenshot", test))]
    pub(crate) fn min_metrics_ticks(self) -> u64 {
        if self.drawer_tab() == Some(DrawerTab::Monitor) {
            2
        } else {
            1
        }
    }

    /// Whether the first rows must be ticked once the list has loaded.
    pub(crate) fn checks_rows(self) -> bool {
        matches!(
            self,
            Self::PodsSelected | Self::NodesSelected | Self::RestartBulkConfirm
        )
    }

    /// How many of the first rows `checks_rows` ticks.
    pub(crate) fn checked_count(self) -> usize {
        if self == Self::RestartBulkConfirm {
            4
        } else {
            2
        }
    }

    /// Whether the screen shows the Topology graph, so a screenshot waits for its feeds and build.
    #[cfg(any(feature = "screenshot", test))]
    pub(crate) fn shows_topology(self) -> bool {
        matches!(
            self,
            Self::Topology
                | Self::TopologyProblems
                | Self::TopologyRbac
                | Self::TopologySelected
                | Self::TopologyCurves
                | Self::TopologyTraffic
                | Self::TopologyTrafficCurves
                | Self::TopologyTrafficFixture
                | Self::TopologyTrafficFixtureCurves
        )
    }

    /// Whether the screen shows pod usage, so a screenshot waits for a metrics tick.
    #[cfg(any(feature = "screenshot", test))]
    pub(crate) fn shows_pod_usage(self) -> bool {
        matches!(
            self,
            Self::Pods
                | Self::PodsSelected
                | Self::Shortcuts
                | Self::PodsCursor
                | Self::PodDrawer(DrawerTab::Containers | DrawerTab::Monitor)
                | Self::PodMonitorSourceFixture
                | Self::KindDrawer(_, DrawerTab::Monitor)
        )
    }

    /// Whether the screen shows Network and Disk I/O (or PVC usage), so a screenshot waits for kubelet rounds: the
    /// Monitor tabs. A feed that is unavailable settles at once.
    #[cfg(any(feature = "screenshot", test))]
    pub(crate) fn shows_kubelet_stats(self) -> bool {
        match self {
            // The Capacity panel sums the volumes of the same feed.
            Self::Overview => true,
            // The Used column and the Usage bars of PVCs read the same feed.
            Self::Kind(ResourceKind::PersistentVolumeClaims)
            | Self::KindDrawer(ResourceKind::PersistentVolumeClaims, DrawerTab::Overview) => true,
            _ => self.drawer_tab() == Some(DrawerTab::Monitor),
        }
    }

    /// Whether the screen shows node usage.
    #[cfg(any(feature = "screenshot", test))]
    pub(crate) fn shows_node_usage(self) -> bool {
        matches!(
            self,
            Self::Overview
                | Self::Nodes
                | Self::NodesSelected
                | Self::CordonConfirm
                | Self::UnlockConfirm
                | Self::NodeDrawer(DrawerTab::Overview | DrawerTab::Monitor)
        )
    }

    /// Whether a tool dialog opens once the session is live.
    pub(crate) fn opens_dialog(self) -> bool {
        matches!(
            self,
            Self::WhoCan
                | Self::CheckPermissions
                | Self::AccountPermissions
                | Self::TestTraffic
                | Self::Shortcuts
                | Self::CordonConfirm
                | Self::UnlockConfirm
                | Self::ShellConfirmFixture
                | Self::AttachConfirm
                | Self::NodeShellConfirm
                | Self::NodeShellOptions
                | Self::DebugContainerOptions
                | Self::NodeShellConfirmStaging
                | Self::NodeTaintsEditor
                | Self::NodeTaintsEditorInvalid
                | Self::NodeLabelsEditor
                | Self::NodeLabelsBulkEditor
                | Self::DrainDialog
                | Self::DrainDialogSkipPdbs
                | Self::LeftoverSweepFixture
                | Self::ScalePopover
                | Self::ScaleConfirm
                | Self::RestartBulkConfirm
                | Self::DeleteConfirm
                | Self::DeleteBulkConfirm
                | Self::RestartPodConfirm
                | Self::EvictConfirm
                | Self::RenewConfirm
                | Self::EditYamlDiff
                | Self::EditYamlHistory
                | Self::ValuesEdit
                | Self::NewConfigMap
                | Self::RevisionDiff
                | Self::HpaRangePopover
                | Self::ExpandConfirm
                | Self::DefaultClassConfirm
                | Self::PortForwardNewFixture
                | Self::PortForwardConfirmFixture
                | Self::PortForwardRemoveFixture
        )
    }

    /// Whether the screen is a dialog drawn from fixed data: it waits for no cluster.
    #[cfg(any(feature = "screenshot", test))]
    pub(crate) fn is_dialog_fixture(self) -> bool {
        matches!(
            self,
            Self::ShellConfirmFixture
                | Self::AttachConfirm
                | Self::RestartPodConfirm
                | Self::EvictConfirm
                | Self::RenewConfirm
                | Self::NodeShellConfirm
                | Self::NodeShellOptions
                | Self::DebugContainerOptions
                | Self::NodeShellConfirmStaging
                | Self::NodeTaintsEditor
                | Self::NodeTaintsEditorInvalid
                | Self::NodeLabelsEditor
                | Self::NodeLabelsBulkEditor
                | Self::DrainDialog
                | Self::DrainDialogSkipPdbs
                | Self::LeftoverSweepFixture
                | Self::RevisionDiff
        )
    }

    /// Whether the screen is a dock tab drawn from fixed data: it waits for no cluster, only for the
    /// tab to open.
    #[cfg(any(feature = "screenshot", test))]
    pub(crate) fn is_dock_fixture(self) -> bool {
        matches!(
            self,
            Self::NodeShellTabFixture
                | Self::DebugShellTabFixture
                | Self::DrainProgress
                | Self::DrainProgressStuck
        )
    }

    /// Whether the screen is drawn from fixed forwards and waits for no cluster.
    #[cfg(any(feature = "screenshot", test))]
    pub(crate) fn is_port_forward_fixture(self) -> bool {
        matches!(
            self,
            Self::PortForwards
                | Self::PortForwardsList
                | Self::PortForwardNewFixture
                | Self::PortForwardConfirmFixture
                | Self::PortForwardRemoveFixture
        )
    }

    /// Whether the log dock must be open on a pod.
    pub(crate) fn has_dock(self) -> bool {
        matches!(
            self,
            Self::LogsDock
                | Self::LogsZoomed
                | Self::LogsPopout
                | Self::LogsWorkload
                | Self::ShellFixture
                | Self::ShellDockFixture
                | Self::ShellPasteFixture
                | Self::ShellPickerFixture
                | Self::ShellFindFixture
                | Self::NodeShellTabFixture
                | Self::DebugShellTabFixture
                | Self::DrainProgress
                | Self::DrainProgressStuck
        )
    }

    fn parse(text: &str) -> Option<Self> {
        match text {
            "overview" => Some(Self::Overview),
            "switcher" => Some(Self::Switcher),
            "cordon-confirm" => Some(Self::CordonConfirm),
            "unlock-confirm" => Some(Self::UnlockConfirm),
            "scale-popover" => Some(Self::ScalePopover),
            "scale-confirm" => Some(Self::ScaleConfirm),
            "restart-bulk-confirm" => Some(Self::RestartBulkConfirm),
            "delete-confirm" => Some(Self::DeleteConfirm),
            "delete-bulk-confirm" => Some(Self::DeleteBulkConfirm),
            "restart-pod-confirm" => Some(Self::RestartPodConfirm),
            "evict-confirm" => Some(Self::EvictConfirm),
            "edit-yaml-diff" => Some(Self::EditYamlDiff),
            "edit-yaml-history" => Some(Self::EditYamlHistory),
            "values-edit" => Some(Self::ValuesEdit),
            "new-config-map" => Some(Self::NewConfigMap),
            "revision-diff" => Some(Self::RevisionDiff),
            "hpa-range-popover" => Some(Self::HpaRangePopover),
            "expand-confirm" => Some(Self::ExpandConfirm),
            "default-class-confirm" => Some(Self::DefaultClassConfirm),
            "renew-confirm" => Some(Self::RenewConfirm),
            "pods" => Some(Self::Pods),
            "nodes" => Some(Self::Nodes),
            "issues" => Some(Self::Issues),
            "issues-drawer" => Some(Self::IssuesDrawer),
            "topology" => Some(Self::Topology),
            "topology-problems" => Some(Self::TopologyProblems),
            "topology-rbac" => Some(Self::TopologyRbac),
            "topology-selected" => Some(Self::TopologySelected),
            "topology-curves" => Some(Self::TopologyCurves),
            "topology-traffic" => Some(Self::TopologyTraffic),
            "topology-traffic-curves" => Some(Self::TopologyTrafficCurves),
            "topology-traffic-fixture" => Some(Self::TopologyTrafficFixture),
            "topology-traffic-fixture-curves" => Some(Self::TopologyTrafficFixtureCurves),
            "pod-drawer" => Some(Self::PodDrawer(DrawerTab::Overview)),
            "pod-containers" => Some(Self::PodDrawer(DrawerTab::Containers)),
            "pod-events" => Some(Self::PodDrawer(DrawerTab::Events)),
            "pod-monitor" => Some(Self::PodDrawer(DrawerTab::Monitor)),
            "pod-monitor-source-fixture" => Some(Self::PodMonitorSourceFixture),
            "node-monitor" => Some(Self::NodeDrawer(DrawerTab::Monitor)),
            "node-drawer" => Some(Self::NodeDrawer(DrawerTab::Overview)),
            "node-events" => Some(Self::NodeDrawer(DrawerTab::Events)),
            "pod-yaml" => Some(Self::PodDrawer(DrawerTab::Yaml)),
            "node-yaml" => Some(Self::NodeDrawer(DrawerTab::Yaml)),
            "logs-dock" => Some(Self::LogsDock),
            "logs-zoomed" => Some(Self::LogsZoomed),
            "logs-popout" => Some(Self::LogsPopout),
            "logs-workload" => Some(Self::LogsWorkload),
            "shell-fixture" => Some(Self::ShellFixture),
            "shell-dock-fixture" => Some(Self::ShellDockFixture),
            "shell-paste-fixture" => Some(Self::ShellPasteFixture),
            "shell-picker-fixture" => Some(Self::ShellPickerFixture),
            "shell-confirm-fixture" => Some(Self::ShellConfirmFixture),
            "attach-confirm" => Some(Self::AttachConfirm),
            "node-shell-confirm" => Some(Self::NodeShellConfirm),
            "node-shell-options" => Some(Self::NodeShellOptions),
            "debug-container-options" => Some(Self::DebugContainerOptions),
            "node-shell-confirm-staging" => Some(Self::NodeShellConfirmStaging),
            "node-taints-editor" => Some(Self::NodeTaintsEditor),
            "node-labels-editor" => Some(Self::NodeLabelsEditor),
            "node-labels-bulk-editor" => Some(Self::NodeLabelsBulkEditor),
            "node-taints-editor-invalid" => Some(Self::NodeTaintsEditorInvalid),
            "drain-dialog" => Some(Self::DrainDialog),
            "drain-dialog-skip-pdbs" => Some(Self::DrainDialogSkipPdbs),
            "drain-progress" => Some(Self::DrainProgress),
            "drain-progress-stuck" => Some(Self::DrainProgressStuck),
            "leftover-sweep-fixture" => Some(Self::LeftoverSweepFixture),
            "node-shell-tab-fixture" => Some(Self::NodeShellTabFixture),
            "debug-shell-tab-fixture" => Some(Self::DebugShellTabFixture),
            "shell-find-fixture" => Some(Self::ShellFindFixture),
            "port-forwards" => Some(Self::PortForwards),
            "port-forwards-list" => Some(Self::PortForwardsList),
            "port-forward-new-fixture" => Some(Self::PortForwardNewFixture),
            "port-forward-confirm-fixture" => Some(Self::PortForwardConfirmFixture),
            "port-forward-remove-fixture" => Some(Self::PortForwardRemoveFixture),
            "pods-selected" => Some(Self::PodsSelected),
            "nodes-selected" => Some(Self::NodesSelected),
            "shortcuts" => Some(Self::Shortcuts),
            "pods-cursor" => Some(Self::PodsCursor),
            "who-can" => Some(Self::WhoCan),
            "check-permissions" => Some(Self::CheckPermissions),
            "account-permissions" => Some(Self::AccountPermissions),
            "test-traffic" => Some(Self::TestTraffic),
            "settings" => Some(Self::Settings(
                SettingsPage::Clusters,
                SettingsSize::Standard,
            )),
            // The whole Clusters form fits, so a screenshot can show its footer.
            "settings-tall" => Some(Self::Settings(SettingsPage::Clusters, SettingsSize::Tall)),
            "settings-general" => Some(Self::Settings(
                SettingsPage::General,
                SettingsSize::Standard,
            )),
            "settings-environments" => Some(Self::Settings(
                SettingsPage::Environments,
                SettingsSize::Standard,
            )),
            "settings-logs" => Some(Self::Settings(SettingsPage::Logs, SettingsSize::Standard)),
            "settings-terminal" => Some(Self::Settings(
                SettingsPage::TerminalAndShell,
                SettingsSize::Standard,
            )),
            "settings-appearance" => Some(Self::Settings(
                SettingsPage::Appearance,
                SettingsSize::Standard,
            )),
            "settings-metrics" => Some(Self::SettingsMetrics),
            "settings-metrics-fixture" => Some(Self::SettingsMetricsFixture),
            "settings-shortcuts" => Some(Self::Settings(
                SettingsPage::KeyboardShortcuts,
                SettingsSize::Tall,
            )),
            _ => {
                if let Some(plural) = text.strip_suffix("-menu") {
                    return ResourceKind::from_plural(plural).map(Self::KindMenu);
                }
                if let Some(plural) = text.strip_suffix("-drawer") {
                    let kind = ResourceKind::from_plural(plural)?;
                    return Some(Self::KindDrawer(kind, DrawerTab::Overview));
                }
                if let Some(plural) = text.strip_suffix("-yaml") {
                    let kind = ResourceKind::from_plural(plural)?;
                    return Some(Self::KindDrawer(kind, DrawerTab::Yaml));
                }
                if let Some(plural) = text.strip_suffix("-monitor") {
                    // Only the workloads that own pods have a Monitor tab.
                    let kind =
                        ResourceKind::from_plural(plural).filter(|kind| kind.has_monitor())?;
                    return Some(Self::KindDrawer(kind, DrawerTab::Monitor));
                }
                for (suffix, tab) in [
                    ("-values", DrawerTab::Values),
                    ("-manifest", DrawerTab::Manifest),
                ] {
                    if let Some(plural) = text.strip_suffix(suffix) {
                        // Only a release drawer has the Helm tabs.
                        let kind = ResourceKind::from_plural(plural)
                            .filter(|kind| *kind == ResourceKind::HelmReleases)?;
                        return Some(Self::KindDrawer(kind, tab));
                    }
                }
                if let Some(plural) = text.strip_suffix("-events") {
                    let kind = ResourceKind::from_plural(plural)?;
                    return Some(Self::KindDrawer(kind, DrawerTab::Events));
                }
                ResourceKind::from_plural(text).map(Self::Kind)
            }
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct LaunchOptions {
    pub(crate) kubeconfig: Option<PathBuf>,
    pub(crate) context: Option<String>,
    pub(crate) namespace: Option<NamespaceScope>,
    /// The start screen's quick filter text; a `label:` text becomes chips.
    pub(crate) filter: Option<String>,
    /// The row a drawer screen opens: `name` or `namespace/name`; the first row without it.
    pub(crate) select: Option<String>,
    pub(crate) theme: Option<ThemePreference>,
    /// `--color-theme`: the colour family for this run only; never saved.
    pub(crate) color_theme: Option<ColorTheme>,
    /// `--config-dir`: where `settings.json` lives; the environment or the OS default without it.
    pub(crate) config_dir: Option<PathBuf>,
    pub(crate) screen: LaunchScreen,
    pub(crate) screenshot: Option<PathBuf>,
    /// `--window-width`: the window width in pixels, within `WINDOW_WIDTH_RANGE`; the default
    /// width without it.
    pub(crate) window_width: Option<u16>,
    /// `--palette`: the command palette opens with this text typed, once the session is live.
    pub(crate) palette: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum LaunchRequest {
    Run(Box<LaunchOptions>),
    Help,
}

/// Parses the flags (without the program name). The error is a one-line message for the
/// user; the caller adds the usage text.
pub(crate) fn parse_launch_options(
    args: impl Iterator<Item = String>,
) -> Result<LaunchRequest, String> {
    let mut args = args;
    let mut options = LaunchOptions {
        kubeconfig: None,
        context: None,
        namespace: None,
        filter: None,
        select: None,
        theme: None,
        color_theme: None,
        config_dir: None,
        screen: LaunchScreen::Overview,
        screenshot: None,
        window_width: None,
        palette: None,
    };
    while let Some(flag) = args.next() {
        if flag == "--help" {
            return Ok(LaunchRequest::Help);
        }
        let mut value = || {
            args.next()
                .ok_or_else(|| format!("missing value for {flag}"))
        };
        match flag.as_str() {
            "--kubeconfig" => options.kubeconfig = Some(PathBuf::from(value()?)),
            "--context" => options.context = Some(value()?),
            "--namespace" => options.namespace = Some(parse_namespaces(&value()?)?),
            "--filter" => options.filter = Some(value()?),
            "--select" => options.select = Some(value()?),
            "--theme" => options.theme = Some(parse_theme(&value()?)?),
            "--color-theme" => options.color_theme = Some(parse_color_theme(&value()?)?),
            "--config-dir" => options.config_dir = Some(PathBuf::from(value()?)),
            "--screen" => {
                let text = value()?;
                if let Some(custom) = parse_custom(&text) {
                    options.screen = custom;
                    continue;
                }
                options.screen = LaunchScreen::parse(&text)
                    .ok_or_else(|| format!("invalid value '{text}' for --screen"))?;
            }
            "--screenshot" => options.screenshot = Some(PathBuf::from(value()?)),
            "--window-width" => options.window_width = Some(parse_window_width(&value()?)?),
            "--palette" => options.palette = Some(value()?),
            _ => return Err(format!("unknown flag '{flag}'")),
        }
    }
    Ok(LaunchRequest::Run(Box::new(options)))
}

/// `custom:<crd-name>` with an optional `-drawer`, `-events`, or `-yaml` suffix, which is stripped
/// first so CRD names keep their dashes. `None` for any other `--screen` value.
fn parse_custom(text: &str) -> Option<LaunchScreen> {
    let spec = text.strip_prefix("custom:")?;
    let (name, tab) = [
        ("-drawer", DrawerTab::Overview),
        ("-events", DrawerTab::Events),
        ("-yaml", DrawerTab::Yaml),
    ]
    .into_iter()
    .find_map(|(suffix, tab)| Some((spec.strip_suffix(suffix)?, Some(tab))))
    .unwrap_or((spec, None));
    Some(LaunchScreen::Custom {
        crd_name: name.to_owned().leak(),
        tab,
    })
}

/// The window widths the layout is designed for; the narrow end shows one column.
const WINDOW_WIDTH_RANGE: std::ops::RangeInclusive<u16> = 800..=3840;

fn parse_window_width(text: &str) -> Result<u16, String> {
    match text.parse::<u16>() {
        Ok(width) if WINDOW_WIDTH_RANGE.contains(&width) => Ok(width),
        _ => Err(format!(
            "invalid value '{text}' for --window-width: use {} to {} pixels",
            WINDOW_WIDTH_RANGE.start(),
            WINDOW_WIDTH_RANGE.end()
        )),
    }
}

/// `a` or `a,b,c`: the namespaces to show. Empty parts are ignored.
fn parse_namespaces(text: &str) -> Result<NamespaceScope, String> {
    let names: Vec<String> = text
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect();
    match NamespaceScope::of_namespaces(names) {
        NamespaceScope::All => Err("--namespace needs at least one namespace".to_owned()),
        scope if scope.namespaces().len() > MAX_NAMESPACES => Err(format!(
            "at most {MAX_NAMESPACES} namespaces for --namespace"
        )),
        scope => Ok(scope),
    }
}

fn parse_color_theme(text: &str) -> Result<ColorTheme, String> {
    match text {
        "default" => Ok(ColorTheme::Default),
        "zed-one" => Ok(ColorTheme::ZedOne),
        _ => Err(format!("invalid value '{text}' for --color-theme")),
    }
}

fn parse_theme(text: &str) -> Result<ThemePreference, String> {
    match text {
        "system" => Ok(ThemePreference::System),
        "light" => Ok(ThemePreference::Light),
        "dark" => Ok(ThemePreference::Dark),
        _ => Err(format!("invalid value '{text}' for --theme")),
    }
}

/// The kubectl chain: `--kubeconfig`, else every non-empty `KUBECONFIG` entry, else
/// `<home>/.kube/config`. Paths are made absolute; empty means no kubeconfig could be located.
pub(crate) fn kubeconfig_chain(
    flag: Option<PathBuf>,
    kubeconfig_env: Option<OsString>,
    home: Option<PathBuf>,
) -> Vec<PathBuf> {
    let chain: Vec<PathBuf> = match (flag, kubeconfig_env) {
        (Some(flag), _) => vec![flag],
        (None, Some(value)) => non_empty_entries(&value).collect(),
        (None, None) => Vec::new(),
    };
    let chain = if chain.is_empty() {
        home.map(|home| home.join(".kube").join("config"))
            .into_iter()
            .collect()
    } else {
        chain
    };
    chain.into_iter().map(absolute).collect()
}

/// The files to load on their own: the registry files in registry order, then the files of the
/// watched folders; absolute, without chain members and without duplicates (a file in the registry
/// loads there, not again as a folder file).
pub(crate) fn standalone_files(
    registered: &[PathBuf],
    folder_files: &[PathBuf],
    chain: &[PathBuf],
) -> Vec<PathBuf> {
    let same = |a: &PathBuf, b: &PathBuf| {
        same_path_text(&a.to_string_lossy(), &b.to_string_lossy(), PathStyle::HOST)
    };
    let mut files: Vec<PathBuf> = Vec::new();
    for file in registered.iter().chain(folder_files).cloned().map(absolute) {
        if !chain.iter().any(|member| same(member, &file))
            && !files.iter().any(|known| same(known, &file))
        {
            files.push(file);
        }
    }
    files
}

/// No I/O and no symlink resolution, so the path is a stable registry key.
fn absolute(path: PathBuf) -> PathBuf {
    std::path::absolute(&path).unwrap_or(path)
}

fn non_empty_entries(value: &OsStr) -> impl Iterator<Item = PathBuf> {
    std::env::split_paths(value).filter(|entry| !entry.as_os_str().is_empty())
}

#[cfg(test)]
#[path = "launch_options_tests.rs"]
mod launch_options_tests;
