use futures::Stream;
use k8s_openapi::api::autoscaling::v2::{
    HorizontalPodAutoscaler, MetricSpec, MetricStatus, MetricTarget, MetricValueStatus,
};

use crate::connection::ClusterConnection;
use crate::namespace::NamespaceScope;
use crate::pod_status::non_negative;
use crate::resource_watch::{WatchUpdate, summary_watch};
use crate::workload::{ControllerRef, WorkloadCondition, condition, label_terms, optional_count};

const DEFAULT_MIN_REPLICAS: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HorizontalPodAutoscalerSummary {
    pub namespace: String,
    pub name: String,
    pub created_at: Option<jiff::Timestamp>,
    /// `key=value` terms in key order.
    pub labels: Vec<String>,
    /// `spec.scaleTargetRef`.
    pub target: ControllerRef,
    /// Defaults to 1.
    pub min_replicas: u32,
    pub max_replicas: u32,
    pub current_replicas: u32,
    pub desired_replicas: u32,
    pub metrics: Vec<HpaMetric>,
    /// `AbleToScale`, `ScalingActive`, `ScalingLimited`.
    pub conditions: Vec<WorkloadCondition>,
    pub last_scaled_at: Option<jiff::Timestamp>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HpaMetric {
    /// `cpu` or `memory` for resource sources, else the metric name.
    pub name: String,
    pub source: MetricSource,
    pub target: MetricValue,
    /// The `status.currentMetrics` entry of the same source, name, and container or object.
    pub current: Option<MetricValue>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MetricSource {
    Resource,
    ContainerResource {
        container: String,
    },
    Pods,
    /// `kind/name` of the described object.
    Object {
        object: String,
    },
    External,
}

/// One shape for a target and for a current value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MetricValue {
    /// A percentage of the pods' requests.
    Utilization(u32),
    /// A quantity as written.
    AverageValue(String),
    /// A quantity as written.
    Value(String),
}

impl ClusterConnection {
    /// Watches horizontal pod autoscalers (`autoscaling/v2`) in `scope`. Yields batched
    /// snapshots ordered by (namespace, name).
    pub fn watch_horizontal_pod_autoscalers(
        &self,
        scope: NamespaceScope,
    ) -> impl Stream<Item = WatchUpdate<HorizontalPodAutoscalerSummary>> + Send + 'static {
        summary_watch(
            self,
            self.scoped_apis(&scope),
            "watching horizontal pod autoscalers",
            horizontal_pod_autoscaler_summary,
        )
    }
}

pub(crate) fn horizontal_pod_autoscaler_summary(
    autoscaler: &HorizontalPodAutoscaler,
) -> HorizontalPodAutoscalerSummary {
    let spec = autoscaler.spec.as_ref();
    let status = autoscaler.status.as_ref();
    let currents: &[MetricStatus] = status
        .and_then(|status| status.current_metrics.as_deref())
        .unwrap_or_default();
    HorizontalPodAutoscalerSummary {
        namespace: autoscaler.metadata.namespace.clone().unwrap_or_default(),
        name: autoscaler.metadata.name.clone().unwrap_or_default(),
        created_at: autoscaler
            .metadata
            .creation_timestamp
            .as_ref()
            .map(|time| time.0),
        labels: label_terms(&autoscaler.metadata),
        target: spec.map_or_else(
            || ControllerRef {
                kind: String::new(),
                name: String::new(),
            },
            |spec| ControllerRef {
                kind: spec.scale_target_ref.kind.clone(),
                name: spec.scale_target_ref.name.clone(),
            },
        ),
        min_replicas: spec
            .and_then(|spec| spec.min_replicas)
            .map_or(DEFAULT_MIN_REPLICAS, non_negative),
        max_replicas: spec.map_or(0, |spec| non_negative(spec.max_replicas)),
        current_replicas: optional_count(status.and_then(|status| status.current_replicas)),
        desired_replicas: status.map_or(0, |status| non_negative(status.desired_replicas)),
        metrics: spec
            .into_iter()
            .flat_map(|spec| spec.metrics.iter().flatten())
            .filter_map(|metric| hpa_metric(metric, currents))
            .collect(),
        conditions: status
            .into_iter()
            .flat_map(|status| status.conditions.iter().flatten())
            .map(|item| {
                condition(
                    &item.type_,
                    &item.status,
                    item.reason.as_deref(),
                    item.message.as_deref(),
                    item.last_transition_time.as_ref().map(|time| time.0),
                )
            })
            .collect(),
        last_scaled_at: status
            .and_then(|status| status.last_scale_time.as_ref())
            .map(|time| time.0),
    }
}

