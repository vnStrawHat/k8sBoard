//! Row builders for the policy kinds: PodDisruptionBudgets, HorizontalPodAutoscalers, and
//! ResourceQuotas.

use cluster::{
    BlockCause, ByteAmount, CpuAmount, DisruptionState, HorizontalPodAutoscalerSummary, HpaMetric,
    MetricSource, MetricValue, PodDisruptionBudgetSummary, QuotaItem, ResourceQuotaSummary,
    quantity_ratio,
};

use crate::kind_diagnosis::find_condition;
use crate::kind_row::{
    DetailRow, DetailSection, KindCell, KindObject, KindRow, LiveContent, chips, percent,
};
use crate::status_tone::{StatusLabel, StatusTone};
use crate::table_selection::ResourceKey;
use crate::usage_format::{Measure, format_percent};
use crate::workload_rows::condition_row;

const DEFAULT_EVICTION_POLICY: &str = "IfHealthyBudget (default)";

/// The PDB drawer section the Show selected pods menu item scrolls to.
pub(crate) const SELECTED_PODS_TITLE: &str = "Selected pods";

pub(crate) fn pod_disruption_budget_row(budget: &PodDisruptionBudgetSummary) -> KindRow {
    let allowed = allowed_label(budget);
    let mut budget_rows = Vec::new();
    if let Some(min_available) = &budget.min_available {
        budget_rows.push(DetailRow::field(
            "Min available",
            KindCell::Text(min_available.clone().into()),
        ));
    }
    if let Some(max_unavailable) = &budget.max_unavailable {
        budget_rows.push(DetailRow::field(
            "Max unavailable",
            KindCell::Text(max_unavailable.clone().into()),
        ));
    }
    budget_rows.push(DetailRow::field(
        "Healthy",
        KindCell::Text(
            format!(
                "{} of {} (needs {})",
                budget.current_healthy, budget.expected_pods, budget.desired_healthy
            )
            .into(),
        ),
    ));
    budget_rows.push(DetailRow::field(
        "Allowed disruptions",
        KindCell::Toned(allowed.clone()),
    ));
    budget_rows.push(DetailRow::field(
        "Unhealthy eviction",
        KindCell::Text(
            budget
                .unhealthy_pod_eviction_policy
                .clone()
                .unwrap_or_else(|| DEFAULT_EVICTION_POLICY.to_owned())
                .into(),
        ),
    ));
    if budget.is_status_stale {
        budget_rows.push(DetailRow::Note(
            "Status describes an older version of this budget; the controller has not caught up"
                .into(),
        ));
    }
    KindRow {
        namespace: Some(budget.namespace.clone()),
        name: budget.name.clone(),
        created_at: budget.created_at,
        status: budget_status(budget),
        cells: vec![
            KindCell::text_or_absent(budget.min_available.as_deref()),
            KindCell::text_or_absent(budget.max_unavailable.as_deref()),
            KindCell::Toned(allowed),
            KindCell::age(budget.created_at),
        ],
        sections: vec![
            DetailSection {
                title: "Budget",
                rows: budget_rows,
            },
            selector_section(budget),
            DetailSection {
                title: SELECTED_PODS_TITLE,
                rows: vec![DetailRow::Live(LiveContent::SelectedPods)],
            },
            DetailSection {
                title: "Conditions",
                rows: budget.conditions.iter().map(condition_row).collect(),
            },
        ],
        event: None,
        related_pods: None,
        labels: chips(&budget.labels),
        object: KindObject::PodDisruptionBudget(budget.clone()),
    }
}

/// The disruptions the budget allows right now, as a colored count: Bad when the budget refuses
/// every eviction, muted when it selects no pods.
fn allowed_label(budget: &PodDisruptionBudgetSummary) -> StatusLabel {
    let (text, tone) = match budget.disruption_state() {
        DisruptionState::Allowed(count) => (count.to_string(), StatusTone::Ok),
        DisruptionState::Blocked(_) => ("0".to_owned(), StatusTone::Bad),
        DisruptionState::NoPods => ("0".to_owned(), StatusTone::Done),
    };
    StatusLabel {
        text: text.into(),
        tone,
    }
}

