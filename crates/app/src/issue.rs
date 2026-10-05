//! The model of the Issues engine: what a problem is, how severe, and what the screen shows. Pure:
//! no GPUI type except `SharedString`, and no logging, since event messages are arbitrary text.

use cluster::InvolvedObject;
use gpui_kit::SharedString;

use crate::kind_row::{REPLICA_SET_KIND, deployment_of_replica_set};
use crate::status_tone::StatusTone;
use crate::table_selection::ResourceKey;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum IssueSeverity {
    /// Something is down or broken.
    Critical,
    /// Something is degraded, or close to a limit.
    Warning,
}

impl IssueSeverity {
    pub(crate) fn tone(self) -> StatusTone {
        match self {
            Self::Critical => StatusTone::Bad,
            Self::Warning => StatusTone::Warn,
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Critical => "Critical",
            Self::Warning => "Warning",
        }
    }
}

/// Rule ids, declared in evaluation order: pods, nodes, namespaces, workload and policy kinds,
/// certificates, volume usage, events. `Ord` is the order
/// in which `dedupe` lets a rule claim an object.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum IssueRule {
    PodImage,
    PodCrash,
    PodWaiting,
    PodUnschedulable,
    PodFailed,
    PodExited,
    PodStartup,
    PodNotReady,
    PodStuck,
    PodRestarts,
    PodMemory,
    PodCpu,
    NodeNotReady,
    NodeNetwork,
    NodePressure,
    NodeCondition,
    NodeMemory,
    NodeCpu,
    NamespaceStuck,
    KindRollout,
    KindJob,
    KindClaim,
    PvcPending,
    KindAutoscaler,
    KindDisruptionBudget,
    KindQuota,
    QuotaNearLimit,
    CertExpired,
    CertExpiring,
    VolumeFull,
    EventFailedCreate,
    EventJobFailed,
    EventBurst,
}

/// An object as the engine names it. `kind` is the API kind (`Pod`, `Deployment`): an owner or an
/// event can name a kind that k8sBoard has no screen for, so it is text, not a `ResourceKind`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct IssueObject {
    pub(crate) kind: String,
    pub(crate) namespace: Option<String>,
    pub(crate) name: String,
}

impl IssueObject {
    pub(crate) fn new(kind: &str, namespace: Option<&str>, name: &str) -> Self {
        Self {
            kind: kind.to_owned(),
            namespace: namespace.map(str::to_owned),
            name: name.to_owned(),
        }
    }

    pub(crate) fn pod(namespace: &str, name: &str) -> Self {
        Self::new("Pod", Some(namespace), name)
    }

    pub(crate) fn node(name: &str) -> Self {
        Self::new("Node", None, name)
    }

    /// The object an event is about. A ReplicaSet named `{deployment}-{hash}` is reported as its
    /// Deployment, which is the object users act on and the one a pod group names.
    pub(crate) fn of_involved(object: &InvolvedObject) -> Self {
        if object.kind == REPLICA_SET_KIND
            && let Some(deployment) = deployment_of_replica_set(&object.name)
        {
            return Self::new("Deployment", object.namespace.as_deref(), deployment);
        }
        Self::new(&object.kind, object.namespace.as_deref(), &object.name)
    }

    /// The row a click reveals; `None` when k8sBoard has no screen for the kind.
    pub(crate) fn target(&self) -> Option<ResourceKey> {
        ResourceKey::of_object(&self.kind, self.namespace.as_deref(), &self.name)
    }
}

/// What identifies an issue across refreshes. A group of pods is keyed by its workload.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct IssueKey {
    pub(crate) rule: IssueRule,
    pub(crate) object: IssueObject,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum IssueAction {
    Open,
    /// Reads the logs of the pod of `Issue::shown` (a group: its representative pod).
    ViewLogs {
        container: Option<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Issue {
    pub(crate) key: IssueKey,
    pub(crate) severity: IssueSeverity,
    /// The pill text: `CrashLoopBackOff`, `Not ready`.
    pub(crate) reason: SharedString,
    /// The plain-language cause, from the WHY rules.
    pub(crate) cause: String,
    /// The Object column: the pod, or the workload of a group of two or more pods.
    pub(crate) shown: IssueObject,
    /// The object the rule fired on: `shown`, or the representative pod of a group. View logs
    /// reads this pod.
    pub(crate) subject: IssueObject,
    pub(crate) container: Option<String>,
    /// At least 1: the pods of a group.
    pub(crate) count: usize,
    /// When the problem began, as far as the board knows: the rule's onset, else when it was first
    /// seen. Orders the board and ends the grace; the Age column shows `onset` instead.
    pub(crate) since: jiff::Timestamp,
    /// When the cluster says the problem began; `None` when no rule knows, so the Age column
    /// shows nothing rather than the time k8sBoard first saw it.
    pub(crate) onset: Option<jiff::Timestamp>,
    /// The row a click reveals; `None` when k8sBoard has no screen for `shown`.
    pub(crate) target: Option<ResourceKey>,
    pub(crate) action: IssueAction,
}

/// What a rule reports about one object, before grouping, grace, and sorting.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Finding {
    pub(crate) rule: IssueRule,
    pub(crate) severity: IssueSeverity,
    pub(crate) object: IssueObject,
    pub(crate) reason: SharedString,
    pub(crate) cause: String,
    pub(crate) container: Option<String>,
    /// When the problem began, when the API says; `None` means first seen.
    pub(crate) onset: Option<jiff::Timestamp>,
    /// The finding shows only once its age reaches this; applied by the board, never by a rule.
    pub(crate) grace: Option<jiff::SignedDuration>,
    pub(crate) action: IssueAction,
    /// Set for pods: the workload that pods with the same problem are grouped under.
    pub(crate) workload: Option<IssueObject>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn critical_sorts_before_warning() {
        assert!(IssueSeverity::Critical < IssueSeverity::Warning);
        assert_eq!(IssueSeverity::Critical.tone(), StatusTone::Bad);
        assert_eq!(IssueSeverity::Warning.tone(), StatusTone::Warn);
    }

    #[test]
    fn involved_replica_set_is_reported_as_its_deployment() {
        let replica_set = InvolvedObject {
            kind: "ReplicaSet".to_owned(),
            namespace: Some("shop".to_owned()),
            name: "api-7d9f8c".to_owned(),
        };
        assert_eq!(
            IssueObject::of_involved(&replica_set),
            IssueObject::new("Deployment", Some("shop"), "api")
        );
        let standalone = InvolvedObject {
            name: "api-canary".to_owned(),
            ..replica_set
        };
        assert_eq!(
            IssueObject::of_involved(&standalone).kind,
            "ReplicaSet",
            "a name without a pod-template hash is not a Deployment's"
        );
    }

    #[test]
    fn target_needs_a_screen_for_the_kind() {
        assert!(IssueObject::pod("shop", "api-0").target().is_some());
        assert!(IssueObject::node("node-a").target().is_some());
        assert_eq!(IssueObject::new("Widget", Some("shop"), "w").target(), None);
    }
}
