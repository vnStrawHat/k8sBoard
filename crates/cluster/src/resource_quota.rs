use futures::Stream;
use k8s_openapi::api::core::v1::{ResourceQuota, ScopedResourceSelectorRequirement};

use crate::connection::ClusterConnection;
use crate::namespace::NamespaceScope;
use crate::resource_watch::{WatchUpdate, summary_watch};
use crate::workload::label_terms;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResourceQuotaSummary {
    pub namespace: String,
    pub name: String,
    pub created_at: Option<jiff::Timestamp>,
    /// `key=value` terms in key order.
    pub labels: Vec<String>,
    /// One per limited resource, in key order.
    pub items: Vec<QuotaItem>,
    /// `spec.scopes`, then the scope selector expressions.
    pub scopes: Vec<String>,
}

/// Quantities as written by the API server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuotaItem {
    pub resource: String,
    pub hard: String,
    /// `None` until the quota controller has synced.
    pub used: Option<String>,
}

impl ClusterConnection {
    /// Watches resource quotas in `scope`. Yields batched snapshots ordered by
    /// (namespace, name).
    pub fn watch_resource_quotas(
        &self,
        scope: NamespaceScope,
    ) -> impl Stream<Item = WatchUpdate<ResourceQuotaSummary>> + Send + 'static {
        summary_watch(
            self,
            self.scoped_apis(&scope),
            "watching resource quotas",
            resource_quota_summary,
        )
    }
}

pub(crate) fn resource_quota_summary(quota: &ResourceQuota) -> ResourceQuotaSummary {
    let spec = quota.spec.as_ref();
    let status = quota.status.as_ref();
    // The status is empty until the controller syncs; the spec limits are known by then.
    let hard = status
        .and_then(|status| status.hard.as_ref())
        .or_else(|| spec.and_then(|spec| spec.hard.as_ref()));
    let used = status.and_then(|status| status.used.as_ref());
    let selector_scopes = spec
        .and_then(|spec| spec.scope_selector.as_ref())
        .into_iter()
        .flat_map(|selector| selector.match_expressions.iter().flatten())
        .map(scope_expression);
    ResourceQuotaSummary {
        namespace: quota.metadata.namespace.clone().unwrap_or_default(),
        name: quota.metadata.name.clone().unwrap_or_default(),
        created_at: quota
            .metadata
            .creation_timestamp
            .as_ref()
            .map(|time| time.0),
        labels: label_terms(&quota.metadata),
        items: hard
            .into_iter()
            .flatten()
            .map(|(resource, hard)| QuotaItem {
                resource: resource.clone(),
                hard: hard.0.clone(),
                used: used
                    .and_then(|used| used.get(resource))
                    .map(|used| used.0.clone()),
            })
            .collect(),
        scopes: spec
            .into_iter()
            .flat_map(|spec| spec.scopes.iter().flatten())
            .cloned()
            .chain(selector_scopes)
            .collect(),
    }
}

/// `{scopeName} {operator} ({values})`; the values are omitted when there are none.
fn scope_expression(expression: &ScopedResourceSelectorRequirement) -> String {
    let values = expression.values.as_deref().unwrap_or_default();
    if values.is_empty() {
        return format!("{} {}", expression.scope_name, expression.operator);
    }
    format!(
        "{} {} ({})",
        expression.scope_name,
        expression.operator,
        values.join(", ")
    )
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use k8s_openapi::api::core::v1::{ResourceQuotaSpec, ResourceQuotaStatus, ScopeSelector};
    use k8s_openapi::apimachinery::pkg::api::resource::Quantity;

    use super::*;

    fn quantities(pairs: &[(&str, &str)]) -> BTreeMap<String, Quantity> {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), Quantity((*value).to_owned())))
            .collect()
    }

    #[test]
    fn items_from_status_hard_with_used() {
        let summary = resource_quota_summary(&ResourceQuota {
            status: Some(ResourceQuotaStatus {
                hard: Some(quantities(&[("pods", "10"), ("requests.cpu", "4")])),
                used: Some(quantities(&[("pods", "7")])),
            }),
            ..Default::default()
        });
        assert_eq!(
            summary.items,
            [
                QuotaItem {
                    resource: "pods".to_owned(),
                    hard: "10".to_owned(),
                    used: Some("7".to_owned()),
                },
                QuotaItem {
                    resource: "requests.cpu".to_owned(),
                    hard: "4".to_owned(),
                    used: None,
                },
            ]
        );
    }

    #[test]
    fn items_fall_back_to_spec_hard() {
        let summary = resource_quota_summary(&ResourceQuota {
            spec: Some(ResourceQuotaSpec {
                hard: Some(quantities(&[("pods", "10")])),
                ..Default::default()
            }),
            ..Default::default()
        });
        assert_eq!(summary.items.len(), 1);
        assert_eq!(summary.items[0].hard, "10");
        assert_eq!(summary.items[0].used, None);
    }

    #[test]
    fn scopes_include_scope_selector() {
        let summary = resource_quota_summary(&ResourceQuota {
            spec: Some(ResourceQuotaSpec {
                scopes: Some(vec!["NotTerminating".to_owned()]),
                scope_selector: Some(ScopeSelector {
                    match_expressions: Some(vec![
                        ScopedResourceSelectorRequirement {
                            scope_name: "PriorityClass".to_owned(),
                            operator: "In".to_owned(),
                            values: Some(vec!["high".to_owned(), "low".to_owned()]),
                        },
                        ScopedResourceSelectorRequirement {
                            scope_name: "BestEffort".to_owned(),
                            operator: "Exists".to_owned(),
                            values: None,
                        },
                    ]),
                }),
                ..Default::default()
            }),
            ..Default::default()
        });
        assert_eq!(
            summary.scopes,
            [
                "NotTerminating",
                "PriorityClass In (high, low)",
                "BestEffort Exists"
            ]
        );
    }
}