fn budget_status(budget: &PodDisruptionBudgetSummary) -> StatusLabel {
    let (text, tone) = match budget.disruption_state() {
        DisruptionState::Allowed(1) => ("1 disruption allowed".to_owned(), StatusTone::Ok),
        DisruptionState::Allowed(count) => (format!("{count} disruptions allowed"), StatusTone::Ok),
        DisruptionState::Blocked(BlockCause::SyncFailed) => {
            ("Budget not computed".to_owned(), StatusTone::Bad)
        }
        DisruptionState::Blocked(BlockCause::UnhealthyPods | BlockCause::NoRoom) => {
            ("0 disruptions allowed".to_owned(), StatusTone::Bad)
        }
        DisruptionState::NoPods => ("Selects no pods".to_owned(), StatusTone::Done),
    };
    StatusLabel {
        text: text.into(),
        tone,
    }
}

fn selector_section(budget: &PodDisruptionBudgetSummary) -> DetailSection {
    let row = match &budget.selector {
        None => DetailRow::Note("No selector: selects no pods".into()),
        Some(selector) if selector.selects_everything() => {
            DetailRow::Note(format!("All pods in {}", budget.namespace).into())
        }
        Some(selector) => DetailRow::Chips(chips(&selector.terms())),
    };
    DetailSection {
        title: "Selector",
        rows: vec![row],
    }
}

// ---- HorizontalPodAutoscalers ----

const SCALING_ACTIVE: &str = "ScalingActive";
const ABLE_TO_SCALE: &str = "AbleToScale";
const SCALING_LIMITED: &str = "ScalingLimited";

/// `74%` for a utilization, else the quantity as written.
fn value_text(value: &MetricValue) -> String {
    match value {
        MetricValue::Utilization(percent) => format!("{percent}%"),
        MetricValue::AverageValue(text) | MetricValue::Value(text) => text.clone(),
    }
}

/// The metric name, with its container for a container resource source.
fn metric_label(metric: &HpaMetric) -> String {
    match &metric.source {
        MetricSource::ContainerResource { container } => format!("{} ({container})", metric.name),
        MetricSource::Resource
        | MetricSource::Pods
        | MetricSource::Object { .. }
        | MetricSource::External => metric.name.clone(),
    }
}

/// `cpu 74% / 70%`, `queue_depth 9k / 1k`, or `cpu <unknown> / 70%`.
pub(crate) fn metric_text(metric: &HpaMetric) -> String {
    let current = metric
        .current
        .as_ref()
        .map_or_else(|| "<unknown>".to_owned(), value_text);
    format!(
        "{} {current} / {}",
        metric_label(metric),
        value_text(&metric.target)
    )
}

/// Whether the current value is above the target; `None` when it is unknown or of another variant.
pub(crate) fn is_above_target(metric: &HpaMetric) -> Option<bool> {
    match (metric.current.as_ref()?, &metric.target) {
        (MetricValue::Utilization(current), MetricValue::Utilization(target)) => {
            Some(current > target)
        }
        (MetricValue::AverageValue(current), MetricValue::AverageValue(target))
        | (MetricValue::Value(current), MetricValue::Value(target)) => {
            Some(quantity_ratio(current, target)? > 1.0)
        }
        _ => None,
    }
}

/// The tone of the Replicas cell: Bad when the controller says the metrics want more than
/// `maxReplicas`, Warn when the HPA sits at its maximum whatever its conditions say (no room
/// is left to scale up).
fn replicas_tone(hpa: &HorizontalPodAutoscalerSummary) -> Option<StatusTone> {
    if is_at_max(hpa) {
        Some(StatusTone::Bad)
    } else if hpa.current_replicas >= hpa.max_replicas {
        Some(StatusTone::Warn)
    } else {
        None
    }
}

/// The controller's own verdict that the metrics want more than `maxReplicas`.
pub(crate) fn is_at_max(hpa: &HorizontalPodAutoscalerSummary) -> bool {
    find_condition(&hpa.conditions, SCALING_LIMITED).is_some_and(|condition| {
        condition.is_true && condition.reason.as_deref() == Some("TooManyReplicas")
    })
}

/// A target scaled to zero by hand: intended, so not a problem.
pub(crate) fn is_scaling_disabled(hpa: &HorizontalPodAutoscalerSummary) -> bool {
    find_condition(&hpa.conditions, SCALING_ACTIVE).is_some_and(|condition| {
        !condition.is_true && condition.reason.as_deref() == Some("ScalingDisabled")
    })
}

fn is_condition_false(hpa: &HorizontalPodAutoscalerSummary, name: &str) -> bool {
    find_condition(&hpa.conditions, name).is_some_and(|condition| !condition.is_true)
}

