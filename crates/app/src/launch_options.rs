use std::ffi::{OsStr, OsString};
use std::path::PathBuf;

use crate::app_shell::Screen;
use crate::resource_kind::ResourceKind;

pub(crate) const USAGE: &str = "\
Usage: k8sboard [options]

Options:
  --kubeconfig <path>    kubeconfig file (default: first KUBECONFIG entry, else ~/.kube/config)
  --context <name>       context to open (default: the kubeconfig current-context)
  --namespace <name>     namespace to show (default: all namespaces if allowed)
  --theme light|dark     colour theme (default: follow the system)
  --screen pods|nodes|pod-drawer|pod-containers|node-drawer|logs-dock|logs-zoomed|
           namespaces|events|deployments|statefulsets|daemonsets|replicasets|jobs|cronjobs|
           services|ingresses|configmaps|<kind>-drawer
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
    PodDrawer,
    PodContainers,
    NodeDrawer,
    LogsDock,
    LogsZoomed,
    /// `--screen <plural>`, e.g. `deployments`.
    Kind(ResourceKind),
    /// `--screen <plural>-drawer`: the kind's first row selected.
    KindDrawer(ResourceKind),
}

impl LaunchScreen {
    /// The list screen this request opens on.
    pub(crate) fn screen(self) -> Screen {
        match self {
            Self::Pods
            | Self::PodDrawer
            | Self::PodContainers
            | Self::LogsDock
            | Self::LogsZoomed => Screen::Pods,
            Self::Nodes | Self::NodeDrawer => Screen::Nodes,
            Self::Kind(kind) | Self::KindDrawer(kind) => Screen::Kind(kind),
        }
    }

    /// Whether a row must be selected so the drawer is open.
    pub(crate) fn has_drawer(self) -> bool {
        matches!(
            self,
            Self::PodDrawer | Self::PodContainers | Self::NodeDrawer | Self::KindDrawer(_)
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
            "pod-drawer" => Some(Self::PodDrawer),
            "pod-containers" => Some(Self::PodContainers),
            "node-drawer" => Some(Self::NodeDrawer),
            "logs-dock" => Some(Self::LogsDock),
            "logs-zoomed" => Some(Self::LogsZoomed),
            _ => match text.strip_suffix("-drawer") {
                Some(plural) => ResourceKind::from_plural(plural).map(Self::KindDrawer),
                None => ResourceKind::from_plural(text).map(Self::Kind),
            },
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
    pub(crate) namespace: Option<String>,
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
            "--namespace" => options.namespace = Some(value()?),
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
