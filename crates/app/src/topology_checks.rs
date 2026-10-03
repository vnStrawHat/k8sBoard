//! The config checks drawn in the Topology graph (W11 pin 2): Services that select no pod, refs to
//! objects that are not there, claims that are not bound, and the WHY boxes of the kind
//! diagnosis. Pure: no GPUI context, no logging.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use cluster::{PersistentVolumeClaimSummary, PodSummary, SecretSummary};
use jiff::Timestamp;

use crate::age::format_age;
use crate::issue_kind_rules::PVC_PENDING_GRACE;
use crate::kind_diagnosis::{CERTIFICATE_TITLE, DiagnosisInputs, NO_TLS_SECRET, kind_diagnosis};
use crate::kind_row::{KindObject, KindRow};
use crate::status_tone::StatusTone;
use crate::topology_access::ClusterAdminGrant;
use crate::topology_graph::{
    FeedRows, NodeId, NodeLook, Relation, TopologyInputs, TopologyKind, TopologyNode, all_rows,
    health_of_matches,
};

const TLS_SECRET_TYPE: &str = "kubernetes.io/tls";
/// The title of the Service box about a selector that finds no pod; `ServiceNoPods` covers it.
const NO_MATCHING_PODS_TITLE: &str = "NO MATCHING PODS";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum CheckRule {
    ServiceNoPods,
    IngressMissingService,
    MissingConfigMap,
    MissingSecret,
    MissingClaim,
    ClaimNotBound,
    HpaMissingTarget,
    ObjectDiagnosis,
    MissingServiceAccount,
    MissingRole,
    ClusterAdminAccount,
}

