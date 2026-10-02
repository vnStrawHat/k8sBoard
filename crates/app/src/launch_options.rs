use std::ffi::{OsStr, OsString};
use std::path::PathBuf;

use cluster::NamespaceScope;

use crate::app_shell::Screen;
use crate::drawer::DrawerTab;
use crate::namespace_picker::MAX_NAMESPACES;
use crate::resource_kind::ResourceKind;

pub(crate) const USAGE: &str = "\
Usage: k8sboard [options]

Options:
  --kubeconfig <path>    kubeconfig file (default: first KUBECONFIG entry, else ~/.kube/config)
  --context <name>       context to open (default: the kubeconfig current-context)
  --namespace <a[,b]>    namespaces to show, at most 5 (default: all namespaces if allowed)
  --filter <text>        quick filter of the start screen; label:k=v,k2!=v2 becomes label chips
  --select <name>       with a drawer screen, open the row named <name> or <namespace>/<name>
                         (default: the first row)
  --theme light|dark     colour theme (default: follow the system)
  --screen overview|pods|nodes|issues|issues-drawer|pod-drawer|pod-containers|pod-events|pod-monitor|node-drawer|node-events|node-monitor|pod-yaml|node-yaml|logs-dock|logs-zoomed|logs-workload|pods-selected|nodes-selected|
           namespaces|events|deployments|statefulsets|daemonsets|replicasets|jobs|cronjobs|
           services|ingresses|configmaps|<kind>-drawer|<kind>-events|<kind>-monitor|<kind>-yaml|releases-values|releases-manifest|
           customresourcedefinitions|custom:<crd-name>[-drawer|-events|-yaml]|who-can|check-permissions|account-permissions|test-traffic
                         screen to open (default: pods)
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
    /// `--screen pod-drawer|pod-containers|pod-events|pod-yaml`: a pod drawer on that tab.
    PodDrawer(DrawerTab),
    /// `--screen node-drawer|node-events|node-yaml`.
    NodeDrawer(DrawerTab),
    LogsDock,
    LogsZoomed,
    /// `--screen logs-workload`: the dock zoomed on the workload that owns the logs pod.
    LogsWorkload,
    /// `--screen pods-selected|nodes-selected`: the first two rows are ticked.
    PodsSelected,
    NodesSelected,
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
    /// `--screen <plural>`, e.g. `deployments`.
    Kind(ResourceKind),
    /// `--screen <plural>-drawer|<plural>-events|<plural>-yaml`: the kind's first row selected, on that tab.
    KindDrawer(ResourceKind, DrawerTab),
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
            Self::Overview => Screen::Overview,
            Self::Pods
            | Self::PodDrawer(_)
            | Self::LogsDock
            | Self::LogsZoomed
            | Self::LogsWorkload
            | Self::PodsSelected => Screen::Pods,
            // The sidebar group of Custom Resources starts open; the shell then resolves the kind
            // against the CRD list.
            Self::Custom { .. } => Screen::Kind(ResourceKind::Crds),
            Self::Nodes | Self::NodeDrawer(_) | Self::NodesSelected => Screen::Nodes,
            Self::Issues | Self::IssuesDrawer => Screen::Issues,
            Self::Kind(kind) | Self::KindDrawer(kind, _) => Screen::Kind(kind),
            Self::WhoCan => Screen::Kind(ResourceKind::ClusterRoles),
            Self::TestTraffic => Screen::Kind(ResourceKind::NetworkPolicies),
            Self::CheckPermissions | Self::AccountPermissions => {
                Screen::Kind(ResourceKind::ServiceAccounts)
            }
        }
    }

    /// Whether a row must be selected so the drawer is open.
    pub(crate) fn has_drawer(self) -> bool {
        matches!(
            self,
            Self::PodDrawer(_)
                | Self::NodeDrawer(_)
                | Self::KindDrawer(..)
                | Self::IssuesDrawer
                | Self::Custom { tab: Some(_), .. }
        )
    }

    /// The drawer tab this request opens on; `None` for screens without a drawer.
    pub(crate) fn drawer_tab(self) -> Option<DrawerTab> {
        match self {
            Self::PodDrawer(tab) | Self::NodeDrawer(tab) | Self::KindDrawer(_, tab) => Some(tab),
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
        matches!(self, Self::PodsSelected | Self::NodesSelected)
    }

    /// Whether the screen shows pod usage, so a screenshot waits for a metrics tick.
    #[cfg(any(feature = "screenshot", test))]
    pub(crate) fn shows_pod_usage(self) -> bool {
        matches!(
            self,
            Self::Pods
                | Self::PodsSelected
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
                | Self::NodeDrawer(DrawerTab::Overview | DrawerTab::Monitor)
        )
    }

    /// Whether a tool dialog opens once the session is live.
    pub(crate) fn opens_dialog(self) -> bool {
        matches!(
            self,
            Self::WhoCan | Self::CheckPermissions | Self::AccountPermissions | Self::TestTraffic
        )
    }

    /// Whether the log dock must be open on a pod.
    pub(crate) fn has_log_dock(self) -> bool {
        matches!(self, Self::LogsDock | Self::LogsZoomed | Self::LogsWorkload)
    }

    fn parse(text: &str) -> Option<Self> {
        match text {
            "overview" => Some(Self::Overview),
            "pods" => Some(Self::Pods),
            "nodes" => Some(Self::Nodes),
            "issues" => Some(Self::Issues),
            "issues-drawer" => Some(Self::IssuesDrawer),
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
            "pods-selected" => Some(Self::PodsSelected),
            "nodes-selected" => Some(Self::NodesSelected),
            "who-can" => Some(Self::WhoCan),
            "check-permissions" => Some(Self::CheckPermissions),
            "account-permissions" => Some(Self::AccountPermissions),
            "test-traffic" => Some(Self::TestTraffic),
            _ => {
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ThemeChoice {
    Light,
    Dark,
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
    pub(crate) theme: Option<ThemeChoice>,
    pub(crate) screen: LaunchScreen,
    pub(crate) screenshot: Option<PathBuf>,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum LaunchRequest {
    Run(LaunchOptions),
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
        screen: LaunchScreen::Pods,
        screenshot: None,
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
            _ => return Err(format!("unknown flag '{flag}'")),
        }
    }
    Ok(LaunchRequest::Run(options))
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

fn parse_theme(text: &str) -> Result<ThemeChoice, String> {
    match text {
        "light" => Ok(ThemeChoice::Light),
        "dark" => Ok(ThemeChoice::Dark),
        _ => Err(format!("invalid value '{text}' for --theme")),
    }
}

/// `--kubeconfig`, else the first entry of `KUBECONFIG`, else `<home>/.kube/config`.
/// `None` means no kubeconfig could be located at all.
pub(crate) fn kubeconfig_path(
    flag: Option<PathBuf>,
    kubeconfig_env: Option<OsString>,
    home: Option<PathBuf>,
) -> Option<PathBuf> {
    if flag.is_some() {
        return flag;
    }
    let first_env_entry = kubeconfig_env
        .as_deref()
        .and_then(|value| non_empty_entries(value).next());
    first_env_entry.or_else(|| home.map(|home| home.join(".kube").join("config")))
}

/// Merging kubeconfigs is not supported, so the user is told when entries are ignored.
pub(crate) fn has_ignored_kubeconfig_entries(kubeconfig_env: Option<&OsStr>) -> bool {
    kubeconfig_env.is_some_and(|value| non_empty_entries(value).nth(1).is_some())
}

fn non_empty_entries(value: &OsStr) -> impl Iterator<Item = PathBuf> {
    std::env::split_paths(value).filter(|entry| !entry.as_os_str().is_empty())
}

#[cfg(test)]
#[path = "launch_options_tests.rs"]
mod launch_options_tests;
