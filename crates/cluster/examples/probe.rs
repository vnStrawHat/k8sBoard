//! Read-only probe of one cluster context. Prints domain summaries only: never
//! credentials, and never `Debug` output of kube types.
//!
//! ```text
//! cargo run -p k8sboard-cluster --example probe -- --kubeconfig <path> [--context <name>] [--namespace <name>] [--watch-seconds <n>]
//! ```

use std::collections::BTreeMap;
use std::error::Error;
use std::io::{self, StdoutLock, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use cluster::{
    AccessDecision, ClusterConnection, ClusterError, ContainerState, ContainerSummary, Kubeconfig,
    MetricsApi, NamespaceScope, NodeReadiness, NodeScheduling, PodStatus, PodSummary, StatusReason,
    Termination, WatchUpdate,
};
use futures::StreamExt;

const USAGE: &str = "usage: probe --kubeconfig <path> [--context <name>] [--namespace <name>] [--watch-seconds <n>]";
const MAX_LISTED_PODS: usize = 30;
const MAX_DETAILED_PODS: usize = 20;
const NONE_TEXT: &str = "<none>";

struct Args {
    kubeconfig: PathBuf,
    context: Option<String>,
    namespace: Option<String>,
    watch_seconds: Option<u64>,
}

enum Parsed {
    Run(Args),
    Help,
}

fn parse_args(mut args: impl Iterator<Item = String>) -> Result<Parsed, String> {
    let mut kubeconfig = None;
    let mut context = None;
    let mut namespace = None;
    let mut watch_seconds = None;
    while let Some(flag) = args.next() {
        let mut value = |name: &str| args.next().ok_or(format!("missing value for {name}"));
        match flag.as_str() {
            "--help" => return Ok(Parsed::Help),
            "--kubeconfig" => kubeconfig = Some(PathBuf::from(value("--kubeconfig")?)),
            "--context" => context = Some(value("--context")?),
            "--namespace" => namespace = Some(value("--namespace")?),
            "--watch-seconds" => {
                watch_seconds = Some(parse_watch_seconds(&value("--watch-seconds")?)?)
            }
            other => return Err(format!("unknown argument '{other}'")),
        }
    }
    let kubeconfig = kubeconfig.ok_or("missing required --kubeconfig")?;
    Ok(Parsed::Run(Args {
        kubeconfig,
        context,
        namespace,
        watch_seconds,
    }))
}

/// A positive integer number of seconds.
fn parse_watch_seconds(text: &str) -> Result<u64, String> {
    match text.parse::<u64>() {
        Ok(seconds) if seconds > 0 => Ok(seconds),
        _ => Err(format!(
            "--watch-seconds needs a positive integer, got '{text}'"
        )),
    }
}

/// Counts of what one watch stream produced. Never holds object contents.
#[derive(Default)]
struct WatchStats {
    snapshots: usize,
    last_items: usize,
    failures: usize,
    last_error: Option<String>,
}

impl WatchStats {
    fn record<T>(&mut self, update: WatchUpdate<T>) {
        match update {
            WatchUpdate::Snapshot(items) => {
                self.snapshots += 1;
                self.last_items = items.len();
            }
            WatchUpdate::Failed(error) => {
                self.failures += 1;
                self.last_error = Some(error.to_string());
            }
        }
    }

    /// Nothing at all happened: no snapshot and no failure.
    fn is_silent(&self) -> bool {
        self.snapshots == 0 && self.failures == 0
    }

    fn line(&self, kind: &str) -> String {
        let mut line = format!(
            "watch {kind}: {} snapshots, last {} items, {} failures",
            self.snapshots, self.last_items, self.failures
        );
        if let Some(error) = &self.last_error {
            line.push_str("; last error: ");
            line.push_str(error);
        }
        line
    }
}

/// Runs the three watches together for `seconds` and prints one line per kind.
async fn watch_for(
    probe: &mut Probe,
    connection: &ClusterConnection,
    scope: NamespaceScope,
    seconds: u64,
) -> io::Result<()> {
    probe.section(&format!("watch ({seconds}s)"))?;
    let mut pods = Box::pin(connection.watch_pods(scope));
    let mut nodes = Box::pin(connection.watch_nodes());
    let mut namespaces = Box::pin(connection.watch_namespaces());
    let (mut pod_stats, mut node_stats, mut namespace_stats) = (
        WatchStats::default(),
        WatchStats::default(),
        WatchStats::default(),
    );
    let timer = tokio::time::sleep(Duration::from_secs(seconds));
    tokio::pin!(timer);
    loop {
        tokio::select! {
            () = &mut timer => break,
            Some(update) = pods.next() => pod_stats.record(update),
            Some(update) = nodes.next() => node_stats.record(update),
            Some(update) = namespaces.next() => namespace_stats.record(update),
            else => break,
        }
    }
    for (kind, stats) in [
        ("pods", &pod_stats),
        ("nodes", &node_stats),
        ("namespaces", &namespace_stats),
    ] {
        writeln!(probe.out, "{}", stats.line(kind))?;
        if stats.is_silent() {
            probe.all_succeeded = false;
        }
    }
    Ok(())
}

#[tokio::main]
async fn main() -> ExitCode {
    let args = match parse_args(std::env::args().skip(1)) {
        Ok(Parsed::Run(args)) => args,
        Ok(Parsed::Help) => {
            let _ = writeln!(io::stdout().lock(), "{USAGE}");
            return ExitCode::SUCCESS;
        }
        Err(message) => {
            eprintln!("error: {message}\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    match run(&args).await {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(error) => {
            eprintln!("error: cannot write to stdout: {error}");
            ExitCode::from(1)
        }
    }
}

/// Prints every section. Returns whether all of them succeeded.
async fn run(args: &Args) -> io::Result<bool> {
    let mut probe = Probe {
        out: io::stdout().lock(),
        all_succeeded: true,
    };

    let kubeconfig = match Kubeconfig::load(&args.kubeconfig) {
        Ok(kubeconfig) => kubeconfig,
        Err(error) => {
            print_error_chain("", &error);
            return Ok(false);
        }
    };
    probe.print_kubeconfig(&kubeconfig)?;

    let context = match kubeconfig.resolve_context(args.context.as_deref()) {
        Ok(context) => context.name.clone(),
        Err(error) => {
            print_error_chain("", &error);
            return Ok(false);
        }
    };
    writeln!(probe.out, "\ncontext: {context}")?;
    let connection = match ClusterConnection::open(&kubeconfig, &context).await {
        Ok(connection) => connection,
        Err(error) => {
            print_error_chain("", &error);
            return Ok(false);
        }
    };

    let scope = match &args.namespace {
        Some(namespace) => NamespaceScope::Named(namespace.clone()),
        None => NamespaceScope::All,
    };
    let scope_label = args.namespace.as_deref().unwrap_or("all namespaces");

    probe.section("server")?;
    match connection.server_version().await {
        Ok(version) => writeln!(probe.out, "  {}  {}", version.git_version, version.platform)?,
        Err(error) => probe.fail(&error)?,
    }

    probe.section("metrics.k8s.io")?;
    match connection.metrics_api().await {
        Ok(MetricsApi::Available { group_version }) => {
            writeln!(probe.out, "  available {group_version}")?;
        }
        Ok(MetricsApi::Unavailable {
            group_version,
            reason,
        }) => writeln!(probe.out, "  unavailable {group_version}: {reason}")?,
        Ok(MetricsApi::NotInstalled) => writeln!(probe.out, "  not installed")?,
        Err(error) => probe.fail(&error)?,
    }

    probe.section(&format!("access ({scope_label})"))?;
    match connection.review_access(scope.clone()).await {
        Ok(report) => {
            for review in &report.reviews {
                match &review.decision {
                    AccessDecision::Allowed => {
                        writeln!(probe.out, "  {:<26} allowed", review.check)?
                    }
                    AccessDecision::Denied {
                        reason: Some(reason),
                    } => {
                        writeln!(probe.out, "  {:<26} denied: {reason}", review.check)?;
                    }
                    AccessDecision::Denied { reason: None } => {
                        writeln!(probe.out, "  {:<26} denied", review.check)?;
                    }
                }
            }
        }
        Err(error) => probe.fail(&error)?,
    }

    match connection.list_namespaces().await {
        Ok(namespaces) => {
            probe.section(&format!("namespaces ({})", namespaces.len()))?;
            writeln!(probe.out, "  NAME  PHASE  CREATED")?;
            for namespace in &namespaces {
                let created = timestamp_text(namespace.created_at);
                writeln!(
                    probe.out,
                    "  {}  {:?}  {created}",
                    namespace.name, namespace.phase
                )?;
            }
        }
        Err(error) => {
            probe.section("namespaces")?;
            probe.fail(&error)?;
        }
    }

    match connection.list_nodes().await {
        Ok(nodes) => {
            probe.section(&format!("nodes ({})", nodes.len()))?;
            writeln!(
                probe.out,
                "  NAME  STATUS  ROLES  TAINTS  VERSION  INTERNAL-IP  CREATED"
            )?;
            for node in &nodes {
                let readiness = match node.status.readiness {
                    NodeReadiness::Ready => "Ready",
                    NodeReadiness::NotReady => "NotReady",
                    NodeReadiness::Unknown => "Unknown",
                };
                let scheduling = match node.status.scheduling {
                    NodeScheduling::Enabled => "",
                    NodeScheduling::Disabled => ",SchedulingDisabled",
                };
                let taints: Vec<String> = node.taints.iter().map(ToString::to_string).collect();
                writeln!(
                    probe.out,
                    "  {}  {readiness}{scheduling}  {}  {}  {}  {}  {}",
                    node.name,
                    joined_or_none(&node.roles),
                    joined_or_none(&taints),
                    node.kubelet_version,
                    node.internal_ip.as_deref().unwrap_or(NONE_TEXT),
                    timestamp_text(node.created_at),
                )?;
            }
        }
        Err(error) => {
            probe.section("nodes")?;
            probe.fail(&error)?;
        }
    }

    match connection.list_pods(scope.clone()).await {
        Ok(pods) => probe.print_pods(scope_label, &pods)?,
        Err(error) => {
            probe.section("pods")?;
            probe.fail(&error)?;
        }
    }

    if let Some(seconds) = args.watch_seconds {
        watch_for(&mut probe, &connection, scope, seconds).await?;
    }

    Ok(probe.all_succeeded)
}

struct Probe {
    out: StdoutLock<'static>,
    all_succeeded: bool,
}

impl Probe {
    fn section(&mut self, title: &str) -> io::Result<()> {
        writeln!(self.out, "\n{title}")
    }

    /// Records the failure and prints the error chain to stderr.
    fn fail(&mut self, error: &ClusterError) -> io::Result<()> {
        self.all_succeeded = false;
        writeln!(self.out, "  failed (details on stderr)")?;
        print_error_chain(error_kind(error), error);
        Ok(())
    }

    fn print_kubeconfig(&mut self, kubeconfig: &Kubeconfig) -> io::Result<()> {
        writeln!(self.out, "kubeconfig: {}", kubeconfig.path().display())?;
        let current = kubeconfig.current_context();
        for context in kubeconfig.contexts() {
            let marker = if current == Some(context.name.as_str()) {
                '*'
            } else {
                ' '
            };
            writeln!(
                self.out,
                "{marker} {}  cluster={}  user={}  namespace={}",
                context.name,
                context.cluster,
                context.user.as_deref().unwrap_or("-"),
                context.namespace.as_deref().unwrap_or("-"),
            )?;
        }
        writeln!(
            self.out,
            "current-context: {}",
            current.unwrap_or("(not set)")
        )
    }

    fn print_pods(&mut self, scope_label: &str, pods: &[PodSummary]) -> io::Result<()> {
        self.section(&format!("pods ({}) in {scope_label}", pods.len()))?;

        let mut histogram: BTreeMap<String, usize> = BTreeMap::new();
        for pod in pods {
            *histogram.entry(pod.status.to_string()).or_default() += 1;
        }
        let mut counts: Vec<_> = histogram.into_iter().collect();
        counts.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
        for (status, count) in &counts {
            writeln!(self.out, "  {count:>5}  {status}")?;
        }

        writeln!(
            self.out,
            "\n  NAMESPACE  NAME  STATUS  READY  RESTARTS  NODE  CREATED"
        )?;
        for pod in pods.iter().take(MAX_LISTED_PODS) {
            writeln!(
                self.out,
                "  {}  {}  {}  {}  {}  {}  {}",
                pod.namespace,
                pod.name,
                pod.status,
                pod.ready,
                pod.restarts,
                pod.node_name.as_deref().unwrap_or(NONE_TEXT),
                timestamp_text(pod.created_at),
            )?;
        }

        let unhealthy = pods
            .iter()
            .filter(|pod| !is_running_or_completed(&pod.status));
        for pod in unhealthy.take(MAX_DETAILED_PODS) {
            writeln!(
                self.out,
                "\n  {}/{}  {}",
                pod.namespace, pod.name, pod.status
            )?;
            for container in &pod.containers {
                writeln!(self.out, "    {}", container_text(container))?;
            }
        }
        Ok(())
    }
}

fn is_running_or_completed(status: &PodStatus) -> bool {
    matches!(
        status,
        PodStatus::Reason(StatusReason::Running | StatusReason::Completed)
    )
}

fn container_text(container: &ContainerSummary) -> String {
    let state = match &container.state {
        ContainerState::Waiting { reason } => format!("waiting({})", reason_text(reason.as_ref())),
        ContainerState::Running { .. } => "running".to_owned(),
        ContainerState::Terminated(termination) => {
            format!("terminated({})", termination_text(termination))
        }
        ContainerState::NotReported => "not-reported".to_owned(),
    };
    let last = container
        .last_termination
        .as_ref()
        .map_or_else(|| "-".to_owned(), termination_text);
    format!(
        "{:?}  {}  {state}  {}  {}  {last}",
        container.kind, container.name, container.is_ready, container.restart_count,
    )
}

fn termination_text(termination: &Termination) -> String {
    let mut text = format!(
        "{} exit={}",
        reason_text(termination.reason.as_ref()),
        termination.exit_code
    );
    if let Some(signal) = termination.signal {
        text.push_str(&format!(" signal={signal}"));
    }
    text
}

fn reason_text(reason: Option<&StatusReason>) -> String {
    reason.map_or_else(|| "-".to_owned(), ToString::to_string)
}

fn timestamp_text(timestamp: Option<impl ToString>) -> String {
    timestamp.map_or_else(|| "-".to_owned(), |timestamp| timestamp.to_string())
}

fn joined_or_none(items: &[String]) -> String {
    if items.is_empty() {
        return NONE_TEXT.to_owned();
    }
    items.join(",")
}

/// Prints `error: ...` and one `caused by:` line per source. Domain errors only:
/// their messages never carry credentials.
fn print_error_chain(kind: &str, error: &(impl Error + ?Sized)) {
    eprintln!("error: {kind}{error}");
    let mut source = error.source();
    while let Some(cause) = source {
        eprintln!("caused by: {cause}");
        source = cause.source();
    }
}

/// Names the variant, because `Display` words it differently (for example "is not allowed").
fn error_kind(error: &ClusterError) -> &'static str {
    match error {
        ClusterError::Kubeconfig(_) => "Kubeconfig: ",
        ClusterError::InvalidConfig { .. } => "InvalidConfig: ",
        ClusterError::Unreachable { .. } => "Unreachable: ",
        ClusterError::TimedOut { .. } => "TimedOut: ",
        ClusterError::CredentialsUnavailable { .. } => "CredentialsUnavailable: ",
        ClusterError::Unauthorized { .. } => "Unauthorized: ",
        ClusterError::Forbidden { .. } => "Forbidden: ",
        ClusterError::Api { .. } => "Api: ",
        ClusterError::UnexpectedResponse { .. } => "UnexpectedResponse: ",
    }
}
