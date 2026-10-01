//! Row builders for the workload kinds.

use crate::kind_row::{DetailRow, DetailSection, KindCell, KindRow, PodOwner, chips};
use crate::status_tone::{StatusLabel, StatusTone};
use cluster::{DeploymentSummary, TemplateContainer, WorkloadCondition};

/// The condition and reason the Deployment controller reports when a rollout stops.
const PROGRESSING: &str = "Progressing";
const DEADLINE_EXCEEDED: &str = "ProgressDeadlineExceeded";

/// How ready replicas compare with the desired count.
pub(crate) fn replica_tone(ready: u32, desired: u32) -> StatusTone {
    if desired == 0 {
        StatusTone::Done
    } else if ready >= desired {
        StatusTone::Ok
    } else if ready == 0 {
        StatusTone::Bad
    } else {
        StatusTone::Warn
    }
}

pub(crate) fn deployment_row(deployment: &DeploymentSummary) -> KindRow {
    let ready = StatusLabel {
        text: format!("{}/{}", deployment.ready, deployment.desired).into(),
        tone: replica_tone(deployment.ready, deployment.desired),
    };
    let cells = vec![
        KindCell::Toned(ready),
        KindCell::count(deployment.up_to_date),
        KindCell::count(deployment.available),
        KindCell::text_or_absent(non_empty(&deployment.strategy)),
        KindCell::Age {
            at: deployment.created_at,
            tone: None,
        },
    ];
    let mut replicas = vec![
        DetailRow::field("Desired", KindCell::count(deployment.desired)),
        DetailRow::field("Ready", KindCell::count(deployment.ready)),
        DetailRow::field("Up-to-date", KindCell::count(deployment.up_to_date)),
        DetailRow::field("Available", KindCell::count(deployment.available)),
        DetailRow::field(
            "Strategy",
            KindCell::text_or_absent(strategy_text(deployment).as_deref()),
        ),
        DetailRow::field(
            "Revision",
            KindCell::text_or_absent(deployment.revision.as_deref()),
        ),
    ];
    if deployment.is_paused {
        replicas.push(DetailRow::field("Paused", KindCell::Text("Yes".into())));
    }
    let mut sections = vec![
        DetailSection {
            title: "Replicas",
            rows: replicas,
        },
        DetailSection {
            title: "Selector",
            rows: vec![DetailRow::Chips(chips(&deployment.selector))],
        },
        DetailSection {
            title: "Containers",
            rows: container_rows(&deployment.containers),
        },
    ];
    let ports = port_rows(&deployment.containers);
    if !ports.is_empty() {
        sections.push(DetailSection {
            title: "Ports",
            rows: ports,
        });
    }
    sections.push(DetailSection {
        title: "Conditions",
        rows: deployment.conditions.iter().map(condition_row).collect(),
    });
    KindRow {
        namespace: Some(deployment.namespace.clone()),
        name: deployment.name.clone(),
        created_at: deployment.created_at,
        status: deployment_status(deployment),
        cells,
        sections,
        related_pods: Some(PodOwner::Deployment {
            namespace: deployment.namespace.clone(),
            name: deployment.name.clone(),
        }),
        labels: chips(&deployment.labels),
    }
}

/// First match wins: scaled to zero, a stalled rollout, paused, then availability.
fn deployment_status(deployment: &DeploymentSummary) -> StatusLabel {
    let label = |text: &str, tone| StatusLabel {
        text: text.to_owned().into(),
        tone,
    };
    if deployment.desired == 0 {
        return label("Scaled to zero", StatusTone::Done);
    }
    let is_deadline_exceeded = deployment.conditions.iter().any(|condition| {
        condition.name == PROGRESSING
            && !condition.is_true
            && condition.reason.as_deref() == Some(DEADLINE_EXCEEDED)
    });
    if is_deadline_exceeded {
        return label("Progress deadline exceeded", StatusTone::Bad);
    }
    if deployment.is_paused {
        return label("Paused", StatusTone::Info);
    }
    label(
        &format!("{}/{} available", deployment.available, deployment.desired),
        replica_tone(deployment.available, deployment.desired),
    )
}

/// `RollingUpdate · max surge 25% · max unavailable 25%`, or `None` when the strategy is unset.
fn strategy_text(deployment: &DeploymentSummary) -> Option<String> {
    let strategy = non_empty(&deployment.strategy)?;
    let surge = deployment
        .max_surge
        .as_ref()
        .map(|surge| format!("max surge {surge}"));
    let unavailable = deployment
        .max_unavailable
        .as_ref()
        .map(|unavailable| format!("max unavailable {unavailable}"));
    let parts = std::iter::once(strategy.to_owned())
        .chain(surge)
        .chain(unavailable)
        .collect::<Vec<_>>();
    Some(parts.join(" · "))
}

fn non_empty(text: &str) -> Option<&str> {
    (!text.is_empty()).then_some(text)
}

fn container_rows(containers: &[TemplateContainer]) -> Vec<DetailRow> {
    containers
        .iter()
        .map(|container| DetailRow::Field {
            label: container.name.clone().into(),
            value: KindCell::Mono(container.image.clone().into()),
        })
        .collect()
}

/// `{port}/{protocol} · {name} · {container}`, with the name left out when the port has none.
/// The port comes first so a truncated line still shows it.
fn port_rows(containers: &[TemplateContainer]) -> Vec<DetailRow> {
    containers
        .iter()
        .flat_map(|container| {
            container.ports.iter().map(move |port| {
                let name = port.name.as_ref().map(|name| format!(" · {name}"));
                DetailRow::Port {
                    text: format!(
                        "{}/{}{} · {}",
                        port.port,
                        port.protocol,
                        name.unwrap_or_default(),
                        container.name
                    )
                    .into(),
                }
            })
        })
        .collect()
}

/// `True` is ok; `False` is a warning, with the reason when the controller gave one.
fn condition_row(condition: &WorkloadCondition) -> DetailRow {
    let (text, tone) = match (condition.is_true, &condition.reason) {
        (true, _) => ("True".to_owned(), StatusTone::Ok),
        (false, Some(reason)) => (format!("False · {reason}"), StatusTone::Warn),
        (false, None) => ("False".to_owned(), StatusTone::Warn),
    };
    DetailRow::Field {
        label: condition.name.clone().into(),
        value: KindCell::Toned(StatusLabel {
            text: text.into(),
            tone,
        }),
    }
}

#[cfg(test)]
#[path = "workload_rows_tests.rs"]
mod workload_rows_tests;