/// `{kind lowercased}/{name}`, the owner format of 0005.
fn target_text(hpa: &HorizontalPodAutoscalerSummary) -> String {
    format!("{}/{}", hpa.target.kind.to_lowercase(), hpa.target.name)
}

pub(crate) fn horizontal_pod_autoscaler_row(hpa: &HorizontalPodAutoscalerSummary) -> KindRow {
    let at_max = is_at_max(hpa);
    let replicas = match replicas_tone(hpa) {
        Some(tone) => KindCell::Toned(StatusLabel {
            text: hpa.current_replicas.to_string().into(),
            tone,
        }),
        None => KindCell::count(hpa.current_replicas),
    };
    let mut sections = vec![DetailSection {
        title: "Scaling",
        rows: scaling_rows(hpa),
    }];
    let metric_rows: Vec<DetailRow> = hpa
        .metrics
        .iter()
        .map(|metric| metric_row(metric, at_max))
        .collect();
    if !metric_rows.is_empty() {
        sections.push(DetailSection {
            title: "Metrics",
            rows: metric_rows,
        });
    }
    sections.push(DetailSection {
        title: "Scaling events",
        rows: vec![DetailRow::Live(LiveContent::ScalingEvents)],
    });
    sections.push(DetailSection {
        title: "Conditions",
        rows: hpa.conditions.iter().map(condition_row).collect(),
    });
    KindRow {
        namespace: Some(hpa.namespace.clone()),
        name: hpa.name.clone(),
        created_at: hpa.created_at,
        status: hpa_status(hpa),
        cells: vec![
            KindCell::Text(target_text(hpa).into()),
            KindCell::Text(format!("{} / {}", hpa.min_replicas, hpa.max_replicas).into()),
            replicas,
            KindCell::Toned(hpa_status(hpa)),
            metrics_cell(hpa, at_max),
            KindCell::age(hpa.created_at),
        ],
        sections,
        event: None,
        related_pods: None,
        labels: chips(&hpa.labels),
        object: KindObject::HorizontalPodAutoscaler(hpa.clone()),
    }
}

fn scaling_rows(hpa: &HorizontalPodAutoscalerSummary) -> Vec<DetailRow> {
    let target = match ResourceKey::of_owner(&hpa.namespace, &hpa.target) {
        Some(key) => DetailRow::Link {
            label: "Target".into(),
            text: target_text(hpa).into(),
            target: key,
        },
        None => DetailRow::field("Target", KindCell::Text(target_text(hpa).into())),
    };
    let last_scaled = match hpa.last_scaled_at {
        Some(at) => KindCell::age(Some(at)),
        None => KindCell::Text("Never".into()),
    };
    vec![
        target,
        DetailRow::field("Min replicas", KindCell::count(hpa.min_replicas)),
        DetailRow::field("Max replicas", KindCell::count(hpa.max_replicas)),
        DetailRow::field("Current", KindCell::count(hpa.current_replicas)),
        DetailRow::field("Desired", KindCell::count(hpa.desired_replicas)),
        DetailRow::field("Last scaled", last_scaled),
    ]
}

/// The first metric's text, plus ` +{n}` for the others. Bad at max, Warn when a metric is above
/// target or has no current value.
fn metrics_cell(hpa: &HorizontalPodAutoscalerSummary, at_max: bool) -> KindCell {
    let Some(first) = hpa.metrics.first() else {
        return KindCell::Absent;
    };
    let mut text = metric_text(first);
    if hpa.metrics.len() > 1 {
        text.push_str(&format!(" +{}", hpa.metrics.len() - 1));
    }
    let needs_attention = hpa
        .metrics
        .iter()
        .any(|metric| metric.current.is_none() || is_above_target(metric) == Some(true));
    let tone = if at_max {
        Some(StatusTone::Bad)
    } else if needs_attention {
        Some(StatusTone::Warn)
    } else {
        None
    };
    match tone {
        Some(tone) => KindCell::Toned(StatusLabel {
            text: text.into(),
            tone,
        }),
        None => KindCell::Text(text.into()),
    }
}

