use std::fmt;

use futures::future::try_join_all;
use k8s_openapi::api::authorization::v1::{
    NonResourceAttributes, ResourceAttributes, SelfSubjectAccessReview,
    SelfSubjectAccessReviewSpec, SelfSubjectRulesReview, SelfSubjectRulesReviewSpec,
    SubjectAccessReviewStatus, SubjectRulesReviewStatus,
};
use k8s_openapi::serde::Serialize;
use k8s_openapi::serde::de::DeserializeOwned;
use kube::Api;
use kube::api::PostParams;

use crate::connection::{ClusterConnection, ClusterError};
use crate::custom_resource_definition::{CustomResourceType, ResourceScope};
use crate::debug_shell::AttachPermit;
use crate::metrics_api::METRICS_GROUP;
use crate::namespace::NamespaceScope;
use crate::object_yaml::ObjectKind;
use crate::pod_shell::ExecPermit;
use crate::port_forward::PortForwardPermit;
use crate::rbac_evaluation::{AccessRequest, RequestTarget, ResourceRequest};
use crate::role::RbacRule;

const RBAC_GROUP: &str = "rbac.authorization.k8s.io";

/// One permission the UI needs to know about.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AccessCheck {
    ListPods,
    GetPodLogs,
    /// Servers before 1.35 authorize a WebSocket exec as `get` (kubernetes#78741); 1.35 adds
    /// `create` (KEP-4006). A shell needs both.
    GetPodExec,
    CreatePodExec,
    /// Port-forward is authorized like exec: `get` before 1.35, plus `create` after.
    GetPodPortForward,
    CreatePodPortForward,
    /// A debug shell attaches like a shell execs: `get` before 1.35, plus `create` after (0037).
    GetPodAttach,
    CreatePodAttach,
    /// Node shell (0037): creates and deletes its own privileged pod.
    CreatePods,
    DeletePods,
    /// Debug container (0037): `patch pods/ephemeralcontainers`.
    PatchPodEphemeralContainers,
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
    ListLimitRanges,
    ListPodDisruptionBudgets,
    ListPersistentVolumeClaims,
    ListPersistentVolumes,
    ListStorageClasses,
    ListServiceAccounts,
    ListRoles,
    ListClusterRoles,
    ListRoleBindings,
    ListClusterRoleBindings,
    ListCustomResourceDefinitions,
    /// Cordon and uncordon (0030): the one permission of the first write.
    PatchNodes,
    /// Scale a Deployment (0032): `patch deployments/scale`.
    PatchDeploymentScale,
    PatchStatefulSetScale,
    /// Restart, pause, resume, and roll back a Deployment (0032).
    PatchDeployments,
    PatchStatefulSets,
    PatchDaemonSets,
    PatchCronJobs,
    /// Trigger now and Re-run create a Job (0032).
    CreateJobs,
    /// HPA min / max, PVC Expand, and Set as default storage class (0032b).
    PatchHorizontalPodAutoscalers,
    PatchPersistentVolumeClaims,
    PatchStorageClasses,
    /// Drain (0034): `create pods/eviction`, namespaced.
    CreatePodEviction,
    /// Edit YAML (0031): `update` on the kind's resource. Reviewed lazily per kind, so it is not in
    /// `ALL`.
    Update(ObjectKind),
    /// Delete (0033): `delete` on the kind's resource. Lazy per kind, so not in `ALL`.
    Delete(ObjectKind),
    /// Edit values (0047): `patch` on the kind's resource. Lazy per kind, so not in `ALL`.
    Patch(ObjectKind),
    /// New from templates (0042): `create` on the kind's resource. Lazy per kind, so not in `ALL`.
    Create(ObjectKind),
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
    pub const ALL: [AccessCheck; 55] = [
        Self::ListPods,
        Self::GetPodLogs,
        Self::GetPodExec,
        Self::CreatePodExec,
        Self::GetPodPortForward,
        Self::CreatePodPortForward,
        Self::GetPodAttach,
        Self::CreatePodAttach,
        Self::CreatePods,
        Self::DeletePods,
        Self::PatchPodEphemeralContainers,
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
        Self::ListLimitRanges,
        Self::ListPodDisruptionBudgets,
        Self::ListPersistentVolumeClaims,
        Self::ListPersistentVolumes,
        Self::ListStorageClasses,
        Self::ListServiceAccounts,
        Self::ListRoles,
        Self::ListClusterRoles,
        Self::ListRoleBindings,
        Self::ListClusterRoleBindings,
        Self::ListCustomResourceDefinitions,
        Self::PatchNodes,
        Self::PatchDeploymentScale,
        Self::PatchStatefulSetScale,
        Self::PatchDeployments,
        Self::PatchStatefulSets,
        Self::PatchDaemonSets,
        Self::PatchCronJobs,
        Self::CreateJobs,
        Self::PatchHorizontalPodAutoscalers,
        Self::PatchPersistentVolumeClaims,
        Self::PatchStorageClasses,
        Self::CreatePodEviction,
    ];

    fn target(self) -> CheckTarget {
        let (verb, group, resource, subresource, is_namespaced) = match self {
            Self::ListPods => ("list", "", "pods", None, true),
            Self::GetPodLogs => ("get", "", "pods", Some("log"), true),
            Self::GetPodExec => ("get", "", "pods", Some("exec"), true),
            Self::CreatePodExec => ("create", "", "pods", Some("exec"), true),
            Self::GetPodPortForward => ("get", "", "pods", Some("portforward"), true),
            Self::CreatePodPortForward => ("create", "", "pods", Some("portforward"), true),
            Self::GetPodAttach => ("get", "", "pods", Some("attach"), true),
            Self::CreatePodAttach => ("create", "", "pods", Some("attach"), true),
            Self::CreatePods => ("create", "", "pods", None, true),
            Self::DeletePods => ("delete", "", "pods", None, true),
            Self::PatchPodEphemeralContainers => {
                ("patch", "", "pods", Some("ephemeralcontainers"), true)
            }
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
            Self::ListLimitRanges => ("list", "", "limitranges", None, true),
            Self::ListPodDisruptionBudgets => {
                ("list", "policy", "poddisruptionbudgets", None, true)
            }
            Self::ListPersistentVolumeClaims => ("list", "", "persistentvolumeclaims", None, true),
            Self::ListPersistentVolumes => ("list", "", "persistentvolumes", None, false),
            Self::ListStorageClasses => ("list", "storage.k8s.io", "storageclasses", None, false),
            Self::ListServiceAccounts => ("list", "", "serviceaccounts", None, true),
            Self::ListRoles => ("list", RBAC_GROUP, "roles", None, true),
            Self::ListClusterRoles => ("list", RBAC_GROUP, "clusterroles", None, false),
            Self::ListRoleBindings => ("list", RBAC_GROUP, "rolebindings", None, true),
            Self::ListClusterRoleBindings => {
                ("list", RBAC_GROUP, "clusterrolebindings", None, false)
            }
            Self::ListCustomResourceDefinitions => (
                "list",
                "apiextensions.k8s.io",
                "customresourcedefinitions",
                None,
                false,
            ),
            Self::PatchNodes => ("patch", "", "nodes", None, false),
            Self::PatchDeploymentScale => ("patch", "apps", "deployments", Some("scale"), true),
            Self::PatchStatefulSetScale => ("patch", "apps", "statefulsets", Some("scale"), true),
            Self::PatchDeployments => ("patch", "apps", "deployments", None, true),
            Self::PatchStatefulSets => ("patch", "apps", "statefulsets", None, true),
            Self::PatchDaemonSets => ("patch", "apps", "daemonsets", None, true),
            Self::PatchCronJobs => ("patch", "batch", "cronjobs", None, true),
            Self::CreateJobs => ("create", "batch", "jobs", None, true),
            Self::PatchHorizontalPodAutoscalers => (
                "patch",
                "autoscaling",
                "horizontalpodautoscalers",
                None,
                true,
            ),
            Self::PatchPersistentVolumeClaims => {
                ("patch", "", "persistentvolumeclaims", None, true)
            }
            Self::PatchStorageClasses => ("patch", "storage.k8s.io", "storageclasses", None, false),
            Self::CreatePodEviction => ("create", "", "pods", Some("eviction"), true),
            Self::Update(kind) => {
                let (group, resource) = kind.resource();
                ("update", group, resource, None, kind.is_namespaced())
            }
            Self::Delete(kind) => {
                let (group, resource) = kind.resource();
                ("delete", group, resource, None, kind.is_namespaced())
            }
            Self::Patch(kind) => {
                let (group, resource) = kind.resource();
                ("patch", group, resource, None, kind.is_namespaced())
            }
            Self::Create(kind) => {
                let (group, resource) = kind.resource();
                ("create", group, resource, None, kind.is_namespaced())
            }
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

/// One review per reviewed check, in the order they were asked (`AccessCheck::ALL` for
/// `review_access`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccessReport {
    pub reviews: Vec<AccessReview>,
}

/// What the API server says the caller may do in one namespace (SelfSubjectRulesReview).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RulesReview {
    /// Resource rules first, then non-resource rules; one rule per binding, duplicates kept.
    pub rules: Vec<RbacRule>,
    /// The authorizers could not list every rule, so the rules shown are granted but others
    /// may exist.
    pub is_incomplete: bool,
    pub evaluation_error: Option<String>,
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

    /// The proof a shell may open: `Some` only when both exec verbs are allowed. Unknown,
    /// missing, or one denial means `None` (fail closed). The only non-test `ExecPermit`.
    pub fn exec_permit(&self) -> Option<ExecPermit> {
        (self.is_allowed(AccessCheck::GetPodExec) && self.is_allowed(AccessCheck::CreatePodExec))
            .then(ExecPermit::granted)
    }

    /// The proof a port-forward may start: `Some` only when both verbs are allowed. The only
    /// non-test `PortForwardPermit`.
    pub fn port_forward_permit(&self) -> Option<PortForwardPermit> {
        (self.is_allowed(AccessCheck::GetPodPortForward)
            && self.is_allowed(AccessCheck::CreatePodPortForward))
        .then(PortForwardPermit::granted)
    }

    /// The proof a debug shell may attach: `Some` only when both attach verbs are allowed. The only
    /// non-test `AttachPermit`.
    pub fn attach_permit(&self) -> Option<AttachPermit> {
        (self.is_allowed(AccessCheck::GetPodAttach)
            && self.is_allowed(AccessCheck::CreatePodAttach))
        .then(AttachPermit::granted)
    }

    /// One review per entry of `checks` (the list that was reviewed), in that order: Allowed only
    /// if allowed in every report that contains it; else the first denial. A check no report
    /// contains is left out.
    pub(crate) fn all_of(checks: &[AccessCheck], reports: Vec<AccessReport>) -> AccessReport {
        let reviews = checks
            .iter()
            .copied()
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
        self.review_access_for(&AccessCheck::ALL, scope).await
    }

    /// `review_access` over a given list, with the same scope rules: the lazy per-kind checks
    /// (`Update`, 0031) go through here.
    pub async fn review_access_for(
        &self,
        checks: &[AccessCheck],
        scope: NamespaceScope,
    ) -> Result<AccessReport, ClusterError> {
        let NamespaceScope::Several(namespaces) = &scope else {
            let namespace = scope.namespaces().first().map(String::as_str);
            let reviews = self.review_checks(checks, namespace).await?;
            return Ok(AccessReport { reviews });
        };
        let (namespaced, cluster_scoped): (Vec<_>, Vec<_>) = checks
            .iter()
            .copied()
            .partition(|check| check.target().is_namespaced);
        let mut reports = vec![AccessReport {
            reviews: self.review_checks(&cluster_scoped, None).await?,
        }];
        for namespace in namespaces {
            reports.push(AccessReport {
                reviews: self.review_checks(&namespaced, Some(namespace)).await?,
            });
        }
        Ok(AccessReport::all_of(checks, reports))
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
        let decision = self
            .review_spec(resource_spec(resource_attributes(check, namespace)))
            .await?;
        Ok(AccessReview { check, decision })
    }

    /// Asks the API server (SelfSubjectRulesReview, non-mutating) which rules apply to the
    /// caller in `namespace`.
    pub async fn review_rules(&self, namespace: &str) -> Result<RulesReview, ClusterError> {
        let review = SelfSubjectRulesReview {
            spec: SelfSubjectRulesReviewSpec {
                namespace: Some(namespace.to_owned()),
            },
            ..Default::default()
        };
        let response = self.post_review("reviewing rules", review).await?;
        Ok(rules_review(response.status))
    }

    /// Asks whether the caller may make `request` (SelfSubjectAccessReview, non-mutating).
    pub async fn review_request(
        &self,
        request: &AccessRequest,
    ) -> Result<AccessDecision, ClusterError> {
        self.review_spec(request_spec(request)).await
    }

    /// Asks whether the user may `list` a custom resource. One cluster-wide review for `All` or a
    /// cluster-scoped resource, else one per picked namespace, concurrently (at most 5 picked
    /// namespaces, so at most 5 in flight); the first denial wins. Like `review_access`, a request
    /// error fails the whole call.
    pub async fn review_custom_access(
        &self,
        resource: &CustomResourceType,
        scope: &NamespaceScope,
    ) -> Result<AccessDecision, ClusterError> {
        let namespaces = match resource.scope {
            ResourceScope::Cluster => vec![None],
            ResourceScope::Namespaced => review_targets(scope),
        };
        let decisions = try_join_all(namespaces.into_iter().map(|namespace| {
            self.review_spec(resource_spec(custom_resource_attributes(
                resource, namespace,
            )))
        }))
        .await?;
        Ok(first_denial(decisions))
    }

    async fn review_spec(
        &self,
        spec: SelfSubjectAccessReviewSpec,
    ) -> Result<AccessDecision, ClusterError> {
        let review = SelfSubjectAccessReview {
            spec,
            ..Default::default()
        };
        let response = self.post_review("reviewing access", review).await?;
        Ok(access_decision(response.status))
    }

    /// Posts one review object and returns the answered review. The reviews only ask the API server
    /// what the caller may do; they change nothing, so they are the one `create` outside
    /// `object_write.rs` (the SSAR row of the 0030 allow-list).
    #[allow(clippy::disallowed_methods)]
    async fn post_review<K: AccessReviewObject>(
        &self,
        action: &'static str,
        review: K,
    ) -> Result<K, ClusterError> {
        let api = Api::<K>::all(self.client().clone());
        self.run(action, api.create(&PostParams::default(), &review))
            .await
    }
}

