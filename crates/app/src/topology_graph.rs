//! The Topology graph (W11): the objects of one namespace as nodes and their relations as edges,
//! built from the live lists. Pure: no GPUI context, no logging. Secret nodes carry names only.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use cluster::{
    ContainerKind, ControllerRef, EnvFromSource, EnvSource, NodeSummary, PodSummary, RoleKind,
    Selector, ServiceAccountSummary, ServiceSummary, VolumeSource,
};
use gpui_kit::SharedString;
use gpui_kit::assets::IconName;
use jiff::Timestamp;

use crate::access_bindings::{BindingIndex, BoundRole, pod_account};
use crate::kind_join::ServiceHealth;
use crate::kind_row::{KindObject, KindRow};
use crate::resource_kind::{POD_ICON, ResourceKind};
use crate::status_tone::{StatusTone, pod_status_label};
use crate::table_selection::ResourceKey;
use crate::topology_access::{
    AccessBindings, binding_node, cluster_admin_grant, names_namespace_account,
};
use crate::topology_checks::{ConfigCheck, GraphParts, claim_problem, graph_checks};
use crate::topology_layout::Placement;

/// More listed rows plus namespace pods than this are not built: the cost is bounded by the input.
pub(crate) const RAW_LIMIT: usize = 5_000;
/// An owner with more non-failing pods than this shows them as one group node.
pub(crate) const POD_GROUP_LIMIT: usize = 6;
/// More visible nodes than this are not drawn.
pub(crate) const NODE_LIMIT: usize = 500;

/// The label keys that name an app, in priority order.
const APP_LABEL_KEYS: [&str; 3] = ["app.kubernetes.io/name", "app", "k8s-app"];
/// Every pod mounts it through the projected service-account volume (decision 9).
const ROOT_CA_CONFIG_MAP: &str = "kube-root-ca.crt";
const EXTERNAL_NAME: &str = "ExternalName";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum TopologyKind {
    Ingress,
    HorizontalPodAutoscaler,
    Service,
    Deployment,
    StatefulSet,
    DaemonSet,
    ReplicaSet,
    Pod,
    ConfigMap,
    Secret,
    PersistentVolumeClaim,
    ServiceAccount,
    RoleBinding,
    ClusterRoleBinding,
    Role,
    ClusterRole,
}

impl TopologyKind {
    pub(crate) fn placement(self) -> Placement {
        match self {
            Self::Ingress | Self::HorizontalPodAutoscaler => Placement::Column(0),
            Self::Service | Self::Deployment | Self::StatefulSet | Self::DaemonSet => {
                Placement::Column(1)
            }
            Self::ReplicaSet => Placement::Column(2),
            Self::Pod => Placement::Column(3),
            Self::ConfigMap | Self::Secret | Self::PersistentVolumeClaim => Placement::ConfigRow,
            Self::ServiceAccount
            | Self::RoleBinding
            | Self::ClusterRoleBinding
            | Self::Role
            | Self::ClusterRole => Placement::AccessRow,
        }
    }

    /// The icon on a card chip.
    pub(crate) fn icon(self) -> IconName {
        self.resource_kind().map_or(POD_ICON, ResourceKind::icon)
    }

    /// The two letters of the Topology export, which draws no icon.
    pub(crate) fn badge(self) -> &'static str {
        self.resource_kind().map_or("Po", ResourceKind::badge)
    }

    /// The Kubernetes `kind`, as `ResourceKey::of_object` takes it.
    pub(crate) fn object_kind(self) -> &'static str {
        self.resource_kind()
            .map_or("Pod", ResourceKind::object_kind)
    }

    /// The kind as a card caption and a check text name it.
    pub(crate) fn caption_label(self) -> &'static str {
        match self {
            Self::HorizontalPodAutoscaler => "HPA",
            Self::PersistentVolumeClaim => "PVC",
            other => other.object_kind(),
        }
    }

    pub(crate) fn filter(self) -> KindFilter {
        match self {
            Self::Ingress => KindFilter::Ingress,
            Self::Service => KindFilter::Service,
            Self::HorizontalPodAutoscaler
            | Self::Deployment
            | Self::StatefulSet
            | Self::DaemonSet
            | Self::ReplicaSet
            | Self::Pod => KindFilter::Workload,
            Self::ConfigMap | Self::Secret | Self::PersistentVolumeClaim => KindFilter::Config,
            Self::ServiceAccount
            | Self::RoleBinding
            | Self::ClusterRoleBinding
            | Self::Role
            | Self::ClusterRole => KindFilter::Rbac,
        }
    }

    /// The explorer kind whose watch feeds this one; `None` for pods, which the session lists.
    pub(crate) fn resource_kind(self) -> Option<ResourceKind> {
        Some(match self {
            Self::Ingress => ResourceKind::Ingresses,
            Self::HorizontalPodAutoscaler => ResourceKind::HorizontalPodAutoscalers,
            Self::Service => ResourceKind::Services,
            Self::Deployment => ResourceKind::Deployments,
            Self::StatefulSet => ResourceKind::StatefulSets,
            Self::DaemonSet => ResourceKind::DaemonSets,
            Self::ReplicaSet => ResourceKind::ReplicaSets,
            Self::Pod => return None,
            Self::ConfigMap => ResourceKind::ConfigMaps,
            Self::Secret => ResourceKind::Secrets,
            Self::PersistentVolumeClaim => ResourceKind::PersistentVolumeClaims,
            Self::ServiceAccount => ResourceKind::ServiceAccounts,
            Self::RoleBinding => ResourceKind::RoleBindings,
            Self::ClusterRoleBinding => ResourceKind::ClusterRoleBindings,
            Self::Role => ResourceKind::Roles,
            Self::ClusterRole => ResourceKind::ClusterRoles,
        })
    }

    /// The topology kind of an explorer kind that feeds the graph. ClusterRoles have no feed
    /// (decision 45), so they are not here.
    pub(crate) fn of_resource_kind(kind: ResourceKind) -> Option<Self> {
        [
            Self::Ingress,
            Self::HorizontalPodAutoscaler,
            Self::Service,
            Self::Deployment,
            Self::StatefulSet,
            Self::DaemonSet,
            Self::ReplicaSet,
            Self::ConfigMap,
            Self::Secret,
            Self::PersistentVolumeClaim,
            Self::ServiceAccount,
            Self::RoleBinding,
            Self::ClusterRoleBinding,
            Self::Role,
        ]
        .into_iter()
        .find(|candidate| candidate.resource_kind() == Some(kind))
    }

    /// The workload kind a controller or scale target reference names.
    fn of_workload_kind(kind: &str) -> Option<Self> {
        [
            Self::Deployment,
            Self::StatefulSet,
            Self::DaemonSet,
            Self::ReplicaSet,
        ]
        .into_iter()
        .find(|candidate| candidate.object_kind() == kind)
    }

    fn is_config(self) -> bool {
        self.placement() == Placement::ConfigRow
    }

    pub(crate) fn is_access(self) -> bool {
        self.placement() == Placement::AccessRow
    }

    /// Whether the kind's nodes appear only when something refers to them (decisions 8 and 43).
    fn appears_on_reference(self) -> bool {
        self.is_config() || self.is_access()
    }
}

