//! The data model that one table delegate and one drawer renderer share for every kind.
//! Row builders (`*_rows.rs`) are pure: they take a summary and produce a `KindRow`.

use cluster::{
    BindingSummary, ConfigMapSummary, ControllerRef, CrdSummary, CronJobSummary, CronSchedule,
    CustomObjectSummary, DaemonSetSummary, DeploymentSummary, HelmReleaseSummary,
    HorizontalPodAutoscalerSummary, IngressSummary, JobSummary, NamespaceSummary,
    NetworkPolicySummary, PersistentVolumeClaimSummary, PersistentVolumeSummary,
    PodDisruptionBudgetSummary, PodSummary, ReplicaSetSummary, ResourceQuotaSummary, RoleSummary,
    SecretSummary, ServiceAccountSummary, ServiceSummary, StatefulSetSummary,
};
use gpui_kit::SharedString;

use crate::status_tone::{StatusLabel, StatusTone};
use crate::table_selection::ResourceKey;

/// A table row and its drawer content, built on tokio so the main thread only swaps a `Vec`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct KindRow {
    /// `None` for cluster-scoped kinds.
    pub(crate) namespace: Option<String>,
    pub(crate) name: String,
    pub(crate) created_at: Option<jiff::Timestamp>,
    /// The drawer subtitle.
    pub(crate) status: StatusLabel,
    /// Exactly `kind.columns().len()` cells; the Name column is not included.
    pub(crate) cells: Vec<KindCell>,
    /// The drawer body, in order.
    pub(crate) sections: Vec<DetailSection>,
    pub(crate) related_pods: Option<PodOwner>,
    /// `Some` for Events only.
    pub(crate) event: Option<EventDetail>,
    pub(crate) labels: Vec<SharedString>,
    pub(crate) object: KindObject,
}

/// The summary a row was built from, for content computed at paint time (decision 16). Kinds
/// whose rows need nothing beyond their cells use `Plain`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum KindObject {
    Plain,
    Deployment(DeploymentSummary),
    CronJob(CronJobSummary),
    StatefulSet(StatefulSetSummary),
    DaemonSet(DaemonSetSummary),
    ReplicaSet(ReplicaSetSummary),
    Job(JobSummary),
    Service(ServiceSummary),
    Ingress(IngressSummary),
    ConfigMap(ConfigMapSummary),
    NetworkPolicy(NetworkPolicySummary),
    HorizontalPodAutoscaler(HorizontalPodAutoscalerSummary),
    ResourceQuota(ResourceQuotaSummary),
    PodDisruptionBudget(PodDisruptionBudgetSummary),
    PersistentVolumeClaim(PersistentVolumeClaimSummary),
    PersistentVolume(PersistentVolumeSummary),
    /// A Role or a ClusterRole (`namespace: None`).
    Role(RoleSummary),
    /// A RoleBinding or a ClusterRoleBinding (`namespace: None`).
    Binding(BindingSummary),
    ServiceAccount(ServiceAccountSummary),
    /// Key names and sizes only: a Secret row never holds a value.
    Secret(SecretSummary),
    /// A Helm release: names, numbers, and chart facts, never a value.
    HelmRelease(HelmReleaseSummary),
    /// A custom resource definition: names, versions, and schema field names, no instance data.
    Crd(CrdSummary),
    /// An object of a custom resource: metadata, masked column values, conditions, never a raw value.
    Custom(CustomObjectSummary),
    /// A namespace: phase, labels, and the deletion conditions that explain a stuck deletion.
    Namespace(NamespaceSummary),
}

/// The paint-time content of a `DetailRow::Live`, read from the row's `KindObject` and the
/// session's live lists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LiveContent {
    Revisions,
    NextRuns,
    RecentJobs,
    NotReadyPods,
    Endpoints,
    UsedBy,
    ConfigMapData,
    SelectedPods,
    ScalingEvents,
    BlockedCreations,
    NamespaceQuotas,
    ClaimUsage,
    MountedBy,
    ClassVolumes,
    RoleBindings,
    RoleSubjects,
    BoundRoles,
    ServiceAccountPods,
    /// The masked keys of a Secret, and the Reveal and Copy controls (`secret_values.rs`).
    IngressTls,
    SecretData,
    /// The certificate of a TLS Secret.
    Certificate,
    /// The user values that changed from the previous revision to the latest, masked, with Reveal
    /// (`helm_release_view.rs`).
    HelmValuesChange,
    /// Every revision of a Helm release, newest first, from the related history watch.
    HelmHistory,
    /// The conditions of a custom object, from its summary.
    CustomConditions,
    /// The status fields of a custom object, from the related fields watch.
    CustomStatus,
    /// The spec fields of a custom object, from the related fields watch.
    CustomSpec,
}

