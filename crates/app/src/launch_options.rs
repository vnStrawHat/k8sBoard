use std::ffi::{OsStr, OsString};
use std::path::PathBuf;

use cluster::NamespaceScope;

use crate::app_shell::Screen;
use crate::cluster_view::MAX_VIEWED_CLUSTERS;
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
  --view <a[,b]>         contexts to view together, at most 5; the first in the switcher order is the
                         primary; wins over --context (default: the one cluster of --context)
  --namespace <a[,b]>    namespaces to show, at most 5 (default: all namespaces if allowed)
  --filter <text>        quick filter of the start screen; label:k=v,k2!=v2 becomes label chips
  --select <name>       with a drawer screen, open the row named <name> or <namespace>/<name>
                         (default: the first row)
  --theme system|light|dark
                         colour theme (default: the saved theme, else follow the system)
  --config-dir <path>    settings folder (default: K8SBOARD_CONFIG_DIR, else the OS config folder)
  --screen overview|switcher|cordon-confirm|unlock-confirm|scale-popover|scale-confirm|restart-bulk-confirm|delete-confirm|delete-bulk-confirm|edit-yaml-diff|pods|pods-multi|nodes|issues|issues-drawer|topology|topology-problems|topology-selected|pod-drawer|pod-containers|pod-events|pod-monitor|node-drawer|node-events|node-monitor|pod-yaml|node-yaml|logs-dock|logs-zoomed|logs-workload|shell-fixture|shell-dock-fixture|shell-paste-fixture|shell-picker-fixture|shell-confirm-fixture|node-shell-confirm|node-shell-options|debug-container-options|node-shell-confirm-staging|leftover-sweep-fixture|node-shell-tab-fixture|debug-shell-tab-fixture|shell-find-fixture|port-forwards|port-forwards-list|port-forward-new-fixture|port-forward-confirm-fixture|port-forward-remove-fixture|pods-selected|nodes-selected|shortcuts|pods-cursor|
           namespaces|events|deployments|statefulsets|daemonsets|replicasets|jobs|cronjobs|
           services|ingresses|configmaps|<kind>-drawer|<kind>-events|<kind>-monitor|<kind>-yaml|releases-values|releases-manifest|
           customresourcedefinitions|custom:<crd-name>[-drawer|-events|-yaml]|who-can|check-permissions|account-permissions|test-traffic|settings|settings-tall|settings-appearance|settings-shortcuts
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
    /// `--screen pods-multi`: Pods over the clusters `--view` names; the screenshot waits for every
    /// slot to be Live with a loaded list, or Failed.
    PodsMulti,
    Nodes,
    /// `--screen issues`.
    Issues,
    /// `--screen issues-drawer`: Issues, then the first issue revealed with its drawer.
    IssuesDrawer,
    /// `--screen topology`.
    Topology,
    /// `--screen topology-problems`: the same screen with Problems only on.
    TopologyProblems,
    /// `--screen topology-selected`: the first Deployment selected, its drawer open, and motion
    /// reduced so the capture does not depend on the clock.
    TopologySelected,
    /// `--screen pod-drawer|pod-containers|pod-events|pod-yaml`: a pod drawer on that tab.
    PodDrawer(DrawerTab),
    /// `--screen node-drawer|node-events|node-yaml`.
    NodeDrawer(DrawerTab),
    LogsDock,
    LogsZoomed,
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
    /// `--screen edit-yaml-diff`: the Edit YAML view on its Diff tab, drawn from fixed data (W10). It
    /// waits for no cluster and can never send. Screenshot builds only.
    EditYamlDiff,
    /// `--screen settings|settings-appearance|settings-shortcuts`: the main window opens as usual,
    /// then the Settings window on that page, which is what the screenshot captures.
    Settings(SettingsPage, SettingsSize),
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
            | Self::NodeShellOptions => Screen::Nodes,
            Self::DebugContainerOptions
            | Self::LeftoverSweepFixture
            | Self::NodeShellTabFixture
            | Self::DebugShellTabFixture => Screen::Pods,
            Self::ShellConfirmFixture | Self::DeleteBulkConfirm => Screen::Pods,
            Self::EditYamlDiff => Screen::Kind(ResourceKind::Deployments),
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
            | Self::PodsMulti
            | Self::PodDrawer(_)
            | Self::LogsDock
            | Self::LogsZoomed
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
            Self::Settings(..) => Screen::Overview,
            Self::Topology | Self::TopologyProblems | Self::TopologySelected => Screen::Topology,
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
            Self::Topology | Self::TopologyProblems | Self::TopologySelected
        )
    }

    /// Whether the screen shows pod usage, so a screenshot waits for a metrics tick.
    #[cfg(any(feature = "screenshot", test))]
    pub(crate) fn shows_pod_usage(self) -> bool {
        matches!(
            self,
            Self::Pods
                | Self::PodsMulti
                | Self::PodsSelected
                | Self::Shortcuts
                | Self::PodsCursor
                | Self::PodDrawer(DrawerTab::Containers | DrawerTab::Monitor)
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
                | Self::NodeShellConfirm
                | Self::NodeShellOptions
                | Self::DebugContainerOptions
                | Self::NodeShellConfirmStaging
                | Self::LeftoverSweepFixture
                | Self::ScalePopover
                | Self::ScaleConfirm
                | Self::RestartBulkConfirm
                | Self::DeleteConfirm
                | Self::DeleteBulkConfirm
                | Self::EditYamlDiff
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
                | Self::NodeShellConfirm
                | Self::NodeShellOptions
                | Self::DebugContainerOptions
                | Self::NodeShellConfirmStaging
                | Self::LeftoverSweepFixture
        )
    }

    /// Whether the screen is a dock tab drawn from fixed data: it waits for no cluster, only for the
    /// tab to open.
    #[cfg(any(feature = "screenshot", test))]
    pub(crate) fn is_dock_fixture(self) -> bool {
        matches!(self, Self::NodeShellTabFixture | Self::DebugShellTabFixture)
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
                | Self::LogsWorkload
                | Self::ShellFixture
                | Self::ShellDockFixture
                | Self::ShellPasteFixture
                | Self::ShellPickerFixture
                | Self::ShellFindFixture
                | Self::NodeShellTabFixture
                | Self::DebugShellTabFixture
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
            "edit-yaml-diff" => Some(Self::EditYamlDiff),
            "pods" => Some(Self::Pods),
            "pods-multi" => Some(Self::PodsMulti),
            "nodes" => Some(Self::Nodes),
            "issues" => Some(Self::Issues),
            "issues-drawer" => Some(Self::IssuesDrawer),
            "topology" => Some(Self::Topology),
            "topology-problems" => Some(Self::TopologyProblems),
            "topology-selected" => Some(Self::TopologySelected),
            "pod-drawer" => Some(Self::PodDrawer(DrawerTab::Overview)),
            "pod-containers" => Some(Self::PodDrawer(DrawerTab::Containers)),
            "pod-events" => Some(Self::PodDrawer(DrawerTab::Events)),
            "pod-monitor" => Some(Self::PodDrawer(DrawerTab::Monitor)),
            "node-monitor" => Some(Self::NodeDrawer(DrawerTab::Monitor)),
            "node-drawer" => Some(Self::NodeDrawer(DrawerTab::Overview)),
            "node-events" => Some(Self::NodeDrawer(DrawerTab::Events)),
            "pod-yaml" => Some(Self::PodDrawer(DrawerTab::Yaml)),
            "node-yaml" => Some(Self::NodeDrawer(DrawerTab::Yaml)),
            "logs-dock" => Some(Self::LogsDock),
            "logs-zoomed" => Some(Self::LogsZoomed),
            "logs-workload" => Some(Self::LogsWorkload),
            "shell-fixture" => Some(Self::ShellFixture),
            "shell-dock-fixture" => Some(Self::ShellDockFixture),
            "shell-paste-fixture" => Some(Self::ShellPasteFixture),
            "shell-picker-fixture" => Some(Self::ShellPickerFixture),
            "shell-confirm-fixture" => Some(Self::ShellConfirmFixture),
            "node-shell-confirm" => Some(Self::NodeShellConfirm),
            "node-shell-options" => Some(Self::NodeShellOptions),
            "debug-container-options" => Some(Self::DebugContainerOptions),
            "node-shell-confirm-staging" => Some(Self::NodeShellConfirmStaging),
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
            "settings-appearance" => Some(Self::Settings(
                SettingsPage::Appearance,
                SettingsSize::Standard,
            )),
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
    /// `--view`: the contexts to view together, at most `MAX_VIEWED_CLUSTERS`; empty views the one
    /// cluster the start rules pick.
    pub(crate) view: Vec<String>,
    /// The start screen's quick filter text; a `label:` text becomes chips.
    pub(crate) filter: Option<String>,
    /// The row a drawer screen opens: `name` or `namespace/name`; the first row without it.
    pub(crate) select: Option<String>,
    pub(crate) theme: Option<ThemePreference>,
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
        view: Vec::new(),
        filter: None,
        select: None,
        theme: None,
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
            "--view" => options.view = parse_view(&value()?)?,
            "--filter" => options.filter = Some(value()?),
            "--select" => options.select = Some(value()?),
            "--theme" => options.theme = Some(parse_theme(&value()?)?),
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

/// `a` or `a,b`: the contexts to view together. Empty parts are ignored.
fn parse_view(text: &str) -> Result<Vec<String>, String> {
    let mut contexts: Vec<String> = Vec::new();
    for context in text
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
    {
        if !contexts.iter().any(|known| known == context) {
            contexts.push(context.to_owned());
        }
    }
    match contexts.len() {
        0 => Err("--view needs at least one context".to_owned()),
        count if count > MAX_VIEWED_CLUSTERS => {
            Err(format!("at most {MAX_VIEWED_CLUSTERS} contexts for --view"))
        }
        _ => Ok(contexts),
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

/// The registry files to load on their own: absolute, in registry order, without chain members
/// and without duplicates.
pub(crate) fn standalone_files(registered: &[PathBuf], chain: &[PathBuf]) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = Vec::new();
    for file in registered.iter().cloned().map(absolute) {
        if !chain.contains(&file) && !files.contains(&file) {
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
