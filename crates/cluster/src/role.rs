use futures::Stream;
use k8s_openapi::api::rbac::v1::{ClusterRole, PolicyRule, Role};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use kube::Api;

use crate::connection::ClusterConnection;
use crate::namespace::NamespaceScope;
use crate::resource_watch::{WatchUpdate, summary_watch};
use crate::selector::Selector;
use crate::workload::label_terms;

const WILDCARD: &str = "*";
/// The API server marks its default roles with this label (a label, not an annotation).
const BOOTSTRAPPING_LABEL: &str = "kubernetes.io/bootstrapping=rbac-defaults";

/// A Role (`namespace: Some`) or a ClusterRole (`namespace: None`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoleSummary {
    pub namespace: Option<String>,
    pub name: String,
    pub created_at: Option<jiff::Timestamp>,
    /// `key=value` terms in key order.
    pub labels: Vec<String>,
    /// In API order.
    pub rules: Vec<RbacRule>,
    /// `aggregationRule.clusterRoleSelectors`; always empty for a Role.
    pub aggregation: Vec<Selector>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RbacRule {
    pub api_groups: Vec<String>,
    pub resources: Vec<String>,
    pub resource_names: Vec<String>,
    pub verbs: Vec<String>,
    pub non_resource_urls: Vec<String>,
}

impl RbacRule {
    /// The cluster-admin shape: verb, resource, and API group are all `*`, and no resource name
    /// narrows it.
    pub fn grants_everything(&self) -> bool {
        self.resource_names.is_empty()
            && [&self.verbs, &self.resources, &self.api_groups]
                .iter()
                .all(|values| values.iter().any(|value| value == WILDCARD))
    }
}

impl RoleSummary {
    pub fn grants_everything(&self) -> bool {
        self.rules.iter().any(RbacRule::grants_everything)
    }

    pub fn is_aggregated(&self) -> bool {
        !self.aggregation.is_empty()
    }

    pub fn is_built_in(&self) -> bool {
        self.labels.iter().any(|term| term == BOOTSTRAPPING_LABEL)
    }
}

impl ClusterConnection {
    /// Watches roles in `scope`. Yields batched snapshots ordered by (namespace, name).
    pub fn watch_roles(
        &self,
        scope: NamespaceScope,
    ) -> impl Stream<Item = WatchUpdate<RoleSummary>> + Send + 'static {
        summary_watch(
            self,
            self.scoped_apis(&scope),
            "watching roles",
            role_summary,
        )
    }

    /// Watches cluster roles. Yields batched snapshots ordered by name.
    pub fn watch_cluster_roles(
        &self,
    ) -> impl Stream<Item = WatchUpdate<RoleSummary>> + Send + 'static {
        let api = Api::<ClusterRole>::all(self.client().clone());
        summary_watch(
            self,
            vec![(None, api)],
            "watching cluster roles",
            cluster_role_summary,
        )
    }
}

pub(crate) fn role_summary(role: &Role) -> RoleSummary {
    summary_of(&role.metadata, role.rules.as_deref(), Vec::new())
}

pub(crate) fn cluster_role_summary(role: &ClusterRole) -> RoleSummary {
    let aggregation = role
        .aggregation_rule
        .iter()
        .flat_map(|rule| rule.cluster_role_selectors.iter().flatten())
        .map(Selector::of)
        .collect();
    let mut summary = summary_of(&role.metadata, role.rules.as_deref(), aggregation);
    summary.namespace = None;
    summary
}

fn summary_of(
    metadata: &ObjectMeta,
    rules: Option<&[PolicyRule]>,
    aggregation: Vec<Selector>,
) -> RoleSummary {
    RoleSummary {
        namespace: metadata.namespace.clone(),
        name: metadata.name.clone().unwrap_or_default(),
        created_at: metadata.creation_timestamp.as_ref().map(|time| time.0),
        labels: label_terms(metadata),
        rules: rules.unwrap_or_default().iter().map(rule).collect(),
        aggregation,
    }
}

