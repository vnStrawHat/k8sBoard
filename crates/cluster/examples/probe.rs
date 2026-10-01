//! Read-only probe of one cluster context. Prints domain summaries only: never
//! credentials, and never `Debug` output of kube types. With `--watch-seconds` it runs
//! the pods, nodes, namespaces, nine workload, network, and config watches plus two
//! events watches together, and prints counts per kind. With `--yaml` it reads the masked
//! YAML of the first pod and the first node and prints line counts and masking checks, never
//! the YAML text. The access section doubles as the RBAC probe of the context.
//!
//! ```text
//! cargo run -p k8sboard-cluster --example probe -- --kubeconfig <path> [--context <name>] [--namespace <name>] [--watch-seconds <n>] [--logs-seconds <n>] [--yaml]
//! ```

use std::collections::BTreeMap;
use std::error::Error;
use std::io::{self, StdoutLock, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use cluster::{
    AccessDecision, ClusterConnection, ClusterError, ContainerKind, ContainerState,
    ContainerSummary, EnvValues, EventFilter, Kubeconfig, LogRequest, LogSource, LogUpdate,
    MetricsApi, NamespaceScope, NodeReadiness, NodeScheduling, ObjectKind, ObjectRef, PodStatus,
    PodSummary, StatusReason, Termination, WatchUpdate,
};
use futures::stream::{self, BoxStream};
use futures::{Stream, StreamExt};

const USAGE: &str = "usage: probe --kubeconfig <path> [--context <name>] [--namespace <name>] [--watch-seconds <n>] [--logs-seconds <n>] [--yaml]";
const MAX_LISTED_PODS: usize = 30;
const MAX_DETAILED_PODS: usize = 20;
const NONE_TEXT: &str = "<none>";

struct Args {
    kubeconfig: PathBuf,
    context: Option<String>,
    namespace: Option<String>,
    watch_seconds: Option<u64>,
    logs_seconds: Option<u64>,
    yaml: bool,
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
    let mut logs_seconds = None;
    let mut yaml = false;
    while let Some(flag) = args.next() {
        let mut value = |name: &str| args.next().ok_or(format!("missing value for {name}"));
        match flag.as_str() {
            "--help" => return Ok(Parsed::Help),
            "--yaml" => yaml = true,
            "--kubeconfig" => kubeconfig = Some(PathBuf::from(value("--kubeconfig")?)),
            "--context" => context = Some(value("--context")?),
            "--namespace" => namespace = Some(value("--namespace")?),
            "--watch-seconds" => {
                watch_seconds = Some(parse_seconds(
                    "--watch-seconds",
                    &value("--watch-seconds")?,
                )?)
            }
            "--logs-seconds" => {
                logs_seconds = Some(parse_seconds("--logs-seconds", &value("--logs-seconds")?)?)
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
        logs_seconds,
        yaml,
    }))
}

/// A positive integer number of seconds.
fn parse_seconds(flag: &str, text: &str) -> Result<u64, String> {
    match text.parse::<u64>() {
        Ok(seconds) if seconds > 0 => Ok(seconds),
        _ => Err(format!("{flag} needs a positive integer, got '{text}'")),
    }
}

/// What one watch update contributed. Never holds object contents.
enum Tally {
    Snapshot(usize),
    Failed(String),
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
    fn record(&mut self, tally: Tally) {
        match tally {
            Tally::Snapshot(items) => {
                self.snapshots += 1;
                self.last_items = items;
            }
            Tally::Failed(error) => {
                self.failures += 1;
                self.last_error = Some(error);
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

/// A watch stream reduced to tallies, tagged with the kind it watches.
type TallySource = (&'static str, BoxStream<'static, Tally>);

fn tally_source<T: Send + 'static>(
    kind: &'static str,
    updates: impl Stream<Item = WatchUpdate<T>> + Send + 'static,
) -> TallySource {
    let tallies = updates.map(|update| match update {
        WatchUpdate::Snapshot(items) => Tally::Snapshot(items.len()),
        WatchUpdate::Failed(error) => Tally::Failed(error_summary(&error)),
    });
    (kind, tallies.boxed())
}

/// Runs all fourteen watches together for `seconds` and prints one line per kind.
async fn watch_for(
    probe: &mut Probe,
    connection: &ClusterConnection,
    scope: NamespaceScope,
    seconds: u64,
) -> io::Result<()> {
    probe.section(&format!("watch ({seconds}s)"))?;
    let sources = vec![
        tally_source("pods", connection.watch_pods(scope.clone())),
        tally_source("nodes", connection.watch_nodes()),
        tally_source("namespaces", connection.watch_namespaces()),
        tally_source("deployments", connection.watch_deployments(scope.clone())),
        tally_source(
            "stateful sets",
            connection.watch_stateful_sets(scope.clone()),
        ),
        tally_source("daemon sets", connection.watch_daemon_sets(scope.clone())),
        tally_source("replica sets", connection.watch_replica_sets(scope.clone())),
        tally_source("jobs", connection.watch_jobs(scope.clone())),
        tally_source("cron jobs", connection.watch_cron_jobs(scope.clone())),
        tally_source("services", connection.watch_services(scope.clone())),
        tally_source("ingresses", connection.watch_ingresses(scope.clone())),
        tally_source("config maps", connection.watch_config_maps(scope.clone())),
        tally_source(
            "events",
            connection.watch_events(scope.clone(), EventFilter::All),
        ),
        tally_source(
            "warning events",
            connection.watch_events(scope, EventFilter::WarningsOnly),
        ),
    ];
    let mut stats: Vec<(&'static str, WatchStats)> = sources
        .iter()
        .map(|(kind, _)| (*kind, WatchStats::default()))
        .collect();
    let mut merged = stream::select_all(
        sources
            .into_iter()
            .enumerate()
            .map(|(index, (_, tallies))| tallies.map(move |tally| (index, tally))),
    );
    let timer = tokio::time::sleep(Duration::from_secs(seconds));
    tokio::pin!(timer);
    loop {
        tokio::select! {
            () = &mut timer => break,
            item = merged.next() => match item {
                Some((index, tally)) => {
                    if let Some((_, kind_stats)) = stats.get_mut(index) {
                        kind_stats.record(tally);
                    }
                }
                None => break,
            },
        }
    }
    for (kind, kind_stats) in &stats {
        writeln!(probe.out, "{}", kind_stats.line(kind))?;
        if kind_stats.is_silent() {
            probe.all_succeeded = false;
        }
    }
    Ok(())
}

/// Counts of what one log stream produced. Never holds log text.
#[derive(Default)]
struct LogStats {
    is_started: bool,
    lines: usize,
    batches: usize,
    failures: usize,
    last_error: Option<String>,
}

impl LogStats {
    fn record(&mut self, update: LogUpdate) {
        match update {
            LogUpdate::Started => self.is_started = true,
            LogUpdate::Lines(lines) => {
                self.batches += 1;
                self.lines += lines.len();
            }
            LogUpdate::Failed(error) => {
                self.failures += 1;
                self.last_error = Some(error_summary(&error));
            }
        }
    }

    fn line(&self, target: &str, is_ended: bool) -> String {
        let started = if self.is_started {
            "started"
        } else {
            "not started"
        };
        let ended = if is_ended { "yes" } else { "no" };
        let mut line = format!(
            "logs {target}: {started}, {} lines in {} batches, ended: {ended}, {} failures",
            self.lines, self.batches, self.failures
        );
        if let Some(error) = &self.last_error {
            line.push_str("; last error: ");
            line.push_str(error);
        }
        line
    }
}

/// The first pod with a running main container, with that container.
fn pod_with_running_container(pods: &[PodSummary]) -> Option<(&PodSummary, &ContainerSummary)> {
    pods.iter().find_map(|pod| {
        let container = pod.containers.iter().find(|container| {
            container.kind == ContainerKind::Main
                && matches!(container.state, ContainerState::Running { .. })
        })?;
        Some((pod, container))
    })
}

/// Streams the current logs of one running container for `seconds` and prints counts only.
async fn logs_for(
    probe: &mut Probe,
    connection: &ClusterConnection,
    pods: &[PodSummary],
    seconds: u64,
) -> io::Result<()> {
    probe.section(&format!("logs ({seconds}s)"))?;
    let Some((pod, container)) = pod_with_running_container(pods) else {
        probe.all_succeeded = false;
        return writeln!(probe.out, "logs: no running pod in scope");
    };
    let target = format!("{}/{}/{}", pod.namespace, pod.name, container.name);
    let mut updates = Box::pin(connection.pod_logs(LogRequest {
        namespace: pod.namespace.clone(),
        pod: pod.name.clone(),
        container: container.name.clone(),
        source: LogSource::Current,
    }));
    let mut stats = LogStats::default();
    let mut is_ended = false;
    let timer = tokio::time::sleep(Duration::from_secs(seconds));
    tokio::pin!(timer);
    loop {
        tokio::select! {
            () = &mut timer => break,
            update = updates.next() => match update {
                Some(update) => stats.record(update),
                None => {
                    is_ended = true;
                    break;
                }
            },
        }
    }
    writeln!(probe.out, "{}", stats.line(&target, is_ended))?;
    if !stats.is_started && stats.failures == 0 {
        probe.all_succeeded = false;
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

    let mut first_node = None;
    match connection.list_nodes().await {
        Ok(nodes) => {
            first_node = nodes.first().map(|node| node.name.clone());
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

    let pods = connection.list_pods(scope.clone()).await;
    match &pods {
        Ok(pods) => probe.print_pods(scope_label, pods)?,
        Err(error) => {
            probe.section("pods")?;
            probe.fail(error)?;
        }
    }

    if args.yaml {
        probe.section("yaml")?;
        let first_pod = pods.as_ref().ok().and_then(|pods| pods.first());
        let pod_ref = first_pod.and_then(|pod| {
            let namespace = Some(pod.namespace.clone());
            ObjectRef::new(ObjectKind::Pod, namespace, pod.name.clone())
        });
        let node_ref = first_node
            .clone()
            .and_then(|name| ObjectRef::new(ObjectKind::Node, None, name));
        let targets = [
            (
                first_pod.map(|pod| format!("{}/{}", pod.namespace, pod.name)),
                pod_ref,
                "pod",
            ),
            (first_node, node_ref, "node"),
        ];
        for (name, object, label) in targets {
            let (Some(name), Some(object)) = (name, object) else {
                probe.all_succeeded = false;
                writeln!(probe.out, "  yaml {label}: none in scope")?;
                continue;
            };
            probe.print_yaml(label, &name, &object, &connection).await?;
        }
    }

    if let Some(seconds) = args.watch_seconds {
        watch_for(&mut probe, &connection, scope, seconds).await?;
    }
    if let Some(seconds) = args.logs_seconds {
        let pods = pods.as_ref().map_or(&[][..], Vec::as_slice);
        logs_for(&mut probe, &connection, pods, seconds).await?;
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

    /// Reads one object's masked YAML and prints counts and masking checks, never the text.
    async fn print_yaml(
        &mut self,
        label: &str,
        name: &str,
        object: &ObjectRef,
        connection: &ClusterConnection,
    ) -> io::Result<()> {
        let yaml = match connection.object_yaml(object, EnvValues::Hidden).await {
            Ok(yaml) => yaml,
            Err(error) => {
                writeln!(self.out, "yaml {label} {name}: failed (details on stderr)")?;
                self.all_succeeded = false;
                print_error_chain(error_kind(&error), &error);
                return Ok(());
            }
        };
        let managed_fields = if yaml.text.contains("managedFields") {
            "PRESENT"
        } else {
            "absent"
        };
        let last_applied = if yaml.text.contains("last-applied-configuration: <hidden>") {
            "hidden"
        } else if yaml.text.contains("last-applied-configuration") {
            "VISIBLE"
        } else {
            "absent"
        };
        writeln!(
            self.out,
            "yaml {label} {name}: {} lines, {} env values hidden, managedFields {managed_fields}, last-applied {last_applied}",
            yaml.text.lines().count(),
            yaml.hidden_env_values,
        )
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

/// The error plus the first line of its cause, so `Unreachable` shows its io error. Domain
/// errors only: their messages never carry credentials.
fn error_summary(error: &ClusterError) -> String {
    let cause = error
        .source()
        .and_then(|source| source.to_string().lines().next().map(str::to_owned));
    match cause {
        Some(cause) if !cause.is_empty() => format!("{error}: {cause}"),
        _ => error.to_string(),
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
