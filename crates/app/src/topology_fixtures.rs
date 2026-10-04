//! Fixtures of the Topology tests: the objects of a `shop` namespace, built into rows by the real
//! row builders, and a `Fixture` that assembles the inputs of a build.

use std::collections::{BTreeMap, BTreeSet};

use cluster::{
    BindingSummary, ConfigMapSummary, ContainerKind, ContainerProbes, ContainerState,
    ContainerSummary, ControllerRef, DaemonSetSummary, DeploymentSummary, EnvEntry, EnvFromEntry,
    EnvFromSource, EnvSource, HorizontalPodAutoscalerSummary, IngressPath, IngressSummary,
    IngressTls, MountEntry, NodeSummary, PersistentVolumeClaimSummary, PodStatus, PodSummary,
    RbacRule, ReadyCount, ReplicaSetSummary, RoleKind, RoleRef, RoleSummary, SecretDetails,
    SecretSummary, ServiceAccountSummary, ServicePortSummary, ServiceSummary, StatefulSetSummary,
    StatusReason, Subject, SubjectKind, VolumeSource,
};
use jiff::Timestamp;

use crate::access_rows::{
    cluster_role_binding_row, role_binding_row, role_row, service_account_row,
};
use crate::config_map_rows::config_map_row;
use crate::kind_row::KindRow;
use crate::network_rows::{ingress_row, service_row};
use crate::policy_rows::horizontal_pod_autoscaler_row;
use crate::secret_rows::secret_row;
use crate::storage_rows::persistent_volume_claim_row;
use crate::topology_graph::{
    FeedRows, KindFilter, NodeId, TopologyBuild, TopologyFilter, TopologyGraph, TopologyInputs,
    TopologyKind, build_topology,
};
use crate::workload_rows::{daemon_set_row, deployment_row, replica_set_row, stateful_set_row};

pub(crate) const NAMESPACE: &str = "shop";

/// A fixed "now": 2024-05-01T10:00:00Z.
pub(crate) fn now() -> Timestamp {
    "2024-05-01T10:00:00Z".parse().expect("valid time")
}

pub(crate) fn controller(kind: &str, name: &str) -> ControllerRef {
    ControllerRef {
        kind: kind.to_owned(),
        name: name.to_owned(),
    }
}

fn terms(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| (*item).to_owned()).collect()
}

/// A running, ready pod of `shop`; `owner` is its controller as `(kind, name)`.
pub(crate) fn pod(name: &str, labels: &[&str], owner: Option<(&str, &str)>) -> PodSummary {
    PodSummary {
        is_finished: false,
        namespace: NAMESPACE.to_owned(),
        name: name.to_owned(),
        status: PodStatus::Reason(StatusReason::Running),
        ready: ReadyCount { ready: 1, total: 1 },
        restarts: 0,
        node_name: None,
        created_at: None,
        pod_ip: None,
        qos_class: None,
        service_account: None,
        controller: owner.map(|(kind, name)| controller(kind, name)),
        conditions: Vec::new(),
        containers: Vec::new(),
        status_message: None,
        labels: terms(labels),
        host_network: false,
        image_pull_secrets: Vec::new(),
    }
}

/// `pod` in another namespace.
pub(crate) fn pod_in(namespace: &str, name: &str, labels: &[&str]) -> PodSummary {
    PodSummary {
        is_finished: false,
        namespace: namespace.to_owned(),
        ..pod(name, labels, None)
    }
}

/// A pod whose only container waits in `CrashLoopBackOff`.
pub(crate) fn crashing_pod(name: &str, labels: &[&str], owner: Option<(&str, &str)>) -> PodSummary {
    let mut crashing = pod(name, labels, owner);
    crashing.status = PodStatus::Reason(StatusReason::CrashLoopBackOff);
    crashing.ready = ReadyCount { ready: 0, total: 1 };
    let mut container = container("api");
    container.is_ready = false;
    container.state = ContainerState::Waiting {
        reason: Some(StatusReason::CrashLoopBackOff),
        message: None,
    };
    crashing.containers = vec![container];
    crashing
}