/// The objects `post_review` may create. The trait is private and has two implementors, so no
/// other kind can go through the one `create` that is allowed outside `object_write.rs`.
trait AccessReviewObject:
    kube::Resource<DynamicType = ()> + Clone + Serialize + DeserializeOwned + fmt::Debug
{
}

impl AccessReviewObject for SelfSubjectAccessReview {}

impl AccessReviewObject for SelfSubjectRulesReview {}

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

/// The one builder of review attributes: checks, custom resources, and typed requests all
/// go through it.
fn attributes_of(verb: &str, resource: &ResourceRequest) -> ResourceAttributes {
    ResourceAttributes {
        group: Some(resource.group.clone()),
        namespace: resource.namespace.clone(),
        resource: Some(resource.resource.clone()),
        subresource: resource.subresource.clone(),
        name: resource.name.clone(),
        verb: Some(verb.to_owned()),
        ..Default::default()
    }
}

fn resource_spec(attributes: ResourceAttributes) -> SelfSubjectAccessReviewSpec {
    SelfSubjectAccessReviewSpec {
        resource_attributes: Some(attributes),
        non_resource_attributes: None,
    }
}

/// `namespace` is ignored by cluster-scoped checks.
fn resource_attributes(check: AccessCheck, namespace: Option<&str>) -> ResourceAttributes {
    let target = check.target();
    let resource = ResourceRequest {
        group: target.group.to_owned(),
        resource: target.resource.to_owned(),
        subresource: target.subresource.map(str::to_owned),
        name: None,
        namespace: namespace
            .filter(|_| target.is_namespaced)
            .map(str::to_owned),
    };
    attributes_of(target.verb, &resource)
}

