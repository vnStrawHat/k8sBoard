//! Which pods of a workload a log tab follows, as pure functions over the pods list.

use std::collections::HashMap;

use cluster::{ContainerKind, ContainerSummary, LogLine, NamespaceScope, PodSummary};

use crate::kind_row::{PodOwner, STATEFUL_SET_KIND, deployment_of_pod, owns_pod};
use crate::screenshot::controller_owner_of;

const MAX_WORKLOAD_PODS: usize = 10;
pub(crate) const MAX_WORKLOAD_STREAMS: usize = 20;

/// Pods of `owner`: ready (ready == total > 0) first, then newest `created_at` (`None` last),
/// then name.
pub(crate) fn ranked_pods<'a>(owner: &PodOwner, pods: &'a [PodSummary]) -> Vec<&'a PodSummary> {
    let mut ranked: Vec<&PodSummary> = pods.iter().filter(|pod| owns_pod(owner, pod)).collect();
    ranked.sort_by(|a, b| {
        is_ready(b)
            .cmp(&is_ready(a))
            .then_with(|| b.created_at.cmp(&a.created_at))
            .then_with(|| a.name.cmp(&b.name))
    });
    ranked
}

fn is_ready(pod: &PodSummary) -> bool {
    pod.ready.total > 0 && pod.ready.ready == pod.ready.total
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct MemberChange {
    pub(crate) joined: Vec<String>,
    pub(crate) left: Vec<String>,
}

/// Sticky: listed members stay, even below the pod limit; up to `slots` best-ranked newcomers
/// join. A member absent from `ranked` left.
pub(crate) fn member_change(
    current: &[String],
    ranked: &[&PodSummary],
    slots: usize,
) -> MemberChange {
    let is_listed = |name: &String| ranked.iter().any(|pod| pod.name == *name);
    let left = current
        .iter()
        .filter(|name| !is_listed(name))
        .cloned()
        .collect();
    let joined = ranked
        .iter()
        .filter(|pod| !current.contains(&pod.name))
        .take(slots)
        .map(|pod| pod.name.clone())
        .collect();
    MemberChange { joined, left }
}

/// Newcomers that fit: `min(pod_limit - members, (MAX_WORKLOAD_STREAMS - live_streams) /
/// streams_per_pod)`. `live_streams` counts every live stream, member or leaver in grace.
pub(crate) fn join_slots(members: usize, live_streams: usize, streams_per_pod: usize) -> usize {
    let free_pods = pod_limit(streams_per_pod).saturating_sub(members);
    let free_streams = MAX_WORKLOAD_STREAMS.saturating_sub(live_streams) / streams_per_pod.max(1);
    free_pods.min(free_streams)
}

/// `clamp(20 / n, 1, 10)` for `n` selected containers; no selection yet allows 10.
pub(crate) fn pod_limit(selected_containers: usize) -> usize {
    if selected_containers == 0 {
        return MAX_WORKLOAD_PODS;
    }
    (MAX_WORKLOAD_STREAMS / selected_containers).clamp(1, MAX_WORKLOAD_PODS)
}

/// The last `-` segment of the pod name (`api-7d9f8c-x2k4q` is `x2k4q`). A StatefulSet pod keeps
/// its whole name, because the ordinal is what tells replicas apart.
pub(crate) fn pod_short_name<'a>(owner: &PodOwner, pod: &'a str) -> &'a str {
    if matches!(owner, PodOwner::Controller { kind, .. } if *kind == STATEFUL_SET_KIND) {
        return pod;
    }
    pod.rsplit('-').next().unwrap_or(pod)
}

/// `{workload}-{suffix}`, how a shell or log tab names a pod: the workload and the part of the pod
/// name that tells its replicas apart (`api-m8n2p` for pod `api-7d9f8c-m8n2p` of Deployment `api`).
/// A StatefulSet pod and a pod no controller owns keep their whole name.
pub(crate) fn pod_tab_name(pod: &PodSummary) -> String {
    let Some(owner) = controller_owner_of(pod) else {
        return pod.name.clone();
    };
    let suffix = pod_short_name(&owner, &pod.name);
    if suffix == pod.name {
        return pod.name.clone();
    }
    let workload = deployment_of_pod(pod).or_else(|| {
        pod.controller
            .as_ref()
            .map(|controller| controller.name.as_str())
    });
    match workload {
        Some(workload) => format!("{workload}-{suffix}"),
        None => pod.name.clone(),
    }
}

