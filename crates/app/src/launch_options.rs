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
  --theme light|dark     colour theme (default: follow the system)
  --screen pods|nodes|pod-drawer|pod-containers|pod-events|pod-monitor|node-drawer|node-events|node-monitor|pod-yaml|node-yaml|logs-dock|logs-zoomed|pods-selected|nodes-selected|
           namespaces|events|deployments|statefulsets|daemonsets|replicasets|jobs|cronjobs|
           services|ingresses|configmaps|<kind>-drawer|<kind>-events|<kind>-monitor|<kind>-yaml
                         screen to open (default: pods)
  --screenshot <path>    write a PNG and exit (needs a build with --features screenshot)
  --help                 print this help
";

/// The screen to open. The drawer values open Pods, Nodes, or a kind with a row already
/// selected, and the logs values open Pods with the log dock on a pod.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LaunchScreen {
    Pods,
    Nodes,
    /// `--screen pod-drawer|pod-containers|pod-events|pod-yaml`: a pod drawer on that tab.
    PodDrawer(DrawerTab),
    /// `--screen node-drawer|node-events|node-yaml`.
    NodeDrawer(DrawerTab),
    LogsDock,
    LogsZoomed,
    /// `--screen pods-selected|nodes-selected`: the first two rows are ticked.
    PodsSelected,
    NodesSelected,
    /// `--screen <plural>`, e.g. `deployments`.
    Kind(ResourceKind),
    /// `--screen <plural>-drawer|<plural>-events|<plural>-yaml`: the kind's first row selected, on that tab.
    KindDrawer(ResourceKind, DrawerTab),
}

impl LaunchScreen {
    /// The list screen this request opens on.
    pub(crate) fn screen(self) -> Screen {
        match self {
            Self::Pods
            | Self::PodDrawer(_)
            | Self::LogsDock
            | Self::LogsZoomed
            | Self::PodsSelected => Screen::Pods,
            Self::Nodes | Self::NodeDrawer(_) | Self::NodesSelected => Screen::Nodes,
            Self::Kind(kind) | Self::KindDrawer(kind, _) => Screen::Kind(kind),
        }
    }

    /// Whether a row must be selected so the drawer is open.
    pub(crate) fn has_drawer(self) -> bool {
        matches!(
            self,
            Self::PodDrawer(_) | Self::NodeDrawer(_) | Self::KindDrawer(..)
        )
    }

    /// The drawer tab this request opens on; `None` for screens without a drawer.
    pub(crate) fn drawer_tab(self) -> Option<DrawerTab> {
        match self {
            Self::PodDrawer(tab) | Self::NodeDrawer(tab) | Self::KindDrawer(_, tab) => Some(tab),
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

    /// Whether the screen shows Network and Disk I/O, so a screenshot waits for kubelet rounds: the
    /// Monitor tabs. A feed that is unavailable settles at once.
    #[cfg(any(feature = "screenshot", test))]
    pub(crate) fn shows_kubelet_stats(self) -> bool {
        self.drawer_tab() == Some(DrawerTab::Monitor)
    }

    /// Whether the screen shows node usage.
    #[cfg(any(feature = "screenshot", test))]
    pub(crate) fn shows_node_usage(self) -> bool {
        matches!(
            self,
            Self::Nodes
                | Self::NodesSelected
                | Self::NodeDrawer(DrawerTab::Overview | DrawerTab::Monitor)
        )
    }

    /// Whether the log dock must be open on a pod.
    pub(crate) fn has_log_dock(self) -> bool {
        matches!(self, Self::LogsDock | Self::LogsZoomed)
    }

    fn parse(text: &str) -> Option<Self> {
        match text {
            "pods" => Some(Self::Pods),
            "nodes" => Some(Self::Nodes),
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
            "pods-selected" => Some(Self::PodsSelected),
            "nodes-selected" => Some(Self::NodesSelected),
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
            "--theme" => options.theme = Some(parse_theme(&value()?)?),
            "--screen" => {
                let text = value()?;
                options.screen = LaunchScreen::parse(&text)
                    .ok_or_else(|| format!("invalid value '{text}' for --screen"))?;
            }
            "--screenshot" => options.screenshot = Some(PathBuf::from(value()?)),
            _ => return Err(format!("unknown flag '{flag}'")),
        }
    }
    Ok(LaunchRequest::Run(options))
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
