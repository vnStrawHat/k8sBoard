use std::fmt;

use futures::future::try_join_all;
use k8s_openapi::api::authorization::v1::{
    ResourceAttributes, SelfSubjectAccessReview, SelfSubjectAccessReviewSpec,
    SubjectAccessReviewStatus,
};
use kube::Api;
use kube::api::PostParams;

use crate::connection::{ClusterConnection, ClusterError};
use crate::metrics_api::METRICS_GROUP;
use crate::namespace::NamespaceScope;

/// One permission the UI needs to know about.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AccessCheck {
    ListPods,
    GetPodLogs,
    CreatePodExec,
    CreatePodPortForward,
    ListSecrets,
    ListNodes,
    GetNodeProxy,
    ListEvents,
    WatchPods,
    ListNamespaces,
    ListDeployments,
    ListStatefulSets,
    ListDaemonSets,
    ListReplicaSets,
    ListJobs,
    ListCronJobs,
    ListServices,
    ListIngresses,
    ListConfigMaps,
    ListPodMetrics,
    ListNodeMetrics,
    ListEndpointSlices,
    ListNetworkPolicies,
    ListHorizontalPodAutoscalers,
    ListResourceQuotas,
    ListPodDisruptionBudgets,
    ListPersistentVolumeClaims,
    ListPersistentVolumes,
    ListStorageClasses,
}

/// The API resource a check asks about.
struct CheckTarget {
    verb: &'static str,
    /// The API group; empty is the core group.
    group: &'static str,
    resource: &'static str,
    subresource: Option<&'static str>,
    is_namespaced: bool,
}

impl AccessCheck {
    pub const ALL: [AccessCheck; 29] = [
        Self::ListPods,
        Self::GetPodLogs,
        Self::CreatePodExec,
        Self::CreatePodPortForward,
        Self::ListSecrets,
        Self::ListNodes,
        Self::GetNodeProxy,
        Self::ListEvents,
        Self::WatchPods,
        Self::ListNamespaces,
        Self::ListDeployments,
        Self::ListStatefulSets,
        Self::ListDaemonSets,
        Self::ListReplicaSets,
        Self::ListJobs,
        Self::ListCronJobs,
        Self::ListServices,
        Self::ListIngresses,
        Self::ListConfigMaps,
        Self::ListPodMetrics,
        Self::ListNodeMetrics,
        Self::ListEndpointSlices,
        Self::ListNetworkPolicies,
        Self::ListHorizontalPodAutoscalers,
        Self::ListResourceQuotas,
        Self::ListPodDisruptionBudgets,
        Self::ListPersistentVolumeClaims,
        Self::ListPersistentVolumes,
        Self::ListStorageClasses,
    ];