/// Characters that fit the 9 rem prefix column of a log row (mono `text_xs`, 0.45 rem each).
const PREFIX_COLUMN_CHARS: usize = 20;

/// The `{pod}/{container}` cell of a workload log row. A StatefulSet pod drops the owner name and
/// keeps the ordinal (`-0`); other owners keep the last `-` segment. A pod that has no suffix to
/// keep shows its whole name cut from the start, so the end of the name stays readable.
pub(crate) fn pod_origin_label(owner: &PodOwner, pod: &str, container: &str) -> String {
    let suffix = match owner {
        PodOwner::Controller { kind, name, .. } if *kind == STATEFUL_SET_KIND => pod
            .strip_prefix(name.as_str())
            .filter(|rest| !rest.is_empty()),
        _ => Some(pod_short_name(owner, pod)).filter(|short| *short != pod),
    };
    match suffix {
        Some(suffix) => format!("{suffix}/{container}"),
        None => cut_from_start(&format!("{pod}/{container}"), PREFIX_COLUMN_CHARS),
    }
}

/// `text` as its last `max_chars` characters behind a `…`; unchanged when it already fits.
fn cut_from_start(text: &str, max_chars: usize) -> String {
    let count = text.chars().count();
    if count <= max_chars {
        return text.to_owned();
    }
    let tail: String = text.chars().skip(count - (max_chars - 1)).collect();
    format!("…{tail}")
}

pub(crate) fn scope_covers(scope: &NamespaceScope, namespace: &str) -> bool {
    match scope {
        NamespaceScope::All => true,
        NamespaceScope::Named(name) => name == namespace,
        NamespaceScope::Several(names) => names.iter().any(|name| name == namespace),
    }
}

/// Container names over `members`, in first-seen order with init containers last.
pub(crate) fn container_names(members: &[&PodSummary]) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    let mut init_names: Vec<String> = Vec::new();
    for container in members.iter().flat_map(|pod| &pod.containers) {
        let group = if container.kind == ContainerKind::Init {
            &mut init_names
        } else {
            &mut names
        };
        if !group.contains(&container.name) {
            group.push(container.name.clone());
        }
    }
    init_names.retain(|name| !names.contains(name));
    names.extend(init_names);
    names
}

/// The `SYS` line for a container whose restart count rose, timed by the last termination when
/// the pod reports one, else `now`. Only the reason, exit code, and count are read: no message.
pub(crate) fn restart_marker(container: &ContainerSummary, now: jiff::Timestamp) -> LogLine {
    let name = &container.name;
    let count = container.restart_count;
    let (text, time) = match &container.last_termination {
        Some(termination) => {
            let cause = match &termination.reason {
                Some(reason) => format!("terminated: {reason} (exit {})", termination.exit_code),
                None => format!("terminated (exit {})", termination.exit_code),
            };
            (
                format!("── container {name} {cause} · restart #{count} ──"),
                termination.finished_at.unwrap_or(now),
            )
        }
        None => (
            format!("── container {name} restarted · restart #{count} ──"),
            now,
        ),
    };
    LogLine {
        timestamp: Some(time),
        text,
    }
}

/// The last restart count seen per (namespace, pod, container): pods of one name live in several
/// namespaces.
pub(crate) type RestartBaselines = HashMap<(String, String, String), u32>;

