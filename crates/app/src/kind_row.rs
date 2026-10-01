//! The data model that one table delegate and one drawer renderer share for every kind.
//! Row builders (`*_rows.rs`) are pure: they take a summary and produce a `KindRow`.

use cluster::PodSummary;
use gpui_kit::SharedString;

use crate::status_tone::{StatusLabel, StatusTone};

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
    pub(crate) labels: Vec<SharedString>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum KindCell {
    Text(SharedString),
    Mono(SharedString),
    Toned(StatusLabel),
    Absent,
    /// Rendered at paint time, so ages never go stale. `tone` colours it.
    Age {
        at: Option<jiff::Timestamp>,
        tone: Option<StatusTone>,
    },
    /// `format_age(started_at, finished_at.unwrap_or(now))`, read at paint time so a running
    /// job keeps counting; `Absent` when not started.
    Duration {
        started_at: Option<jiff::Timestamp>,
        finished_at: Option<jiff::Timestamp>,
    },
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

/// The `kind` of a controller owner reference. The pods section orders StatefulSet pods and
/// shows the node of DaemonSet pods.
pub(crate) const STATEFUL_SET_KIND: &str = "StatefulSet";
pub(crate) const DAEMON_SET_KIND: &str = "DaemonSet";
pub(crate) const REPLICA_SET_KIND: &str = "ReplicaSet";
pub(crate) const JOB_KIND: &str = "Job";

/// Who owns the pods that a drawer lists.
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
}

/// The characters of a pod-template hash: Kubernetes `SafeEncodeString` drops vowels, `0`,
/// `1`, `3`, and a few lookalikes.
const POD_TEMPLATE_HASH_ALPHABET: &str = "bcdfghjklmnpqrstvwxz2456789";

pub(crate) fn owns_pod(owner: &PodOwner, pod: &PodSummary) -> bool {
    match owner {
        PodOwner::Controller {
            namespace,
            kind,
            name,
        } => {
            *namespace == pod.namespace
                && pod
                    .controller
                    .as_ref()
                    .is_some_and(|controller| controller.kind == *kind && controller.name == *name)
        }
        PodOwner::Deployment { namespace, name } => {
            *namespace == pod.namespace
                && pod.controller.as_ref().is_some_and(|controller| {
                    controller.kind == REPLICA_SET_KIND
                        && is_deployment_replica_set(name, &controller.name)
                })
        }
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
    fn owns_pod_requires_same_namespace() {
        let owner = deployment("ns", "api");
        let controller = Some(("ReplicaSet", "api-7d9f8c"));
        assert!(owns_pod(&owner, &pod("ns", controller)));
        assert!(!owns_pod(&owner, &pod("other", controller)));
    }
}
