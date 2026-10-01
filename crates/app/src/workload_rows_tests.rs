use cluster::{ContainerPort, DeploymentSummary, TemplateContainer, WorkloadCondition};

use super::*;
use crate::resource_kind::ResourceKind;

fn deployment() -> DeploymentSummary {
    DeploymentSummary {
        namespace: "team-a".to_owned(),
        name: "api".to_owned(),
        created_at: None,
        labels: vec!["app=api".to_owned()],
        desired: 3,
        ready: 3,
        up_to_date: 3,
        available: 3,
        strategy: "RollingUpdate".to_owned(),
        max_surge: Some("25%".to_owned()),
        max_unavailable: Some("25%".to_owned()),
        is_paused: false,
        revision: Some("7".to_owned()),
        selector: vec!["app=api".to_owned()],
        containers: vec![TemplateContainer {
            name: "web".to_owned(),
            image: "nginx:1.27".to_owned(),
            ports: vec![
                ContainerPort {
                    name: Some("http".to_owned()),
                    port: 8080,
                    protocol: "TCP".to_owned(),
                },
                ContainerPort {
                    name: None,
                    port: 9090,
                    protocol: "UDP".to_owned(),
                },
            ],
        }],
        conditions: Vec::new(),
    }
}

fn condition(name: &str, is_true: bool, reason: Option<&str>) -> WorkloadCondition {
    WorkloadCondition {
        name: name.to_owned(),
        is_true,
        reason: reason.map(str::to_owned),
    }
}

fn section<'a>(row: &'a KindRow, title: &str) -> Option<&'a DetailSection> {
    row.sections.iter().find(|section| section.title == title)
}

#[test]
fn workload_row_cells_match_column_count() {
    let row = deployment_row(&deployment());
    assert_eq!(row.cells.len(), ResourceKind::Deployments.columns().len());
    assert_eq!(row.namespace.as_deref(), Some("team-a"));
}

#[test]
fn replica_tone_by_ready_and_desired() {
    assert_eq!(replica_tone(0, 0), StatusTone::Done);
    assert_eq!(replica_tone(3, 3), StatusTone::Ok);
    assert_eq!(replica_tone(4, 3), StatusTone::Ok);
    assert_eq!(replica_tone(0, 3), StatusTone::Bad);
    assert_eq!(replica_tone(1, 3), StatusTone::Warn);
}

#[test]
fn deployment_status_prefers_deadline_then_pause() {
    let mut stalled = deployment();
    stalled.is_paused = true;
    stalled.conditions = vec![condition(PROGRESSING, false, Some(DEADLINE_EXCEEDED))];
    let status = deployment_status(&stalled);
    assert_eq!(status.text, "Progress deadline exceeded");
    assert_eq!(status.tone, StatusTone::Bad);

    stalled.conditions.clear();
    let status = deployment_status(&stalled);
    assert_eq!(status.text, "Paused");
    assert_eq!(status.tone, StatusTone::Info);
}

#[test]
fn deployment_status_scaled_to_zero_wins_over_pause() {
    let mut scaled_down = deployment();
    scaled_down.is_paused = true;
    scaled_down.desired = 0;
    let status = deployment_status(&scaled_down);
    assert_eq!(status.text, "Scaled to zero");
    assert_eq!(status.tone, StatusTone::Done);
}

#[test]
fn deployment_status_reports_availability() {
    let mut partial = deployment();
    partial.available = 1;
    let status = deployment_status(&partial);
    assert_eq!(status.text, "1/3 available");
    assert_eq!(status.tone, StatusTone::Warn);
}

#[test]
fn deployment_ready_cell_is_toned_by_replicas() {
    let mut partial = deployment();
    partial.ready = 1;
    let row = deployment_row(&partial);
    assert_eq!(
        row.cells.first(),
        Some(&KindCell::Toned(StatusLabel {
            text: "1/3".into(),
            tone: StatusTone::Warn,
        }))
    );
}

#[test]
fn deployment_strategy_reads_surge_and_unavailable() {
    assert_eq!(
        strategy_text(&deployment()).as_deref(),
        Some("RollingUpdate · max surge 25% · max unavailable 25%")
    );
    let mut recreate = deployment();
    recreate.strategy = "Recreate".to_owned();
    recreate.max_surge = None;
    recreate.max_unavailable = None;
    assert_eq!(strategy_text(&recreate).as_deref(), Some("Recreate"));
    recreate.strategy.clear();
    assert_eq!(strategy_text(&recreate), None);
    assert_eq!(
        deployment_row(&recreate).cells.get(3),
        Some(&KindCell::Absent)
    );
}

#[test]
fn deployment_ports_section_lists_named_and_unnamed_ports() {
    let row = deployment_row(&deployment());
    let ports = section(&row, "Ports").expect("ports section");
    assert_eq!(
        ports.rows,
        [
            DetailRow::Port {
                text: "8080/TCP · http · web".into()
            },
            DetailRow::Port {
                text: "9090/UDP · web".into()
            },
        ]
    );
}

#[test]
fn deployment_without_ports_omits_the_ports_section() {
    let mut portless = deployment();
    portless.containers[0].ports.clear();
    assert!(section(&deployment_row(&portless), "Ports").is_none());
}

#[test]
fn deployment_paused_field_appears_only_when_paused() {
    let has_paused = |deployment: &DeploymentSummary| {
        let row = deployment_row(deployment);
        section(&row, "Replicas")
            .expect("replicas section")
            .rows
            .iter()
            .any(|row| matches!(row, DetailRow::Field { label, .. } if label == "Paused"))
    };
    let mut paused = deployment();
    assert!(!has_paused(&paused));
    paused.is_paused = true;
    assert!(has_paused(&paused));
}

#[test]
fn deployment_conditions_show_reason_when_false() {
    let mut stalled = deployment();
    stalled.conditions = vec![
        condition("Available", true, None),
        condition("Progressing", false, Some("NewReplicaSetAvailable")),
    ];
    let row = deployment_row(&stalled);
    let conditions = section(&row, "Conditions").expect("conditions section");
    let texts: Vec<_> = conditions
        .rows
        .iter()
        .filter_map(|row| match row {
            DetailRow::Field {
                value: KindCell::Toned(label),
                ..
            } => Some((label.text.to_string(), label.tone)),
            _ => None,
        })
        .collect();
    assert_eq!(
        texts,
        [
            ("True".to_owned(), StatusTone::Ok),
            (
                "False · NewReplicaSetAvailable".to_owned(),
                StatusTone::Warn
            ),
        ]
    );
}

#[test]
fn deployment_related_pods_use_the_deployment_owner() {
    let row = deployment_row(&deployment());
    assert_eq!(
        row.related_pods,
        Some(PodOwner::Deployment {
            namespace: "team-a".to_owned(),
            name: "api".to_owned(),
        })
    );
    assert_eq!(row.labels, ["app=api"]);
}