/// The `list` question for a custom resource; `namespace` is ignored for cluster-scoped ones.
fn custom_resource_attributes(
    resource: &CustomResourceType,
    namespace: Option<&str>,
) -> ResourceAttributes {
    let request = ResourceRequest {
        group: resource.group.clone(),
        resource: resource.plural.clone(),
        subresource: None,
        name: None,
        namespace: namespace
            .filter(|_| resource.scope == ResourceScope::Namespaced)
            .map(str::to_owned),
    };
    attributes_of("list", &request)
}

fn request_spec(request: &AccessRequest) -> SelfSubjectAccessReviewSpec {
    match &request.target {
        RequestTarget::Resource(resource) => resource_spec(attributes_of(&request.verb, resource)),
        RequestTarget::NonResource { path } => SelfSubjectAccessReviewSpec {
            resource_attributes: None,
            non_resource_attributes: Some(NonResourceAttributes {
                path: Some(path.clone()),
                verb: Some(request.verb.clone()),
            }),
        },
    }
}

fn rules_review(status: Option<SubjectRulesReviewStatus>) -> RulesReview {
    let Some(status) = status else {
        return RulesReview {
            rules: Vec::new(),
            is_incomplete: false,
            evaluation_error: None,
        };
    };
    let resource_rules = status.resource_rules.into_iter().map(|rule| RbacRule {
        api_groups: rule.api_groups.unwrap_or_default(),
        resources: rule.resources.unwrap_or_default(),
        resource_names: rule.resource_names.unwrap_or_default(),
        verbs: rule.verbs,
        non_resource_urls: Vec::new(),
    });
    let non_resource_rules = status.non_resource_rules.into_iter().map(|rule| RbacRule {
        api_groups: Vec::new(),
        resources: Vec::new(),
        resource_names: Vec::new(),
        verbs: rule.verbs,
        non_resource_urls: rule.non_resource_urls.unwrap_or_default(),
    });
    RulesReview {
        rules: resource_rules.chain(non_resource_rules).collect(),
        is_incomplete: status.incomplete,
        evaluation_error: status.evaluation_error.filter(|error| !error.is_empty()),
    }
}