/// What identifies a metric in both the spec and the status.
type MetricIdentity = (MetricSource, String);

/// `None` for an unknown metric type, or a target without a value.
fn hpa_metric(spec: &MetricSpec, currents: &[MetricStatus]) -> Option<HpaMetric> {
    let (identity, target) = spec_metric(spec)?;
    let target_value = target_value(target)?;
    let current = currents
        .iter()
        .filter_map(status_metric)
        .find(|(status_identity, _)| *status_identity == identity)
        .and_then(|(_, value)| current_value(value, &target_value));
    let (source, name) = identity;
    Some(HpaMetric {
        name,
        source,
        target: target_value,
        current,
    })
}

fn spec_metric(spec: &MetricSpec) -> Option<(MetricIdentity, &MetricTarget)> {
    match spec.type_.as_str() {
        "Resource" => {
            let metric = spec.resource.as_ref()?;
            Some((
                (MetricSource::Resource, metric.name.clone()),
                &metric.target,
            ))
        }
        "ContainerResource" => {
            let metric = spec.container_resource.as_ref()?;
            let source = MetricSource::ContainerResource {
                container: metric.container.clone(),
            };
            Some(((source, metric.name.clone()), &metric.target))
        }
        "Pods" => {
            let metric = spec.pods.as_ref()?;
            Some((
                (MetricSource::Pods, metric.metric.name.clone()),
                &metric.target,
            ))
        }
        "Object" => {
            let metric = spec.object.as_ref()?;
            let source = MetricSource::Object {
                object: object_text(&metric.described_object.kind, &metric.described_object.name),
            };
            Some(((source, metric.metric.name.clone()), &metric.target))
        }
        "External" => {
            let metric = spec.external.as_ref()?;
            Some((
                (MetricSource::External, metric.metric.name.clone()),
                &metric.target,
            ))
        }
        _ => None,
    }
}

fn status_metric(status: &MetricStatus) -> Option<(MetricIdentity, &MetricValueStatus)> {
    match status.type_.as_str() {
        "Resource" => {
            let metric = status.resource.as_ref()?;
            Some((
                (MetricSource::Resource, metric.name.clone()),
                &metric.current,
            ))
        }
        "ContainerResource" => {
            let metric = status.container_resource.as_ref()?;
            let source = MetricSource::ContainerResource {
                container: metric.container.clone(),
            };
            Some(((source, metric.name.clone()), &metric.current))
        }
        "Pods" => {
            let metric = status.pods.as_ref()?;
            Some((
                (MetricSource::Pods, metric.metric.name.clone()),
                &metric.current,
            ))
        }
        "Object" => {
            let metric = status.object.as_ref()?;
            let source = MetricSource::Object {
                object: object_text(&metric.described_object.kind, &metric.described_object.name),
            };
            Some(((source, metric.metric.name.clone()), &metric.current))
        }
        "External" => {
            let metric = status.external.as_ref()?;
            Some((
                (MetricSource::External, metric.metric.name.clone()),
                &metric.current,
            ))
        }
        _ => None,
    }
}

fn object_text(kind: &str, name: &str) -> String {
    format!("{kind}/{name}")
}

fn target_value(target: &MetricTarget) -> Option<MetricValue> {
    match target.type_.as_str() {
        "Utilization" => target.average_utilization.map(utilization),
        "AverageValue" => target
            .average_value
            .as_ref()
            .map(|value| MetricValue::AverageValue(value.0.clone())),
        "Value" => target
            .value
            .as_ref()
            .map(|value| MetricValue::Value(value.0.clone())),
        _ => None,
    }
}

fn utilization(percent: i32) -> MetricValue {
    MetricValue::Utilization(non_negative(percent))
}

/// The value in the target's variant when present, else the first present of utilization,
/// average value, value.
fn current_value(current: &MetricValueStatus, target: &MetricValue) -> Option<MetricValue> {
    let utilization = current.average_utilization.map(utilization);
    let average = current
        .average_value
        .as_ref()
        .map(|value| MetricValue::AverageValue(value.0.clone()));
    let value = current
        .value
        .as_ref()
        .map(|value| MetricValue::Value(value.0.clone()));
    let preferred = match target {
        MetricValue::Utilization(_) => utilization.clone(),
        MetricValue::AverageValue(_) => average.clone(),
        MetricValue::Value(_) => value.clone(),
    };
    preferred.or(utilization).or(average).or(value)
}

#[cfg(test)]
#[path = "autoscaler_tests.rs"]
mod autoscaler_tests;