/// The kind chips of the toolbar: each shows or hides a group of node kinds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum KindFilter {
    Ingress,
    Service,
    Workload,
    Config,
    Rbac,
}

impl KindFilter {
    pub(crate) const ALL: [Self; 5] = [
        Self::Ingress,
        Self::Service,
        Self::Workload,
        Self::Config,
        Self::Rbac,
    ];
    /// The chips that are on when Topology opens. RBAC costs four more watches, so it is asked for
    /// (decision 42).
    pub(crate) const DEFAULT: [Self; 4] =
        [Self::Ingress, Self::Service, Self::Workload, Self::Config];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Ingress => "Ingress",
            Self::Service => "Service",
            Self::Workload => "Workload",
            Self::Config => "Config",
            Self::Rbac => "RBAC",
        }
    }

    pub(crate) fn tooltip(self) -> String {
        match self {
            Self::Rbac => "Show service accounts, bindings, and roles".to_owned(),
            other => format!("Show {} nodes", other.label()),
        }
    }
}

/// Stable identity across rebuilds: pins, expansion, layout seeding, and selection use it.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum NodeId {
    Object {
        kind: TopologyKind,
        name: String,
    },
    PodGroup {
        owner_kind: TopologyKind,
        owner: String,
    },
    /// A ghost: referenced, not listed.
    Missing {
        kind: TopologyKind,
        name: String,
    },
    /// A ghost: the Service selects no pod (W11 "0 pods match").
    NoPods {
        service: String,
    },
}

