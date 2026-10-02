//! Read-only probe of one cluster context. Prints domain summaries only: never
//! credentials, and never `Debug` output of kube types. With `--watch-seconds` it runs
//! the pods, nodes, namespaces, nine workload, network, and config watches, the endpointslices watch, four policy watches (network policies, autoscalers, quotas, disruption budgets), three storage watches (claims, volumes, classes), and two
//! events watches together, and prints counts per kind. With `--metrics-seconds` it polls pod
//! and node metrics (metrics.k8s.io) and prints one count-and-sum line per poll. With `--kubelet-seconds` it polls the kubelet stats of every Ready node through the node proxy (cAdvisor disk I/O for the first one) and prints counts per node per round. With `--counts` it prints one object-count line per kind (`limit=1` lists, nothing else is read). With `--yaml` it reads the masked
//! YAML of the first pod and the first node and prints line counts and masking checks, never
//! the YAML text. With `--secrets` it prints Secret counts by type and certificate parse counts, then reads one TLS secret and prints its key and byte counts, never a value. With `--helm` it prints Helm release counts by status, then reads the first release and prints line and document counts, never values, manifest text, notes, or descriptions. With `--crds` it prints CRD counts and printer-column support, then access, count, object watch, and YAML lines for the established CRDs (add `--watch-seconds` for the watch line and `--yaml` for the YAML line), never object names or values. The access section doubles as the RBAC probe of the context. With `--analysis` it prints the RBAC snapshot counts and coverage, the caller's rules review count, and the Who-can grant count for `get secrets`, counts only.
//!
//! ```text
//! cargo run -p k8sboard-cluster --example probe -- --kubeconfig <path> [--context <name>] [--namespace <name[,name...]>] [--watch-seconds <n>] [--logs-seconds <n>] [--metrics-seconds <n>] [--kubelet-seconds <n>] [--counts] [--yaml] [--secrets] [--helm] [--crds] [--analysis]
//! ```