/// First match wins.
fn hpa_status(hpa: &HorizontalPodAutoscalerSummary) -> StatusLabel {
    let label = |text: &str, tone| StatusLabel {
        text: text.to_owned().into(),
        tone,
    };
    if is_scaling_disabled(hpa) {
        return label("Scaled to zero", StatusTone::Done);
    }
    if is_condition_false(hpa, SCALING_ACTIVE) {
        return label("Scaling inactive", StatusTone::Bad);
    }
    if is_condition_false(hpa, ABLE_TO_SCALE) {
        return label("Cannot scale", StatusTone::Bad);
    }
    if is_at_max(hpa) {
        return label("At max replicas", StatusTone::Bad);
    }
    if hpa.desired_replicas > hpa.current_replicas {
        return label("Scaling up", StatusTone::Warn);
    }
    if hpa.desired_replicas < hpa.current_replicas {
        return label("Scaling down", StatusTone::Info);
    }
    let noun = if hpa.current_replicas == 1 {
        "replica"
    } else {
        "replicas"
    };
    label(&format!("{} {noun}", hpa.current_replicas), StatusTone::Ok)
}

/// A bar for a metric with a known current value, else a warning field.
fn metric_row(metric: &HpaMetric, at_max: bool) -> DetailRow {
    let label = metric_bar_label(metric);
    let Some(current) = &metric.current else {
        return DetailRow::field(
            label,
            KindCell::Toned(StatusLabel {
                text: "current value unknown".into(),
                tone: StatusTone::Warn,
            }),
        );
    };
    let (ratio, text) = match (current, &metric.target) {
        (MetricValue::Utilization(current), MetricValue::Utilization(target)) => (
            Some(f64::from(*current) / 100.0),
            format!("{current}% / target {target}%"),
        ),
        (MetricValue::AverageValue(current), MetricValue::AverageValue(target))
        | (MetricValue::Value(current), MetricValue::Value(target)) => (
            quantity_ratio(current, target),
            format!("{current} / target {target}"),
        ),
        _ => (
            None,
            format!(
                "{} / target {}",
                value_text(current),
                value_text(&metric.target)
            ),
        ),
    };
    let Some(ratio) = ratio else {
        return DetailRow::field(label, KindCell::Text(text.into()));
    };
    let tone = match (is_above_target(metric), at_max) {
        (Some(true), true) => Some(StatusTone::Bad),
        (Some(true), false) => Some(StatusTone::Warn),
        _ => None,
    };
    DetailRow::Bar {
        label: label.into(),
        // A utilization bar is of 100 %; a value bar is of its target, so it is full at the target.
        percent: percent(ratio),
        text: text.into(),
        tone,
    }
}

fn metric_bar_label(metric: &HpaMetric) -> String {
    match (&metric.source, metric.name.as_str(), &metric.target) {
        (MetricSource::Resource, "cpu", MetricValue::Utilization(_)) => {
            "CPU utilization".to_owned()
        }
        (MetricSource::Resource, "memory", MetricValue::Utilization(_)) => {
            "Memory utilization".to_owned()
        }
        _ => metric_label(metric),
    }
}

// ---- ResourceQuotas ----

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum QuotaMeasure {
    Cpu,
    Bytes,
    Count,
}

/// By the resource name: CPU, byte quantities (memory, storage, hugepages), or plain counts.
fn quota_measure(resource: &str) -> QuotaMeasure {
    if resource.ends_with("cpu") {
        QuotaMeasure::Cpu
    } else if resource.contains("hugepages")
        || resource.ends_with("memory")
        || resource.ends_with("storage")
    {
        QuotaMeasure::Bytes
    } else {
        QuotaMeasure::Count
    }
}

/// Used over hard; `None` before the quota controller reports usage, or for a limit of zero.
pub(crate) fn quota_ratio(item: &QuotaItem) -> Option<f64> {
    quantity_ratio(item.used.as_deref()?, &item.hard)
}

/// `1.5 / 4 cores`, `3 / 10`; the quantities as written when they do not parse.
pub(crate) fn quota_text(item: &QuotaItem) -> String {
    let Some(used) = item.used.as_deref() else {
        return format!("— / {}", item.hard);
    };
    let formatted = match quota_measure(&item.resource) {
        QuotaMeasure::Cpu => CpuAmount::parse(used)
            .zip(CpuAmount::parse(&item.hard))
            .map(|(used, hard)| Measure::Cpu.format_pair(used.cores(), hard.cores(), " / ")),
        QuotaMeasure::Bytes => ByteAmount::parse(used)
            .zip(ByteAmount::parse(&item.hard))
            .map(|(used, hard)| {
                Measure::Bytes.format_pair(used.bytes() as f64, hard.bytes() as f64, " / ")
            }),
        QuotaMeasure::Count => Some(format!("{used} / {}", item.hard)),
    };
    formatted.unwrap_or_else(|| format!("{used} / {}", item.hard))
}

