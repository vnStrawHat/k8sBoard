use cluster::{
    HorizontalPodAutoscalerSummary, HpaMetric, MetricSource, MetricValue, QuotaItem,
    ResourceQuotaSummary, Selector, WorkloadCondition,
};

use super::*;
use crate::resource_kind::ResourceKind;

fn budget(expected: u32, healthy: u32, allowed: u32) -> PodDisruptionBudgetSummary {
    PodDisruptionBudgetSummary {
        namespace: "team-a".to_owned(),
        name: "web".to_owned(),
        created_at: None,
        labels: Vec::new(),
        min_available: Some("2".to_owned()),
        max_unavailable: None,
        selector: Selector::of_labels(&["app=web".to_owned()]),
        current_healthy: healthy,
        desired_healthy: 2,
        expected_pods: expected,
        disruptions_allowed: allowed,
        unhealthy_pod_eviction_policy: None,
        conditions: Vec::new(),
        is_status_stale: false,
    }
}

fn toned(text: &str, tone: StatusTone) -> KindCell {
    KindCell::Toned(StatusLabel {
        text: text.to_owned().into(),
        tone,
    })
}

#[test]
fn pdb_row_cells_match_column_count() {
    let row = pod_disruption_budget_row(&budget(3, 3, 1));
    assert_eq!(
        row.cells.len(),
        ResourceKind::PodDisruptionBudgets.columns().len()
    );
    assert_eq!(row.cells[0], KindCell::Text("2".into()));
    assert_eq!(row.cells[1], KindCell::Absent);
}

#[test]
fn pdb_allowed_cell_tone_by_state() {
    let cell =
        |budget: &PodDisruptionBudgetSummary| pod_disruption_budget_row(budget).cells[2].clone();
    assert_eq!(cell(&budget(3, 3, 1)), toned("1", StatusTone::Ok));
    assert_eq!(cell(&budget(3, 2, 0)), toned("0", StatusTone::Bad));
    assert_eq!(cell(&budget(0, 0, 0)), toned("0", StatusTone::Done));
}

#[test]
fn pdb_status_singular_and_plural() {
    let status = |allowed| pod_disruption_budget_row(&budget(5, 5, allowed)).status;
    assert_eq!(status(1).text.as_ref(), "1 disruption allowed");
    assert_eq!(status(2).text.as_ref(), "2 disruptions allowed");
    assert_eq!(status(2).tone, StatusTone::Ok);
}

#[test]
fn pdb_status_by_state() {
    let status = |budget: &PodDisruptionBudgetSummary| pod_disruption_budget_row(budget).status;
    let blocked = status(&budget(3, 3, 0));
    assert_eq!(blocked.text.as_ref(), "0 disruptions allowed");
    assert_eq!(blocked.tone, StatusTone::Bad);
    let none = status(&budget(0, 0, 0));
    assert_eq!(none.text.as_ref(), "Selects no pods");
    assert_eq!(none.tone, StatusTone::Done);
}

#[test]
fn pdb_sync_failed_status() {
    let mut failed = budget(0, 0, 0);
    failed.conditions = vec![WorkloadCondition {
        name: "DisruptionAllowed".to_owned(),
        is_true: false,
        reason: Some("SyncFailed".to_owned()),
        message: Some("found no controller ref".to_owned()),
    }];
    let status = pod_disruption_budget_row(&failed).status;
    assert_eq!(status.text.as_ref(), "Budget not computed");
    assert_eq!(status.tone, StatusTone::Bad);
}

#[test]
fn pdb_stale_status_note() {
    let note = DetailRow::Note(
        "Status describes an older version of this budget; the controller has not caught up".into(),
    );
    let fresh = pod_disruption_budget_row(&budget(3, 3, 1));
    assert!(
        !fresh
            .section("Budget")
            .expect("section")
            .rows
            .contains(&note)
    );
    let mut stale = budget(3, 3, 1);
    stale.is_status_stale = true;
    let row = pod_disruption_budget_row(&stale);
    assert_eq!(
        row.section("Budget").expect("section").rows.last(),
        Some(&note)
    );
}