impl NodeId {
    fn object(kind: TopologyKind, name: &str) -> Self {
        Self::Object {
            kind,
            name: name.to_owned(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NodeLook {
    Plain,
    Ghost,
    /// The kind's feed is loading or off, so the target could not be checked (decision 4).
    Unchecked,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TopologyNode {
    pub(crate) id: NodeId,
    pub(crate) kind: TopologyKind,
    pub(crate) look: NodeLook,
    /// The object name; a group reads `48 pods`; a `NoPods` ghost reads `selector app=x`.
    pub(crate) name: SharedString,
    /// `Deployment · 2/3`, `CrashLoopBackOff · api`, `Secret · not checked`.
    pub(crate) caption: SharedString,
    /// `None` is neutral.
    pub(crate) tone: Option<StatusTone>,
    /// The app label of the node, or of its first neighbour that has one.
    pub(crate) group: Option<String>,
    /// How many objects the node stands for: 1, a group's pods, 0 for ghosts and unchecked ones.
    pub(crate) objects: usize,
    /// The drawer and reveal target; `None` for ghosts, groups, and unchecked nodes.
    pub(crate) key: Option<ResourceKey>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum Relation {
    Owns,
    RoutesTo,
    Mounts,
    /// Workload to ServiceAccount, ServiceAccount to binding, binding to role.
    Access,
    /// Traffic mode only (spec 0049): a pair that talks with no Resources edge between its nodes.
    /// Never part of `TopologyGraph.edges`.
    Calls,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct TopologyEdge {
    pub(crate) from: usize,
    pub(crate) to: usize,
    pub(crate) relation: Relation,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct TopologyGraph {
    /// Sorted by `NodeId`.
    pub(crate) nodes: Vec<TopologyNode>,
    /// Sorted by `(from, to, relation)`, without duplicates.
    pub(crate) edges: Vec<TopologyEdge>,
    pub(crate) checks: Vec<ConfigCheck>,
    /// The sum of `objects` of the visible nodes.
    pub(crate) resources: usize,
}

impl TopologyGraph {
    /// The tone of the check of a ghost node, which an edge into it takes; `None` for any other
    /// node or a ghost without a check.
    pub(crate) fn ghost_tone(&self, node: usize) -> Option<StatusTone> {
        let node = &self.nodes[node];
        if node.look != NodeLook::Ghost {
            return None;
        }
        self.checks
            .iter()
            .find(|check| check.node == node.id)
            .map(|check| check.tone)
    }
}

pub(crate) enum TopologyBuild {
    Graph(TopologyGraph),
    TooLarge(TooLarge),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TooLarge {
    /// The listed rows plus namespace pods exceed `RAW_LIMIT`.
    Objects(usize),
    /// The visible nodes exceed `NODE_LIMIT`.
    Nodes(usize),
}

/// What one kind's feed holds. A kind whose chip is off has no entry at all.
pub(crate) enum FeedRows<'a> {
    Ready(&'a [KindRow]),
    Loading,
    /// The watch failed before its first snapshot and keeps retrying.
    Failed,
    /// The feed does not run: the access review denies the list.
    Off,
}

pub(crate) struct TopologyInputs<'a> {
    pub(crate) namespace: &'a str,
    /// `None` until the pods list has loaded; pods of other namespaces are skipped.
    pub(crate) pods: Option<&'a [PodSummary]>,
    pub(crate) rows: &'a [(TopologyKind, FeedRows<'a>)],
    pub(crate) nodes: &'a [NodeSummary],
    pub(crate) filter: &'a TopologyFilter,
    pub(crate) expanded: &'a BTreeSet<NodeId>,
    pub(crate) now: Timestamp,
}

pub(crate) struct TopologyFilter {
    pub(crate) kinds: BTreeSet<KindFilter>,
    pub(crate) problems_only: bool,
    /// `None` picks automatically (decision 27).
    pub(crate) group_by: Option<GroupBy>,
}

impl TopologyFilter {
    /// The default chips on, nothing filtered, the group chosen automatically.
    pub(crate) fn initial() -> Self {
        Self {
            kinds: KindFilter::DEFAULT.into_iter().collect(),
            problems_only: false,
            group_by: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GroupBy {
    App,
    Components,
}

impl GroupBy {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::App => "app",
            Self::Components => "components",
        }
    }
}

/// The choice, else `App` when a pod of the namespace has an app label, else `Components`: one big
/// "Ungrouped" band would say nothing (decision 27).
pub(crate) fn resolve_group_by(
    choice: Option<GroupBy>,
    namespace: &str,
    pods: &[PodSummary],
) -> GroupBy {
    if let Some(choice) = choice {
        return choice;
    }
    let has_app_label = pods
        .iter()
        .any(|pod| pod.namespace == namespace && app_label(&pod.labels).is_some());
    if has_app_label {
        GroupBy::App
    } else {
        GroupBy::Components
    }
}

/// The value of the first of `APP_LABEL_KEYS` that `labels` (`key=value` terms) has.
fn app_label(labels: &[String]) -> Option<String> {
    APP_LABEL_KEYS.iter().find_map(|key| {
        labels.iter().find_map(|term| {
            let (name, value) = term.split_once('=')?;
            (name == *key && !value.is_empty()).then(|| value.to_owned())
        })
    })
}

/// Only a failing tone colors a node.
fn problem_tone(tone: StatusTone) -> Option<StatusTone> {
    matches!(tone, StatusTone::Bad | StatusTone::Warn).then_some(tone)
}

/// The graph of `inputs`: nodes, edges, checks, then the filters. Nodes are sorted by `NodeId` and
/// edges by `(from, to, relation)`, so the input order does not change the result.
pub(crate) fn build_topology(inputs: &TopologyInputs) -> TopologyBuild {
    let namespace_pods: Vec<&PodSummary> = inputs
        .pods
        .unwrap_or_default()
        .iter()
        .filter(|pod| pod.namespace == inputs.namespace)
        .collect();
    let listed: usize = inputs
        .rows
        .iter()
        .map(|(kind, feed)| match feed {
            FeedRows::Ready(rows) => counted_rows(*kind, rows, inputs.namespace),
            FeedRows::Loading | FeedRows::Failed | FeedRows::Off => 0,
        })
        .sum();
    let raw = listed + namespace_pods.len();
    if raw > RAW_LIMIT {
        return TopologyBuild::TooLarge(TooLarge::Objects(raw));
    }
    let is_rbac = inputs.filter.kinds.contains(&KindFilter::Rbac);
    // Only the bindings of the layer are copied, and only while its chip is on.
    let bindings = if is_rbac {
        AccessBindings::collect(all_rows(inputs), inputs.namespace)
    } else {
        AccessBindings::default()
    };
    let access = BindingIndex::build(&bindings.lists());
    let mut parts = GraphParts::new(inputs, &namespace_pods);
    let builder = Builder::new(inputs, &namespace_pods, &access, is_rbac);
    builder.add_rows(&mut parts);
    builder.add_pods(&mut parts);
    builder.add_owner_edges(&mut parts);
    builder.add_service_edges(&mut parts);
    builder.add_ingress_edges(&mut parts);
    builder.add_scale_targets(&mut parts);
    builder.add_config_refs(&mut parts);
    builder.add_access_edges(&mut parts);
    builder.add_unbound_claims(&mut parts);
    let checks = graph_checks(&parts, inputs);
    parts.raise_tones(&checks);
    finish(parts, checks, inputs)
}

/// How many rows of a feed count toward `RAW_LIMIT`. The ClusterRoleBindings list is cluster wide,
/// so only the bindings that name an account of the namespace count (decision 50); the whole list
/// stays in the feed's memory.
fn counted_rows(kind: TopologyKind, rows: &[KindRow], namespace: &str) -> usize {
    if kind != TopologyKind::ClusterRoleBinding {
        return rows.len();
    }
    rows.iter()
        .filter(|row| {
            matches!(&row.object, KindObject::Binding(binding)
                if names_namespace_account(binding, namespace))
        })
        .count()
}

/// Whether a feed can answer "is it listed".
fn is_ready(inputs: &TopologyInputs, kind: TopologyKind) -> bool {
    inputs
        .rows
        .iter()
        .any(|(feed_kind, feed)| *feed_kind == kind && matches!(feed, FeedRows::Ready(_)))
}

/// The state of one build: the lookups every rule reads.
struct Builder<'a> {
    inputs: &'a TopologyInputs<'a>,
    pods: &'a [&'a PodSummary],
    /// The owner of each ReplicaSet row, by name.
    replica_set_owners: HashMap<&'a str, &'a ControllerRef>,
    /// The collapsed pod sets, by controller `(kind, name)`.
    collapsed: HashSet<(TopologyKind, &'a str)>,
    /// The first ingress path that routes to each Service.
    ingress_paths: HashMap<&'a str, &'a str>,
    /// The roles each account holds, from the bindings the RBAC layer reads.
    access: &'a BindingIndex<'a>,
    /// Whether the RBAC chip is on: without it no account, binding, or role is drawn.
    is_rbac: bool,
}

impl<'a> Builder<'a> {
    fn new(
        inputs: &'a TopologyInputs<'a>,
        pods: &'a [&'a PodSummary],
        access: &'a BindingIndex<'a>,
        is_rbac: bool,
    ) -> Self {
        let mut replica_set_owners = HashMap::new();
        let mut ingress_paths: HashMap<&str, &str> = HashMap::new();
        for (_, row) in all_rows(inputs) {
            match &row.object {
                KindObject::ReplicaSet(set) => {
                    if let Some(owner) = &set.owner {
                        replica_set_owners.insert(set.name.as_str(), owner);
                    }
                }
                KindObject::Ingress(ingress) => {
                    for rule in &ingress.rules {
                        if let (Some(service), Some(path)) = (&rule.service, &rule.path) {
                            ingress_paths.entry(service).or_insert(path);
                        }
                    }
                }
                _ => {}
            }
        }
        let mut healthy_pods: HashMap<(TopologyKind, &str), usize> = HashMap::new();
        for pod in pods {
            if let Some((kind, name)) = controller_of(pod)
                && !is_bad(pod)
            {
                *healthy_pods.entry((kind, name)).or_default() += 1;
            }
        }
        let collapsed = healthy_pods
            .into_iter()
            .filter(|((kind, name), count)| {
                let id = NodeId::PodGroup {
                    owner_kind: *kind,
                    owner: (*name).to_owned(),
                };
                *count > POD_GROUP_LIMIT && !inputs.expanded.contains(&id)
            })
            .map(|(key, _)| key)
            .collect();
        Self {
            inputs,
            pods,
            replica_set_owners,
            collapsed,
            ingress_paths,
            access,
            is_rbac,
        }
    }

    /// The node a pod is drawn as: itself, or its collapsed set (failing pods stay single).
    fn pod_node(&self, pod: &PodSummary) -> NodeId {
        if let Some((owner_kind, owner)) = controller_of(pod)
            && !is_bad(pod)
            && self.collapsed.contains(&(owner_kind, owner))
        {
            return NodeId::PodGroup {
                owner_kind,
                owner: owner.to_owned(),
            };
        }
        NodeId::object(TopologyKind::Pod, &pod.name)
    }

    /// The nodes of every listed row except the lazy kinds: the config and access kinds, which
    /// appear only when something refers to them, and inactive ReplicaSets (decision 7).
    fn add_rows(&self, parts: &mut GraphParts) {
        for (kind, row) in all_rows(self.inputs) {
            if kind.appears_on_reference() {
                continue;
            }
            if let KindObject::ReplicaSet(set) = &row.object
                && set.desired == 0
                && set.current == 0
            {
                continue;
            }
            if let Some(node) = self.row_node(kind, row) {
                parts.nodes.insert(node.id.clone(), node);
            }
        }
    }

    /// The node of a listed row; `None` for a row whose object does not match its kind.
    fn row_node(&self, kind: TopologyKind, row: &KindRow) -> Option<TopologyNode> {
        let (extra, labels): (String, &[String]) = match &row.object {
            KindObject::Ingress(ingress) => (
                ingress.hosts.first().cloned().unwrap_or_default(),
                &ingress.labels,
            ),
            KindObject::Service(service) => (
                self.ingress_paths
                    .get(service.name.as_str())
                    .map_or_else(|| service.service_type.clone(), |path| (*path).to_owned()),
                &service.labels,
            ),
            KindObject::Deployment(deployment) => (
                format!("{}/{}", deployment.ready, deployment.desired),
                &deployment.labels,
            ),
            KindObject::StatefulSet(set) => (format!("{}/{}", set.ready, set.desired), &set.labels),
            KindObject::DaemonSet(set) => (format!("{}/{}", set.ready, set.desired), &set.labels),
            KindObject::ReplicaSet(set) => (
                set.revision
                    .as_ref()
                    .map_or_else(String::new, |revision| format!("rev {revision}")),
                &set.labels,
            ),
            KindObject::HorizontalPodAutoscaler(hpa) => (
                format!("{}\u{2013}{}", hpa.min_replicas, hpa.max_replicas),
                &hpa.labels,
            ),
            KindObject::PersistentVolumeClaim(claim) => (claim.phase.clone(), &claim.labels),
            KindObject::ConfigMap(map) => (String::new(), &map.labels),
            KindObject::Secret(secret) => (secret.secret_type.clone(), &secret.labels),
            KindObject::ServiceAccount(account) => (self.account_extra(account), &account.labels),
            KindObject::Binding(binding) => (String::new(), &binding.labels),
            KindObject::Role(role) => (
                format!(
                    "{} {}",
                    role.rules.len(),
                    if role.rules.len() == 1 {
                        "rule"
                    } else {
                        "rules"
                    }
                ),
                &role.labels,
            ),
            _ => return None,
        };
        Some(TopologyNode {
            id: NodeId::object(kind, &row.name),
            kind,
            look: NodeLook::Plain,
            name: row.name.clone().into(),
            caption: caption(kind, &extra).into(),
            tone: problem_tone(row.status.tone),
            // An access node joins the band of the workload that leads to it (spread_groups), so the
            // access row stays under its app; its own labels name the operator that made it.
            group: if kind.is_access() {
                None
            } else {
                app_label(labels)
            },
            objects: 1,
            key: ResourceKey::of_object(kind.object_kind(), Some(self.inputs.namespace), &row.name),
        })
    }

    /// One node per pod, or per collapsed set.
    fn add_pods(&self, parts: &mut GraphParts) {
        let mut groups: BTreeMap<NodeId, Vec<&PodSummary>> = BTreeMap::new();
        for pod in self.pods {
            let id = self.pod_node(pod);
            if matches!(id, NodeId::PodGroup { .. }) {
                groups.entry(id).or_default().push(pod);
                continue;
            }
            let label = pod_status_label(pod);
            let caption =
                if label.tone == StatusTone::Ok {
                    format!("{} \u{b7} {}", label.text, pod.ready)
                } else {
                    match pod.containers.iter().find(|container| {
                        container.kind != ContainerKind::Init && !container.is_ready
                    }) {
                        Some(container) => format!("{} \u{b7} {}", label.text, container.name),
                        None => label.text.to_string(),
                    }
                };
            parts.nodes.insert(
                id.clone(),
                TopologyNode {
                    id,
                    kind: TopologyKind::Pod,
                    look: NodeLook::Plain,
                    name: pod.name.clone().into(),
                    caption: caption.into(),
                    tone: problem_tone(label.tone),
                    group: app_label(&pod.labels),
                    objects: 1,
                    key: Some(ResourceKey::of_pod(pod)),
                },
            );
        }
        for (id, members) in groups {
            let tones: Vec<StatusTone> = members
                .iter()
                .map(|pod| pod_status_label(pod).tone)
                .collect();
            let running = tones.iter().filter(|tone| **tone == StatusTone::Ok).count();
            let failing = tones
                .iter()
                .filter(|tone| **tone == StatusTone::Warn)
                .count();
            let caption = if failing == 0 {
                format!("{running} running")
            } else {
                format!("{running} running \u{b7} {failing} failing")
            };
            parts.nodes.insert(
                id.clone(),
                TopologyNode {
                    id,
                    kind: TopologyKind::Pod,
                    look: NodeLook::Plain,
                    name: format!("{} pods", members.len()).into(),
                    caption: caption.into(),
                    tone: (failing > 0).then_some(StatusTone::Warn),
                    group: members.iter().find_map(|pod| app_label(&pod.labels)),
                    objects: members.len(),
                    key: None,
                },
            );
        }
    }

    /// Deployment to ReplicaSet, and ReplicaSet, StatefulSet, DaemonSet to their pods.
    fn add_owner_edges(&self, parts: &mut GraphParts) {
        for (_, row) in all_rows(self.inputs) {
            let KindObject::ReplicaSet(set) = &row.object else {
                continue;
            };
            let Some(owner) = set
                .owner
                .as_ref()
                .filter(|owner| owner.kind == "Deployment")
            else {
                continue;
            };
            let from = NodeId::object(TopologyKind::Deployment, &owner.name);
            let to = NodeId::object(TopologyKind::ReplicaSet, &set.name);
            if parts.nodes.contains_key(&from) && parts.nodes.contains_key(&to) {
                parts.edges.insert((from, to, Relation::Owns));
            }
        }
        for pod in self.pods {
            let Some((kind, name)) = controller_of(pod) else {
                continue;
            };
            let from = NodeId::object(kind, name);
            if parts.nodes.contains_key(&from) {
                parts
                    .edges
                    .insert((from, self.pod_node(pod), Relation::Owns));
            }
        }
    }

    /// Service to the pods its selector matches, or to a `NoPods` ghost.
    fn add_service_edges(&self, parts: &mut GraphParts<'a>) {
        let Some(label_index) = self.label_index() else {
            return;
        };
        for (_, row) in all_rows(self.inputs) {
            let KindObject::Service(service) = &row.object else {
                continue;
            };
            let matching = self.matching_pods(service, &label_index);
            let from = NodeId::object(TopologyKind::Service, &service.name);
            let Some(matching) = matching else {
                parts.service_pods.insert(service.name.as_str(), Vec::new());
                continue;
            };
            if matching.is_empty() {
                let to = NodeId::NoPods {
                    service: service.name.clone(),
                };
                let terms = service.selector.join(", ");
                parts.nodes.insert(
                    to.clone(),
                    TopologyNode {
                        id: to.clone(),
                        kind: TopologyKind::Pod,
                        look: NodeLook::Ghost,
                        name: format!("selector {terms}").into(),
                        caption: "0 pods match".into(),
                        tone: None,
                        group: None,
                        objects: 0,
                        key: None,
                    },
                );
                parts.edges.insert((from.clone(), to, Relation::RoutesTo));
            }
            for pod in &matching {
                parts
                    .edges
                    .insert((from.clone(), self.pod_node(pod), Relation::RoutesTo));
            }
            parts.service_pods.insert(service.name.as_str(), matching);
        }
    }

    /// The namespace pods by label term, so a Service reads only the pods that carry its first
    /// term instead of every pod. `None` until the pods list has loaded.
    fn label_index(&self) -> Option<HashMap<&'a str, Vec<&'a PodSummary>>> {
        self.inputs.pods?;
        let mut index: HashMap<&str, Vec<&PodSummary>> = HashMap::new();
        for pod in self.pods {
            for term in &pod.labels {
                index.entry(term.as_str()).or_default().push(pod);
            }
        }
        Some(index)
    }

    /// The pods `service` selects; `None` for a Service that selects nothing by design (no
    /// selector, or an ExternalName).
    fn matching_pods(
        &self,
        service: &ServiceSummary,
        label_index: &HashMap<&'a str, Vec<&'a PodSummary>>,
    ) -> Option<Vec<&'a PodSummary>> {
        if service.service_type == EXTERNAL_NAME {
            return None;
        }
        let selector = Selector::of_labels(&service.selector)?;
        let candidates = label_index
            .get(service.selector.first()?.as_str())
            .map_or(&[][..], Vec::as_slice);
        Some(
            candidates
                .iter()
                .copied()
                .filter(|pod| selector.matches(&pod.labels))
                .collect(),
        )
    }

    /// Ingress to its backend Services and TLS Secrets.
    fn add_ingress_edges(&self, parts: &mut GraphParts) {
        for (_, row) in all_rows(self.inputs) {
            let KindObject::Ingress(ingress) = &row.object else {
                continue;
            };
            let from = NodeId::object(TopologyKind::Ingress, &ingress.name);
            let services: BTreeSet<&str> = ingress
                .rules
                .iter()
                .filter_map(|rule| rule.service.as_deref())
                .chain(ingress.default_service.as_deref())
                .collect();
            for service in services {
                if let Some(to) = self.target(parts, TopologyKind::Service, service) {
                    parts.edges.insert((from.clone(), to, Relation::RoutesTo));
                }
            }
            let secrets: BTreeSet<&str> = ingress
                .tls
                .iter()
                .filter_map(|tls| tls.secret_name.as_deref())
                .filter(|name| !name.is_empty())
                .collect();
            for secret in secrets {
                if let Some(to) = self.target(parts, TopologyKind::Secret, secret) {
                    parts.edges.insert((from.clone(), to, Relation::Mounts));
                }
            }
        }
    }

    /// HPA to the Deployment, StatefulSet, or ReplicaSet it scales.
    fn add_scale_targets(&self, parts: &mut GraphParts) {
        for (_, row) in all_rows(self.inputs) {
            let KindObject::HorizontalPodAutoscaler(hpa) = &row.object else {
                continue;
            };
            let Some(kind) = TopologyKind::of_workload_kind(&hpa.target.kind) else {
                continue;
            };
            let from = NodeId::object(TopologyKind::HorizontalPodAutoscaler, &hpa.name);
            if let Some(to) = self.target(parts, kind, &hpa.target.name) {
                parts.edges.insert((from, to, Relation::Owns));
            }
        }
    }

    /// Config refs of the pods, aggregated to the top visible workload (decision 12).
    fn add_config_refs(&self, parts: &mut GraphParts) {
        let mut refs: BTreeMap<NodeId, BTreeSet<(TopologyKind, &'a str)>> = BTreeMap::new();
        for pod in self.pods {
            let mut pod_refs = pod_refs(pod);
            if self.is_rbac {
                pod_refs.insert((TopologyKind::ServiceAccount, pod_account(pod)));
            }
            if pod_refs.is_empty() {
                continue;
            }
            let source = self.top_owner(pod, parts);
            refs.entry(source).or_default().extend(pod_refs);
        }
        for (source, targets) in refs {
            for (kind, name) in targets {
                let relation = if kind == TopologyKind::ServiceAccount {
                    Relation::Access
                } else {
                    Relation::Mounts
                };
                if let Some(to) = self.target(parts, kind, name) {
                    parts.edges.insert((source.clone(), to, relation));
                }
            }
        }
    }

    /// The caption words after `ServiceAccount`: the groups that reach it (not drawn, decision 44)
    /// and a token that is not mounted.
    fn account_extra(&self, account: &ServiceAccountSummary) -> String {
        let mut extra = Vec::new();
        let via_groups = self
            .access
            .bound_roles(&account.namespace, &account.name)
            .iter()
            .filter(|bound| bound.group.is_some())
            .count();
        if via_groups > 0 {
            extra.push(format!("+{via_groups} via groups"));
        }
        if account.automount_token == Some(false) {
            extra.push("token off".to_owned());
        }
        extra.join(" \u{b7} ")
    }

    /// Each drawn account to the bindings that name it directly, and each binding to its role. A
    /// group binding is not drawn (decision 44), but a cluster-admin grant through one is checked.
    fn add_access_edges(&self, parts: &mut GraphParts) {
        let accounts: Vec<NodeId> = parts
            .nodes
            .values()
            .filter(|node| node.kind == TopologyKind::ServiceAccount)
            .map(|node| node.id.clone())
            .collect();
        for account in accounts {
            let (NodeId::Object { name, .. } | NodeId::Missing { name, .. }) = &account else {
                continue;
            };
            for bound in self.access.bound_roles(self.inputs.namespace, name) {
                if bound.group.is_some() {
                    continue;
                }
                let Some(binding) = binding_node(&bound.binding).and_then(|id| match id {
                    NodeId::Object { kind, name } => self.ensure_node(parts, kind, &name),
                    _ => None,
                }) else {
                    continue;
                };
                parts
                    .edges
                    .insert((account.clone(), binding.clone(), Relation::Access));
                if let Some(role) = self.role_node(parts, bound) {
                    parts.edges.insert((binding, role, Relation::Access));
                }
            }
            if let Some(grant) =
                cluster_admin_grant(self.access, self.inputs.namespace, &account, name)
            {
                parts.grants.push(grant);
            }
        }
    }

    /// The node of the role a binding grants: a Role as `target` finds it, a ClusterRole as a
    /// plain node (it has no feed, decision 45). `None` for a kind the API rejects.
    fn role_node(&self, parts: &mut GraphParts, bound: &BoundRole) -> Option<NodeId> {
        match &bound.role.kind {
            RoleKind::Role => self.target(parts, TopologyKind::Role, &bound.role.name),
            RoleKind::ClusterRole => {
                let name = &bound.role.name;
                let id = NodeId::object(TopologyKind::ClusterRole, name);
                parts
                    .nodes
                    .entry(id.clone())
                    .or_insert_with(|| TopologyNode {
                        id: id.clone(),
                        kind: TopologyKind::ClusterRole,
                        look: NodeLook::Plain,
                        name: name.clone().into(),
                        caption: caption(TopologyKind::ClusterRole, "").into(),
                        tone: None,
                        group: None,
                        objects: 1,
                        key: ResourceKey::of_object(
                            TopologyKind::ClusterRole.object_kind(),
                            None,
                            name,
                        ),
                    });
                Some(id)
            }
            RoleKind::Other(_) => None,
        }
    }

    /// The Deployment behind a pod's ReplicaSet, else its controller, when that is a node; else the
    /// pod itself (or its group).
    fn top_owner(&self, pod: &PodSummary, parts: &GraphParts) -> NodeId {
        if let Some((kind, name)) = controller_of(pod) {
            if kind == TopologyKind::ReplicaSet
                && let Some(owner) = self.replica_set_owners.get(name)
                && owner.kind == "Deployment"
            {
                let id = NodeId::object(TopologyKind::Deployment, &owner.name);
                if parts.nodes.contains_key(&id) {
                    return id;
                }
            }
            let id = NodeId::object(kind, name);
            if parts.nodes.contains_key(&id) {
                return id;
            }
        }
        self.pod_node(pod)
    }

    /// A claim that is not bound shows even when nothing mounts it.
    fn add_unbound_claims(&self, parts: &mut GraphParts) {
        for (kind, row) in all_rows(self.inputs) {
            let KindObject::PersistentVolumeClaim(claim) = &row.object else {
                continue;
            };
            if claim_problem(claim, self.inputs.now).is_some() {
                self.ensure_node(parts, kind, &claim.name);
            }
        }
    }

    /// The node a reference to `kind`/`name` points at: the listed object (config kinds appear
    /// now), a `Missing` ghost when the feed is ready and does not list it, or an unchecked node
    /// while the feed cannot answer. `None` for a listed object that has no node (an inactive
    /// ReplicaSet).
    fn target(&self, parts: &mut GraphParts, kind: TopologyKind, name: &str) -> Option<NodeId> {
        if kind == TopologyKind::ConfigMap && name == ROOT_CA_CONFIG_MAP {
            return None;
        }
        let id = NodeId::object(kind, name);
        if parts.nodes.contains_key(&id) {
            return Some(id);
        }
        if !is_ready(self.inputs, kind) {
            parts.nodes.insert(
                id.clone(),
                TopologyNode {
                    id: id.clone(),
                    kind,
                    look: NodeLook::Unchecked,
                    name: name.to_owned().into(),
                    caption: format!("{} \u{b7} not checked", kind.caption_label()).into(),
                    tone: None,
                    group: None,
                    objects: 0,
                    key: None,
                },
            );
            return Some(id);
        }
        if parts.is_listed(kind, name) {
            return self.ensure_node(parts, kind, name);
        }
        let ghost = NodeId::Missing {
            kind,
            name: name.to_owned(),
        };
        parts
            .nodes
            .entry(ghost.clone())
            .or_insert_with(|| TopologyNode {
                id: ghost.clone(),
                kind,
                look: NodeLook::Ghost,
                name: name.to_owned().into(),
                caption: format!("Missing {}", kind.caption_label()).into(),
                tone: None,
                group: None,
                objects: 0,
                key: None,
            });
        Some(ghost)
    }

    /// Creates the node of a listed config object on first use. A listed object of another kind
    /// has its node already, or is hidden on purpose.
    fn ensure_node(
        &self,
        parts: &mut GraphParts,
        kind: TopologyKind,
        name: &str,
    ) -> Option<NodeId> {
        let id = NodeId::object(kind, name);
        if parts.nodes.contains_key(&id) {
            return Some(id);
        }
        if !kind.appears_on_reference() {
            return None;
        }
        let row = parts.row(kind, name)?;
        let node = self.row_node(kind, row)?;
        parts.nodes.insert(id.clone(), node);
        Some(id)
    }
}

/// `{Kind} · {extra}`, or the kind alone without an extra.
fn caption(kind: TopologyKind, extra: &str) -> String {
    if extra.is_empty() {
        kind.caption_label().to_owned()
    } else {
        format!("{} \u{b7} {extra}", kind.caption_label())
    }
}

fn is_bad(pod: &PodSummary) -> bool {
    pod_status_label(pod).tone == StatusTone::Bad
}

/// The controller of a pod when it is a workload that has a node of its own.
pub(crate) fn controller_of(pod: &PodSummary) -> Option<(TopologyKind, &str)> {
    let controller = pod.controller.as_ref()?;
    let kind = TopologyKind::of_workload_kind(&controller.kind)?;
    (kind != TopologyKind::Deployment).then_some((kind, controller.name.as_str()))
}

/// Every listed row with its kind, in feed order.
pub(crate) fn all_rows<'a>(
    inputs: &'a TopologyInputs<'a>,
) -> impl Iterator<Item = (TopologyKind, &'a KindRow)> {
    inputs.rows.iter().flat_map(|(kind, feed)| {
        let rows: &[KindRow] = match feed {
            FeedRows::Ready(rows) => rows,
            FeedRows::Loading | FeedRows::Failed | FeedRows::Off => &[],
        };
        rows.iter().map(|row| (*kind, row))
    })
}

/// The config objects a pod refers to, from env, envFrom, volume mounts, and pull secrets.
fn pod_refs(pod: &PodSummary) -> BTreeSet<(TopologyKind, &str)> {
    let mut refs = BTreeSet::new();
    for container in &pod.containers {
        for env in &container.env {
            match &env.source {
                EnvSource::ConfigMapKey { name, .. } => {
                    refs.insert((TopologyKind::ConfigMap, name.as_str()));
                }
                EnvSource::SecretKey { name, .. } => {
                    refs.insert((TopologyKind::Secret, name.as_str()));
                }
                EnvSource::Literal
                | EnvSource::Field { .. }
                | EnvSource::ResourceField { .. }
                | EnvSource::Unknown => {}
            }
        }
        for entry in &container.env_from {
            match &entry.source {
                EnvFromSource::ConfigMap { name } => {
                    refs.insert((TopologyKind::ConfigMap, name.as_str()));
                }
                EnvFromSource::Secret { name } => {
                    refs.insert((TopologyKind::Secret, name.as_str()));
                }
                EnvFromSource::Unknown => {}
            }
        }
        for mount in &container.mounts {
            match &mount.source {
                VolumeSource::ConfigMap { name } => {
                    refs.insert((TopologyKind::ConfigMap, name.as_str()));
                }
                VolumeSource::Secret { name } => {
                    refs.insert((TopologyKind::Secret, name.as_str()));
                }
                VolumeSource::PersistentVolumeClaim { claim } => {
                    refs.insert((TopologyKind::PersistentVolumeClaim, claim.as_str()));
                }
                VolumeSource::Projected {
                    config_maps,
                    secrets,
                } => {
                    refs.extend(
                        config_maps
                            .iter()
                            .map(|name| (TopologyKind::ConfigMap, name.as_str())),
                    );
                    refs.extend(
                        secrets
                            .iter()
                            .map(|name| (TopologyKind::Secret, name.as_str())),
                    );
                }
                VolumeSource::EmptyDir
                | VolumeSource::HostPath { .. }
                | VolumeSource::DownwardApi
                | VolumeSource::Other => {}
            }
        }
    }
    refs.extend(
        pod.image_pull_secrets
            .iter()
            .map(|name| (TopologyKind::Secret, name.as_str())),
    );
    refs.retain(|(kind, name)| !(*kind == TopologyKind::ConfigMap && *name == ROOT_CA_CONFIG_MAP));
    refs
}

/// Applies the filters to the parts and numbers the result: the visible nodes in `NodeId` order,
/// the edges between them, and the checks that are drawn.
fn finish(parts: GraphParts, checks: Vec<ConfigCheck>, inputs: &TopologyInputs) -> TopologyBuild {
    let filter = inputs.filter;
    let GraphParts { nodes, edges, .. } = parts;
    let mut kept: BTreeSet<NodeId> = nodes
        .values()
        .filter(|node| filter.kinds.contains(&node.kind.filter()))
        .map(|node| node.id.clone())
        .collect();
    let has_edge_between = |edge: &(NodeId, NodeId, Relation), kept: &BTreeSet<NodeId>| {
        kept.contains(&edge.0) && kept.contains(&edge.1)
    };
    if filter.problems_only {
        let problems: BTreeSet<&NodeId> = nodes
            .values()
            .filter(|node| {
                kept.contains(&node.id)
                    && (node.tone.is_some() || checks.iter().any(|check| check.node == node.id))
            })
            .map(|node| &node.id)
            .collect();
        let mut with_neighbours: BTreeSet<NodeId> =
            problems.iter().map(|id| (*id).clone()).collect();
        for edge in edges.iter().filter(|edge| has_edge_between(edge, &kept)) {
            if problems.contains(&edge.0) {
                with_neighbours.insert(edge.1.clone());
            }
            if problems.contains(&edge.1) {
                with_neighbours.insert(edge.0.clone());
            }
        }
        kept = with_neighbours;
    }
    // A config object that no visible node refers to is noise, unless it has a check (decision 8);
    // ghosts and unchecked nodes need a visible node that refers to them.
    let connected: BTreeSet<&NodeId> = edges
        .iter()
        .filter(|edge| has_edge_between(edge, &kept))
        .flat_map(|edge| [&edge.0, &edge.1])
        .collect();
    let orphans: Vec<NodeId> = kept
        .iter()
        .filter(|id| {
            let Some(node) = nodes.get(*id) else {
                return false;
            };
            if connected.contains(id) {
                return false;
            }
            // A ghost or an unchecked node stands in for what a visible node refers to; without
            // one it has no place. A config object may stay when it has a check of its own.
            let is_stand_in = node.look != NodeLook::Plain;
            let has_check = checks.iter().any(|check| check.node == **id);
            is_stand_in || (node.kind.appears_on_reference() && !has_check)
        })
        .cloned()
        .collect();
    for id in &orphans {
        kept.remove(id);
    }
    if kept.len() > NODE_LIMIT {
        return TopologyBuild::TooLarge(TooLarge::Nodes(kept.len()));
    }
    let index: HashMap<&NodeId, usize> = kept.iter().enumerate().map(|(i, id)| (id, i)).collect();
    let mut visible: Vec<TopologyNode> = nodes
        .iter()
        .filter(|(id, _)| kept.contains(*id))
        .map(|(_, node)| node.clone())
        .collect();
    let mut edges: Vec<TopologyEdge> = edges
        .iter()
        .filter_map(|(from, to, relation)| {
            Some(TopologyEdge {
                from: *index.get(from)?,
                to: *index.get(to)?,
                relation: *relation,
            })
        })
        .collect();
    edges.sort();
    spread_groups(&mut visible, &edges);
    let checks = checks
        .into_iter()
        .filter(|check| index.contains_key(&check.node))
        .collect();
    let resources = visible.iter().map(|node| node.objects).sum();
    TopologyBuild::Graph(TopologyGraph {
        nodes: visible,
        edges,
        checks,
        resources,
    })
}

/// A node without an app label takes the group of its first neighbour that has one: one pass over
/// the edges in `(from, to)` order.
fn spread_groups(nodes: &mut [TopologyNode], edges: &[TopologyEdge]) {
    for edge in edges {
        let (from, to) = (edge.from, edge.to);
        let from_group = nodes[from].group.clone();
        let to_group = nodes[to].group.clone();
        match (from_group, to_group) {
            (None, Some(group)) => nodes[from].group = Some(group),
            (Some(group), None) => nodes[to].group = Some(group),
            _ => {}
        }
    }
}

/// What a service selects, as `ServiceHealth` reads it (the 0012 slice core agrees: see the test).
pub(crate) fn health_of_matches(matching: Option<&[&PodSummary]>) -> ServiceHealth {
    ServiceHealth {
        matching_pods: matching.map(<[_]>::len),
        endpoints: None,
    }
}

#[cfg(test)]
#[path = "topology_graph_tests.rs"]
mod topology_graph_tests;
