use k8s_openapi::api::autoscaling::v2::{
    CrossVersionObjectReference, ExternalMetricSource, ExternalMetricStatus,
    HorizontalPodAutoscalerCondition, HorizontalPodAutoscalerSpec, HorizontalPodAutoscalerStatus,
    MetricIdentifier, ResourceMetricSource, ResourceMetricStatus,
};
use k8s_openapi::apimachinery::pkg::api::resource::Quantity;

use super::*;

fn target_ref() -> CrossVersionObjectReference {
    CrossVersionObjectReference {
        kind: "Deployment".to_owned(),
        name: "web".to_owned(),
        api_version: None,
    }
}

fn quantity(text: &str) -> Quantity {
    Quantity(text.to_owned())
}

fn utilization_target(percent: i32) -> MetricTarget {
    MetricTarget {
        type_: "Utilization".to_owned(),
        average_utilization: Some(percent),
        ..Default::default()
    }
}

fn resource_spec(name: &str, target: MetricTarget) -> MetricSpec {
    MetricSpec {
        type_: "Resource".to_owned(),
        resource: Some(ResourceMetricSource {
            name: name.to_owned(),
            target,
        }),
        ..Default::default()
    }
}

fn resource_status(name: &str, current: MetricValueStatus) -> MetricStatus {
    MetricStatus {
        type_: "Resource".to_owned(),
        resource: Some(ResourceMetricStatus {
            name: name.to_owned(),
            current,
        }),
        ..Default::default()
    }
}

fn external_spec(name: &str, target: MetricTarget) -> MetricSpec {
    MetricSpec {
        type_: "External".to_owned(),
        external: Some(ExternalMetricSource {
            metric: MetricIdentifier {
                name: name.to_owned(),
                selector: None,
            },
            target,
        }),
        ..Default::default()
    }
}

fn external_status(name: &str, current: MetricValueStatus) -> MetricStatus {
    MetricStatus {
        type_: "External".to_owned(),
        external: Some(ExternalMetricStatus {
            metric: MetricIdentifier {
                name: name.to_owned(),
                selector: None,
            },
            current,
        }),
        ..Default::default()
    }
}

fn utilization_status(percent: i32) -> MetricValueStatus {
    MetricValueStatus {
        average_utilization: Some(percent),
        ..Default::default()
    }
}

fn autoscaler(
    spec: HorizontalPodAutoscalerSpec,
    status: HorizontalPodAutoscalerStatus,
) -> HorizontalPodAutoscaler {
    HorizontalPodAutoscaler {
        spec: Some(spec),
        status: Some(status),
        ..Default::default()
    }
}

fn spec_with_metrics(metrics: Vec<MetricSpec>) -> HorizontalPodAutoscalerSpec {
    HorizontalPodAutoscalerSpec {
        max_replicas: 10,
        metrics: Some(metrics),
        scale_target_ref: target_ref(),
        ..Default::default()
    }
}

fn status_with_currents(currents: Vec<MetricStatus>) -> HorizontalPodAutoscalerStatus {
    HorizontalPodAutoscalerStatus {
        current_metrics: Some(currents),
        desired_replicas: 3,
        ..Default::default()
    }
}

#[test]
fn min_replicas_defaults_to_one() {
    let summary = horizontal_pod_autoscaler_summary(&autoscaler(
        spec_with_metrics(Vec::new()),
        HorizontalPodAutoscalerStatus::default(),
    ));
    assert_eq!(summary.min_replicas, 1);
    assert_eq!(summary.max_replicas, 10);
}

#[test]
fn resource_utilization_metric_reads_current() {
    let summary = horizontal_pod_autoscaler_summary(&autoscaler(
        spec_with_metrics(vec![resource_spec("cpu", utilization_target(70))]),
        status_with_currents(vec![resource_status("cpu", utilization_status(74))]),
    ));
    assert_eq!(
        summary.metrics,
        [HpaMetric {
            name: "cpu".to_owned(),
            source: MetricSource::Resource,
            target: MetricValue::Utilization(70),
            current: Some(MetricValue::Utilization(74)),
        }]
    );
}