use std::collections::BTreeMap;
use std::error::Error;
use std::io::{self, StdoutLock, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use cluster::{
    AccessCheck, AccessDecision, AccessReport, AccessRequest, ClusterConnection, ClusterError,
    ColumnValue, ContainerKind, ContainerState, ContainerSummary, CrdState, CrdSummary,
    CronJobSummary, EnvValues, EventFilter, GrantNames, HelmReleaseSummary, HelmRevisionRef,
    Kubeconfig, KubeletTargets, LogRequest, LogSource, LogUpdate, MetricsApi, NamespaceCoverage,
    NamespaceScope, NodeKubeletStats, NodeMetrics, NodeReadiness, NodeScheduling, NodeSummary,
    ObjectKind, ObjectRef, PodMetrics, PodStatus, PodSummary, RequestTarget, ResourceRequest,
    SecretDetails, SecretSummary, StatusReason, Termination, ValueVisibility, WatchUpdate,
};
use futures::stream::{self, BoxStream};
use futures::{Stream, StreamExt};
use tokio::sync::watch;

const USAGE: &str = "usage: probe --kubeconfig <path> [--context <name>] [--namespace <name[,name...]>] [--watch-seconds <n>] [--logs-seconds <n>] [--metrics-seconds <n>] [--kubelet-seconds <n>] [--counts] [--yaml] [--secrets] [--helm] [--crds] [--analysis]";
const MAX_LISTED_PODS: usize = 30;
const MAX_DETAILED_PODS: usize = 20;
const NONE_TEXT: &str = "<none>";

struct Args {
    kubeconfig: PathBuf,
    context: Option<String>,
    namespace: Option<String>,
    watch_seconds: Option<u64>,
    logs_seconds: Option<u64>,
    metrics_seconds: Option<u64>,
    kubelet_seconds: Option<u64>,
    counts: bool,
    yaml: bool,
    secrets: bool,
    helm: bool,
    crds: bool,
    analysis: bool,
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
    let mut metrics_seconds = None;
    let mut kubelet_seconds = None;
    let mut counts = false;
    let mut yaml = false;
    let mut secrets = false;
    let mut helm = false;
    let mut crds = false;
    let mut analysis = false;
    while let Some(flag) = args.next() {
        let mut value = |name: &str| args.next().ok_or(format!("missing value for {name}"));
        match flag.as_str() {
            "--help" => return Ok(Parsed::Help),
            "--counts" => counts = true,
            "--yaml" => yaml = true,
            "--secrets" => secrets = true,
            "--helm" => helm = true,
            "--crds" => crds = true,
            "--analysis" => analysis = true,
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
            "--metrics-seconds" => {
                metrics_seconds = Some(parse_seconds(
                    "--metrics-seconds",
                    &value("--metrics-seconds")?,
                )?)
            }
            "--kubelet-seconds" => {
                kubelet_seconds = Some(parse_seconds(
                    "--kubelet-seconds",
                    &value("--kubelet-seconds")?,
                )?)
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
        metrics_seconds,
        kubelet_seconds,
        counts,
        yaml,
        secrets,
        helm,
        crds,
        analysis,
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
    Snapshot { items: usize, note: Option<String> },
    Failed(String),
}

/// Counts of what one watch stream produced. Never holds object contents.
#[derive(Default)]
struct WatchStats {
    snapshots: usize,
    last_items: usize,
    last_note: Option<String>,
    failures: usize,
    last_error: Option<String>,
}

impl WatchStats {
    fn record(&mut self, tally: Tally) {
        match tally {
            Tally::Snapshot { items, note } => {
                self.snapshots += 1;
                self.last_items = items;
                self.last_note = note;
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
        if let Some(note) = &self.last_note {
            line.push_str("; ");
            line.push_str(note);
        }
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
    tally_source_noted(kind, updates, |_| None)
}

/// Like `tally_source`, with a short note about the last snapshot appended to the kind's line.
fn tally_source_noted<T: Send + 'static>(
    kind: &'static str,
    updates: impl Stream<Item = WatchUpdate<T>> + Send + 'static,
    note: fn(&[T]) -> Option<String>,
) -> TallySource {
    let tallies = updates.map(move |update| match update {
        WatchUpdate::Snapshot(items) => Tally::Snapshot {
            items: items.len(),
            note: note(&items),
        },
        WatchUpdate::Failed(error) => Tally::Failed(error_summary(&error)),
    });
    (kind, tallies.boxed())
}

/// The next run of the first cron job that has one, in its own zone: shows that the bundled
/// time zone database works on this machine. Prints a time only, never the cron job.
fn next_run_note(cron_jobs: &[CronJobSummary]) -> Option<String> {
    let now = jiff::Timestamp::now();
    let next = cron_jobs
        .iter()
        .find_map(|cron_job| cron_job.timetable.as_ref().ok()?.next_after(now))?;
    Some(format!("next {}", next.strftime("%Y-%m-%d %H:%M:%S %Z")))
}

/// Runs all the watches together for `seconds` and prints one line per kind.
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
        tally_source_noted(
            "cron jobs",
            connection.watch_cron_jobs(scope.clone()),
            next_run_note,
        ),
        tally_source("services", connection.watch_services(scope.clone())),
        tally_source("ingresses", connection.watch_ingresses(scope.clone())),
        tally_source("config maps", connection.watch_config_maps(scope.clone())),
        tally_source(
            "endpointslices",
            connection.watch_endpoint_slices(scope.clone()),
        ),
        tally_source(
            "network policies",
            connection.watch_network_policies(scope.clone()),
        ),
        tally_source(
            "horizontal pod autoscalers",
            connection.watch_horizontal_pod_autoscalers(scope.clone()),
        ),
        tally_source(
            "resource quotas",
            connection.watch_resource_quotas(scope.clone()),
        ),
        tally_source(
            "pod disruption budgets",
            connection.watch_pod_disruption_budgets(scope.clone()),
        ),
        tally_source(
            "persistent volume claims",
            connection.watch_persistent_volume_claims(scope.clone()),
        ),
        tally_source("persistent volumes", connection.watch_persistent_volumes()),
        tally_source("storage classes", connection.watch_storage_classes()),
        tally_source(
            "service accounts",
            connection.watch_service_accounts(scope.clone()),
        ),
        tally_source("secrets", connection.watch_secrets(scope.clone())),
        tally_source("tls secrets", connection.watch_tls_secrets(scope.clone())),
        tally_source(
            "helm releases",
            connection.watch_helm_releases(scope.clone()),
        ),
        tally_source("roles", connection.watch_roles(scope.clone())),
        tally_source("cluster roles", connection.watch_cluster_roles()),
        tally_source(
            "role bindings",
            connection.watch_role_bindings(scope.clone()),
        ),
        tally_source(
            "cluster role bindings",
            connection.watch_cluster_role_bindings(),
        ),
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

/// The kinds `--counts` counts, in sidebar order, with the check that gates each.
const COUNT_KINDS: [(ObjectKind, &str, AccessCheck); 27] = [
    (ObjectKind::Pod, "pods", AccessCheck::ListPods),
    (ObjectKind::Node, "nodes", AccessCheck::ListNodes),
    (
        ObjectKind::Namespace,
        "namespaces",
        AccessCheck::ListNamespaces,
    ),
    (ObjectKind::Event, "events", AccessCheck::ListEvents),
    (
        ObjectKind::Deployment,
        "deployments",
        AccessCheck::ListDeployments,
    ),
    (
        ObjectKind::StatefulSet,
        "statefulsets",
        AccessCheck::ListStatefulSets,
    ),
    (
        ObjectKind::DaemonSet,
        "daemonsets",
        AccessCheck::ListDaemonSets,
    ),
    (
        ObjectKind::ReplicaSet,
        "replicasets",
        AccessCheck::ListReplicaSets,
    ),
    (ObjectKind::Job, "jobs", AccessCheck::ListJobs),
    (ObjectKind::CronJob, "cronjobs", AccessCheck::ListCronJobs),
    (ObjectKind::Service, "services", AccessCheck::ListServices),
    (ObjectKind::Ingress, "ingresses", AccessCheck::ListIngresses),
    (
        ObjectKind::ConfigMap,
        "configmaps",
        AccessCheck::ListConfigMaps,
    ),
    (
        ObjectKind::NetworkPolicy,
        "networkpolicies",
        AccessCheck::ListNetworkPolicies,
    ),
    (
        ObjectKind::HorizontalPodAutoscaler,
        "horizontalpodautoscalers",
        AccessCheck::ListHorizontalPodAutoscalers,
    ),
    (
        ObjectKind::ResourceQuota,
        "resourcequotas",
        AccessCheck::ListResourceQuotas,
    ),
    (
        ObjectKind::PodDisruptionBudget,
        "poddisruptionbudgets",
        AccessCheck::ListPodDisruptionBudgets,
    ),
    (
        ObjectKind::PersistentVolumeClaim,
        "persistentvolumeclaims",
        AccessCheck::ListPersistentVolumeClaims,
    ),
    (
        ObjectKind::PersistentVolume,
        "persistentvolumes",
        AccessCheck::ListPersistentVolumes,
    ),
    (
        ObjectKind::StorageClass,
        "storageclasses",
        AccessCheck::ListStorageClasses,
    ),
    (
        ObjectKind::ServiceAccount,
        "serviceaccounts",
        AccessCheck::ListServiceAccounts,
    ),
    (ObjectKind::Secret, "secrets", AccessCheck::ListSecrets),
    (ObjectKind::Role, "roles", AccessCheck::ListRoles),
    (
        ObjectKind::ClusterRole,
        "clusterroles",
        AccessCheck::ListClusterRoles,
    ),
    (
        ObjectKind::RoleBinding,
        "rolebindings",
        AccessCheck::ListRoleBindings,
    ),
    (
        ObjectKind::ClusterRoleBinding,
        "clusterrolebindings",
        AccessCheck::ListClusterRoleBindings,
    ),
    (
        ObjectKind::CustomResourceDefinition,
        "customresourcedefinitions",
        AccessCheck::ListCustomResourceDefinitions,
    ),
];

/// One line per kind: the object count, `unknown`, or `denied` without a request when the
/// access review already said no. A missing report (the review failed) counts anyway.
async fn counts_for(
    probe: &mut Probe,
    connection: &ClusterConnection,
    scope: &NamespaceScope,
    access: Option<&AccessReport>,
) -> io::Result<()> {
    probe.section("counts")?;
    for (kind, plural, check) in COUNT_KINDS {
        if access.is_some_and(|report| !report.is_allowed(check)) {
            writeln!(probe.out, "  count {plural} denied")?;
            continue;
        }
        match connection.count_objects(kind, scope).await {
            Ok(Some(count)) => writeln!(probe.out, "  count {plural} {count}")?,
            Ok(None) => writeln!(probe.out, "  count {plural} unknown")?,
            Err(error) => {
                writeln!(probe.out, "  count {plural}")?;
                probe.fail(&error)?;
            }
        }
    }
    Ok(())
}

/// How long `--secrets` waits for the first Secrets snapshot.
const SECRETS_SNAPSHOT_TIMEOUT: Duration = Duration::from_secs(15);

/// `--secrets`: Secret counts by type, certificate parse counts, and the key count and byte
/// total of one `secret_values` call. Prints counts, dates, and `{namespace}/{name}` only:
/// never a value, a subject alternative name, or a registry host.
async fn secrets_for(
    probe: &mut Probe,
    connection: &ClusterConnection,
    scope: NamespaceScope,
) -> io::Result<()> {
    probe.section("secrets")?;
    let updates = connection.watch_secrets(scope);
    tokio::pin!(updates);
    let first = tokio::time::timeout(SECRETS_SNAPSHOT_TIMEOUT, updates.next()).await;
    let secrets = match first {
        Ok(Some(WatchUpdate::Snapshot(secrets))) => secrets,
        Ok(Some(WatchUpdate::Failed(error))) => return probe.fail(&error),
        Ok(None) | Err(_) => {
            probe.all_succeeded = false;
            return writeln!(
                probe.out,
                "  no secrets snapshot in {SECRETS_SNAPSHOT_TIMEOUT:?}"
            );
        }
    };
    writeln!(
        probe.out,
        "  secrets {}: {}",
        secrets.len(),
        type_counts(&secrets)
    )?;
    let tls: Vec<&SecretSummary> = secrets
        .iter()
        .filter(|secret| secret.secret_type == "kubernetes.io/tls")
        .collect();
    let leaves = tls.iter().filter_map(|secret| match &secret.details {
        SecretDetails::Certificate { chain } => Some((chain.first()?.not_after, *secret)),
        _ => None,
    });
    let parsed = leaves.clone().count();
    write!(
        probe.out,
        "  tls certificates {parsed}/{} parsed",
        tls.len()
    )?;
    if let Some((not_after, secret)) = leaves.min_by_key(|(not_after, _)| *not_after) {
        write!(
            probe.out,
            ", earliest leaf not-after {not_after} ({}/{})",
            secret.namespace, secret.name
        )?;
    }
    writeln!(probe.out)?;
    let Some(first_tls) = tls.first() else {
        return writeln!(probe.out, "  first tls secret none");
    };
    let target = format!("{}/{}", first_tls.namespace, first_tls.name);
    writeln!(probe.out, "  first tls secret {target}")?;
    match connection
        .secret_values(&first_tls.namespace, &first_tls.name)
        .await
    {
        Ok(values) => {
            let bytes: usize = values.iter().map(|value| value.size_bytes()).sum();
            writeln!(
                probe.out,
                "  secret values {target}: {} keys, {bytes} bytes",
                values.len()
            )
        }
        Err(error) => {
            writeln!(probe.out, "  secret values {target}")?;
            probe.fail(&error)
        }
    }
}

/// `{type} {count} · …`, by count then name.
fn type_counts(secrets: &[SecretSummary]) -> String {
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for secret in secrets {
        *counts.entry(secret.secret_type.as_str()).or_default() += 1;
    }
    let mut counts: Vec<_> = counts.into_iter().collect();
    counts.sort_by(|left, right| right.1.cmp(&left.1).then(left.0.cmp(right.0)));
    let parts: Vec<String> = counts
        .iter()
        .map(|(secret_type, count)| format!("{secret_type} {count}"))
        .collect();
    parts.join(" \u{b7} ")
}

/// How long `--helm` waits for each first snapshot.
const HELM_SNAPSHOT_TIMEOUT: Duration = Duration::from_secs(30);

/// `--helm`: release counts by status, then one revision read of the first release. Prints
/// counts, line counts, `{namespace}/{name}`, and revision numbers only: never a value, manifest
/// text, notes, or description text.
async fn helm_for(
    probe: &mut Probe,
    connection: &ClusterConnection,
    scope: NamespaceScope,
) -> io::Result<()> {
    probe.section("helm")?;
    let updates = connection.watch_helm_releases(scope);
    tokio::pin!(updates);
    let first = tokio::time::timeout(HELM_SNAPSHOT_TIMEOUT, updates.next()).await;
    let releases = match first {
        Ok(Some(WatchUpdate::Snapshot(releases))) => releases,
        Ok(Some(WatchUpdate::Failed(error))) => return probe.fail(&error),
        Ok(None) | Err(_) => {
            probe.all_succeeded = false;
            return writeln!(
                probe.out,
                "  no helm releases snapshot in {HELM_SNAPSHOT_TIMEOUT:?}"
            );
        }
    };
    let decoded = releases
        .iter()
        .filter(|release| release.chart.is_some())
        .count();
    writeln!(
        probe.out,
        "  helm releases {}: {}; payload decoded {decoded}/{}",
        releases.len(),
        status_counts(&releases),
        releases.len()
    )?;
    let Some(release) = releases.first() else {
        return writeln!(probe.out, "  first helm release none");
    };
    let target = format!("{}/{}", release.namespace, release.name);
    writeln!(
        probe.out,
        "  first helm release {target} rev {}",
        release.revision
    )?;
    let history = connection.watch_helm_history(&release.namespace, &release.name);
    tokio::pin!(history);
    let revisions = match tokio::time::timeout(HELM_SNAPSHOT_TIMEOUT, history.next()).await {
        Ok(Some(WatchUpdate::Snapshot(revisions))) => revisions,
        Ok(Some(WatchUpdate::Failed(error))) => return probe.fail(&error),
        Ok(None) | Err(_) => {
            probe.all_succeeded = false;
            return writeln!(probe.out, "  no helm history snapshot for {target}");
        }
    };
    writeln!(
        probe.out,
        "  helm history {target}: {} revisions",
        revisions.len()
    )?;
    let revision = HelmRevisionRef {
        namespace: release.namespace.clone(),
        release: release.name.clone(),
        revision: release.revision,
    };
    let description_chars = release
        .description
        .as_ref()
        .map_or(0, |text| text.chars().count());
    match connection
        .helm_release_detail(&revision, EnvValues::Hidden)
        .await
    {
        Ok(detail) => {
            let manifest = detail.manifest.as_str();
            let documents = match manifest.is_empty() {
                true => 0,
                false => manifest.lines().filter(|line| *line == "---").count() + 1,
            };
            writeln!(
                probe.out,
                "  helm detail {target} rev {}: values {} lines ({} hidden), computed {} lines, manifest {documents} documents {} lines ({} env values hidden), notes {} lines, description {description_chars} chars",
                release.revision,
                detail.user_values.as_str().lines().count(),
                detail.hidden_user_values,
                detail.computed_values.as_str().lines().count(),
                manifest.lines().count(),
                detail.hidden_env_values,
                detail.notes_lines,
            )?;
        }
        Err(error) => {
            writeln!(probe.out, "  helm detail {target} rev {}", release.revision)?;
            probe.fail(&error)?;
        }
    }
    let earlier = revisions
        .iter()
        .map(|revision| revision.revision)
        .find(|number| *number < release.revision);
    let Some(earlier) = earlier else {
        return Ok(());
    };
    let before = HelmRevisionRef {
        revision: earlier,
        ..revision.clone()
    };
    match connection
        .helm_values_diff(&before, &revision, ValueVisibility::Masked)
        .await
    {
        Ok(diff) => writeln!(
            probe.out,
            "  helm diff {target} rev {earlier} \u{2192} {}: user {} changes, computed {} changes",
            release.revision,
            diff.user.len() + diff.omitted_user,
            diff.computed.len() + diff.omitted_computed
        ),
        Err(error) => {
            writeln!(probe.out, "  helm diff {target} rev {earlier}")?;
            probe.fail(&error)
        }
    }
}

/// `{status} {count} · …`, by count then name.
fn status_counts(releases: &[HelmReleaseSummary]) -> String {
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for release in releases {
        *counts.entry(release.status.label()).or_default() += 1;
    }
    let mut counts: Vec<_> = counts.into_iter().collect();
    counts.sort_by(|left, right| right.1.cmp(&left.1).then(left.0.cmp(right.0)));
    let parts: Vec<String> = counts
        .iter()
        .map(|(status, count)| format!("{status} {count}"))
        .collect();
    if parts.is_empty() {
        return NONE_TEXT.to_owned();
    }
    parts.join(" \u{b7} ")
}

/// How long `--crds` waits for the first CRD or custom object snapshot.
const CRDS_SNAPSHOT_TIMEOUT: Duration = Duration::from_secs(30);
/// Most CRDs `--crds` prints a line for.
const MAX_LISTED_CRDS: usize = 10;

/// The first snapshot of `updates`, or `None` after the failure was recorded.
async fn first_snapshot<T>(
    probe: &mut Probe,
    what: &str,
    updates: impl Stream<Item = WatchUpdate<T>>,
) -> io::Result<Option<Vec<T>>> {
    tokio::pin!(updates);
    match tokio::time::timeout(CRDS_SNAPSHOT_TIMEOUT, updates.next()).await {
        Ok(Some(WatchUpdate::Snapshot(items))) => Ok(Some(items)),
        Ok(Some(WatchUpdate::Failed(error))) => {
            probe.fail(&error)?;
            Ok(None)
        }
        Ok(None) | Err(_) => {
            probe.all_succeeded = false;
            writeln!(
                probe.out,
                "  no {what} snapshot in {CRDS_SNAPSHOT_TIMEOUT:?}"
            )?;
            Ok(None)
        }
    }
}

/// `--crds`: CRD counts, printer-column support per CRD, then access, count, object watch, and
/// YAML lines for the established CRDs. Prints counts and CRD or column names only: never an
/// object name, a field, or a value.
async fn crds_for(
    probe: &mut Probe,
    connection: &ClusterConnection,
    scope: NamespaceScope,
    args: &Args,
) -> io::Result<()> {
    probe.section("crds")?;
    let Some(crds) = first_snapshot(probe, "crds", connection.watch_crds()).await? else {
        return Ok(());
    };
    let established: Vec<&CrdSummary> = crds
        .iter()
        .filter(|crd| crd.state == CrdState::Established && crd.preferred_version().is_some())
        .collect();
    writeln!(
        probe.out,
        "  crds: {} ({} established)",
        crds.len(),
        established.len()
    )?;
    let (mut total_columns, mut total_unsupported) = (0, 0);
    for (index, crd) in established.iter().enumerate() {
        let Some(version) = crd.preferred_version() else {
            continue;
        };
        let unsupported: Vec<&str> = version
            .printer_columns
            .iter()
            .filter(|column| !column.is_supported)
            .map(|column| column.name.as_str())
            .collect();
        total_columns += version.printer_columns.len();
        total_unsupported += unsupported.len();
        if index < MAX_LISTED_CRDS {
            writeln!(
                probe.out,
                "  crd {} {} {:?} columns {} unsupported {}",
                crd.name,
                version.name,
                crd.scope,
                version.printer_columns.len(),
                unsupported.len()
            )?;
        }
        for name in unsupported {
            writeln!(probe.out, "  unsupported {}: {name}", crd.name)?;
        }
    }
    writeln!(
        probe.out,
        "  printer columns: {total_columns} in total, {total_unsupported} unsupported"
    )?;
    let mut target = None;
    let mut target_has_instances = false;
    for crd in established.iter().take(MAX_LISTED_CRDS) {
        let Some(version) = crd.preferred_version() else {
            continue;
        };
        let resource = crd.resource(version);
        let label = format!("{}.{}", resource.plural, resource.group);
        match connection.review_custom_access(&resource, &scope).await {
            Ok(AccessDecision::Allowed) => writeln!(probe.out, "  access list {label}: allowed")?,
            Ok(AccessDecision::Denied { .. }) => {
                writeln!(probe.out, "  access list {label}: denied")?;
                continue;
            }
            Err(error) => {
                writeln!(probe.out, "  access list {label}")?;
                probe.fail(&error)?;
                continue;
            }
        }
        let count = match connection.count_custom_objects(&resource).await {
            Ok(Some(count)) => {
                writeln!(probe.out, "  count {label} {count}")?;
                Some(count)
            }
            Ok(None) => {
                writeln!(probe.out, "  count {label} unknown")?;
                None
            }
            Err(error) => {
                writeln!(probe.out, "  count {label}")?;
                probe.fail(&error)?;
                None
            }
        };
        // The first CRD with instances makes the better watch and YAML target.
        let has_instances = count.is_some_and(|count| count > 0);
        if target.is_none() || (has_instances && !target_has_instances) {
            target = Some((crd, version, resource));
            target_has_instances = has_instances;
        }
    }
    let Some((crd, version, resource)) = target else {
        return Ok(());
    };
    if args.watch_seconds.is_none() && !args.yaml {
        return Ok(());
    }
    let label = format!("{}.{}", resource.plural, resource.group);
    let updates =
        connection.watch_custom_objects(&resource, &version.printer_columns, scope.clone());
    let Some(objects) = first_snapshot(probe, &label, updates).await? else {
        return Ok(());
    };
    if args.watch_seconds.is_some() {
        let total = objects.len() * version.printer_columns.len();
        let filled = objects
            .iter()
            .flat_map(|object| &object.columns)
            .filter(|value| !matches!(value, ColumnValue::Absent))
            .count();
        writeln!(
            probe.out,
            "  custom {label}: {} objects, {filled} of {total} column values filled",
            objects.len()
        )?;
    }
    if args.yaml {
        let first = objects.first().and_then(|object| {
            ObjectRef::custom(
                resource.clone(),
                object.namespace.clone(),
                object.name.clone(),
            )
        });
        let Some(object) = first else {
            probe.all_succeeded = false;
            return writeln!(probe.out, "  yaml custom {}: no object", crd.name);
        };
        match connection.object_yaml(&object, EnvValues::Hidden).await {
            Ok(yaml) => writeln!(
                probe.out,
                "  yaml custom {}: {} lines, masked {}",
                crd.name,
                yaml.text.lines().count(),
                if yaml.text.starts_with("# k8sBoard hid") {
                    "yes"
                } else {
                    "no"
                }
            )?,
            Err(error) => {
                writeln!(probe.out, "  yaml custom {}", crd.name)?;
                probe.fail(&error)?;
            }
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
        tail_lines: 1000,
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

/// One update of either metrics poll.
enum MetricsUpdate {
    Pods(WatchUpdate<PodMetrics>),
    Nodes(WatchUpdate<NodeMetrics>),
}

/// Listed pods with no running container: metrics-server has no sample for them.
fn pods_not_running(pods: &[PodSummary]) -> usize {
    pods.iter()
        .filter(|pod| {
            !pod.containers
                .iter()
                .any(|container| matches!(container.state, ContainerState::Running { .. }))
        })
        .count()
}

/// Seconds since the newest server scrape, or `unknown` without a readable timestamp.
fn newest_scrape_age(samples: impl Iterator<Item = Option<jiff::Timestamp>>) -> String {
    match samples.flatten().max() {
        Some(newest) => format!(
            "{}s ago",
            (jiff::Timestamp::now().as_second() - newest.as_second()).max(0)
        ),
        None => "unknown".to_owned(),
    }
}

fn usage_sums(nanocores: u128, bytes: u128) -> String {
    format!(
        "cpu {:.3} cores, memory {:.1} GiB",
        nanocores as f64 / 1e9,
        bytes as f64 / f64::from(1u32 << 30)
    )
}

fn pod_metrics_line(samples: &[PodMetrics], listed: &[PodSummary]) -> String {
    let usages = samples
        .iter()
        .flat_map(|pod| &pod.containers)
        .map(|container| container.usage);
    let containers = usages.clone().count();
    let nanocores: u128 = usages
        .clone()
        .map(|usage| u128::from(usage.cpu.nanocores()))
        .sum();
    let bytes: u128 = usages.map(|usage| u128::from(usage.memory.bytes())).sum();
    format!(
        "pod metrics: {} pods (listed {}, not running {}), {containers} containers, {}, newest scrape {}",
        samples.len(),
        listed.len(),
        pods_not_running(listed),
        usage_sums(nanocores, bytes),
        newest_scrape_age(samples.iter().map(|pod| pod.sampled_at)),
    )
}

fn node_metrics_line(samples: &[NodeMetrics]) -> String {
    let nanocores: u128 = samples
        .iter()
        .map(|node| u128::from(node.usage.cpu.nanocores()))
        .sum();
    let bytes: u128 = samples
        .iter()
        .map(|node| u128::from(node.usage.memory.bytes()))
        .sum();
    format!(
        "node metrics: {} nodes, {}, newest scrape {}",
        samples.len(),
        usage_sums(nanocores, bytes),
        newest_scrape_age(samples.iter().map(|node| node.sampled_at)),
    )
}

/// Polls pod and node metrics for `seconds` and prints one line per update: counts and sums
/// only, never names.
async fn metrics_for(
    probe: &mut Probe,
    connection: &ClusterConnection,
    scope: NamespaceScope,
    listed: &[PodSummary],
    seconds: u64,
) -> io::Result<()> {
    probe.section(&format!("metrics ({seconds}s)"))?;
    let mut updates = stream::select(
        connection
            .poll_pod_metrics(scope)
            .map(MetricsUpdate::Pods)
            .boxed(),
        connection
            .poll_node_metrics()
            .map(MetricsUpdate::Nodes)
            .boxed(),
    );
    let (mut pod_polls, mut node_polls) = (0, 0);
    let timer = tokio::time::sleep(Duration::from_secs(seconds));
    tokio::pin!(timer);
    loop {
        let update = tokio::select! {
            () = &mut timer => break,
            update = updates.next() => update,
        };
        let line = match update {
            Some(MetricsUpdate::Pods(WatchUpdate::Snapshot(pods))) => {
                pod_polls += 1;
                pod_metrics_line(&pods, listed)
            }
            Some(MetricsUpdate::Nodes(WatchUpdate::Snapshot(nodes))) => {
                node_polls += 1;
                node_metrics_line(&nodes)
            }
            Some(MetricsUpdate::Pods(WatchUpdate::Failed(error))) => {
                probe.all_succeeded = false;
                format!("pod metrics failed: {}", error_summary(&error))
            }
            Some(MetricsUpdate::Nodes(WatchUpdate::Failed(error))) => {
                probe.all_succeeded = false;
                format!("node metrics failed: {}", error_summary(&error))
            }
            None => break,
        };
        writeln!(probe.out, "  {line}")?;
    }
    if pod_polls == 0 || node_polls == 0 {
        probe.all_succeeded = false;
    }
    Ok(())
}

/// Decimal gigabytes, for a network counter.
fn gigabytes(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / 1e9)
}

/// One node of one kubelet round, as lines of counts only: no pod, claim, or image names.
fn kubelet_node_lines(stats: &NodeKubeletStats) -> Vec<String> {
    let node = &stats.node;
    let mut lines = vec![match &stats.summary {
        Ok(summary) => {
            let pvcs: usize = summary.pods.iter().map(|pod| pod.volumes.len()).sum();
            let network = summary.network.map_or_else(
                || "none".to_owned(),
                |counters| {
                    format!(
                        "rx {} tx {}",
                        gigabytes(counters.rx_bytes),
                        gigabytes(counters.tx_bytes)
                    )
                },
            );
            format!(
                "kubelet {node}: {} pods, {pvcs} PVCs, network {network}",
                summary.pods.len()
            )
        }
        Err(error) => format!("kubelet {node} failed: {}", error_summary(error)),
    }];
    match &stats.disk_io {
        Some(Ok(sample)) => {
            let with_reads = sample
                .containers
                .iter()
                .filter(|container| container.counters.read_bytes > 0)
                .count();
            let with_writes = sample
                .containers
                .iter()
                .filter(|container| container.counters.write_bytes > 0)
                .count();
            lines.push(format!(
                "disk io: {} containers ({with_reads} with reads, {with_writes} with writes), node root series {}",
                sample.containers.len(),
                if sample.node.is_some() { "yes" } else { "no" },
            ));
        }
        Some(Err(error)) => lines.push(format!(
            "kubelet {node} disk io failed: {}",
            error_summary(error)
        )),
        None => {}
    }
    lines
}

/// Polls the kubelet stats of every Ready node for `seconds` (cAdvisor disk I/O for the
/// first one only) and prints counts per node per round.
async fn kubelet_for(
    probe: &mut Probe,
    connection: &ClusterConnection,
    ready_nodes: &[String],
    seconds: u64,
) -> io::Result<()> {
    probe.section(&format!("kubelet stats ({seconds}s)"))?;
    let Some(disk_node) = ready_nodes.first() else {
        probe.all_succeeded = false;
        return writeln!(probe.out, "  no Ready node");
    };
    let (sender, receiver) = watch::channel(KubeletTargets {
        summary_nodes: ready_nodes.to_vec(),
        disk_io_nodes: vec![disk_node.clone()],
    });
    let mut updates = Box::pin(connection.poll_kubelet_stats(receiver));
    let mut rounds = 0;
    let timer = tokio::time::sleep(Duration::from_secs(seconds));
    tokio::pin!(timer);
    loop {
        let update = tokio::select! {
            () = &mut timer => break,
            update = updates.next() => update,
        };
        match update {
            Some(WatchUpdate::Snapshot(nodes)) => {
                rounds += 1;
                writeln!(probe.out, "  round {rounds}")?;
                for line in nodes.iter().flat_map(kubelet_node_lines) {
                    writeln!(probe.out, "    {line}")?;
                }
            }
            Some(WatchUpdate::Failed(error)) => {
                probe.all_succeeded = false;
                writeln!(probe.out, "  round failed: {}", error_summary(&error))?;
            }
            None => break,
        }
    }
    // The sender stays alive until here: dropping it would end the poll.
    drop(sender);
    if rounds < 2 {
        probe.all_succeeded = false;
    }
    Ok(())
}

/// Counts only, never names: the RBAC snapshot, the caller's own rules review, and the
/// Who-can grant count for `get secrets`.
async fn analysis_for(
    probe: &mut Probe,
    connection: &ClusterConnection,
    namespace: &str,
) -> io::Result<()> {
    probe.section("analysis")?;
    let fallback: Vec<String> = match connection.list_namespaces().await {
        Ok(namespaces) => namespaces.into_iter().map(|item| item.name).collect(),
        Err(error) => {
            probe.fail(&error)?;
            Vec::new()
        }
    };
    match connection.read_rbac(&fallback).await {
        Ok(snapshot) => {
            let coverage = &snapshot.coverage;
            let listed = |is_listed: bool| if is_listed { "listed" } else { "denied" };
            let scope = |coverage: &NamespaceCoverage| match coverage {
                NamespaceCoverage::AllNamespaces => "all".to_owned(),
                NamespaceCoverage::Namespaces(namespaces) => {
                    format!("{} namespaces", namespaces.len())
                }
            };
            writeln!(
                probe.out,
                "  rbac: cluster roles {} · cluster bindings {} · roles {} · role bindings {} · roles {} · cluster roles {} · role bindings {} · cluster role bindings {}",
                listed(coverage.cluster_roles),
                listed(coverage.cluster_bindings),
                scope(&coverage.roles),
                scope(&coverage.role_bindings),
                snapshot.roles.len(),
                snapshot.cluster_roles.len(),
                snapshot.role_bindings.len(),
                snapshot.cluster_role_bindings.len(),
            )?;
            let request = AccessRequest {
                verb: "get".to_owned(),
                target: RequestTarget::Resource(ResourceRequest {
                    group: String::new(),
                    resource: "secrets".to_owned(),
                    subresource: None,
                    name: None,
                    namespace: Some(namespace.to_owned()),
                }),
            };
            let grants = snapshot.who_can(&request);
            let only_named = grants
                .iter()
                .filter(|grant| matches!(grant.names, GrantNames::Only(_)))
                .count();
            writeln!(
                probe.out,
                "  who can get secrets in {namespace}: {} grants ({only_named} only named)",
                grants.len(),
            )?;
        }
        Err(error) => probe.fail(&error)?,
    }
    match connection.review_rules(namespace).await {
        Ok(review) => {
            let incomplete = if review.is_incomplete {
                ", incomplete"
            } else {
                ""
            };
            writeln!(
                probe.out,
                "  rules review {namespace}: {} rules{incomplete}",
                review.rules.len(),
            )?;
        }
        Err(error) => probe.fail(&error)?,
    }
    match connection.read_network_policies(namespace).await {
        Ok(policies) => writeln!(
            probe.out,
            "  network policies {namespace}: {}",
            policies.len(),
        ),
        Err(error) => probe.fail(&error),
    }
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
        Some(namespaces) => NamespaceScope::of_namespaces(
            namespaces
                .split(',')
                .filter(|name| !name.is_empty())
                .map(str::to_owned),
        ),
        None => NamespaceScope::All,
    };
    let scope_label = match scope.namespaces() {
        [] => "all namespaces".to_owned(),
        names => names.join(","),
    };

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
    let access = match connection.review_access(scope.clone()).await {
        Ok(report) => Some(report),
        Err(error) => {
            probe.fail(&error)?;
            None
        }
    };
    for review in access.iter().flat_map(|report| &report.reviews) {
        match &review.decision {
            AccessDecision::Allowed => writeln!(probe.out, "  {:<26} allowed", review.check)?,
            AccessDecision::Denied {
                reason: Some(reason),
            } => writeln!(probe.out, "  {:<26} denied: {reason}", review.check)?,
            AccessDecision::Denied { reason: None } => {
                writeln!(probe.out, "  {:<26} denied", review.check)?;
            }
        }
    }
    if args.analysis {
        let namespace = scope
            .namespaces()
            .first()
            .map_or(connection.default_namespace(), String::as_str);
        analysis_for(&mut probe, &connection, namespace).await?;
    }
    if args.counts {
        counts_for(&mut probe, &connection, &scope, access.as_ref()).await?;
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
    let mut ready_nodes = Vec::new();
    match connection.list_nodes().await {
        Ok(nodes) => {
            first_node = nodes.first().map(|node| node.name.clone());
            ready_nodes = nodes
                .iter()
                .filter(|node| node.status.readiness == NodeReadiness::Ready)
                .map(|node| node.name.clone())
                .collect();
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
            probe.print_node_field_counts(&nodes)?;
        }
        Err(error) => {
            probe.section("nodes")?;
            probe.fail(&error)?;
        }
    }

    let pods = connection.list_pods(scope.clone()).await;
    match &pods {
        Ok(pods) => {
            probe.print_pods(&scope_label, pods)?;
            probe.print_container_spec_counts(pods)?;
        }
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

    if args.secrets {
        secrets_for(&mut probe, &connection, scope.clone()).await?;
    }
    if args.helm {
        helm_for(&mut probe, &connection, scope.clone()).await?;
    }
    if args.crds {
        crds_for(&mut probe, &connection, scope.clone(), args).await?;
    }
    if let Some(seconds) = args.watch_seconds {
        watch_for(&mut probe, &connection, scope.clone(), seconds).await?;
    }
    if let Some(seconds) = args.metrics_seconds {
        let pods = pods.as_ref().map_or(&[][..], Vec::as_slice);
        metrics_for(&mut probe, &connection, scope, pods, seconds).await?;
    }
    if let Some(seconds) = args.kubelet_seconds {
        kubelet_for(&mut probe, &connection, &ready_nodes, seconds).await?;
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

    /// Counts only: how many containers carry each new spec field. Never prints a value.
    fn print_container_spec_counts(&mut self, pods: &[PodSummary]) -> io::Result<()> {
        let containers: Vec<&ContainerSummary> =
            pods.iter().flat_map(|pod| &pod.containers).collect();
        let count =
            |has: fn(&ContainerSummary) -> bool| containers.iter().filter(|c| has(c)).count();
        self.section("container spec fields (counts)")?;
        writeln!(
            self.out,
            "  containers {}  liveness {}  readiness {}  startup {}  env {}  envFrom {}  mounts {}  ports {}  resources {}  digest {}  started {}",
            containers.len(),
            count(|c| c.probes.liveness.is_some()),
            count(|c| c.probes.readiness.is_some()),
            count(|c| c.probes.startup.is_some()),
            count(|c| !c.env.is_empty()),
            count(|c| !c.env_from.is_empty()),
            count(|c| !c.mounts.is_empty()),
            count(|c| !c.ports.is_empty()),
            count(|c| !c.resources.is_empty()),
            count(|c| c.image_digest.is_some()),
            count(|c| c.is_started.is_some()),
        )?;
        let env_entries: usize = containers.iter().map(|c| c.env.len()).sum();
        let mount_entries: usize = containers.iter().map(|c| c.mounts.len()).sum();
        writeln!(
            self.out,
            "  env entries {env_entries}  mount entries {mount_entries}"
        )?;
        let with_condition_text = pods
            .iter()
            .flat_map(|pod| &pod.conditions)
            .filter(|condition| condition.reason.is_some() || condition.message.is_some())
            .count();
        let with_status_message = pods
            .iter()
            .filter(|pod| pod.status_message.is_some())
            .count();
        let with_labels = pods.iter().filter(|pod| !pod.labels.is_empty()).count();
        writeln!(
            self.out,
            "  pod conditions with reason or message {with_condition_text}  pods with status message {with_status_message}  pods with labels {with_labels}"
        )
    }

    /// Counts only: node conditions by name and status, plus the other new node fields.
    fn print_node_field_counts(&mut self, nodes: &[NodeSummary]) -> io::Result<()> {
        let mut conditions: BTreeMap<String, usize> = BTreeMap::new();
        for condition in nodes.iter().flat_map(|node| &node.conditions) {
            let key = format!("{}={:?}", condition.name, condition.status);
            *conditions.entry(key).or_default() += 1;
        }
        self.section("node fields (counts)")?;
        let listed: Vec<String> = conditions
            .iter()
            .map(|(key, count)| format!("{key} x{count}"))
            .collect();
        writeln!(self.out, "  conditions {}", joined_or_none(&listed))?;
        writeln!(
            self.out,
            "  addresses {}  resources {}  labels {}",
            nodes.iter().map(|node| node.addresses.len()).sum::<usize>(),
            nodes.iter().map(|node| node.resources.len()).sum::<usize>(),
            nodes.iter().map(|node| node.labels.len()).sum::<usize>(),
        )?;
        let (cpu, memory, pods) = allocatable_totals(nodes);
        writeln!(
            self.out,
            "  allocatable totals: cpu {cpu:.1} cores  memory {:.2} GiB  pods {pods}",
            memory as f64 / (1u64 << 30) as f64,
        )
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
        ContainerState::Waiting { reason, .. } => {
            format!("waiting({})", reason_text(reason.as_ref()))
        }
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
        ClusterError::Namespace { .. } => "Namespace: ",
        ClusterError::Rendered { .. } => "Rendered: ",
    }
}

/// The `cpu`, `memory`, and `pods` allocatable summed over `nodes`: cores, bytes, and pod slots.
fn allocatable_totals(nodes: &[NodeSummary]) -> (f64, u64, u64) {
    let mut totals = (0., 0, 0);
    for resource in nodes.iter().flat_map(|node| &node.resources) {
        let Some(text) = resource.allocatable.as_deref() else {
            continue;
        };
        match resource.name.as_str() {
            "cpu" => totals.0 += cluster::CpuAmount::parse(text).map_or(0., |cpu| cpu.cores()),
            "memory" => {
                totals.1 += cluster::ByteAmount::parse(text).map_or(0, |bytes| bytes.bytes())
            }
            "pods" => totals.2 += text.parse::<u64>().unwrap_or(0),
            _ => {}
        }
    }
    totals
}