/// Containers of `pod` among `streamed` whose restart count rose since `seen`, with the new
/// count; `seen` is updated. A first sight only records, and a lower count (the pod was
/// recreated under the same name) only re-baselines.
pub(crate) fn rising_restarts(
    seen: &mut RestartBaselines,
    pod: &PodSummary,
    streamed: &[&str],
) -> Vec<(String, u32)> {
    let mut rises = Vec::new();
    for container in &pod.containers {
        if !streamed.contains(&container.name.as_str()) {
            continue;
        }
        let key = (
            pod.namespace.clone(),
            pod.name.clone(),
            container.name.clone(),
        );
        let before = seen.insert(key, container.restart_count);
        if before.is_some_and(|before| container.restart_count > before) {
            rises.push((container.name.clone(), container.restart_count));
        }
    }
    rises
}

#[cfg(test)]
mod tests {
    use cluster::{ControllerRef, PodStatus, ReadyCount, StatusReason};

    use super::*;
    use crate::kind_row::{DAEMON_SET_KIND, JOB_KIND, REPLICA_SET_KIND};

    fn pod(name: &str, ready: (u32, u32), created: Option<&str>) -> PodSummary {
        PodSummary {
            is_finished: false,
            namespace: "ns".to_owned(),
            name: name.to_owned(),
            status: PodStatus::Reason(StatusReason::Running),
            ready: ReadyCount {
                ready: ready.0,
                total: ready.1,
            },
            restarts: 0,
            node_name: None,
            created_at: created.map(|time| time.parse().expect("valid time")),
            pod_ip: None,
            qos_class: None,
            service_account: None,
            controller: Some(ControllerRef {
                kind: JOB_KIND.to_owned(),
                name: "batch".to_owned(),
            }),
            conditions: Vec::new(),
            status_message: None,
            labels: Vec::new(),
            host_network: false,
            image_pull_secrets: Vec::new(),
            containers: Vec::new(),
        }
    }

    fn job() -> PodOwner {
        PodOwner::Controller {
            namespace: "ns".to_owned(),
            kind: JOB_KIND,
            name: "batch".to_owned(),
        }
    }

    fn names(pods: &[&PodSummary]) -> Vec<String> {
        pods.iter().map(|pod| pod.name.clone()).collect()
    }

    fn owned_pod(name: &str, kind: &str, owner: &str, labels: &[&str]) -> PodSummary {
        let mut pod = pod(name, (1, 1), None);
        pod.controller = Some(ControllerRef {
            kind: kind.to_owned(),
            name: owner.to_owned(),
        });
        pod.labels = labels.iter().map(|label| (*label).to_owned()).collect();
        pod
    }

    #[test]
    fn tab_name_of_a_deployment_pod_is_the_deployment_and_the_suffix() {
        let pod = owned_pod(
            "api-7d9f8c-m8n2p",
            REPLICA_SET_KIND,
            "api-7d9f8c",
            &["pod-template-hash=7d9f8c"],
        );
        assert_eq!(pod_tab_name(&pod), "api-m8n2p");
    }

    #[test]
    fn tab_name_of_a_standalone_replica_set_pod_keeps_the_replica_set() {
        let pod = owned_pod("web-x2k4q", REPLICA_SET_KIND, "web", &[]);
        assert_eq!(pod_tab_name(&pod), "web-x2k4q");
    }

    #[test]
    fn tab_name_of_a_stateful_set_pod_keeps_its_ordinal() {
        let pod = owned_pod("postgres-0", STATEFUL_SET_KIND, "postgres", &[]);
        assert_eq!(pod_tab_name(&pod), "postgres-0");
    }

    #[test]
    fn tab_name_of_a_daemon_set_or_job_pod_is_the_workload_and_the_suffix() {
        let daemon = owned_pod("node-exporter-k7x2p", DAEMON_SET_KIND, "node-exporter", &[]);
        assert_eq!(pod_tab_name(&daemon), "node-exporter-k7x2p");
        let batch = owned_pod("batch-x1y2z", JOB_KIND, "batch", &[]);
        assert_eq!(pod_tab_name(&batch), "batch-x1y2z");
    }

    #[test]
    fn tab_name_of_a_pod_without_a_controller_is_its_name() {
        let mut bare = pod("tool-x1", (1, 1), None);
        bare.controller = None;
        assert_eq!(pod_tab_name(&bare), "tool-x1");
    }