/// Allowed only when every decision allows; else the first denial.
fn first_denial(decisions: Vec<AccessDecision>) -> AccessDecision {
    decisions
        .into_iter()
        .find(|decision| *decision != AccessDecision::Allowed)
        .unwrap_or(AccessDecision::Allowed)
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

    use k8s_openapi::api::authorization::v1::{NonResourceRule, ResourceRule};

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
        assert_eq!(AccessCheck::ALL.len(), 55);
        let distinct: HashSet<_> = AccessCheck::ALL.into_iter().collect();
        assert_eq!(distinct.len(), 55);
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

    fn assert_get_check(check: AccessCheck, subresource: &str) {
        let attributes = resource_attributes(check, Some("team-a"));
        assert_eq!(attributes.verb.as_deref(), Some("get"), "{check}");
        assert_eq!(attributes.group.as_deref(), Some(""), "{check}");
        assert_eq!(attributes.resource.as_deref(), Some("pods"), "{check}");
        assert_eq!(
            attributes.subresource.as_deref(),
            Some(subresource),
            "{check}"
        );
        assert_eq!(attributes.namespace.as_deref(), Some("team-a"), "{check}");
    }

    #[test]
    fn get_exec_check_targets_the_subresource() {
        assert_get_check(AccessCheck::GetPodExec, "exec");
    }

    #[test]
    fn get_port_forward_check_targets_the_subresource() {
        assert_get_check(AccessCheck::GetPodPortForward, "portforward");
    }

    #[test]
    fn attach_checks_use_get_and_create() {
        assert_get_check(AccessCheck::GetPodAttach, "attach");
        let create = resource_attributes(AccessCheck::CreatePodAttach, Some("team-a"));
        assert_eq!(create.verb.as_deref(), Some("create"));
        assert_eq!(create.resource.as_deref(), Some("pods"));
        assert_eq!(create.subresource.as_deref(), Some("attach"));
    }

    #[test]
    fn ephemeral_check_targets_the_subresource() {
        let attributes = resource_attributes(AccessCheck::PatchPodEphemeralContainers, Some("a"));
        assert_eq!(attributes.verb.as_deref(), Some("patch"));
        assert_eq!(attributes.resource.as_deref(), Some("pods"));
        assert_eq!(
            attributes.subresource.as_deref(),
            Some("ephemeralcontainers")
        );
        assert_eq!(attributes.namespace.as_deref(), Some("a"));
    }

    #[test]
    fn pod_create_and_delete_checks_have_no_subresource() {
        for (check, verb) in [
            (AccessCheck::CreatePods, "create"),
            (AccessCheck::DeletePods, "delete"),
        ] {
            let attributes = resource_attributes(check, Some("kube-system"));
            assert_eq!(attributes.verb.as_deref(), Some(verb), "{check}");
            assert_eq!(attributes.resource.as_deref(), Some("pods"), "{check}");
            assert_eq!(attributes.subresource, None, "{check}");
        }
    }

    #[test]
    fn attach_permit_needs_get_and_create() {
        let both = [AccessCheck::GetPodAttach, AccessCheck::CreatePodAttach];
        assert!(report_allowing(&both).attach_permit().is_some());
        assert!(report_allowing(&both[..1]).attach_permit().is_none());
        assert!(report_allowing(&both[1..]).attach_permit().is_none());
        // Exec rights do not open an attach.
        let exec = [AccessCheck::GetPodExec, AccessCheck::CreatePodExec];
        assert!(report_allowing(&exec).attach_permit().is_none());
        let missing = AccessReport {
            reviews: Vec::new(),
        };
        assert!(missing.attach_permit().is_none());
    }

    fn report_allowing(allowed: &[AccessCheck]) -> AccessReport {
        AccessReport {
            reviews: AccessCheck::ALL
                .into_iter()
                .map(|check| AccessReview {
                    check,
                    decision: if allowed.contains(&check) {
                        AccessDecision::Allowed
                    } else {
                        AccessDecision::Denied { reason: None }
                    },
                })
                .collect(),
        }
    }

    #[test]
    fn exec_permit_needs_get_and_create() {
        let both = [AccessCheck::GetPodExec, AccessCheck::CreatePodExec];
        assert!(report_allowing(&both).exec_permit().is_some());
        assert!(report_allowing(&both[..1]).exec_permit().is_none());
        assert!(report_allowing(&both[1..]).exec_permit().is_none());
        assert!(report_allowing(&[]).exec_permit().is_none());
        let missing = AccessReport {
            reviews: Vec::new(),
        };
        assert!(missing.exec_permit().is_none());
    }

    #[test]
    fn port_forward_permit_needs_get_and_create() {
        let both = [
            AccessCheck::GetPodPortForward,
            AccessCheck::CreatePodPortForward,
        ];
        assert!(report_allowing(&both).port_forward_permit().is_some());
        assert!(report_allowing(&both[..1]).port_forward_permit().is_none());
        assert!(report_allowing(&both[1..]).port_forward_permit().is_none());
        // Exec rights do not open a port-forward.
        let exec = [AccessCheck::GetPodExec, AccessCheck::CreatePodExec];
        assert!(report_allowing(&exec).port_forward_permit().is_none());
        let missing = AccessReport {
            reviews: Vec::new(),
        };
        assert!(missing.port_forward_permit().is_none());
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
    fn patch_nodes_check_is_cluster_scoped() {
        let attributes = resource_attributes(AccessCheck::PatchNodes, Some("team-a"));
        assert_eq!(attributes.verb.as_deref(), Some("patch"));
        assert_eq!(attributes.group.as_deref(), Some(""));
        assert_eq!(attributes.resource.as_deref(), Some("nodes"));
        assert_eq!(attributes.subresource, None);
        assert_eq!(attributes.namespace, None);
        assert_eq!(AccessCheck::PatchNodes.to_string(), "patch nodes");
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
        let report = AccessReport::all_of(&AccessCheck::ALL, vec![cluster_scoped, first, second]);
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
                "get pods/exec",
                "create pods/exec",
                "get pods/portforward",
                "create pods/portforward",
                "get pods/attach",
                "create pods/attach",
                "create pods",
                "delete pods",
                "patch pods/ephemeralcontainers",
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
                "list limitranges",
                "list poddisruptionbudgets",
                "list persistentvolumeclaims",
                "list persistentvolumes",
                "list storageclasses",
                "list serviceaccounts",
                "list roles",
                "list clusterroles",
                "list rolebindings",
                "list clusterrolebindings",
                "list customresourcedefinitions",
                "patch nodes",
                "patch deployments/scale",
                "patch statefulsets/scale",
                "patch deployments",
                "patch statefulsets",
                "patch daemonsets",
                "patch cronjobs",
                "create jobs",
                "patch horizontalpodautoscalers",
                "patch persistentvolumeclaims",
                "patch storageclasses",
                "create pods/eviction",
            ]
        );
    }

    #[test]
    fn eviction_check_targets_the_subresource() {
        let attributes = resource_attributes(AccessCheck::CreatePodEviction, Some("payments"));
        assert_eq!(attributes.verb.as_deref(), Some("create"));
        assert_eq!(attributes.group.as_deref(), Some(""));
        assert_eq!(attributes.resource.as_deref(), Some("pods"));
        assert_eq!(attributes.subresource.as_deref(), Some("eviction"));
        assert_eq!(attributes.namespace.as_deref(), Some("payments"));
        assert_eq!(
            AccessCheck::CreatePodEviction.to_string(),
            "create pods/eviction"
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
    fn list_limit_ranges_attributes() {
        let attributes = resource_attributes(AccessCheck::ListLimitRanges, Some("team-a"));
        assert_eq!(attributes.verb.as_deref(), Some("list"));
        assert_eq!(attributes.group.as_deref(), Some(""));
        assert_eq!(attributes.resource.as_deref(), Some("limitranges"));
        assert_eq!(attributes.namespace.as_deref(), Some("team-a"));
        assert_eq!(AccessCheck::ListLimitRanges.to_string(), "list limitranges");
        assert!(AccessCheck::ALL.contains(&AccessCheck::ListLimitRanges));
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
    fn rbac_checks_use_rbac_group_and_scope() {
        let expected = [
            (
                AccessCheck::ListServiceAccounts,
                "",
                "serviceaccounts",
                Some("team-a"),
            ),
            (AccessCheck::ListRoles, RBAC_GROUP, "roles", Some("team-a")),
            (
                AccessCheck::ListClusterRoles,
                RBAC_GROUP,
                "clusterroles",
                None,
            ),
            (
                AccessCheck::ListRoleBindings,
                RBAC_GROUP,
                "rolebindings",
                Some("team-a"),
            ),
            (
                AccessCheck::ListClusterRoleBindings,
                RBAC_GROUP,
                "clusterrolebindings",
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
        assert_eq!(AccessCheck::ALL.len(), 55);
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

    fn custom_type(scope: ResourceScope) -> CustomResourceType {
        CustomResourceType {
            group: "cert-manager.io".to_owned(),
            version: "v1".to_owned(),
            kind: "Certificate".to_owned(),
            plural: "certificates".to_owned(),
            scope,
        }
    }

    #[test]
    fn crd_check_targets_apiextensions_cluster_scope() {
        let attributes = resource_attributes(AccessCheck::ListCustomResourceDefinitions, Some("a"));
        assert_eq!(attributes.group.as_deref(), Some("apiextensions.k8s.io"));
        assert_eq!(
            attributes.resource.as_deref(),
            Some("customresourcedefinitions")
        );
        assert_eq!(attributes.verb.as_deref(), Some("list"));
        assert_eq!(attributes.namespace, None);
        assert_eq!(
            AccessCheck::ListCustomResourceDefinitions.to_string(),
            "list customresourcedefinitions"
        );
    }

    #[test]
    fn custom_review_asks_list_only() {
        let attributes =
            custom_resource_attributes(&custom_type(ResourceScope::Namespaced), Some("shop"));
        assert_eq!(attributes.verb.as_deref(), Some("list"));
        assert_eq!(attributes.group.as_deref(), Some("cert-manager.io"));
        assert_eq!(attributes.resource.as_deref(), Some("certificates"));
        assert_eq!(attributes.subresource, None);
        assert_eq!(attributes.namespace.as_deref(), Some("shop"));
    }

    #[test]
    fn custom_review_runs_per_namespace_for_several() {
        let several = NamespaceScope::of_namespaces(["b".to_owned(), "a".to_owned()]);
        let targets = review_targets(&several);
        let namespaces: Vec<_> = targets
            .into_iter()
            .map(|namespace| {
                custom_resource_attributes(&custom_type(ResourceScope::Namespaced), namespace)
                    .namespace
            })
            .collect();
        assert_eq!(namespaces, [Some("a".to_owned()), Some("b".to_owned())]);
    }

    #[test]
    fn custom_review_ignores_scope_for_cluster_resources() {
        let attributes =
            custom_resource_attributes(&custom_type(ResourceScope::Cluster), Some("shop"));
        assert_eq!(attributes.namespace, None);
    }

    #[test]
    fn first_custom_denial_wins() {
        let denied = |reason: &str| AccessDecision::Denied {
            reason: Some(reason.to_owned()),
        };
        assert_eq!(
            first_denial(vec![
                AccessDecision::Allowed,
                denied("first"),
                denied("second")
            ]),
            denied("first")
        );
        assert_eq!(
            first_denial(vec![AccessDecision::Allowed]),
            AccessDecision::Allowed
        );
    }

    fn texts(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    fn rules_status(
        resource_rules: Vec<ResourceRule>,
        non_resource_rules: Vec<NonResourceRule>,
    ) -> SubjectRulesReviewStatus {
        SubjectRulesReviewStatus {
            evaluation_error: None,
            incomplete: false,
            non_resource_rules,
            resource_rules,
        }
    }

    #[test]
    fn rules_review_maps_resource_and_url_rules() {
        let status = rules_status(
            vec![ResourceRule {
                api_groups: Some(texts(&["apps"])),
                resource_names: Some(texts(&["web"])),
                resources: Some(texts(&["deployments"])),
                verbs: texts(&["get", "list"]),
            }],
            vec![NonResourceRule {
                non_resource_urls: Some(texts(&["/healthz"])),
                verbs: texts(&["get"]),
            }],
        );
        let review = rules_review(Some(status));
        let [resource, url] = review.rules.as_slice() else {
            panic!("two rules");
        };
        assert_eq!(resource.api_groups, ["apps"]);
        assert_eq!(resource.resources, ["deployments"]);
        assert_eq!(resource.resource_names, ["web"]);
        assert_eq!(resource.verbs, ["get", "list"]);
        assert!(resource.non_resource_urls.is_empty());
        assert_eq!(url.non_resource_urls, ["/healthz"]);
        assert!(url.api_groups.is_empty() && url.resources.is_empty());
        assert!(!review.is_incomplete);
    }

    #[test]
    fn rules_review_incomplete_and_error() {
        let mut status = rules_status(Vec::new(), Vec::new());
        status.incomplete = true;
        status.evaluation_error = Some("webhook down".to_owned());
        let review = rules_review(Some(status));
        assert!(review.is_incomplete);
        assert_eq!(review.evaluation_error.as_deref(), Some("webhook down"));
    }

    #[test]
    fn empty_evaluation_error_is_none() {
        let mut status = rules_status(Vec::new(), Vec::new());
        status.evaluation_error = Some(String::new());
        assert_eq!(rules_review(Some(status)).evaluation_error, None);
        assert_eq!(rules_review(None).evaluation_error, None);
    }

    fn request(target: RequestTarget) -> AccessRequest {
        AccessRequest {
            verb: "get".to_owned(),
            target,
        }
    }

    #[test]
    fn request_attributes_for_resource() {
        let spec = request_spec(&request(RequestTarget::Resource(ResourceRequest {
            group: "apps".to_owned(),
            resource: "deployments".to_owned(),
            subresource: Some("scale".to_owned()),
            name: Some("web".to_owned()),
            namespace: Some("shop".to_owned()),
        })));
        assert_eq!(spec.non_resource_attributes, None);
        let attributes = spec.resource_attributes.expect("resource attributes");
        assert_eq!(attributes.group.as_deref(), Some("apps"));
        assert_eq!(attributes.resource.as_deref(), Some("deployments"));
        assert_eq!(attributes.subresource.as_deref(), Some("scale"));
        assert_eq!(attributes.name.as_deref(), Some("web"));
        assert_eq!(attributes.namespace.as_deref(), Some("shop"));
        assert_eq!(attributes.verb.as_deref(), Some("get"));
    }

    #[test]
    fn request_attributes_for_non_resource() {
        let spec = request_spec(&request(RequestTarget::NonResource {
            path: "/healthz".to_owned(),
        }));
        assert_eq!(spec.resource_attributes, None);
        let attributes = spec.non_resource_attributes.expect("url attributes");
        assert_eq!(attributes.path.as_deref(), Some("/healthz"));
        assert_eq!(attributes.verb.as_deref(), Some("get"));
    }

    #[test]
    fn check_attributes_unchanged() {
        let log = resource_attributes(AccessCheck::GetPodLogs, Some("shop"));
        assert_eq!(log.group.as_deref(), Some(""));
        assert_eq!(log.namespace.as_deref(), Some("shop"));
        assert_eq!(log.resource.as_deref(), Some("pods"));
        assert_eq!(log.subresource.as_deref(), Some("log"));
        assert_eq!(log.verb.as_deref(), Some("get"));
        assert_eq!(log.name, None);
        let nodes = resource_attributes(AccessCheck::ListNodes, Some("shop"));
        assert_eq!(nodes.namespace, None);
        assert_eq!(nodes.subresource, None);
        assert_eq!(nodes.verb.as_deref(), Some("list"));
    }

    #[tokio::test]
    async fn the_patch_nodes_review_posts_one_review_and_nothing_else() {
        use crate::fake_api::FakeApi;
        use crate::object_write::WritePolicy;

        let answer = r#"{"apiVersion":"authorization.k8s.io/v1","kind":"SelfSubjectAccessReview","metadata":{},"spec":{},"status":{"allowed":false,"reason":"no"}}"#;
        // A blocked write policy must not stop a review: it only asks what the caller may do.
        let (connection, api) =
            FakeApi::connection(WritePolicy::Blocked, |_| (201, answer.to_owned()));
        let review = connection
            .review_one(AccessCheck::PatchNodes, None)
            .await
            .expect("the review is answered");
        assert_eq!(review.check, AccessCheck::PatchNodes);
        assert_eq!(
            review.decision,
            AccessDecision::Denied {
                reason: Some("no".to_owned())
            }
        );
        let requests = api.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].method, "POST");
        assert_eq!(
            requests[0].path,
            "/apis/authorization.k8s.io/v1/selfsubjectaccessreviews"
        );
        let body: serde_json::Value = serde_json::from_str(&requests[0].body).expect("JSON");
        let attributes = &body["spec"]["resourceAttributes"];
        assert_eq!(attributes["verb"], "patch");
        assert_eq!(attributes["resource"], "nodes");
    }

    #[test]
    fn workload_write_checks_are_namespaced() {
        let table = [
            (
                AccessCheck::PatchDeploymentScale,
                "patch",
                "apps",
                "deployments",
                Some("scale"),
            ),
            (
                AccessCheck::PatchStatefulSetScale,
                "patch",
                "apps",
                "statefulsets",
                Some("scale"),
            ),
            (
                AccessCheck::PatchDeployments,
                "patch",
                "apps",
                "deployments",
                None,
            ),
            (
                AccessCheck::PatchStatefulSets,
                "patch",
                "apps",
                "statefulsets",
                None,
            ),
            (
                AccessCheck::PatchDaemonSets,
                "patch",
                "apps",
                "daemonsets",
                None,
            ),
            (
                AccessCheck::PatchCronJobs,
                "patch",
                "batch",
                "cronjobs",
                None,
            ),
            (AccessCheck::CreateJobs, "create", "batch", "jobs", None),
            (
                AccessCheck::PatchHorizontalPodAutoscalers,
                "patch",
                "autoscaling",
                "horizontalpodautoscalers",
                None,
            ),
            (
                AccessCheck::PatchPersistentVolumeClaims,
                "patch",
                "",
                "persistentvolumeclaims",
                None,
            ),
        ];
        for (check, verb, group, resource, subresource) in table {
            let attributes = resource_attributes(check, Some("shop"));
            assert_eq!(attributes.namespace.as_deref(), Some("shop"), "{check}");
            assert_eq!(attributes.verb.as_deref(), Some(verb), "{check}");
            assert_eq!(attributes.group.as_deref(), Some(group), "{check}");
            assert_eq!(attributes.resource.as_deref(), Some(resource), "{check}");
            assert_eq!(attributes.subresource.as_deref(), subresource, "{check}");
        }
    }

    #[test]
    fn update_check_is_lazy_not_in_all() {
        let check = AccessCheck::Update(ObjectKind::Deployment);
        assert!(!AccessCheck::ALL.contains(&check));
        assert_eq!(check.to_string(), "update deployments");
        let namespaced = resource_attributes(check, Some("shop"));
        assert_eq!(namespaced.verb.as_deref(), Some("update"));
        assert_eq!(namespaced.group.as_deref(), Some("apps"));
        assert_eq!(namespaced.namespace.as_deref(), Some("shop"));
        let cluster_scoped =
            resource_attributes(AccessCheck::Update(ObjectKind::ClusterRole), Some("shop"));
        assert_eq!(
            cluster_scoped.group.as_deref(),
            Some("rbac.authorization.k8s.io")
        );
        assert_eq!(cluster_scoped.resource.as_deref(), Some("clusterroles"));
        assert_eq!(cluster_scoped.namespace, None);
    }

    #[test]
    fn delete_check_text_and_group() {
        let check = AccessCheck::Delete(ObjectKind::Deployment);
        assert_eq!(check.to_string(), "delete deployments");
        let attributes = resource_attributes(check, Some("shop"));
        assert_eq!(attributes.verb.as_deref(), Some("delete"));
        assert_eq!(attributes.group.as_deref(), Some("apps"));
        assert_eq!(attributes.namespace.as_deref(), Some("shop"));
        let cluster_scoped =
            resource_attributes(AccessCheck::Delete(ObjectKind::Node), Some("shop"));
        assert_eq!(cluster_scoped.namespace, None);
    }

    #[test]
    fn patch_check_targets_patch_verb() {
        let check = AccessCheck::Patch(ObjectKind::Secret);
        assert!(!AccessCheck::ALL.contains(&check));
        assert_eq!(check.to_string(), "patch secrets");
        let attributes = resource_attributes(check, Some("shop"));
        assert_eq!(attributes.verb.as_deref(), Some("patch"));
        assert_eq!(attributes.group.as_deref(), Some(""));
        assert_eq!(attributes.resource.as_deref(), Some("secrets"));
        assert_eq!(attributes.namespace.as_deref(), Some("shop"));
        assert_eq!(
            AccessCheck::Patch(ObjectKind::ConfigMap).to_string(),
            "patch configmaps"
        );
    }

    #[test]
    fn resource_edit_checks_are_static_and_storage_classes_cluster_scoped() {
        let classes = AccessCheck::PatchStorageClasses;
        let attributes = resource_attributes(classes, Some("shop"));
        assert_eq!(attributes.namespace, None);
        assert_eq!(attributes.group.as_deref(), Some("storage.k8s.io"));
        assert_eq!(attributes.resource.as_deref(), Some("storageclasses"));
        for check in [
            AccessCheck::PatchHorizontalPodAutoscalers,
            AccessCheck::PatchPersistentVolumeClaims,
            classes,
        ] {
            assert!(AccessCheck::ALL.contains(&check), "{check}");
            assert_eq!(check.to_string().split(' ').next(), Some("patch"));
        }
    }

    #[test]
    fn lazy_checks_are_distinct_permissions() {
        let update = AccessCheck::Update(ObjectKind::Pod);
        let delete = AccessCheck::Delete(ObjectKind::Pod);
        assert_ne!(update, delete);
        assert!(!AccessCheck::ALL.contains(&delete));
    }

    fn allowing() -> (ClusterConnection, crate::fake_api::FakeApi) {
        use crate::fake_api::FakeApi;
        use crate::object_write::WritePolicy;

        let answer = r#"{"apiVersion":"authorization.k8s.io/v1","kind":"SelfSubjectAccessReview","metadata":{},"spec":{},"status":{"allowed":true}}"#;
        FakeApi::connection(WritePolicy::Blocked, |_| (201, answer.to_owned()))
    }

    #[tokio::test]
    async fn review_access_for_reviews_the_given_list() {
        let (connection, api) = allowing();
        let checks = [
            AccessCheck::Update(ObjectKind::Deployment),
            AccessCheck::Update(ObjectKind::ConfigMap),
        ];
        let report = connection
            .review_access_for(&checks, NamespaceScope::All)
            .await
            .expect("the reviews are answered");
        assert_eq!(report.reviews.len(), 2);
        assert!(report.is_allowed(checks[0]));
        assert!(report.is_allowed(checks[1]));
        assert_eq!(api.requests().len(), 2);
    }

    /// Regression: `all_of` used to iterate `AccessCheck::ALL`, so a `Several` scope dropped every
    /// check that is not in it.
    #[tokio::test]
    async fn lazy_checks_survive_several_scope() {
        let (connection, api) = allowing();
        let check = AccessCheck::Update(ObjectKind::Deployment);
        let scope = NamespaceScope::of_namespaces(["a".to_owned(), "b".to_owned()]);
        let report = connection
            .review_access_for(&[check], scope)
            .await
            .expect("the reviews are answered");
        assert!(report.is_allowed(check), "{report:?}");
        assert_eq!(report.reviews.len(), 1);
        // One namespaced check, asked once per namespace.
        assert_eq!(api.requests().len(), 2);
    }
}