/// A quota blocks only at 100 %.
pub(crate) fn quota_tone(ratio: f64) -> Option<StatusTone> {
    if ratio >= 1.0 {
        Some(StatusTone::Bad)
    } else if ratio >= 0.9 {
        Some(StatusTone::Warn)
    } else {
        None
    }
}

/// The item with the highest ratio, with that ratio.
pub(crate) fn fullest_item(quota: &ResourceQuotaSummary) -> Option<(&QuotaItem, f64)> {
    quota
        .items
        .iter()
        .filter_map(|item| Some((item, quota_ratio(item)?)))
        .max_by(|a, b| a.1.total_cmp(&b.1))
}

/// A usage cell: the text, sorted by permille, toned by `quota_tone`; a plain text before usage
/// is known; `Absent` for a resource the quota does not limit.
fn quota_cell(quota: &ResourceQuotaSummary, resources: &[&str]) -> KindCell {
    let Some(item) = resources
        .iter()
        .find_map(|name| quota.items.iter().find(|item| item.resource == *name))
    else {
        return KindCell::Absent;
    };
    match quota_ratio(item) {
        Some(ratio) => KindCell::Quantity {
            text: quota_text(item).into(),
            value: (ratio * 1000.0).round().max(0.0) as u64,
            tone: quota_tone(ratio),
        },
        None => KindCell::Text(quota_text(item).into()),
    }
}

/// The name a status uses for a limited resource.
fn short_name(item: &QuotaItem) -> String {
    if quota_measure(&item.resource) == QuotaMeasure::Cpu {
        "CPU".to_owned()
    } else if item.resource.ends_with("memory") {
        "memory".to_owned()
    } else {
        item.resource.clone()
    }
}

fn quota_status(quota: &ResourceQuotaSummary) -> StatusLabel {
    let label = |text: String, tone| StatusLabel {
        text: text.into(),
        tone,
    };
    if quota.items.is_empty() {
        return label("No limits".to_owned(), StatusTone::Done);
    }
    match fullest_item(quota) {
        Some((item, ratio)) if ratio >= 1.0 => {
            label(format!("{} at quota", short_name(item)), StatusTone::Bad)
        }
        Some((item, ratio)) if ratio >= 0.9 => label(
            format!("{} {} used", format_percent(ratio), short_name(item)),
            StatusTone::Warn,
        ),
        _ => label("Within quota".to_owned(), StatusTone::Ok),
    }
}

pub(crate) fn resource_quota_row(quota: &ResourceQuotaSummary) -> KindRow {
    let mut sections = vec![DetailSection {
        title: "Usage",
        rows: quota.items.iter().map(usage_row).collect(),
    }];
    if !quota.scopes.is_empty() {
        sections.push(DetailSection {
            title: "Scopes",
            rows: vec![DetailRow::Chips(chips(&quota.scopes))],
        });
    }
    sections.push(DetailSection {
        title: "Blocked creations",
        rows: vec![DetailRow::Live(LiveContent::BlockedCreations)],
    });
    KindRow {
        namespace: Some(quota.namespace.clone()),
        name: quota.name.clone(),
        created_at: quota.created_at,
        status: quota_status(quota),
        cells: vec![
            quota_cell(quota, &["requests.cpu", "cpu"]),
            quota_cell(quota, &["requests.memory", "memory"]),
            quota_cell(quota, &["pods"]),
            KindCell::age(quota.created_at),
        ],
        sections,
        event: None,
        related_pods: None,
        labels: chips(&quota.labels),
        object: KindObject::ResourceQuota(quota.clone()),
    }
}

fn usage_row(item: &QuotaItem) -> DetailRow {
    match quota_ratio(item) {
        Some(ratio) => DetailRow::Bar {
            label: item.resource.clone().into(),
            percent: percent(ratio),
            text: quota_text(item).into(),
            tone: quota_tone(ratio),
        },
        None => DetailRow::field(
            item.resource.clone(),
            KindCell::Text(quota_text(item).into()),
        ),
    }
}

#[cfg(test)]
#[path = "policy_rows_tests.rs"]
mod policy_rows_tests;