#[test]
fn current_matched_by_type_and_name_not_order() {
    let summary = horizontal_pod_autoscaler_summary(&autoscaler(
        spec_with_metrics(vec![
            resource_spec("cpu", utilization_target(70)),
            resource_spec("memory", utilization_target(80)),
        ]),
        status_with_currents(vec![
            resource_status("memory", utilization_status(55)),
            resource_status("cpu", utilization_status(74)),
        ]),
    ));
    assert_eq!(
        summary.metrics[0].current,
        Some(MetricValue::Utilization(74))
    );
    assert_eq!(
        summary.metrics[1].current,
        Some(MetricValue::Utilization(55))
    );
}

#[test]
fn unmatched_metric_has_no_current() {
    let summary = horizontal_pod_autoscaler_summary(&autoscaler(
        spec_with_metrics(vec![resource_spec("cpu", utilization_target(70))]),
        status_with_currents(vec![resource_status("memory", utilization_status(55))]),
    ));
    assert_eq!(summary.metrics[0].current, None);
}

#[test]
fn metric_without_target_value_is_dropped() {
    let no_value = MetricTarget {
        type_: "Utilization".to_owned(),
        ..Default::default()
    };
    let summary = horizontal_pod_autoscaler_summary(&autoscaler(
        spec_with_metrics(vec![
            resource_spec("cpu", no_value),
            resource_spec("memory", utilization_target(80)),
        ]),
        HorizontalPodAutoscalerStatus::default(),
    ));
    assert_eq!(summary.metrics.len(), 1);
    assert_eq!(summary.metrics[0].name, "memory");
}

#[test]
fn target_is_kind_and_name() {
    let summary = horizontal_pod_autoscaler_summary(&autoscaler(
        spec_with_metrics(Vec::new()),
        HorizontalPodAutoscalerStatus::default(),
    ));
    assert_eq!(
        summary.target,
        ControllerRef {
            kind: "Deployment".to_owned(),
            name: "web".to_owned(),
        }
    );
}

#[test]
fn current_value_uses_target_variant() {
    let target = MetricTarget {
        type_: "AverageValue".to_owned(),
        average_value: Some(quantity("1k")),
        ..Default::default()
    };
    let both = MetricValueStatus {
        average_utilization: Some(40),
        average_value: Some(quantity("9k")),
        value: Some(quantity("3")),
    };
    let summary = horizontal_pod_autoscaler_summary(&autoscaler(
        spec_with_metrics(vec![external_spec("queue_depth", target)]),
        status_with_currents(vec![external_status("queue_depth", both)]),
    ));
    assert_eq!(
        summary.metrics[0].current,
        Some(MetricValue::AverageValue("9k".to_owned()))
    );
}

#[test]
fn current_value_falls_back_to_first_present() {
    // Without the target's variant, the first present of utilization, average, value wins.
    let only_value = MetricValueStatus {
        value: Some(quantity("3")),
        ..Default::default()
    };
    let target = MetricTarget {
        type_: "AverageValue".to_owned(),
        average_value: Some(quantity("1k")),
        ..Default::default()
    };
    let summary = horizontal_pod_autoscaler_summary(&autoscaler(
        spec_with_metrics(vec![external_spec("queue_depth", target)]),
        status_with_currents(vec![external_status("queue_depth", only_value)]),
    ));
    assert_eq!(
        summary.metrics[0].current,
        Some(MetricValue::Value("3".to_owned()))
    );
}

#[test]
fn conditions_keep_scaling_limited() {
    let status = HorizontalPodAutoscalerStatus {
        conditions: Some(vec![HorizontalPodAutoscalerCondition {
            type_: "ScalingLimited".to_owned(),
            status: "True".to_owned(),
            reason: Some("TooManyReplicas".to_owned()),
            message: Some("the desired replica count is more than the maximum".to_owned()),
            last_transition_time: None,
        }]),
        ..Default::default()
    };
    let summary =
        horizontal_pod_autoscaler_summary(&autoscaler(spec_with_metrics(Vec::new()), status));
    let [limited] = summary.conditions.as_slice() else {
        panic!("one condition");
    };
    assert_eq!(limited.name, "ScalingLimited");
    assert!(limited.is_true);
    assert_eq!(limited.reason.as_deref(), Some("TooManyReplicas"));
}