    fn strings(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    #[test]
    fn ranked_pods_put_ready_first_then_newest() {
        let pods = [
            pod("old-ready", (1, 1), Some("2024-05-01T10:00:00Z")),
            pod("new-unready", (0, 1), Some("2024-05-01T12:00:00Z")),
            pod("new-ready", (1, 1), Some("2024-05-01T11:00:00Z")),
            pod("b-same-time", (1, 1), Some("2024-05-01T10:00:00Z")),
            pod("no-time", (1, 1), None),
            pod("empty", (0, 0), Some("2024-05-01T13:00:00Z")),
        ];
        let ranked = ranked_pods(&job(), &pods);
        assert_eq!(
            names(&ranked),
            [
                "new-ready",
                "b-same-time",
                "old-ready",
                "no-time",
                "empty",
                "new-unready"
            ]
        );
    }

    #[test]
    fn ranked_pods_keep_only_owned_pods() {
        let mut other = pod("other", (1, 1), None);
        other.controller = Some(ControllerRef {
            kind: JOB_KIND.to_owned(),
            name: "elsewhere".to_owned(),
        });
        let mut api_pod = pod("api-7d9f8c-x2k4q", (1, 1), None);
        api_pod.controller = Some(ControllerRef {
            kind: REPLICA_SET_KIND.to_owned(),
            name: "api-7d9f8c".to_owned(),
        });
        let pods = [pod("mine", (1, 1), None), other, api_pod];
        assert_eq!(names(&ranked_pods(&job(), &pods)), ["mine"]);
        let deployment = PodOwner::Deployment {
            namespace: "ns".to_owned(),
            name: "api".to_owned(),
        };
        assert_eq!(
            names(&ranked_pods(&deployment, &pods)),
            ["api-7d9f8c-x2k4q"]
        );
    }

    #[test]
    fn member_change_fills_free_slots_in_rank_order() {
        let pods = [
            pod("a", (1, 1), None),
            pod("b", (1, 1), None),
            pod("c", (1, 1), None),
        ];
        let ranked: Vec<_> = pods.iter().collect();
        let change = member_change(&[], &ranked, 2);
        assert_eq!(change.joined, ["a", "b"]);
        assert!(change.left.is_empty());
    }

    #[test]
    fn member_change_keeps_listed_members_beyond_limit() {
        let pods = [
            pod("a", (1, 1), None),
            pod("b", (1, 1), None),
            pod("c", (1, 1), None),
        ];
        let ranked: Vec<_> = pods.iter().collect();
        let change = member_change(&strings(&["c"]), &ranked, 0);
        assert!(change.joined.is_empty());
        assert!(change.left.is_empty());
    }

    #[test]
    fn member_change_reports_unlisted_members_as_left() {
        let pods = [pod("a", (1, 1), None)];
        let ranked: Vec<_> = pods.iter().collect();
        let change = member_change(&strings(&["a", "gone"]), &ranked, 5);
        assert_eq!(change.left, ["gone"]);
        assert!(change.joined.is_empty());
    }

    #[test]
    fn member_change_offers_returning_name_as_join() {
        let pods = [pod("db-0", (1, 1), None)];
        let ranked: Vec<_> = pods.iter().collect();
        let change = member_change(&[], &ranked, 1);
        assert_eq!(change.joined, ["db-0"]);
    }

    #[test]
    fn join_slots_counts_every_live_stream() {
        // 18 live streams leave room for one more pod of two containers.
        assert_eq!(join_slots(5, 18, 2), 1);
        assert_eq!(join_slots(5, 20, 2), 0);
        // The pod limit (10 pods) is reached.
        assert_eq!(join_slots(10, 0, 1), 0);
        assert_eq!(join_slots(3, 0, 1), 7);
    }

    #[test]
    fn pod_limit_divides_streams_by_containers() {
        assert_eq!(pod_limit(1), 10);
        assert_eq!(pod_limit(2), 10);
        assert_eq!(pod_limit(3), 6);
        assert_eq!(pod_limit(30), 1);
        assert_eq!(pod_limit(0), 10);
    }

    #[test]
    fn pod_short_name_is_last_segment_except_stateful_sets() {
        let deployment = PodOwner::Deployment {
            namespace: "ns".to_owned(),
            name: "api".to_owned(),
        };
        assert_eq!(pod_short_name(&deployment, "api-7d9f8c-x2k4q"), "x2k4q");
        let stateful = PodOwner::Controller {
            namespace: "ns".to_owned(),
            kind: STATEFUL_SET_KIND,
            name: "postgres".to_owned(),
        };
        assert_eq!(pod_short_name(&stateful, "postgres-0"), "postgres-0");
    }

    #[test]
    fn origin_label_keeps_what_tells_replicas_apart() {
        let stateful = PodOwner::Controller {
            namespace: "ns".to_owned(),
            kind: STATEFUL_SET_KIND,
            name: "argocd-application-controller".to_owned(),
        };
        assert_eq!(
            pod_origin_label(&stateful, "argocd-application-controller-1", "app"),
            "-1/app"
        );
        let deployment = PodOwner::Deployment {
            namespace: "ns".to_owned(),
            name: "api".to_owned(),
        };
        assert_eq!(
            pod_origin_label(&deployment, "api-7d9f8c-x2k4q", "web"),
            "x2k4q/web"
        );
        let daemon = PodOwner::Controller {
            namespace: "ns".to_owned(),
            kind: DAEMON_SET_KIND,
            name: "node-exporter".to_owned(),
        };
        assert_eq!(
            pod_origin_label(&daemon, "node-exporter-k7x2p", "exporter"),
            "k7x2p/exporter"
        );
    }

    #[test]
    fn origin_label_without_a_suffix_is_cut_from_the_start() {
        let stateful = PodOwner::Controller {
            namespace: "ns".to_owned(),
            kind: STATEFUL_SET_KIND,
            name: "argocd-application-controller".to_owned(),
        };
        let label = pod_origin_label(&stateful, "argocd-application-controller", "app");
        assert_eq!(label.chars().count(), PREFIX_COLUMN_CHARS);
        assert!(label.starts_with('…') && label.ends_with("controller/app"));
        assert_eq!(pod_origin_label(&stateful, "db", "c"), "db/c");
    }

    fn container(name: &str, kind: ContainerKind) -> cluster::ContainerSummary {
        cluster::ContainerSummary {
            terminal: cluster::ContainerTerminal::None,
            name: name.to_owned(),
            image: "img".to_owned(),
            kind,
            state: cluster::ContainerState::NotReported,
            is_ready: true,
            restart_count: 0,
            last_termination: None,
            image_digest: None,
            pull_policy: None,
            is_started: None,
            ports: Vec::new(),
            resources: Vec::new(),
            probes: cluster::ContainerProbes::default(),
            env: Vec::new(),
            env_from: Vec::new(),
            mounts: Vec::new(),
        }
    }

    #[test]
    fn container_names_union_in_first_seen_order_init_last() {
        let mut a = pod("a", (1, 1), None);
        a.containers = vec![
            container("setup", ContainerKind::Init),
            container("api", ContainerKind::Main),
            container("proxy", ContainerKind::Sidecar),
        ];
        let mut b = pod("b", (1, 1), None);
        b.containers = vec![
            container("worker", ContainerKind::Main),
            container("api", ContainerKind::Main),
            container("migrate", ContainerKind::Init),
            container("setup", ContainerKind::Init),
        ];
        assert_eq!(
            container_names(&[&a, &b]),
            ["api", "proxy", "worker", "setup", "migrate"]
        );
        assert!(container_names(&[]).is_empty());
    }

    fn restarted(
        name: &str,
        count: u32,
        termination: Option<cluster::Termination>,
    ) -> ContainerSummary {
        ContainerSummary {
            restart_count: count,
            last_termination: termination,
            ..container(name, ContainerKind::Main)
        }
    }

    #[test]
    fn restart_marker_names_reason_exit_and_count() {
        let now: jiff::Timestamp = "2024-05-01T12:00:00Z".parse().expect("valid time");
        let finished: jiff::Timestamp = "2024-05-01T11:59:00Z".parse().expect("valid time");
        let killed = cluster::Termination {
            reason: Some(StatusReason::OomKilled),
            exit_code: 137,
            signal: None,
            started_at: None,
            finished_at: Some(finished),
        };
        let marker = restart_marker(&restarted("api", 14, Some(killed)), now);
        assert_eq!(
            marker.text,
            "── container api terminated: OOMKilled (exit 137) · restart #14 ──"
        );
        assert_eq!(marker.timestamp, Some(finished));
        let plain = restart_marker(&restarted("api", 14, None), now);
        assert_eq!(plain.text, "── container api restarted · restart #14 ──");
        assert_eq!(plain.timestamp, Some(now));
    }

    #[test]
    fn restart_marker_without_a_reason_still_gives_the_exit_code() {
        let now: jiff::Timestamp = "2024-05-01T12:00:00Z".parse().expect("valid time");
        let unknown = cluster::Termination {
            reason: None,
            exit_code: 2,
            signal: None,
            started_at: None,
            finished_at: None,
        };
        let marker = restart_marker(&restarted("api", 3, Some(unknown)), now);
        assert_eq!(
            marker.text,
            "── container api terminated (exit 2) · restart #3 ──"
        );
        assert_eq!(marker.timestamp, Some(now));
    }

    fn key(namespace: &str, pod: &str, container: &str) -> (String, String, String) {
        (namespace.to_owned(), pod.to_owned(), container.to_owned())
    }

    #[test]
    fn same_named_pods_of_two_namespaces_keep_their_own_baselines() {
        let in_namespace = |namespace: &str, count: u32| {
            let mut api = pod("api-0", (1, 1), None);
            api.namespace = namespace.to_owned();
            api.containers = vec![restarted("api", count, None)];
            api
        };
        let mut seen = HashMap::new();
        assert!(rising_restarts(&mut seen, &in_namespace("a", 5), &["api"]).is_empty());
        assert!(rising_restarts(&mut seen, &in_namespace("b", 1), &["api"]).is_empty());
        // Each namespace compares with its own count: no rise, no fake marker.
        assert!(rising_restarts(&mut seen, &in_namespace("a", 5), &["api"]).is_empty());
        assert_eq!(
            rising_restarts(&mut seen, &in_namespace("b", 2), &["api"]),
            [("api".to_owned(), 2)]
        );
    }
    #[test]
    fn rising_restarts_report_rises_only() {
        let with = |count: u32| {
            let mut api = pod("api-0", (1, 1), None);
            api.containers = vec![
                restarted("api", count, None),
                restarted("proxy", count, None),
            ];
            api
        };
        let mut seen = HashMap::new();
        // First sight records without a rise.
        assert!(rising_restarts(&mut seen, &with(3), &["api"]).is_empty());
        assert!(rising_restarts(&mut seen, &with(3), &["api"]).is_empty());
        assert_eq!(
            rising_restarts(&mut seen, &with(5), &["api"]),
            [("api".to_owned(), 5)]
        );
        // A recreated pod starts at a lower count: only the baseline moves.
        assert!(rising_restarts(&mut seen, &with(0), &["api"]).is_empty());
        assert_eq!(seen.get(&key("ns", "api-0", "api")), Some(&0));
        // A container that is not streamed is never read.
        assert!(!seen.contains_key(&key("ns", "api-0", "proxy")));
    }
    #[test]
    fn scope_covers_named_several_and_all() {
        assert!(scope_covers(&NamespaceScope::All, "any"));
        assert!(scope_covers(&NamespaceScope::Named("a".into()), "a"));
        assert!(!scope_covers(&NamespaceScope::Named("a".into()), "b"));
        let several = NamespaceScope::of_namespaces(["a".to_owned(), "b".to_owned()]);
        assert!(scope_covers(&several, "b"));
        assert!(!scope_covers(&several, "c"));
    }
}