#[test]
fn pdb_sections_in_order() {
    let row = pod_disruption_budget_row(&budget(3, 3, 1));
    let titles: Vec<&str> = row.sections.iter().map(|section| section.title).collect();
    assert_eq!(
        titles,
        ["Budget", "Selector", "Selected pods", "Conditions"]
    );
    let budget_rows = &row.section("Budget").expect("section").rows;
    assert_eq!(
        budget_rows[1],
        DetailRow::field("Healthy", KindCell::Text("3 of 3 (needs 2)".into()))
    );
    assert_eq!(
        budget_rows.last(),
        Some(&DetailRow::field(
            "Unhealthy eviction",
            KindCell::Text("IfHealthyBudget (default)".into())
        ))
    );
}

#[test]
fn pdb_selector_section_covers_none_everything_and_terms() {
    let selector_rows = |selector: Option<Selector>| {
        let mut with = budget(3, 3, 1);
        with.selector = selector;
        pod_disruption_budget_row(&with)
            .section("Selector")
            .expect("section")
            .rows
            .clone()
    };
    assert_eq!(
        selector_rows(None),
        [DetailRow::Note("No selector: selects no pods".into())]
    );
    assert_eq!(
        selector_rows(Some(Selector::everything())),
        [DetailRow::Note("All pods in team-a".into())]
    );
    assert_eq!(
        selector_rows(Selector::of_labels(&["app=web".to_owned()])),
        [DetailRow::Chips(vec!["app=web".into()])]
    );
}

// ---- HorizontalPodAutoscalers ----

fn cond(name: &str, is_true: bool, reason: &str) -> WorkloadCondition {
    WorkloadCondition {
        name: name.to_owned(),
        is_true,
        reason: Some(reason.to_owned()),
        message: None,
    }
}

fn metric(name: &str, target: MetricValue, current: Option<MetricValue>) -> HpaMetric {
    HpaMetric {
        name: name.to_owned(),
        source: MetricSource::Resource,
        target,
        current,
    }
}

fn hpa(current: u32, desired: u32, metrics: Vec<HpaMetric>) -> HorizontalPodAutoscalerSummary {
    HorizontalPodAutoscalerSummary {
        namespace: "team-a".to_owned(),
        name: "web".to_owned(),
        created_at: None,
        labels: Vec::new(),
        target: cluster::ControllerRef {
            kind: "Deployment".to_owned(),
            name: "web".to_owned(),
        },
        min_replicas: 2,
        max_replicas: 10,
        current_replicas: current,
        desired_replicas: desired,
        metrics,
        conditions: Vec::new(),
        last_scaled_at: None,
    }
}

fn at_max_conditions() -> Vec<WorkloadCondition> {
    vec![cond("ScalingLimited", true, "TooManyReplicas")]
}

fn cpu(current: Option<u32>, target: u32) -> HpaMetric {
    metric(
        "cpu",
        MetricValue::Utilization(target),
        current.map(MetricValue::Utilization),
    )
}

fn value_metric(current: &str, target: &str) -> HpaMetric {
    metric(
        "queue_depth",
        MetricValue::AverageValue(target.to_owned()),
        Some(MetricValue::AverageValue(current.to_owned())),
    )
}

#[test]
fn hpa_row_cells_match_column_count() {
    let row = horizontal_pod_autoscaler_row(&hpa(3, 3, vec![cpu(Some(74), 70)]));
    assert_eq!(
        row.cells.len(),
        ResourceKind::HorizontalPodAutoscalers.columns().len()
    );
    assert_eq!(row.cells[0], KindCell::Text("deployment/web".into()));
    assert_eq!(row.cells[1], KindCell::Text("2 / 10".into()));
    assert_eq!(row.cells[2], KindCell::Text("3".into()));
    let none = horizontal_pod_autoscaler_row(&hpa(3, 3, Vec::new()));
    assert_eq!(none.cells[3], KindCell::Absent);
}

#[test]
fn metric_text_utilization_value_and_unknown() {
    assert_eq!(metric_text(&cpu(Some(74), 70)), "cpu 74% / 70%");
    assert_eq!(
        metric_text(&value_metric("9k", "1k")),
        "queue_depth 9k / 1k"
    );
    assert_eq!(metric_text(&cpu(None, 70)), "cpu <unknown> / 70%");
    let container = HpaMetric {
        source: MetricSource::ContainerResource {
            container: "app".to_owned(),
        },
        ..cpu(Some(10), 70)
    };
    assert_eq!(metric_text(&container), "cpu (app) 10% / 70%");
}