/// Events only: what the drawer header, subtitle, and menu need.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct EventDetail {
    /// `BackOff · api-7d9f8c-x2k4q`; the object name alone without a reason.
    pub(crate) title: SharedString,
    /// The event reason; `None` when empty. Filter similar needs it.
    pub(crate) reason: Option<SharedString>,
    /// `None` when k8sBoard has no screen for the object's kind.
    pub(crate) object: Option<ResourceKey>,
    /// `kubelet on ip-10-0-1-23`, for the subtitle.
    pub(crate) source: Option<SharedString>,
    /// The full (trimmed, truncated) message, for Copy message.
    pub(crate) message: SharedString,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum KindCell {
    Text(SharedString),
    Mono(SharedString),
    /// Short text that stands for a longer one, which a tooltip gives in full.
    Hinted {
        text: SharedString,
        tooltip: SharedString,
    },
    /// Mono text with a muted `{prefix}/`, cut with an ellipsis.
    Qualified {
        prefix: Option<SharedString>,
        text: SharedString,
    },
    Toned(StatusLabel),
    Absent,
    /// Rendered at paint time, so ages never go stale. `tone` colours it.
    Age {
        at: Option<jiff::Timestamp>,
        tone: Option<StatusTone>,
    },
    /// The next run, painted relative to now so it never goes stale.
    NextRun(CronSchedule),
    /// Mono text that shrinks with an ellipsis, then a pinned ` +{more}` that never does, such as
    /// the first owner and the count of the others.
    MonoWithMore {
        text: SharedString,
        more: usize,
    },
    /// Right-aligned mono text that sorts by `value`, such as a request sum. `tone` colours the text
    /// (`None` for a plain quantity).
    Quantity {
        text: SharedString,
        value: u64,
        tone: Option<StatusTone>,
    },
    /// `format_age(started_at, finished_at.unwrap_or(now))`, read at paint time so a running
    /// job keeps counting; `Absent` when not started.
    Duration {
        started_at: Option<jiff::Timestamp>,
        finished_at: Option<jiff::Timestamp>,
    },
    /// The not-after of a certificate, painted as days left at paint time. Sorts by `not_after`.
    Expiry {
        not_after: jiff::Timestamp,
    },
    /// A printer-column date, painted at render: `5d` in the past, `in 6d` in the future. Sorts by
    /// time.
    Date {
        at: jiff::Timestamp,
        rule: DateRule,
    },
}