    fn target(self) -> CheckTarget {
        let (verb, group, resource, subresource, is_namespaced) = match self {
            Self::ListPods => ("list", "", "pods", None, true),
            Self::GetPodLogs => ("get", "", "pods", Some("log"), true),
            Self::CreatePodExec => ("create", "", "pods", Some("exec"), true),
            Self::CreatePodPortForward => ("create", "", "pods", Some("portforward"), true),
            Self::ListSecrets => ("list", "", "secrets", None, true),
            Self::ListNodes => ("list", "", "nodes", None, false),
            Self::GetNodeProxy => ("get", "", "nodes", Some("proxy"), false),
            Self::ListEvents => ("list", "", "events", None, true),
            Self::WatchPods => ("watch", "", "pods", None, true),
            Self::ListNamespaces => ("list", "", "namespaces", None, false),
            Self::ListDeployments => ("list", "apps", "deployments", None, true),
            Self::ListStatefulSets => ("list", "apps", "statefulsets", None, true),
            Self::ListDaemonSets => ("list", "apps", "daemonsets", None, true),
            Self::ListReplicaSets => ("list", "apps", "replicasets", None, true),
            Self::ListJobs => ("list", "batch", "jobs", None, true),
            Self::ListCronJobs => ("list", "batch", "cronjobs", None, true),
            Self::ListServices => ("list", "", "services", None, true),
            Self::ListIngresses => ("list", "networking.k8s.io", "ingresses", None, true),
            Self::ListConfigMaps => ("list", "", "configmaps", None, true),
            Self::ListPodMetrics => ("list", METRICS_GROUP, "pods", None, true),
            Self::ListNodeMetrics => ("list", METRICS_GROUP, "nodes", None, false),
            Self::ListEndpointSlices => ("list", "discovery.k8s.io", "endpointslices", None, true),
            Self::ListNetworkPolicies => {
                ("list", "networking.k8s.io", "networkpolicies", None, true)
            }
            Self::ListHorizontalPodAutoscalers => (
                "list",
                "autoscaling",
                "horizontalpodautoscalers",
                None,
                true,
            ),
            Self::ListResourceQuotas => ("list", "", "resourcequotas", None, true),
            Self::ListPodDisruptionBudgets => {
                ("list", "policy", "poddisruptionbudgets", None, true)
            }
            Self::ListPersistentVolumeClaims => ("list", "", "persistentvolumeclaims", None, true),
            Self::ListPersistentVolumes => ("list", "", "persistentvolumes", None, false),
            Self::ListStorageClasses => ("list", "storage.k8s.io", "storageclasses", None, false),
        };
        CheckTarget {
            verb,
            group,
            resource,
            subresource,
            is_namespaced,
        }
    }
}