#[test]
fn above_target_by_utilization_and_ratio() {
    assert_eq!(is_above_target(&cpu(Some(74), 70)), Some(true));
    assert_eq!(is_above_target(&cpu(Some(70), 70)), Some(false));
    assert_eq!(is_above_target(&value_metric("9k", "1k")), Some(true));
    assert_eq!(is_above_target(&value_metric("500", "1k")), Some(false));
}

#[test]
fn above_target_unknown_for_other_variant() {
    assert_eq!(is_above_target(&cpu(None, 70)), None);
    let mixed = metric(
        "cpu",
        MetricValue::Utilization(70),
        Some(MetricValue::AverageValue("9k".to_owned())),
    );
    assert_eq!(is_above_target(&mixed), None);
}

#[test]
fn at_max_from_scaling_limited_too_many_replicas() {
    let mut limited = hpa(10, 10, Vec::new());
    assert!(!is_at_max(&limited));
    limited.conditions = at_max_conditions();
    assert!(is_at_max(&limited));
    // Limited by the minimum is not at max.
    limited.conditions = vec![cond("ScalingLimited", true, "TooFewReplicas")];
    assert!(!is_at_max(&limited));
}

#[test]
fn scaling_disabled_is_scaled_to_zero() {
    let mut zero = hpa(0, 0, Vec::new());
    zero.conditions = vec![cond("ScalingActive", false, "ScalingDisabled")];
    assert!(is_scaling_disabled(&zero));
    let status = horizontal_pod_autoscaler_row(&zero).status;
    assert_eq!(status.text.as_ref(), "Scaled to zero");
    assert_eq!(status.tone, StatusTone::Done);
}

#[test]
fn hpa_status_order() {
    let status = |hpa: &HorizontalPodAutoscalerSummary| {
        let status = horizontal_pod_autoscaler_row(hpa).status;
        (status.text.to_string(), status.tone)
    };
    let mut inactive = hpa(3, 5, Vec::new());
    inactive.conditions = vec![
        cond("ScalingActive", false, "FailedGetResourceMetric"),
        cond("AbleToScale", false, "FailedGetScale"),
    ];
    assert_eq!(
        status(&inactive),
        ("Scaling inactive".to_owned(), StatusTone::Bad)
    );
    let mut unable = hpa(3, 5, Vec::new());
    unable.conditions = vec![cond("AbleToScale", false, "FailedUpdateScale")];
    assert_eq!(
        status(&unable),
        ("Cannot scale".to_owned(), StatusTone::Bad)
    );
    let mut capped = hpa(10, 10, Vec::new());
    capped.conditions = at_max_conditions();
    assert_eq!(
        status(&capped),
        ("At max replicas".to_owned(), StatusTone::Bad)
    );
    assert_eq!(
        status(&hpa(3, 5, Vec::new())),
        ("Scaling up".to_owned(), StatusTone::Warn)
    );
    assert_eq!(
        status(&hpa(5, 3, Vec::new())),
        ("Scaling down".to_owned(), StatusTone::Info)
    );
    assert_eq!(
        status(&hpa(3, 3, Vec::new())),
        ("3 replicas".to_owned(), StatusTone::Ok)
    );
    assert_eq!(status(&hpa(1, 1, Vec::new())).0, "1 replica");
}

#[test]
fn hpa_metrics_section_bars() {
    let metrics = vec![
        cpu(Some(74), 70),
        cpu(None, 70),
        value_metric("9k", "1k"),
        metric(
            "memory",
            MetricValue::Utilization(80),
            Some(MetricValue::Utilization(40)),
        ),
    ];
    let mut capped = hpa(10, 10, metrics);
    capped.conditions = at_max_conditions();
    let row = horizontal_pod_autoscaler_row(&capped);
    let rows = &row.section("Metrics").expect("section").rows;
    assert_eq!(
        rows[0],
        DetailRow::Bar {
            label: "CPU utilization".into(),
            percent: 74,
            text: "74% / target 70%".into(),
            tone: Some(StatusTone::Bad),
        }
    );
    assert_eq!(
        rows[1],
        DetailRow::field(
            "CPU utilization",
            KindCell::Toned(StatusLabel {
                text: "current value unknown".into(),
                tone: StatusTone::Warn,
            })
        )
    );
    // A value bar is full at its target and above.
    assert_eq!(
        rows[2],
        DetailRow::Bar {
            label: "queue_depth".into(),
            percent: 100,
            text: "9k / target 1k".into(),
            tone: Some(StatusTone::Bad),
        }
    );
    assert_eq!(
        rows[3],
        DetailRow::Bar {
            label: "Memory utilization".into(),
            percent: 40,
            text: "40% / target 80%".into(),
            tone: None,
        }
    );
    let titles: Vec<&str> = row.sections.iter().map(|section| section.title).collect();
    assert_eq!(
        titles,
        ["Scaling", "Metrics", "Scaling events", "Conditions"]
    );
}