/// How a `KindCell::Date` is toned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DateRule {
    Plain,
    /// Warn within 14 days and Bad once past, like the TLS expiry; no tone otherwise.
    Expiry,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DetailSection {
    pub(crate) title: &'static str,
    pub(crate) rows: Vec<DetailRow>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum DetailRow {
    Field {
        label: SharedString,
        value: KindCell,
    },
    Chips(Vec<SharedString>),
    /// A muted explanation that wraps, such as "No keys".
    Note(SharedString),
    /// A port, followed by its disabled Forward button.
    Port {
        text: SharedString,
    },
    /// A label above its value, for labels that are too long for the label column, such as
    /// ingress hosts.
    Stacked {
        label: SharedString,
        value: KindCell,
    },
    /// Preformatted text that wraps: mono, small, on a muted background.
    Code(SharedString),
    /// Preformatted columns: like `Code`, but long lines scroll sideways instead of wrapping.
    Table(SharedString),
    /// Content computed at paint time from `KindObject` and the session's live lists.
    Live(LiveContent),
    /// A labelled bar. `percent` is 0 to 100 (builders use `percent`); `tone: None` keeps the
    /// kit color. The text sits after the bar, such as `3 / 4`.
    Bar {
        label: SharedString,
        percent: u8,
        text: SharedString,
        tone: Option<StatusTone>,
    },
    /// Like `Link`, with the label above the link, for labels that are too long for the label
    /// column (an ingress host and path).
    StackedLink {
        label: SharedString,
        text: SharedString,
        target: ResourceKey,
    },
    /// A label and a clickable mono value that reveals `target`.
    Link {
        label: SharedString,
        text: SharedString,
        target: ResourceKey,
    },
}

impl KindCell {
    /// A number as text.
    pub(crate) fn count(count: impl std::fmt::Display) -> Self {
        Self::Text(count.to_string().into())
    }

    pub(crate) fn age(at: Option<jiff::Timestamp>) -> Self {
        Self::Age { at, tone: None }
    }

    /// Monospaced text, or `Absent` when it is empty. Join a list before passing it.
    pub(crate) fn mono_or_absent(text: &str) -> Self {
        if text.is_empty() {
            return Self::Absent;
        }
        Self::Mono(text.to_owned().into())
    }

    pub(crate) fn text_or_absent(text: Option<&str>) -> Self {
        text.map_or(Self::Absent, |text| Self::Text(text.to_owned().into()))
    }
}

impl DetailRow {
    pub(crate) fn field(label: impl Into<SharedString>, value: KindCell) -> Self {
        Self::Field {
            label: label.into(),
            value,
        }
    }

    pub(crate) fn stacked(label: impl Into<SharedString>, value: KindCell) -> Self {
        Self::Stacked {
            label: label.into(),
            value,
        }
    }
}

impl KindRow {
    /// The drawer section with `title`, for tests.
    #[cfg(test)]
    pub(crate) fn section(&self, title: &str) -> Option<&DetailSection> {
        self.sections.iter().find(|section| section.title == title)
    }
}

/// Label or selector terms as drawer chips.
pub(crate) fn chips(terms: &[String]) -> Vec<SharedString> {
    terms.iter().cloned().map(SharedString::from).collect()
}

/// `RWO` for `ReadWriteOnce`; an unknown mode is returned as is.
pub(crate) fn access_mode_short(mode: &str) -> &str {
    match mode {
        "ReadWriteOnce" => "RWO",
        "ReadOnlyMany" => "ROX",
        "ReadWriteMany" => "RWX",
        "ReadWriteOncePod" => "RWOP",
        other => other,
    }
}

/// A ratio as a bar percent: rounded, and clamped to 0 to 100 (a NaN reads as 0).
pub(crate) fn percent(ratio: f64) -> u8 {
    if ratio.is_nan() {
        return 0;
    }
    let percent = (ratio * 100.0).round().clamp(0.0, 100.0);
    // Clamped to 0..=100 above, so the cast is exact.
    percent as u8
}

/// The `kind` of a controller owner reference. The pods section orders StatefulSet pods and
/// shows the node of DaemonSet pods.
pub(crate) const STATEFUL_SET_KIND: &str = "StatefulSet";
pub(crate) const DAEMON_SET_KIND: &str = "DaemonSet";
pub(crate) const REPLICA_SET_KIND: &str = "ReplicaSet";
pub(crate) const JOB_KIND: &str = "Job";

/// Which pods a drawer lists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PodOwner {
    /// Pods whose controller owner reference is exactly `kind` and `name`.
    Controller {
        namespace: String,
        kind: &'static str,
        name: String,
    },
    /// Pods belong to a Deployment through its ReplicaSets.
    Deployment { namespace: String, name: String },
    /// Pods scheduled on the node, in any namespace.
    Node { name: String },
}

impl PodOwner {
    /// The namespace of a workload; `None` for a node.
    pub(crate) fn namespace(&self) -> Option<&str> {
        match self {
            Self::Controller { namespace, .. } | Self::Deployment { namespace, .. } => {
                Some(namespace)
            }
            Self::Node { .. } => None,
        }
    }
}

/// The characters of a pod-template hash: Kubernetes `SafeEncodeString` drops vowels, `0`,
/// `1`, `3`, and a few lookalikes.
const POD_TEMPLATE_HASH_ALPHABET: &str = "bcdfghjklmnpqrstvwxz2456789";