pub(crate) fn container(name: &str) -> ContainerSummary {
    ContainerSummary {
        terminal: cluster::ContainerTerminal::None,
        name: name.to_owned(),
        image: "img".to_owned(),
        kind: ContainerKind::Main,
        state: ContainerState::NotReported,
        is_ready: true,
        restart_count: 0,
        last_termination: None,
        image_digest: None,
        pull_policy: None,
        is_started: None,
        ports: Vec::new(),
        resources: Vec::new(),
        probes: ContainerProbes::default(),
        env: Vec::new(),
        env_from: Vec::new(),
        mounts: Vec::new(),
    }
}

/// What a pod's container refers to: one source of each way a config object can be named.
#[derive(Clone, Copy)]
pub(crate) enum Ref<'a> {
    EnvConfigMap(&'a str),
    EnvSecret(&'a str),
    EnvFromConfigMap(&'a str),
    EnvFromSecret(&'a str),
    MountConfigMap(&'a str),
    MountSecret(&'a str),
    MountClaim(&'a str),
    ProjectedConfigMap(&'a str),
    ProjectedSecret(&'a str),
    PullSecret(&'a str),
}

/// `pod` with one container that holds `refs`.
pub(crate) fn pod_with(mut pod: PodSummary, refs: &[Ref]) -> PodSummary {
    let mut main = container("app");
    for reference in refs {
        match *reference {
            Ref::EnvConfigMap(name) => main.env.push(EnvEntry {
                name: "A".to_owned(),
                source: EnvSource::ConfigMapKey {
                    name: name.to_owned(),
                    key: "k".to_owned(),
                },
            }),
            Ref::EnvSecret(name) => main.env.push(EnvEntry {
                name: "B".to_owned(),
                source: EnvSource::SecretKey {
                    name: name.to_owned(),
                    key: "k".to_owned(),
                },
            }),
            Ref::EnvFromConfigMap(name) => main.env_from.push(EnvFromEntry {
                source: EnvFromSource::ConfigMap {
                    name: name.to_owned(),
                },
                prefix: None,
            }),
            Ref::EnvFromSecret(name) => main.env_from.push(EnvFromEntry {
                source: EnvFromSource::Secret {
                    name: name.to_owned(),
                },
                prefix: None,
            }),
            Ref::MountConfigMap(name) => main.mounts.push(mount(VolumeSource::ConfigMap {
                name: name.to_owned(),
            })),
            Ref::MountSecret(name) => main.mounts.push(mount(VolumeSource::Secret {
                name: name.to_owned(),
            })),
            Ref::MountClaim(claim) => {
                main.mounts.push(mount(VolumeSource::PersistentVolumeClaim {
                    claim: claim.to_owned(),
                }))
            }
            Ref::ProjectedConfigMap(name) => main.mounts.push(mount(VolumeSource::Projected {
                config_maps: vec![name.to_owned()],
                secrets: Vec::new(),
            })),
            Ref::ProjectedSecret(name) => main.mounts.push(mount(VolumeSource::Projected {
                config_maps: Vec::new(),
                secrets: vec![name.to_owned()],
            })),
            Ref::PullSecret(name) => pod.image_pull_secrets.push(name.to_owned()),
        }
    }
    pod.containers = vec![main];
    pod
}

fn mount(source: VolumeSource) -> MountEntry {
    MountEntry {
        path: "/mnt".to_owned(),
        volume: "v".to_owned(),
        source,
        is_read_only: false,
        sub_path: None,
    }
}

pub(crate) fn service(name: &str, selector: &[&str]) -> ServiceSummary {
    ServiceSummary {
        namespace: NAMESPACE.to_owned(),
        name: name.to_owned(),
        created_at: None,
        labels: Vec::new(),
        service_type: "ClusterIP".to_owned(),
        cluster_ips: vec!["10.0.0.5".to_owned()],
        is_headless: false,
        external_addresses: Vec::new(),
        ports: vec![ServicePortSummary {
            name: None,
            port: 80,
            target_port: None,
            node_port: None,
            protocol: "TCP".to_owned(),
        }],
        selector: terms(selector),
    }
}

pub(crate) fn deployment(name: &str, desired: u32, ready: u32) -> DeploymentSummary {
    DeploymentSummary {
        namespace: NAMESPACE.to_owned(),
        name: name.to_owned(),
        created_at: None,
        labels: Vec::new(),
        desired,
        ready,
        up_to_date: ready,
        available: ready,
        strategy: "RollingUpdate".to_owned(),
        max_surge: None,
        max_unavailable: None,
        progress_deadline_seconds: 600,
        is_paused: false,
        revision: None,
        selector: Vec::new(),
        containers: Vec::new(),
        conditions: Vec::new(),
        template_change: None,
    }
}

pub(crate) fn replica_set(
    name: &str,
    owner: Option<&str>,
    desired: u32,
    current: u32,
) -> ReplicaSetSummary {
    ReplicaSetSummary {
        namespace: NAMESPACE.to_owned(),
        name: name.to_owned(),
        created_at: None,
        labels: Vec::new(),
        desired,
        current,
        ready: current,
        owner: owner.map(|owner| controller("Deployment", owner)),
        revision: Some("3".to_owned()),
        selector: Vec::new(),
        containers: Vec::new(),
    }
}

pub(crate) fn stateful_set(name: &str, desired: u32, ready: u32) -> StatefulSetSummary {
    StatefulSetSummary {
        namespace: NAMESPACE.to_owned(),
        name: name.to_owned(),
        created_at: None,
        labels: Vec::new(),
        desired,
        ready,
        current: ready,
        updated: ready,
        service_name: None,
        update_strategy: "RollingUpdate".to_owned(),
        pod_management_policy: "OrderedReady".to_owned(),
        selector: Vec::new(),
        containers: Vec::new(),
        claim_templates: Vec::new(),
        claim_retention: None,
    }
}

pub(crate) fn daemon_set(name: &str, desired: u32, ready: u32) -> DaemonSetSummary {
    DaemonSetSummary {
        namespace: NAMESPACE.to_owned(),
        name: name.to_owned(),
        created_at: None,
        labels: Vec::new(),
        desired,
        current: ready,
        ready,
        up_to_date: ready,
        available: ready,
        misscheduled: 0,
        node_selector: Vec::new(),
        update_strategy: "RollingUpdate".to_owned(),
        selector: Vec::new(),
        containers: Vec::new(),
    }
}

pub(crate) fn ingress(
    name: &str,
    backends: &[(&str, &str)],
    default_service: Option<&str>,
    tls_secret: Option<&str>,
) -> IngressSummary {
    IngressSummary {
        namespace: NAMESPACE.to_owned(),
        name: name.to_owned(),
        created_at: None,
        labels: Vec::new(),
        class: None,
        hosts: vec![format!("{name}.example.com")],
        addresses: Vec::new(),
        rules: backends
            .iter()
            .map(|(path, service)| IngressPath {
                host: None,
                path: Some((*path).to_owned()),
                backend: format!("{service}:80"),
                service: Some((*service).to_owned()),
            })
            .collect(),
        default_backend: None,
        default_service: default_service.map(str::to_owned),
        tls: tls_secret
            .map(|secret| IngressTls {
                hosts: Vec::new(),
                secret_name: Some(secret.to_owned()),
            })
            .into_iter()
            .collect(),
    }
}

pub(crate) fn hpa(name: &str, target_kind: &str, target: &str) -> HorizontalPodAutoscalerSummary {
    HorizontalPodAutoscalerSummary {
        namespace: NAMESPACE.to_owned(),
        name: name.to_owned(),
        created_at: None,
        labels: Vec::new(),
        target: controller(target_kind, target),
        min_replicas: 2,
        max_replicas: 6,
        current_replicas: 2,
        desired_replicas: 2,
        metrics: Vec::new(),
        conditions: Vec::new(),
        last_scaled_at: None,
    }
}

pub(crate) fn claim(
    name: &str,
    phase: &str,
    created_at: Option<Timestamp>,
) -> PersistentVolumeClaimSummary {
    PersistentVolumeClaimSummary {
        namespace: NAMESPACE.to_owned(),
        name: name.to_owned(),
        created_at,
        labels: Vec::new(),
        phase: phase.to_owned(),
        is_terminating: false,
        volume: (phase == "Bound" || phase == "Lost").then(|| "pv-1".to_owned()),
        capacity: None,
        requested: None,
        access_modes: Vec::new(),
        storage_class: None,
        volume_mode: None,
        conditions: Vec::new(),
    }
}

pub(crate) fn config_map(name: &str) -> ConfigMapSummary {
    ConfigMapSummary {
        namespace: NAMESPACE.to_owned(),
        name: name.to_owned(),
        created_at: None,
        labels: Vec::new(),
        keys: Vec::new(),
        is_immutable: false,
    }
}

pub(crate) fn secret(name: &str, secret_type: &str) -> SecretSummary {
    SecretSummary {
        namespace: NAMESPACE.to_owned(),
        name: name.to_owned(),
        created_at: None,
        labels: Vec::new(),
        secret_type: secret_type.to_owned(),
        keys: Vec::new(),
        details: SecretDetails::None,
        is_immutable: false,
        is_owned: false,
    }
}

/// A ServiceAccount of `shop`.
pub(crate) fn service_account(name: &str) -> ServiceAccountSummary {
    ServiceAccountSummary {
        namespace: NAMESPACE.to_owned(),
        name: name.to_owned(),
        created_at: None,
        labels: Vec::new(),
        secrets: Vec::new(),
        image_pull_secrets: Vec::new(),
        automount_token: None,
        cloud_identities: Vec::new(),
    }
}

/// A Role of `shop` with `rule_count` read rules.
pub(crate) fn role(name: &str, rule_count: usize) -> RoleSummary {
    let rule = RbacRule {
        api_groups: vec![String::new()],
        resources: vec!["pods".to_owned()],
        resource_names: Vec::new(),
        verbs: vec!["get".to_owned()],
        non_resource_urls: Vec::new(),
    };
    RoleSummary {
        namespace: Some(NAMESPACE.to_owned()),
        name: name.to_owned(),
        created_at: None,
        labels: Vec::new(),
        rules: vec![rule; rule_count],
        aggregation: Vec::new(),
    }
}

/// The subject of a ServiceAccount of `shop`.
pub(crate) fn account_subject(name: &str) -> Subject {
    Subject {
        kind: SubjectKind::ServiceAccount,
        name: name.to_owned(),
        namespace: Some(NAMESPACE.to_owned()),
    }
}

/// The subject of a group such as `system:serviceaccounts`.
pub(crate) fn group_subject(name: &str) -> Subject {
    Subject {
        kind: SubjectKind::Group,
        name: name.to_owned(),
        namespace: None,
    }
}

/// A RoleBinding of `shop` (`namespace: Some`) or, with `None`, a ClusterRoleBinding.
pub(crate) fn binding(
    namespace: Option<&str>,
    name: &str,
    role: (RoleKind, &str),
    subjects: Vec<Subject>,
) -> BindingSummary {
    BindingSummary {
        namespace: namespace.map(str::to_owned),
        name: name.to_owned(),
        created_at: None,
        labels: Vec::new(),
        role: RoleRef {
            kind: role.0,
            name: role.1.to_owned(),
        },
        subjects,
    }
}

/// What one feed holds in a fixture.
enum Feed {
    Ready(Vec<KindRow>),
    Loading,
    Off,
    Failed,
}

/// The inputs of one build: every feed ready and empty, the pods listed and empty, every chip on.
pub(crate) struct Fixture {
    pub(crate) pods: Option<Vec<PodSummary>>,
    feeds: BTreeMap<TopologyKind, Feed>,
    pub(crate) nodes: Vec<NodeSummary>,
    pub(crate) filter: TopologyFilter,
    pub(crate) expanded: BTreeSet<NodeId>,
    pub(crate) now: Timestamp,
}

impl Default for Fixture {
    fn default() -> Self {
        let feeds = [
            TopologyKind::Ingress,
            TopologyKind::Service,
            TopologyKind::Deployment,
            TopologyKind::StatefulSet,
            TopologyKind::DaemonSet,
            TopologyKind::ReplicaSet,
            TopologyKind::ConfigMap,
            TopologyKind::Secret,
            TopologyKind::PersistentVolumeClaim,
            TopologyKind::HorizontalPodAutoscaler,
        ]
        .into_iter()
        .map(|kind| (kind, Feed::Ready(Vec::new())))
        .collect();
        Self {
            pods: Some(Vec::new()),
            feeds,
            nodes: Vec::new(),
            filter: TopologyFilter::initial(),
            expanded: BTreeSet::new(),
            now: now(),
        }
    }
}

impl Fixture {
    fn push(mut self, kind: TopologyKind, row: KindRow) -> Self {
        if let Some(Feed::Ready(rows)) = self.feeds.get_mut(&kind) {
            rows.push(row);
        }
        self
    }

    pub(crate) fn with_pod(mut self, pod: PodSummary) -> Self {
        self.pods.get_or_insert_with(Vec::new).push(pod);
        self
    }

    pub(crate) fn with_pods(mut self, pods: impl IntoIterator<Item = PodSummary>) -> Self {
        self.pods.get_or_insert_with(Vec::new).extend(pods);
        self
    }

    pub(crate) fn without_pods(mut self) -> Self {
        self.pods = None;
        self
    }

    pub(crate) fn with_deployment(self, name: &str, desired: u32, ready: u32) -> Self {
        self.push(
            TopologyKind::Deployment,
            deployment_row(&deployment(name, desired, ready)),
        )
    }

    pub(crate) fn with_deployment_summary(self, summary: DeploymentSummary) -> Self {
        self.push(TopologyKind::Deployment, deployment_row(&summary))
    }

    pub(crate) fn with_replica_set(
        self,
        name: &str,
        owner: Option<&str>,
        desired: u32,
        current: u32,
    ) -> Self {
        self.push(
            TopologyKind::ReplicaSet,
            replica_set_row(&replica_set(name, owner, desired, current)),
        )
    }

    pub(crate) fn with_stateful_set(self, name: &str, desired: u32, ready: u32) -> Self {
        self.push(
            TopologyKind::StatefulSet,
            stateful_set_row(&stateful_set(name, desired, ready)),
        )
    }

    pub(crate) fn with_daemon_set(self, name: &str, desired: u32, ready: u32) -> Self {
        self.push(
            TopologyKind::DaemonSet,
            daemon_set_row(&daemon_set(name, desired, ready)),
        )
    }

    pub(crate) fn with_service(self, name: &str, selector: &[&str]) -> Self {
        self.with_service_summary(service(name, selector))
    }

    pub(crate) fn with_service_summary(self, summary: ServiceSummary) -> Self {
        self.push(TopologyKind::Service, service_row(&summary))
    }

    pub(crate) fn with_ingress(self, summary: IngressSummary) -> Self {
        self.push(TopologyKind::Ingress, ingress_row(&summary))
    }

    pub(crate) fn with_hpa(self, summary: HorizontalPodAutoscalerSummary) -> Self {
        self.push(
            TopologyKind::HorizontalPodAutoscaler,
            horizontal_pod_autoscaler_row(&summary),
        )
    }

    pub(crate) fn with_config_map(self, name: &str) -> Self {
        self.push(TopologyKind::ConfigMap, config_map_row(&config_map(name)))
    }

    pub(crate) fn with_secret(self, name: &str, secret_type: &str) -> Self {
        self.push(TopologyKind::Secret, secret_row(&secret(name, secret_type)))
    }

    pub(crate) fn with_secret_summary(self, summary: SecretSummary) -> Self {
        self.push(TopologyKind::Secret, secret_row(&summary))
    }

    pub(crate) fn with_claim(self, summary: PersistentVolumeClaimSummary) -> Self {
        self.push(
            TopologyKind::PersistentVolumeClaim,
            persistent_volume_claim_row(&summary),
        )
    }

    /// The RBAC chip is on and its four feeds are ready and empty.
    pub(crate) fn with_rbac(mut self) -> Self {
        self.filter.kinds.insert(KindFilter::Rbac);
        for kind in [
            TopologyKind::ServiceAccount,
            TopologyKind::RoleBinding,
            TopologyKind::ClusterRoleBinding,
            TopologyKind::Role,
        ] {
            self.feeds
                .entry(kind)
                .or_insert_with(|| Feed::Ready(Vec::new()));
        }
        self
    }

    pub(crate) fn with_service_account(self, summary: ServiceAccountSummary) -> Self {
        self.push(TopologyKind::ServiceAccount, service_account_row(&summary))
    }

    pub(crate) fn with_role_binding(self, summary: BindingSummary) -> Self {
        self.push(TopologyKind::RoleBinding, role_binding_row(&summary))
    }

    pub(crate) fn with_cluster_role_binding(self, summary: BindingSummary) -> Self {
        self.push(
            TopologyKind::ClusterRoleBinding,
            cluster_role_binding_row(&summary),
        )
    }

    pub(crate) fn with_role(self, summary: RoleSummary) -> Self {
        self.push(TopologyKind::Role, role_row(&summary))
    }

    /// The feed of `kind` has not delivered its first snapshot.
    pub(crate) fn loading(mut self, kind: TopologyKind) -> Self {
        self.feeds.insert(kind, Feed::Loading);
        self
    }

    /// The feed of `kind` does not run (denied).
    pub(crate) fn off(mut self, kind: TopologyKind) -> Self {
        self.feeds.insert(kind, Feed::Off);
        self
    }

    /// The watch of `kind` failed before its first snapshot.
    pub(crate) fn failed(mut self, kind: TopologyKind) -> Self {
        self.feeds.insert(kind, Feed::Failed);
        self
    }

    /// The feed of `kind` is not in the build at all, like a kind whose chip is off.
    pub(crate) fn without_feed(mut self, kind: TopologyKind) -> Self {
        self.feeds.remove(&kind);
        self
    }

    /// Runs `check` on the feeds as the build reads them.
    pub(crate) fn with_rows<R>(&self, check: impl FnOnce(&[(TopologyKind, FeedRows)]) -> R) -> R {
        check(&self.feed_rows())
    }

    fn feed_rows(&self) -> Vec<(TopologyKind, FeedRows<'_>)> {
        self.feeds
            .iter()
            .map(|(kind, feed)| {
                let rows = match feed {
                    Feed::Ready(rows) => FeedRows::Ready(rows),
                    Feed::Loading => FeedRows::Loading,
                    Feed::Off => FeedRows::Off,
                    Feed::Failed => FeedRows::Failed,
                };
                (*kind, rows)
            })
            .collect()
    }

    pub(crate) fn build(&self) -> TopologyBuild {
        let rows = self.feed_rows();
        build_topology(&TopologyInputs {
            namespace: NAMESPACE,
            pods: self.pods.as_deref(),
            rows: &rows,
            nodes: &self.nodes,
            filter: &self.filter,
            expanded: &self.expanded,
            now: self.now,
        })
    }

    /// The graph; the fixtures of the tests never exceed a limit unless they say so.
    pub(crate) fn graph(&self) -> TopologyGraph {
        match self.build() {
            TopologyBuild::Graph(graph) => graph,
            TopologyBuild::TooLarge(too_large) => panic!("unexpectedly too large: {too_large:?}"),
        }
    }
}

/// The node ids of a graph, in node order.
pub(crate) fn ids(graph: &TopologyGraph) -> Vec<NodeId> {
    graph.nodes.iter().map(|node| node.id.clone()).collect()
}

/// `NodeId::Object` of a kind and name.
pub(crate) fn object(kind: TopologyKind, name: &str) -> NodeId {
    NodeId::Object {
        kind,
        name: name.to_owned(),
    }
}

/// The index of a node in a graph.
pub(crate) fn index_of(graph: &TopologyGraph, id: &NodeId) -> usize {
    graph
        .nodes
        .iter()
        .position(|node| node.id == *id)
        .unwrap_or_else(|| panic!("no node {id:?}"))
}

/// Whether the graph has an edge of `relation` between the two nodes.
pub(crate) fn has_edge(
    graph: &TopologyGraph,
    from: &NodeId,
    to: &NodeId,
    relation: crate::topology_graph::Relation,
) -> bool {
    let (Some(from), Some(to)) = (
        graph.nodes.iter().position(|node| node.id == *from),
        graph.nodes.iter().position(|node| node.id == *to),
    ) else {
        return false;
    };
    graph
        .edges
        .iter()
        .any(|edge| edge.from == from && edge.to == to && edge.relation == relation)
}

/// `pod` running as `account`.
pub(crate) fn pod_as(mut pod: PodSummary, account: &str) -> PodSummary {
    pod.service_account = Some(account.to_owned());
    pod
}

/// The RBAC fixture of the test plan: Deployment `api` (its pods run as `api`), the owner-less pod
/// `cron-x` (runs as `default`), accounts `api` and `default`, RoleBinding `api-reader` (account
/// `api` to Role `reader`), RoleBinding `ghost` (account `api` to the missing Role `gone`),
/// ClusterRoleBinding `ci-admin` (account `api` to ClusterRole `cluster-admin`), and
/// ClusterRoleBinding `all-sa` (group `system:serviceaccounts` to ClusterRole `view`). The RBAC
/// chip is on.
pub(crate) fn shop_access() -> Fixture {
    let api_pods = (0..2).map(|n| {
        pod_as(
            pod(
                &format!("api-7d9f-{n}"),
                &[],
                Some(("ReplicaSet", "api-7d9f")),
            ),
            "api",
        )
    });
    Fixture::default()
        .with_rbac()
        .with_deployment("api", 2, 2)
        .with_replica_set("api-7d9f", Some("api"), 2, 2)
        .with_pods(api_pods)
        .with_pod(pod("cron-x", &[], None))
        .with_service_account(service_account("api"))
        .with_service_account(service_account("default"))
        .with_role(role("reader", 3))
        .with_role_binding(binding(
            Some(NAMESPACE),
            "api-reader",
            (RoleKind::Role, "reader"),
            vec![account_subject("api")],
        ))
        .with_role_binding(binding(
            Some(NAMESPACE),
            "ghost",
            (RoleKind::Role, "gone"),
            vec![account_subject("api")],
        ))
        .with_cluster_role_binding(binding(
            None,
            "ci-admin",
            (RoleKind::ClusterRole, "cluster-admin"),
            vec![account_subject("api")],
        ))
        .with_cluster_role_binding(binding(
            None,
            "all-sa",
            (RoleKind::ClusterRole, "view"),
            vec![group_subject("system:serviceaccounts")],
        ))
}