// ---- ResourceQuotas ----

fn item(resource: &str, hard: &str, used: Option<&str>) -> QuotaItem {
    QuotaItem {
        resource: resource.to_owned(),
        hard: hard.to_owned(),
        used: used.map(str::to_owned),
    }
}

fn quota(items: Vec<QuotaItem>) -> ResourceQuotaSummary {
    ResourceQuotaSummary {
        namespace: "team-a".to_owned(),
        name: "compute".to_owned(),
        created_at: None,
        labels: Vec::new(),
        items,
        scopes: Vec::new(),
    }
}

#[test]
fn quota_row_cells_match_column_count() {
    let row = resource_quota_row(&quota(vec![item("pods", "10", Some("3"))]));
    assert_eq!(
        row.cells.len(),
        ResourceKind::ResourceQuotas.columns().len()
    );
    assert_eq!(row.cells[0], KindCell::Absent);
    assert_eq!(
        row.cells[2],
        KindCell::Quantity {
            text: "3 / 10".into(),
            value: 300,
            tone: None,
        }
    );
    let titles: Vec<&str> = row.sections.iter().map(|section| section.title).collect();
    assert_eq!(titles, ["Usage", "Blocked creations"]);
}

#[test]
fn quota_columns_prefer_requests_items() {
    let row = resource_quota_row(&quota(vec![
        item("cpu", "8", Some("8")),
        item("requests.cpu", "4", Some("1")),
        item("memory", "8Gi", Some("1Gi")),
    ]));
    assert!(matches!(
        &row.cells[0],
        KindCell::Quantity {
            value: 250,
            tone: None,
            ..
        }
    ));
    // Without `requests.memory`, `memory` fills the column.
    assert!(matches!(
        &row.cells[1],
        KindCell::Quantity { value: 125, .. }
    ));
}

#[test]
fn quota_measure_by_resource_suffix() {
    assert_eq!(quota_measure("requests.cpu"), QuotaMeasure::Cpu);
    assert_eq!(quota_measure("limits.memory"), QuotaMeasure::Bytes);
    assert_eq!(quota_measure("requests.storage"), QuotaMeasure::Bytes);
    assert_eq!(quota_measure("hugepages-2Mi"), QuotaMeasure::Bytes);
    assert_eq!(quota_measure("requests.hugepages-1Gi"), QuotaMeasure::Bytes);
    assert_eq!(quota_measure("pods"), QuotaMeasure::Count);
    assert_eq!(quota_measure("persistentvolumeclaims"), QuotaMeasure::Count);
}

#[test]
fn quota_tone_thresholds() {
    assert_eq!(quota_tone(0.5), None);
    assert_eq!(quota_tone(0.89), None);
    assert_eq!(quota_tone(0.9), Some(StatusTone::Warn));
    assert_eq!(quota_tone(0.99), Some(StatusTone::Warn));
    assert_eq!(quota_tone(1.0), Some(StatusTone::Bad));
    assert_eq!(quota_tone(1.4), Some(StatusTone::Bad));
}

#[test]
fn quota_status_names_highest_item() {
    let status = |items| {
        let status = resource_quota_row(&quota(items)).status;
        (status.text.to_string(), status.tone)
    };
    assert_eq!(
        status(Vec::new()),
        ("No limits".to_owned(), StatusTone::Done)
    );
    assert_eq!(
        status(vec![item("pods", "10", Some("3"))]),
        ("Within quota".to_owned(), StatusTone::Ok)
    );
    assert_eq!(
        status(vec![
            item("pods", "10", Some("3")),
            item("requests.cpu", "4", Some("3700m"))
        ]),
        ("93% CPU used".to_owned(), StatusTone::Warn)
    );
    assert_eq!(
        status(vec![
            item("requests.memory", "1Gi", Some("1Gi")),
            item("requests.cpu", "4", Some("3700m"))
        ]),
        ("memory at quota".to_owned(), StatusTone::Bad)
    );
    assert_eq!(
        status(vec![item("pods", "10", Some("10"))]),
        ("pods at quota".to_owned(), StatusTone::Bad)
    );
}