pub(crate) fn owns_pod(owner: &PodOwner, pod: &PodSummary) -> bool {
    match owner {
        PodOwner::Node { name } => pod.node_name.as_deref() == Some(name.as_str()),
        PodOwner::Controller { .. } | PodOwner::Deployment { .. } => {
            owns(owner, &pod.namespace, pod.controller.as_ref())
        }
    }
}

/// Whether a pod in `namespace` with `controller` belongs to a workload `owner`. A node owns pods
/// by where they run, which a namespace and controller cannot tell, so it owns none here.
pub(crate) fn owns(owner: &PodOwner, namespace: &str, controller: Option<&ControllerRef>) -> bool {
    match owner {
        PodOwner::Controller {
            namespace: owner_namespace,
            kind,
            name,
        } => {
            owner_namespace == namespace
                && controller
                    .is_some_and(|controller| controller.kind == *kind && controller.name == *name)
        }
        PodOwner::Deployment {
            namespace: owner_namespace,
            name,
        } => {
            owner_namespace == namespace
                && controller.is_some_and(|controller| {
                    controller.kind == REPLICA_SET_KIND
                        && is_deployment_replica_set(name, &controller.name)
                })
        }
        PodOwner::Node { .. } => false,
    }
}

/// A Deployment's ReplicaSet is `{deployment}-{hash}`. The hash never contains `-`, so
/// `api` does not claim `api-worker-7d9f8c`, and its alphabet excludes `api-canary`.
fn is_deployment_replica_set(deployment: &str, replica_set: &str) -> bool {
    replica_set
        .strip_prefix(deployment)
        .and_then(|rest| rest.strip_prefix('-'))
        .is_some_and(|hash| {
            !hash.is_empty() && hash.chars().all(|c| POD_TEMPLATE_HASH_ALPHABET.contains(c))
        })
}

/// The Deployment of a ReplicaSet named `{deployment}-{hash}`; `None` when the suffix is not a
/// pod-template hash (a standalone ReplicaSet).
pub(crate) fn deployment_of_replica_set(replica_set: &str) -> Option<&str> {
    let (deployment, hash) = replica_set.rsplit_once('-')?;
    let is_hash = !hash.is_empty() && hash.chars().all(|c| POD_TEMPLATE_HASH_ALPHABET.contains(c));
    (is_hash && !deployment.is_empty()).then_some(deployment)
}

#[cfg(test)]
mod tests {
    use cluster::{ControllerRef, PodStatus, ReadyCount, StatusReason};

    use super::*;

    fn pod(namespace: &str, controller: Option<(&str, &str)>) -> PodSummary {
        PodSummary {
            namespace: namespace.to_owned(),
            name: "pod".to_owned(),
            status: PodStatus::Reason(StatusReason::Running),
            ready: ReadyCount { ready: 1, total: 1 },
            restarts: 0,
            node_name: None,
            created_at: None,
            pod_ip: None,
            qos_class: None,
            service_account: None,
            controller: controller.map(|(kind, name)| ControllerRef {
                kind: kind.to_owned(),
                name: name.to_owned(),
            }),
            conditions: Vec::new(),
            status_message: None,
            labels: Vec::new(),
            host_network: false,
            image_pull_secrets: Vec::new(),
            containers: Vec::new(),
        }
    }

    fn deployment(namespace: &str, name: &str) -> PodOwner {
        PodOwner::Deployment {
            namespace: namespace.to_owned(),
            name: name.to_owned(),
        }
    }

    #[test]
    fn percent_rounds_and_clamps() {
        assert_eq!(percent(0.0), 0);
        assert_eq!(percent(0.004), 0);
        assert_eq!(percent(0.005), 1);
        assert_eq!(percent(2.0 / 3.0), 67);
        assert_eq!(percent(1.0), 100);
        assert_eq!(percent(1.7), 100);
        assert_eq!(percent(-0.3), 0);
        assert_eq!(percent(f64::NAN), 0);
        assert_eq!(percent(f64::INFINITY), 100);
    }

