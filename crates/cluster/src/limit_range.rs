use std::collections::BTreeMap;

use futures::Stream;
use k8s_openapi::api::core::v1::{LimitRange, LimitRangeItem};
use k8s_openapi::apimachinery::pkg::api::resource::Quantity;

use crate::connection::ClusterConnection;
use crate::namespace::NamespaceScope;
use crate::resource_watch::{WatchUpdate, summary_watch};

/// One LimitRange: the default and bounding quantities it sets per kind of object. Quantities stay
/// as the API wrote them; the Namespace drawer only displays them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LimitRangeSummary {
    pub namespace: String,
    pub name: String,
    /// In `spec.limits` order.
    pub limits: Vec<LimitRangeLimit>,
}

/// One entry of `spec.limits`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LimitRangeLimit {
    /// `type`: `Container`, `Pod`, or `PersistentVolumeClaim`.
    pub kind: String,
    /// Resource to quantity.
    pub default: BTreeMap<String, String>,
    pub default_request: BTreeMap<String, String>,
    pub max: BTreeMap<String, String>,
    pub min: BTreeMap<String, String>,
}

impl ClusterConnection {
    /// Watches LimitRanges in `scope`. Yields batched snapshots ordered by (namespace, name).
    /// Read-only: the summary holds quantities only, so nothing in it is a secret.
    pub fn watch_limit_ranges(
        &self,
        scope: NamespaceScope,
    ) -> impl Stream<Item = WatchUpdate<LimitRangeSummary>> + Send + 'static {
        summary_watch(
            self,
            self.scoped_apis(&scope),
            "watching limit ranges",
            limit_range_summary,
        )
    }
}

pub(crate) fn limit_range_summary(limit_range: &LimitRange) -> LimitRangeSummary {
    LimitRangeSummary {
        namespace: limit_range.metadata.namespace.clone().unwrap_or_default(),
        name: limit_range.metadata.name.clone().unwrap_or_default(),
        limits: limit_range
            .spec
            .iter()
            .flat_map(|spec| &spec.limits)
            .map(limit_of)
            .collect(),
    }
}

fn limit_of(item: &LimitRangeItem) -> LimitRangeLimit {
    LimitRangeLimit {
        kind: item.type_.clone(),
        default: written(item.default.as_ref()),
        default_request: written(item.default_request.as_ref()),
        max: written(item.max.as_ref()),
        min: written(item.min.as_ref()),
    }
}

fn written(quantities: Option<&BTreeMap<String, Quantity>>) -> BTreeMap<String, String> {
    quantities
        .into_iter()
        .flatten()
        .map(|(resource, quantity)| (resource.clone(), quantity.0.clone()))
        .collect()
}

#[cfg(test)]
#[path = "limit_range_tests.rs"]
mod limit_range_tests;
