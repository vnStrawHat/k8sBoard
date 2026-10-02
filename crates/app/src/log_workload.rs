//! Which pods of a workload a log tab follows, as pure functions over the pods list.

use cluster::{ContainerKind, NamespaceScope, PodSummary};

use crate::kind_row::{PodOwner, STATEFUL_SET_KIND, owns_pod};

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

#[cfg(test)]
mod tests {
    use cluster::{ControllerRef, PodStatus, ReadyCount, StatusReason};

    use super::*;
    use crate::kind_row::{JOB_KIND, REPLICA_SET_KIND};

    fn pod(name: &str, ready: (u32, u32), created: Option<&str>) -> PodSummary {
        PodSummary {
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

    fn container(name: &str, kind: ContainerKind) -> cluster::ContainerSummary {
        cluster::ContainerSummary {
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