    #[test]
    fn cell_and_row_constructors_build_plain_values() {
        assert_eq!(KindCell::count(7), KindCell::Text("7".into()));
        assert_eq!(
            KindCell::text_or_absent(Some("x")),
            KindCell::Text("x".into())
        );
        assert_eq!(KindCell::text_or_absent(None), KindCell::Absent);
        assert_eq!(
            DetailRow::field("Desired", KindCell::count(3)),
            DetailRow::Field {
                label: "Desired".into(),
                value: KindCell::Text("3".into()),
            }
        );
        assert_eq!(chips(&["a=b".to_owned()]), ["a=b"]);
    }

    #[test]
    fn owns_pod_through_deployment_replica_set() {
        let owner = deployment("ns", "api");
        let owned = |replica_set| owns_pod(&owner, &pod("ns", Some(("ReplicaSet", replica_set))));
        assert!(owned("api-7d9f8c"));
        assert!(!owned("api-worker-7d9f8c"));
        assert!(!owned("api-canary"));
        assert!(!owned("api-"));
        assert!(!owned("api"));
        assert!(!owned("apix-7d9f8c"));
    }

    #[test]
    fn owns_pod_requires_a_replica_set_controller() {
        let owner = deployment("ns", "api");
        assert!(!owns_pod(
            &owner,
            &pod("ns", Some(("StatefulSet", "api-7d9f8c")))
        ));
        assert!(!owns_pod(&owner, &pod("ns", None)));
    }

    #[test]
    fn owns_matches_namespace_and_controller() {
        let controller = |kind: &str, name: &str| ControllerRef {
            kind: kind.to_owned(),
            name: name.to_owned(),
        };
        let stateful = PodOwner::Controller {
            namespace: "ns".to_owned(),
            kind: STATEFUL_SET_KIND,
            name: "web".to_owned(),
        };
        let web = controller("StatefulSet", "web");
        assert!(owns(&stateful, "ns", Some(&web)));
        assert!(!owns(&stateful, "other", Some(&web)));
        assert!(!owns(&stateful, "ns", None));
        // The Deployment hash rule still applies.
        let api = deployment("ns", "api");
        assert!(owns(
            &api,
            "ns",
            Some(&controller("ReplicaSet", "api-7d9f8c"))
        ));
        assert!(!owns(
            &api,
            "ns",
            Some(&controller("ReplicaSet", "api-worker-7d9f8c"))
        ));
        // A node owns pods by where they run, which this cannot tell.
        let node = PodOwner::Node {
            name: "wk".to_owned(),
        };
        assert!(!owns(&node, "ns", Some(&web)));
    }

    #[test]
    fn owns_pod_by_controller_kind_and_name() {
        let owner = PodOwner::Controller {
            namespace: "ns".to_owned(),
            kind: STATEFUL_SET_KIND,
            name: "web".to_owned(),
        };
        assert!(owns_pod(&owner, &pod("ns", Some(("StatefulSet", "web")))));
        assert!(!owns_pod(&owner, &pod("ns", Some(("DaemonSet", "web")))));
        assert!(!owns_pod(
            &owner,
            &pod("ns", Some(("StatefulSet", "web-2")))
        ));
        assert!(!owns_pod(
            &owner,
            &pod("other", Some(("StatefulSet", "web")))
        ));
        assert!(!owns_pod(&owner, &pod("ns", None)));
    }

    #[test]
    fn owns_pod_on_node_in_any_namespace() {
        let owner = PodOwner::Node {
            name: "node-1".to_owned(),
        };
        let on = |namespace: &str, node: Option<&str>| {
            let mut pod = pod(namespace, None);
            pod.node_name = node.map(str::to_owned);
            owns_pod(&owner, &pod)
        };
        assert!(on("ns", Some("node-1")));
        assert!(on("other", Some("node-1")));
        assert!(!on("ns", Some("node-2")));
        assert!(!on("ns", None));
    }

    #[test]
    fn owns_pod_requires_same_namespace() {
        let owner = deployment("ns", "api");
        let controller = Some(("ReplicaSet", "api-7d9f8c"));
        assert!(owns_pod(&owner, &pod("ns", controller)));
        assert!(!owns_pod(&owner, &pod("other", controller)));
    }
}