impl CheckRule {
    /// The short text of the checks chip for `count` checks of this rule.
    pub(crate) fn chip_label(self, count: usize) -> String {
        let is_one = count == 1;
        match self {
            Self::ServiceNoPods if is_one => "1 Service matches no pods".to_owned(),
            Self::ServiceNoPods => format!("{count} Services match no pods"),
            Self::IngressMissingService if is_one => "1 Ingress to a missing Service".to_owned(),
            Self::IngressMissingService => {
                format!("{count} Ingress routes to missing Services")
            }
            Self::MissingConfigMap if is_one => "1 missing ConfigMap".to_owned(),
            Self::MissingConfigMap => format!("{count} missing ConfigMaps"),
            Self::MissingSecret if is_one => "1 missing Secret".to_owned(),
            Self::MissingSecret => format!("{count} missing Secrets"),
            Self::MissingClaim if is_one => "1 missing PVC".to_owned(),
            Self::MissingClaim => format!("{count} missing PVCs"),
            Self::ClaimNotBound if is_one => "1 PVC not bound".to_owned(),
            Self::ClaimNotBound => format!("{count} PVCs not bound"),
            Self::HpaMissingTarget if is_one => "1 HPA without a target".to_owned(),
            Self::HpaMissingTarget => format!("{count} HPAs without targets"),
            Self::ObjectDiagnosis if is_one => "1 object problem".to_owned(),
            Self::ObjectDiagnosis => format!("{count} object problems"),
            Self::MissingServiceAccount if is_one => "1 missing ServiceAccount".to_owned(),
            Self::MissingServiceAccount => format!("{count} missing ServiceAccounts"),
            Self::MissingRole if is_one => "1 binding to a missing Role".to_owned(),
            Self::MissingRole => format!("{count} bindings to missing Roles"),
            Self::ClusterAdminAccount if is_one => "1 account with cluster-admin".to_owned(),
            Self::ClusterAdminAccount => format!("{count} accounts with cluster-admin"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ConfigCheck {
    pub(crate) rule: CheckRule,
    /// Where it is drawn: a ghost for a missing object.
    pub(crate) node: NodeId,
    /// `Bad` or `Warn`.
    pub(crate) tone: StatusTone,
    /// The full sentence: the dropdown row and the tooltip.
    pub(crate) text: String,
}

/// What the build has so far, which the checks read: the nodes and edges, the listed rows by name,
/// and the pods each workload and Service selects.
pub(crate) struct GraphParts<'a> {
    pub(crate) nodes: BTreeMap<NodeId, TopologyNode>,
    pub(crate) edges: BTreeSet<(NodeId, NodeId, Relation)>,
    rows: HashMap<TopologyKind, HashMap<&'a str, &'a KindRow>>,
    /// The pods each Service selects; an empty list for one that selects nothing by design.
    pub(crate) service_pods: HashMap<&'a str, Vec<&'a PodSummary>>,
    /// The pods of each controller, and of each Deployment through its ReplicaSets.
    owned_pods: HashMap<(TopologyKind, &'a str), Vec<&'a PodSummary>>,
    /// The drawn accounts that hold cluster-admin (RBAC layer).
    pub(crate) grants: Vec<ClusterAdminGrant>,
}

impl<'a> GraphParts<'a> {
    pub(crate) fn new(inputs: &'a TopologyInputs<'a>, pods: &[&'a PodSummary]) -> Self {
        let mut rows: HashMap<TopologyKind, HashMap<&str, &KindRow>> = HashMap::new();
        let mut owned_pods: HashMap<(TopologyKind, &str), Vec<&PodSummary>> = HashMap::new();
        for (kind, row) in all_rows(inputs) {
            rows.entry(kind).or_default().insert(row.name.as_str(), row);
        }
        for pod in pods {
            let Some(controller) = &pod.controller else {
                continue;
            };
            let Some(kind) = [
                TopologyKind::ReplicaSet,
                TopologyKind::StatefulSet,
                TopologyKind::DaemonSet,
            ]
            .into_iter()
            .find(|kind| kind.object_kind() == controller.kind) else {
                continue;
            };
            owned_pods
                .entry((kind, controller.name.as_str()))
                .or_default()
                .push(pod);
        }
        let mut deployment_pods: Vec<(&str, Vec<&PodSummary>)> = Vec::new();
        for (_, row) in all_rows(inputs) {
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
            if let Some(owned) = owned_pods.get(&(TopologyKind::ReplicaSet, set.name.as_str())) {
                deployment_pods.push((owner.name.as_str(), owned.clone()));
            }
        }
        for (name, owned) in deployment_pods {
            owned_pods
                .entry((TopologyKind::Deployment, name))
                .or_default()
                .extend(owned);
        }
        Self {
            nodes: BTreeMap::new(),
            edges: BTreeSet::new(),
            rows,
            service_pods: HashMap::new(),
            owned_pods,
            grants: Vec::new(),
        }
    }

    /// Whether the feed of `kind` lists an object of that name.
    pub(crate) fn is_listed(&self, kind: TopologyKind, name: &str) -> bool {
        self.row(kind, name).is_some()
    }

    pub(crate) fn row(&self, kind: TopologyKind, name: &str) -> Option<&'a KindRow> {
        self.rows.get(&kind)?.get(name).copied()
    }

    /// A node takes the worse of its own tone and the tone of each check drawn on it.
    pub(crate) fn raise_tones(&mut self, checks: &[ConfigCheck]) {
        for check in checks {
            let Some(node) = self.nodes.get_mut(&check.node) else {
                continue;
            };
            let is_worse = match (node.tone, check.tone) {
                (Some(StatusTone::Bad), _) => false,
                (Some(StatusTone::Warn), tone) => tone == StatusTone::Bad,
                (_, _) => true,
            };
            if is_worse {
                node.tone = Some(check.tone);
            }
        }
    }

    /// The text `{Kind} {name}` of a node, for a sentence about it.
    fn owner_text(&self, id: &NodeId) -> String {
        match id {
            NodeId::Object { kind, name } | NodeId::Missing { kind, name } => {
                format!("{} {name}", kind.caption_label())
            }
            NodeId::PodGroup { owner, .. } => format!("Pods of {owner}"),
            NodeId::NoPods { service } => format!("Service {service}"),
        }
    }

    /// The nodes that point at `target`, in id order.
    fn sources_of<'s>(&'s self, target: &'s NodeId) -> impl Iterator<Item = &'s NodeId> {
        self.edges
            .iter()
            .filter(move |(_, to, _)| to == target)
            .map(|(from, _, _)| from)
    }

    /// The pods whose state explains the WHY box of an object.
    fn pods_of(&self, node: &TopologyNode) -> Option<Vec<&'a PodSummary>> {
        match node.kind {
            TopologyKind::Service => self.service_pods.get(node.name.as_ref()).cloned(),
            TopologyKind::Deployment | TopologyKind::StatefulSet | TopologyKind::DaemonSet => Some(
                self.owned_pods
                    .get(&(node.kind, node.name.as_ref()))
                    .cloned()
                    .unwrap_or_default(),
            ),
            _ => None,
        }
    }
}

/// The checks of the graph, sorted by tone (Bad first), then rule, then node, one per
/// `(node, rule)`. A rule whose feed is not ready does not run: its targets are drawn unchecked.
pub(crate) fn graph_checks(parts: &GraphParts, inputs: &TopologyInputs) -> Vec<ConfigCheck> {
    let mut checks: BTreeMap<(NodeId, CheckRule), ConfigCheck> = BTreeMap::new();
    let mut add = |check: ConfigCheck| {
        checks
            .entry((check.node.clone(), check.rule))
            .or_insert(check);
    };
    for node in parts
        .nodes
        .values()
        .filter(|node| node.look == NodeLook::Ghost)
    {
        if let Some(check) = ghost_check(parts, node) {
            add(check);
        }
    }
    for (kind, row) in all_rows(inputs) {
        let KindObject::PersistentVolumeClaim(claim) = &row.object else {
            continue;
        };
        let id = NodeId::Object {
            kind,
            name: claim.name.clone(),
        };
        if let Some((tone, text)) = claim_problem(claim, inputs.now)
            && parts.nodes.contains_key(&id)
        {
            add(ConfigCheck {
                rule: CheckRule::ClaimNotBound,
                node: id,
                tone,
                text,
            });
        }
    }
    for grant in &parts.grants {
        add(grant_check(grant));
    }
    let tls_secrets = tls_secrets_of(parts, inputs);
    for node in parts
        .nodes
        .values()
        .filter(|node| node.look == NodeLook::Plain)
    {
        if let Some(check) = diagnosis_check(parts, inputs, node, tls_secrets.as_deref()) {
            add(check);
        }
    }
    let mut checks: Vec<ConfigCheck> = checks.into_values().collect();
    checks.sort_by(|a, b| {
        let rank = |tone: StatusTone| u8::from(tone != StatusTone::Bad);
        (rank(a.tone), a.rule, &a.node).cmp(&(rank(b.tone), b.rule, &b.node))
    });
    checks
}

/// The check of a ghost node: what is missing, and who refers to it.
fn ghost_check(parts: &GraphParts, node: &TopologyNode) -> Option<ConfigCheck> {
    match &node.id {
        NodeId::NoPods { service } => {
            let terms = match parts
                .row(TopologyKind::Service, service)
                .map(|row| &row.object)
            {
                Some(KindObject::Service(service)) => service.selector.join(", "),
                _ => String::new(),
            };
            Some(ConfigCheck {
                rule: CheckRule::ServiceNoPods,
                node: node.id.clone(),
                tone: StatusTone::Bad,
                text: format!("Service {service} matches no pods (selector {terms})."),
            })
        }
        NodeId::Missing { kind, name } => {
            let owner = parts.sources_of(&node.id).next()?;
            let owner_text = parts.owner_text(owner);
            let label = kind.caption_label();
            let (rule, tone, text) = match kind {
                TopologyKind::Service => (
                    CheckRule::IngressMissingService,
                    StatusTone::Bad,
                    format!("{owner_text} routes to missing Service {name}."),
                ),
                TopologyKind::ConfigMap => (
                    CheckRule::MissingConfigMap,
                    StatusTone::Warn,
                    format!("{owner_text} references missing ConfigMap {name}."),
                ),
                TopologyKind::Secret => (
                    CheckRule::MissingSecret,
                    StatusTone::Warn,
                    format!("{owner_text} references missing Secret {name}."),
                ),
                TopologyKind::PersistentVolumeClaim => (
                    CheckRule::MissingClaim,
                    StatusTone::Bad,
                    format!("{owner_text} mounts missing PVC {name}."),
                ),
                TopologyKind::Deployment
                | TopologyKind::StatefulSet
                | TopologyKind::DaemonSet
                | TopologyKind::ReplicaSet => (
                    CheckRule::HpaMissingTarget,
                    StatusTone::Warn,
                    format!("{owner_text} scales missing {label} {name}."),
                ),
                TopologyKind::ServiceAccount => (
                    CheckRule::MissingServiceAccount,
                    StatusTone::Bad,
                    format!("{owner_text} runs as missing ServiceAccount {name}."),
                ),
                TopologyKind::Role => (
                    CheckRule::MissingRole,
                    StatusTone::Warn,
                    format!("{owner_text} grants missing Role {name}."),
                ),
                TopologyKind::Ingress
                | TopologyKind::HorizontalPodAutoscaler
                | TopologyKind::Pod
                | TopologyKind::RoleBinding
                | TopologyKind::ClusterRoleBinding
                | TopologyKind::ClusterRole => return None,
            };
            Some(ConfigCheck {
                rule,
                node: node.id.clone(),
                tone,
                text,
            })
        }
        NodeId::Object { .. } | NodeId::PodGroup { .. } => None,
    }
}

/// The cluster-admin check, drawn on the binding when the account is its direct subject, else on
/// the account (a group grant has no drawn binding).
fn grant_check(grant: &ClusterAdminGrant) -> ConfigCheck {
    let group = grant
        .group
        .as_ref()
        .map_or_else(String::new, |group| format!(" (group {group})"));
    ConfigCheck {
        rule: CheckRule::ClusterAdminAccount,
        node: grant
            .binding
            .clone()
            .unwrap_or_else(|| grant.account.clone()),
        tone: StatusTone::Warn,
        text: format!(
            "ServiceAccount {} has cluster-admin through {}{group}.",
            grant.account_name, grant.binding_text
        ),
    }
}

/// The `ClaimNotBound` tone and text: `Pending` for longer than the grace is a Warn, `Lost` a Bad.
pub(crate) fn claim_problem(
    claim: &PersistentVolumeClaimSummary,
    now: Timestamp,
) -> Option<(StatusTone, String)> {
    match claim.phase.as_str() {
        "Lost" => {
            let text = match &claim.volume {
                Some(volume) => format!("PVC {} lost its volume {volume}.", claim.name),
                None => format!("PVC {} lost its volume.", claim.name),
            };
            Some((StatusTone::Bad, text))
        }
        "Pending" => {
            let created = claim.created_at?;
            (now.duration_since(created) > PVC_PENDING_GRACE).then(|| {
                (
                    StatusTone::Warn,
                    format!(
                        "PVC {} is Pending for {}.",
                        claim.name,
                        format_age(claim.created_at, now)
                    ),
                )
            })
        }
        _ => None,
    }
}

/// The TLS secrets that an Ingress of the graph names, for the certificate box; `None` while the
/// Secrets feed is not ready.
fn tls_secrets_of(parts: &GraphParts, inputs: &TopologyInputs) -> Option<Vec<SecretSummary>> {
    let secrets = inputs
        .rows
        .iter()
        .find(|(kind, _)| *kind == TopologyKind::Secret)
        .and_then(|(_, feed)| match feed {
            FeedRows::Ready(rows) => Some(rows),
            FeedRows::Loading | FeedRows::Failed | FeedRows::Off => None,
        })?;
    let named: BTreeSet<&str> = parts
        .nodes
        .values()
        .filter(|node| node.kind == TopologyKind::Ingress && node.look == NodeLook::Plain)
        .filter_map(|node| parts.row(TopologyKind::Ingress, node.name.as_ref()))
        .filter_map(|row| match &row.object {
            KindObject::Ingress(ingress) => Some(ingress),
            _ => None,
        })
        .flat_map(|ingress| &ingress.tls)
        .filter_map(|tls| tls.secret_name.as_deref())
        .collect();
    Some(
        secrets
            .iter()
            .filter_map(|row| match &row.object {
                KindObject::Secret(secret)
                    if secret.secret_type == TLS_SECRET_TYPE
                        && named.contains(secret.name.as_str()) =>
                {
                    Some(secret.clone())
                }
                _ => None,
            })
            .collect(),
    )
}

/// The WHY box of an object as a check. Boxes that another rule covers are skipped: a missing TLS
/// secret (`MissingSecret`), a Service with no pod (`ServiceNoPods`), and a lost claim
/// (`ClaimNotBound`).
fn diagnosis_check(
    parts: &GraphParts,
    inputs: &TopologyInputs,
    node: &TopologyNode,
    tls_secrets: Option<&[SecretSummary]>,
) -> Option<ConfigCheck> {
    // Claims have `ClaimNotBound`, and the access kinds have the three RBAC rules.
    if matches!(node.kind, TopologyKind::PersistentVolumeClaim) || node.kind.is_access() {
        return None;
    }
    let row = parts.row(node.kind, node.name.as_ref())?;
    let owned = parts.pods_of(node).filter(|_| inputs.pods.is_some());
    let service = match &row.object {
        KindObject::Service(service) => {
            let matching = parts.service_pods.get(service.name.as_str());
            Some(health_of_matches(
                matching
                    .filter(|_| inputs.pods.is_some())
                    .map(Vec::as_slice),
            ))
        }
        _ => None,
    };
    let diagnosis = kind_diagnosis(
        &row.object,
        &DiagnosisInputs {
            pods: owned.as_deref(),
            nodes: inputs.nodes,
            service,
            bindings: None,
            tls_secrets,
            now: inputs.now,
        },
    )?;
    if diagnosis.title == NO_MATCHING_PODS_TITLE
        || (diagnosis.title == CERTIFICATE_TITLE && diagnosis.text.starts_with(NO_TLS_SECRET))
    {
        return None;
    }
    Some(ConfigCheck {
        rule: CheckRule::ObjectDiagnosis,
        node: node.id.clone(),
        tone: diagnosis.tone,
        text: format!(
            "{} {}: {}.",
            node.kind.caption_label(),
            node.name,
            sentence_case(&diagnosis.title)
        ),
    })
}

/// `NO READY ENDPOINTS` as `No ready endpoints`.
fn sentence_case(title: &str) -> String {
    let mut chars = title.chars();
    match chars.next() {
        Some(first) => first
            .to_uppercase()
            .chain(chars.flat_map(char::to_lowercase))
            .collect(),
        None => String::new(),
    }
}

/// The checks chip: the chip label of the rule when every check has the same one, else `{n} config
/// problems`, toned by the worst check. `None` without checks (the chip is hidden).
pub(crate) fn checks_chip(checks: &[ConfigCheck]) -> Option<(String, StatusTone)> {
    let first = checks.first()?;
    let text = if checks.iter().all(|check| check.rule == first.rule) {
        first.rule.chip_label(checks.len())
    } else {
        format!("{} config problems", checks.len())
    };
    Some((text, first.tone))
}

/// The note under the toolbar about feeds that did not answer: `Not checked: secrets (not
/// permitted), services (watch failed). Loading: ingresses.` `None` when every feed is ready. A
/// kind whose chip is off has no entry in `rows`, so it is not listed.
pub(crate) fn topology_coverage(rows: &[(TopologyKind, FeedRows)]) -> Option<String> {
    let mut listed: Vec<&(TopologyKind, FeedRows)> = rows.iter().collect();
    listed.sort_by_key(|(kind, _)| *kind);
    let not_checked: Vec<String> = listed
        .iter()
        .filter_map(|(kind, feed)| match feed {
            FeedRows::Off => Some(format!("{} (not permitted)", feed_name(*kind))),
            FeedRows::Failed => Some(format!("{} (watch failed)", feed_name(*kind))),
            FeedRows::Ready(_) | FeedRows::Loading => None,
        })
        .collect();
    let loading: Vec<&str> = listed
        .iter()
        .filter(|(_, feed)| matches!(feed, FeedRows::Loading))
        .map(|(kind, _)| feed_name(*kind))
        .collect();
    let mut parts = Vec::new();
    if !not_checked.is_empty() {
        parts.push(format!("Not checked: {}.", not_checked.join(", ")));
    }
    if !loading.is_empty() {
        parts.push(format!("Loading: {}.", loading.join(", ")));
    }
    (!parts.is_empty()).then(|| parts.join(" "))
}

/// The plural a kind goes by in the coverage note.
fn feed_name(kind: TopologyKind) -> &'static str {
    match kind {
        TopologyKind::Ingress => "ingresses",
        TopologyKind::HorizontalPodAutoscaler => "HPAs",
        TopologyKind::Service => "services",
        TopologyKind::Deployment => "deployments",
        TopologyKind::StatefulSet => "statefulsets",
        TopologyKind::DaemonSet => "daemonsets",
        TopologyKind::ReplicaSet => "replicasets",
        TopologyKind::Pod => "pods",
        TopologyKind::ConfigMap => "configmaps",
        TopologyKind::Secret => "secrets",
        TopologyKind::PersistentVolumeClaim => "PVCs",
        TopologyKind::ServiceAccount => "serviceaccounts",
        TopologyKind::RoleBinding => "rolebindings",
        TopologyKind::ClusterRoleBinding => "clusterrolebindings",
        TopologyKind::Role => "roles",
        TopologyKind::ClusterRole => "clusterroles",
    }
}

#[cfg(test)]
#[path = "topology_checks_tests.rs"]
mod topology_checks_tests;