impl fmt::Display for AccessCheck {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let target = self.target();
        write!(formatter, "{} {}", target.verb, target.resource)?;
        // Only this group is spelled out, so the established wording of the built-in kinds
        // (`list deployments`) stays as it is.
        if target.group == METRICS_GROUP {
            write!(formatter, ".{METRICS_GROUP}")?;
        }
        match target.subresource {
            Some(subresource) => write!(formatter, "/{subresource}"),
            None => Ok(()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AccessDecision {
    Allowed,
    Denied { reason: Option<String> },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccessReview {
    pub check: AccessCheck,
    pub decision: AccessDecision,
}

/// One review per `AccessCheck::ALL` entry, in that order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccessReport {
    pub reviews: Vec<AccessReview>,
}

/// The decision of one check in one namespace; `None` is the cluster-wide review.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NamespaceAccess {
    pub namespace: Option<String>,
    pub decision: AccessDecision,
}

impl AccessReport {
    pub fn is_allowed(&self, check: AccessCheck) -> bool {
        self.reviews
            .iter()
            .any(|review| review.check == check && review.decision == AccessDecision::Allowed)
    }

    /// One review per check, in `AccessCheck::ALL` order: Allowed only if allowed in every
    /// report that contains it; else the first denial.
    pub(crate) fn all_of(reports: Vec<AccessReport>) -> AccessReport {
        let reviews = AccessCheck::ALL
            .into_iter()
            .filter_map(|check| {
                let mut decisions = reports
                    .iter()
                    .flat_map(|report| &report.reviews)
                    .filter(|review| review.check == check)
                    .map(|review| &review.decision)
                    .peekable();
                decisions.peek()?;
                let decision = decisions
                    .find(|decision| **decision != AccessDecision::Allowed)
                    .cloned()
                    .unwrap_or(AccessDecision::Allowed);
                Some(AccessReview { check, decision })
            })
            .collect();
        AccessReport { reviews }
    }
}

impl ClusterConnection {
    /// Asks the API server one SelfSubjectAccessReview question per
    /// `AccessCheck::ALL` entry, concurrently. The review is non-mutating and the only
    /// POST this crate sends. A denial is data;
    /// any request error fails the whole call, so a report is never partial.
    ///
    /// For `Several` the cluster-scoped checks run once, then the namespaced checks run
    /// one namespace at a time (22 x N + 4 requests); a check is allowed only when every
    /// namespace allows it. That gates menus, it never filters data.
    pub async fn review_access(&self, scope: NamespaceScope) -> Result<AccessReport, ClusterError> {
        let NamespaceScope::Several(namespaces) = &scope else {
            let namespace = scope.namespaces().first().map(String::as_str);
            let reviews = self.review_checks(&AccessCheck::ALL, namespace).await?;
            return Ok(AccessReport { reviews });
        };
        let (namespaced, cluster_scoped): (Vec<_>, Vec<_>) = AccessCheck::ALL
            .into_iter()
            .partition(|check| check.target().is_namespaced);
        let mut reports = vec![AccessReport {
            reviews: self.review_checks(&cluster_scoped, None).await?,
        }];
        for namespace in namespaces {
            reports.push(AccessReport {
                reviews: self.review_checks(&namespaced, Some(namespace)).await?,
            });
        }
        Ok(AccessReport::all_of(reports))
    }

    /// One review of `check` per namespace of `scope` (one cluster-wide review for `All`), with
    /// the results in scope order. The requests run concurrently: at most 5 picked namespaces,
    /// so at most 5 are in flight. Like `review_access`, a request error fails the whole call.
    pub async fn review_namespaces(
        &self,
        check: AccessCheck,
        scope: &NamespaceScope,
    ) -> Result<Vec<NamespaceAccess>, ClusterError> {
        try_join_all(
            review_targets(scope)
                .into_iter()
                .map(|namespace| async move {
                    let review = self.review_one(check, namespace).await?;
                    Ok(NamespaceAccess {
                        namespace: namespace.map(str::to_owned),
                        decision: review.decision,
                    })
                }),
        )
        .await
    }

    /// Reviews `checks` concurrently. `namespace` applies to the namespaced checks only.
    async fn review_checks(
        &self,
        checks: &[AccessCheck],
        namespace: Option<&str>,
    ) -> Result<Vec<AccessReview>, ClusterError> {
        try_join_all(
            checks
                .iter()
                .map(|check| self.review_one(*check, namespace)),
        )
        .await
    }

    async fn review_one(
        &self,
        check: AccessCheck,
        namespace: Option<&str>,
    ) -> Result<AccessReview, ClusterError> {
        let api = Api::<SelfSubjectAccessReview>::all(self.client().clone());
        let review = SelfSubjectAccessReview {
            spec: SelfSubjectAccessReviewSpec {
                resource_attributes: Some(resource_attributes(check, namespace)),
                non_resource_attributes: None,
            },
            ..Default::default()
        };
        let response = self
            .run(
                "reviewing access",
                api.create(&PostParams::default(), &review),
            )
            .await?;
        Ok(AccessReview {
            check,
            decision: access_decision(response.status),
        })
    }
}

/// The namespace of each review `scope` needs: `None` for the single cluster-wide review.
fn review_targets(scope: &NamespaceScope) -> Vec<Option<&str>> {
    match scope {
        NamespaceScope::All => vec![None],
        _ => scope
            .namespaces()
            .iter()
            .map(|namespace| Some(namespace.as_str()))
            .collect(),
    }
}

/// `namespace` is ignored by cluster-scoped checks.
fn resource_attributes(check: AccessCheck, namespace: Option<&str>) -> ResourceAttributes {
    let target = check.target();
    ResourceAttributes {
        group: Some(target.group.to_owned()),
        namespace: namespace
            .filter(|_| target.is_namespaced)
            .map(str::to_owned),
        resource: Some(target.resource.to_owned()),
        subresource: target.subresource.map(str::to_owned),
        verb: Some(target.verb.to_owned()),
        ..Default::default()
    }
}

fn access_decision(status: Option<SubjectAccessReviewStatus>) -> AccessDecision {
    let Some(status) = status else {
        return AccessDecision::Denied { reason: None };
    };
    if status.allowed {
        return AccessDecision::Allowed;
    }
    let reason = [status.reason, status.evaluation_error]
        .into_iter()
        .flatten()
        .find(|text| !text.is_empty());
    AccessDecision::Denied { reason }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    fn status(
        allowed: bool,
        reason: Option<&str>,
        error: Option<&str>,
    ) -> SubjectAccessReviewStatus {
        SubjectAccessReviewStatus {
            allowed,
            reason: reason.map(str::to_owned),
            evaluation_error: error.map(str::to_owned),
            denied: None,
        }
    }

    #[test]
    fn all_checks_cover_distinct_permissions() {
        assert_eq!(AccessCheck::ALL.len(), 29);
        let distinct: HashSet<_> = AccessCheck::ALL.into_iter().collect();
        assert_eq!(distinct.len(), 29);
    }

    #[test]
    fn pod_log_check_uses_get_on_pods_log() {
        let attributes = resource_attributes(AccessCheck::GetPodLogs, None);
        assert_eq!(attributes.verb.as_deref(), Some("get"));
        assert_eq!(attributes.resource.as_deref(), Some("pods"));
        assert_eq!(attributes.subresource.as_deref(), Some("log"));
        assert_eq!(attributes.group.as_deref(), Some(""));
    }

    #[test]
    fn exec_and_port_forward_checks_use_create() {
        let exec = resource_attributes(AccessCheck::CreatePodExec, None);
        assert_eq!(exec.verb.as_deref(), Some("create"));
        assert_eq!(exec.subresource.as_deref(), Some("exec"));
        let forward = resource_attributes(AccessCheck::CreatePodPortForward, None);
        assert_eq!(forward.verb.as_deref(), Some("create"));
        assert_eq!(forward.subresource.as_deref(), Some("portforward"));
    }

    #[test]
    fn kind_checks_use_their_api_group() {
        let group = |check| resource_attributes(check, None).group;
        for check in [
            AccessCheck::ListDeployments,
            AccessCheck::ListStatefulSets,
            AccessCheck::ListDaemonSets,
            AccessCheck::ListReplicaSets,
        ] {
            assert_eq!(group(check).as_deref(), Some("apps"), "{check}");
        }
        for check in [AccessCheck::ListJobs, AccessCheck::ListCronJobs] {
            assert_eq!(group(check).as_deref(), Some("batch"), "{check}");
        }
        assert_eq!(
            group(AccessCheck::ListIngresses).as_deref(),
            Some("networking.k8s.io")
        );
        for check in [
            AccessCheck::ListServices,
            AccessCheck::ListConfigMaps,
            AccessCheck::ListNamespaces,
            AccessCheck::ListPods,
        ] {
            assert_eq!(group(check).as_deref(), Some(""), "{check}");
        }
    }

    #[test]
    fn namespace_check_is_cluster_scoped() {
        let attributes = resource_attributes(AccessCheck::ListNamespaces, Some("team-a"));
        assert_eq!(attributes.namespace, None);
        assert_eq!(attributes.resource.as_deref(), Some("namespaces"));
        assert_eq!(attributes.verb.as_deref(), Some("list"));
    }

    #[test]
    fn named_scope_sets_namespace_on_namespaced_checks() {
        let attributes = resource_attributes(AccessCheck::ListPods, Some("team-a"));
        assert_eq!(attributes.namespace.as_deref(), Some("team-a"));
    }

    #[test]
    fn all_scope_leaves_namespace_unset() {
        for check in AccessCheck::ALL {
            let attributes = resource_attributes(check, None);
            assert_eq!(attributes.namespace, None, "{check}");
        }
    }

    #[test]
    fn cluster_scoped_checks_ignore_scope() {
        for check in [
            AccessCheck::ListNodes,
            AccessCheck::GetNodeProxy,
            AccessCheck::ListNamespaces,
        ] {
            let attributes = resource_attributes(check, Some("team-a"));
            assert_eq!(attributes.namespace, None, "{check}");
        }
    }

    #[test]
    fn allowed_status_is_allowed() {
        assert_eq!(
            access_decision(Some(status(true, None, None))),
            AccessDecision::Allowed
        );
    }

    #[test]
    fn denied_status_carries_reason() {
        assert_eq!(
            access_decision(Some(status(false, Some("no RBAC rule"), Some("ignored")))),
            AccessDecision::Denied {
                reason: Some("no RBAC rule".to_owned())
            }
        );
    }

    #[test]
    fn evaluation_error_is_used_when_reason_is_empty() {
        assert_eq!(
            access_decision(Some(status(false, Some(""), Some("webhook failed")))),
            AccessDecision::Denied {
                reason: Some("webhook failed".to_owned())
            }
        );
        assert_eq!(
            access_decision(Some(status(false, None, None))),
            AccessDecision::Denied { reason: None }
        );
    }

    #[test]
    fn missing_status_is_denied_without_reason() {
        assert_eq!(
            access_decision(None),
            AccessDecision::Denied { reason: None }
        );
    }

    #[test]
    fn report_is_allowed_looks_up_check() {
        let report = AccessReport {
            reviews: vec![
                AccessReview {
                    check: AccessCheck::ListPods,
                    decision: AccessDecision::Allowed,
                },
                AccessReview {
                    check: AccessCheck::ListSecrets,
                    decision: AccessDecision::Denied { reason: None },
                },
            ],
        };
        assert!(report.is_allowed(AccessCheck::ListPods));
        assert!(!report.is_allowed(AccessCheck::ListSecrets));
        assert!(!report.is_allowed(AccessCheck::ListNodes));
    }

    fn review(check: AccessCheck, reason: Option<&str>) -> AccessReview {
        let decision = match reason {
            None => AccessDecision::Allowed,
            Some(reason) => AccessDecision::Denied {
                reason: Some(reason.to_owned()),
            },
        };
        AccessReview { check, decision }
    }

    #[test]
    fn all_of_allows_only_when_every_report_allows() {
        let cluster_scoped = AccessReport {
            reviews: vec![review(AccessCheck::ListNodes, None)],
        };
        let first = AccessReport {
            reviews: vec![
                review(AccessCheck::ListPods, None),
                review(AccessCheck::ListSecrets, Some("first")),
            ],
        };
        let second = AccessReport {
            reviews: vec![
                review(AccessCheck::ListPods, Some("second")),
                review(AccessCheck::ListSecrets, Some("second")),
            ],
        };
        let report = AccessReport::all_of(vec![cluster_scoped, first, second]);
        assert_eq!(
            report.reviews,
            [
                review(AccessCheck::ListPods, Some("second")),
                review(AccessCheck::ListSecrets, Some("first")),
                review(AccessCheck::ListNodes, None),
            ]
        );
    }

    #[test]
    fn check_display_matches_kubectl_wording() {
        let texts: Vec<_> = AccessCheck::ALL.iter().map(ToString::to_string).collect();
        assert_eq!(
            texts,
            [
                "list pods",
                "get pods/log",
                "create pods/exec",
                "create pods/portforward",
                "list secrets",
                "list nodes",
                "get nodes/proxy",
                "list events",
                "watch pods",
                "list namespaces",
                "list deployments",
                "list statefulsets",
                "list daemonsets",
                "list replicasets",
                "list jobs",
                "list cronjobs",
                "list services",
                "list ingresses",
                "list configmaps",
                "list pods.metrics.k8s.io",
                "list nodes.metrics.k8s.io",
                "list endpointslices",
                "list networkpolicies",
                "list horizontalpodautoscalers",
                "list resourcequotas",
                "list poddisruptionbudgets",
                "list persistentvolumeclaims",
                "list persistentvolumes",
                "list storageclasses",
            ]
        );
    }

    #[test]
    fn policy_checks_use_their_api_groups() {
        let expected = [
            (
                AccessCheck::ListNetworkPolicies,
                "networking.k8s.io",
                "networkpolicies",
            ),
            (
                AccessCheck::ListHorizontalPodAutoscalers,
                "autoscaling",
                "horizontalpodautoscalers",
            ),
            (AccessCheck::ListResourceQuotas, "", "resourcequotas"),
            (
                AccessCheck::ListPodDisruptionBudgets,
                "policy",
                "poddisruptionbudgets",
            ),
        ];
        for (check, group, resource) in expected {
            let attributes = resource_attributes(check, Some("team-a"));
            assert_eq!(attributes.group.as_deref(), Some(group), "{check}");
            assert_eq!(attributes.resource.as_deref(), Some(resource), "{check}");
            assert_eq!(attributes.verb.as_deref(), Some("list"), "{check}");
            assert_eq!(attributes.namespace.as_deref(), Some("team-a"), "{check}");
        }
    }

    #[test]
    fn storage_checks_use_their_api_groups_and_scope() {
        let expected = [
            (
                AccessCheck::ListPersistentVolumeClaims,
                "",
                "persistentvolumeclaims",
                Some("team-a"),
            ),
            (
                AccessCheck::ListPersistentVolumes,
                "",
                "persistentvolumes",
                None,
            ),
            (
                AccessCheck::ListStorageClasses,
                "storage.k8s.io",
                "storageclasses",
                None,
            ),
        ];
        for (check, group, resource, namespace) in expected {
            let attributes = resource_attributes(check, Some("team-a"));
            assert_eq!(attributes.group.as_deref(), Some(group), "{check}");
            assert_eq!(attributes.resource.as_deref(), Some(resource), "{check}");
            assert_eq!(attributes.verb.as_deref(), Some("list"), "{check}");
            assert_eq!(attributes.namespace.as_deref(), namespace, "{check}");
        }
    }

    #[test]
    fn endpoint_slices_check_targets_discovery_group() {
        let attributes = resource_attributes(AccessCheck::ListEndpointSlices, Some("team-a"));
        assert_eq!(attributes.group.as_deref(), Some("discovery.k8s.io"));
        assert_eq!(attributes.resource.as_deref(), Some("endpointslices"));
        assert_eq!(attributes.verb.as_deref(), Some("list"));
        assert_eq!(attributes.namespace.as_deref(), Some("team-a"));
    }

    #[test]
    fn metrics_checks_target_the_metrics_group() {
        let pods = resource_attributes(AccessCheck::ListPodMetrics, Some("team-a"));
        assert_eq!(pods.group.as_deref(), Some("metrics.k8s.io"));
        assert_eq!(pods.resource.as_deref(), Some("pods"));
        assert_eq!(pods.verb.as_deref(), Some("list"));
        assert_eq!(pods.namespace.as_deref(), Some("team-a"));
        let nodes = resource_attributes(AccessCheck::ListNodeMetrics, Some("team-a"));
        assert_eq!(nodes.group.as_deref(), Some("metrics.k8s.io"));
        assert_eq!(nodes.resource.as_deref(), Some("nodes"));
        assert_eq!(nodes.namespace, None);
        assert_eq!(AccessCheck::ALL.len(), 29);
    }

    #[test]
    fn display_names_the_metrics_group_only() {
        assert_eq!(
            AccessCheck::ListPodMetrics.to_string(),
            "list pods.metrics.k8s.io"
        );
        assert_eq!(AccessCheck::ListPods.to_string(), "list pods");
        assert_eq!(AccessCheck::ListDeployments.to_string(), "list deployments");
    }

    #[test]
    fn review_targets_follow_the_scope() {
        assert_eq!(review_targets(&NamespaceScope::All), [None]);
        assert_eq!(
            review_targets(&NamespaceScope::Named("a".to_owned())),
            [Some("a")]
        );
        let several = NamespaceScope::of_namespaces(["b".to_owned(), "a".to_owned()]);
        assert_eq!(review_targets(&several), [Some("a"), Some("b")]);
    }
}
