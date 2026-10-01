use std::fmt;

use futures::future::try_join_all;
use k8s_openapi::api::authorization::v1::{
    ResourceAttributes, SelfSubjectAccessReview, SelfSubjectAccessReviewSpec,
    SubjectAccessReviewStatus,
};
use kube::Api;
use kube::api::PostParams;

use crate::connection::{ClusterConnection, ClusterError};
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
    pub const ALL: [AccessCheck; 19] = [
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

impl AccessReport {
    pub fn is_allowed(&self, check: AccessCheck) -> bool {
        self.reviews
            .iter()
            .any(|review| review.check == check && review.decision == AccessDecision::Allowed)
    }
}

impl ClusterConnection {
    /// Asks the API server one SelfSubjectAccessReview question per
    /// `AccessCheck::ALL` entry, concurrently. The review is non-mutating and the only
    /// POST this crate sends. A denial is data;
    /// any request error fails the whole call, so a report is never partial.
    pub async fn review_access(&self, scope: NamespaceScope) -> Result<AccessReport, ClusterError> {
        let scope = &scope;
        let reviews = try_join_all(
            AccessCheck::ALL
                .into_iter()
                .map(|check| self.review_one(check, scope)),
        )
        .await?;
        Ok(AccessReport { reviews })
    }

    async fn review_one(
        &self,
        check: AccessCheck,
        scope: &NamespaceScope,
    ) -> Result<AccessReview, ClusterError> {
        let api = Api::<SelfSubjectAccessReview>::all(self.client().clone());
        let review = SelfSubjectAccessReview {
            spec: SelfSubjectAccessReviewSpec {
                resource_attributes: Some(resource_attributes(check, scope)),
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

fn resource_attributes(check: AccessCheck, scope: &NamespaceScope) -> ResourceAttributes {
    let target = check.target();
    let namespace = match scope {
        NamespaceScope::Named(namespace) if target.is_namespaced => Some(namespace.clone()),
        _ => None,
    };
    ResourceAttributes {
        group: Some(target.group.to_owned()),
        namespace,
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

    fn named(namespace: &str) -> NamespaceScope {
        NamespaceScope::Named(namespace.to_owned())
    }

    #[test]
    fn all_checks_cover_distinct_permissions() {
        assert_eq!(AccessCheck::ALL.len(), 19);
        let distinct: HashSet<_> = AccessCheck::ALL.into_iter().collect();
        assert_eq!(distinct.len(), 19);
    }

    #[test]
    fn pod_log_check_uses_get_on_pods_log() {
        let attributes = resource_attributes(AccessCheck::GetPodLogs, &NamespaceScope::All);
        assert_eq!(attributes.verb.as_deref(), Some("get"));
        assert_eq!(attributes.resource.as_deref(), Some("pods"));
        assert_eq!(attributes.subresource.as_deref(), Some("log"));
        assert_eq!(attributes.group.as_deref(), Some(""));
    }

    #[test]
    fn exec_and_port_forward_checks_use_create() {
        let exec = resource_attributes(AccessCheck::CreatePodExec, &NamespaceScope::All);
        assert_eq!(exec.verb.as_deref(), Some("create"));
        assert_eq!(exec.subresource.as_deref(), Some("exec"));
        let forward = resource_attributes(AccessCheck::CreatePodPortForward, &NamespaceScope::All);
        assert_eq!(forward.verb.as_deref(), Some("create"));
        assert_eq!(forward.subresource.as_deref(), Some("portforward"));
    }

    #[test]
    fn kind_checks_use_their_api_group() {
        let group = |check| resource_attributes(check, &NamespaceScope::All).group;
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
        let attributes = resource_attributes(AccessCheck::ListNamespaces, &named("team-a"));
        assert_eq!(attributes.namespace, None);
        assert_eq!(attributes.resource.as_deref(), Some("namespaces"));
        assert_eq!(attributes.verb.as_deref(), Some("list"));
    }

    #[test]
    fn named_scope_sets_namespace_on_namespaced_checks() {
        let attributes = resource_attributes(AccessCheck::ListPods, &named("team-a"));
        assert_eq!(attributes.namespace.as_deref(), Some("team-a"));
    }

    #[test]
    fn all_scope_leaves_namespace_unset() {
        for check in AccessCheck::ALL {
            let attributes = resource_attributes(check, &NamespaceScope::All);
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
            let attributes = resource_attributes(check, &named("team-a"));
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
            ]
        );
    }
}