fn rule(rule: &PolicyRule) -> RbacRule {
    RbacRule {
        api_groups: rule.api_groups.clone().unwrap_or_default(),
        resources: rule.resources.clone().unwrap_or_default(),
        resource_names: rule.resource_names.clone().unwrap_or_default(),
        verbs: rule.verbs.clone(),
        non_resource_urls: rule.non_resource_urls.clone().unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use k8s_openapi::api::rbac::v1::AggregationRule;
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::LabelSelector;

    use super::*;

    fn texts(values: &[&str]) -> Option<Vec<String>> {
        Some(values.iter().map(|value| (*value).to_owned()).collect())
    }

    fn policy_rule(groups: &[&str], resources: &[&str], verbs: &[&str]) -> PolicyRule {
        PolicyRule {
            api_groups: texts(groups),
            resources: texts(resources),
            verbs: verbs.iter().map(|verb| (*verb).to_owned()).collect(),
            ..Default::default()
        }
    }

    fn role_with_rules(rules: Vec<PolicyRule>) -> RoleSummary {
        role_summary(&Role {
            rules: Some(rules),
            ..Default::default()
        })
    }

    #[test]
    fn rules_keep_groups_resources_names_verbs_urls() {
        let summary = role_with_rules(vec![
            PolicyRule {
                resource_names: texts(&["api"]),
                ..policy_rule(&["", "apps"], &["pods", "deployments"], &["get", "list"])
            },
            PolicyRule {
                non_resource_urls: texts(&["/healthz"]),
                verbs: vec!["get".to_owned()],
                ..Default::default()
            },
        ]);
        assert_eq!(summary.rules.len(), 2);
        let first = &summary.rules[0];
        assert_eq!(first.api_groups, ["", "apps"]);
        assert_eq!(first.resources, ["pods", "deployments"]);
        assert_eq!(first.resource_names, ["api"]);
        assert_eq!(first.verbs, ["get", "list"]);
        assert!(first.non_resource_urls.is_empty());
        let second = &summary.rules[1];
        assert_eq!(second.non_resource_urls, ["/healthz"]);
        assert!(second.api_groups.is_empty());
    }

    #[test]
    fn grants_everything_needs_all_three_wildcards() {
        assert!(role_with_rules(vec![policy_rule(&["*"], &["*"], &["*"])]).grants_everything());
        assert!(!role_with_rules(vec![policy_rule(&["*"], &["*"], &["get"])]).grants_everything());
        assert!(!role_with_rules(vec![policy_rule(&["*"], &["pods"], &["*"])]).grants_everything());
        assert!(!role_with_rules(vec![policy_rule(&[""], &["*"], &["*"])]).grants_everything());
        assert!(!role_with_rules(Vec::new()).grants_everything());
        // Named objects narrow even a wildcard rule.
        let named = PolicyRule {
            resource_names: texts(&["one"]),
            ..policy_rule(&["*"], &["*"], &["*"])
        };
        assert!(!role_with_rules(vec![named]).grants_everything());
    }

    #[test]
    fn cluster_role_aggregation_selectors() {
        let role = ClusterRole {
            aggregation_rule: Some(AggregationRule {
                cluster_role_selectors: Some(vec![LabelSelector {
                    match_labels: Some(BTreeMap::from([(
                        "rbac.authorization.k8s.io/aggregate-to-admin".to_owned(),
                        "true".to_owned(),
                    )])),
                    ..Default::default()
                }]),
            }),
            ..Default::default()
        };
        let summary = cluster_role_summary(&role);
        assert!(summary.is_aggregated());
        assert_eq!(summary.aggregation.len(), 1);
        assert_eq!(summary.namespace, None);
        assert!(!cluster_role_summary(&ClusterRole::default()).is_aggregated());
    }

    #[test]
    fn built_in_from_bootstrapping_label() {
        let mut role = Role::default();
        role.metadata.labels = Some(BTreeMap::from([(
            "kubernetes.io/bootstrapping".to_owned(),
            "rbac-defaults".to_owned(),
        )]));
        assert!(role_summary(&role).is_built_in());
        assert!(!role_summary(&Role::default()).is_built_in());
    }

    #[test]
    fn role_has_no_aggregation() {
        let mut role = Role::default();
        role.metadata.namespace = Some("shop".to_owned());
        let summary = role_summary(&role);
        assert!(summary.aggregation.is_empty());
        assert!(!summary.is_aggregated());
        assert_eq!(summary.namespace.as_deref(), Some("shop"));
    }
}
