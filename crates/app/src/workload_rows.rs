//! Row builders for the workload kinds.

use cluster::{
    ClaimTemplate, ControllerRef, DaemonSetSummary, DeploymentSummary, PodSummary,
    ReplicaSetSummary, StatefulSetSummary, TemplateContainer, WorkloadCondition,
};

use crate::kind_row::{
    DAEMON_SET_KIND, DetailRow, DetailSection, KindCell, KindObject, KindRow, LiveContent,
    PodOwner, REPLICA_SET_KIND, STATEFUL_SET_KIND, chips,
};
use crate::status_tone::{StatusLabel, StatusTone};
use crate::table_selection::ResourceKey;

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
        selector_section(&deployment.selector),
        containers_section(&deployment.containers),
    ];
    sections.extend(ports_section(&deployment.containers));
    sections.push(DetailSection {
        title: "Conditions",
        rows: deployment.conditions.iter().map(condition_row).collect(),
    });
    sections.push(DetailSection {
        title: "Revisions",
        rows: vec![DetailRow::Live(LiveContent::Revisions)],
    });
    KindRow {
        namespace: Some(deployment.namespace.clone()),
        name: deployment.name.clone(),
        created_at: deployment.created_at,
        status: deployment_status(deployment),
        cells,
        sections,
        event: None,
        related_pods: Some(PodOwner::Deployment {
            namespace: deployment.namespace.clone(),
            name: deployment.name.clone(),
        }),
        labels: chips(&deployment.labels),
        object: KindObject::Deployment(deployment.clone()),
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

pub(crate) fn non_empty(text: &str) -> Option<&str> {
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
pub(crate) fn condition_row(condition: &WorkloadCondition) -> DetailRow {
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

pub(crate) fn stateful_set_row(set: &StatefulSetSummary) -> KindRow {
    let mut sections = vec![
        DetailSection {
            title: "Replicas",
            rows: vec![
                DetailRow::field("Desired", KindCell::count(set.desired)),
                DetailRow::field("Ready", KindCell::count(set.ready)),
                DetailRow::field("Current", KindCell::count(set.current)),
                DetailRow::field("Updated", KindCell::count(set.updated)),
                DetailRow::field(
                    "Service",
                    KindCell::mono_or_absent(set.service_name.as_deref().unwrap_or_default()),
                ),
                DetailRow::field(
                    "Update strategy",
                    KindCell::text_or_absent(non_empty(&set.update_strategy)),
                ),
                DetailRow::field(
                    "Pod management",
                    KindCell::text_or_absent(non_empty(&set.pod_management_policy)),
                ),
            ],
        },
        selector_section(&set.selector),
        containers_section(&set.containers),
    ];
    sections.extend(ports_section(&set.containers));
    sections.push(DetailSection {
        title: "Volume claim templates",
        rows: set
            .claim_templates
            .iter()
            .map(|claim| DetailRow::field(claim.name.clone(), claim_text(claim)))
            .collect(),
    });
    KindRow {
        namespace: Some(set.namespace.clone()),
        name: set.name.clone(),
        created_at: set.created_at,
        status: replicas_status(set.ready, set.desired),
        cells: vec![
            ready_cell(set.ready, set.desired),
            KindCell::mono_or_absent(set.service_name.as_deref().unwrap_or_default()),
            KindCell::text_or_absent(non_empty(&set.update_strategy)),
            KindCell::age(set.created_at),
        ],
        sections,
        event: None,
        related_pods: controller_owner(&set.namespace, STATEFUL_SET_KIND, &set.name),
        labels: chips(&set.labels),
        object: KindObject::Plain,
    }
}

pub(crate) fn daemon_set_row(set: &DaemonSetSummary) -> KindRow {
    let status = if set.desired == 0 {
        StatusLabel {
            text: "No nodes scheduled".into(),
            tone: StatusTone::Done,
        }
    } else {
        ready_status(set.ready, set.desired)
    };
    let node_selector = (!set.node_selector.is_empty()).then(|| set.node_selector.join(", "));
    let mut rollout = vec![
        DetailRow::field("Desired", KindCell::count(set.desired)),
        DetailRow::field("Current", KindCell::count(set.current)),
        DetailRow::field("Ready", KindCell::count(set.ready)),
        DetailRow::field("Up-to-date", KindCell::count(set.up_to_date)),
        DetailRow::field("Available", KindCell::count(set.available)),
    ];
    if set.misscheduled > 0 {
        rollout.push(DetailRow::field(
            "Misscheduled",
            toned_number(set.misscheduled, StatusTone::Warn),
        ));
    }
    rollout.push(DetailRow::field(
        "Update strategy",
        KindCell::text_or_absent(non_empty(&set.update_strategy)),
    ));
    let mut sections = vec![
        DetailSection {
            title: "Rollout",
            rows: rollout,
        },
        DetailSection {
            title: "Node selector",
            rows: vec![DetailRow::Chips(chips(&set.node_selector))],
        },
        selector_section(&set.selector),
        containers_section(&set.containers),
    ];
    sections.extend(ports_section(&set.containers));
    KindRow {
        namespace: Some(set.namespace.clone()),
        name: set.name.clone(),
        created_at: set.created_at,
        status,
        cells: vec![
            KindCell::count(set.desired),
            KindCell::count(set.current),
            toned_number(set.ready, replica_tone(set.ready, set.desired)),
            KindCell::count(set.up_to_date),
            KindCell::count(set.available),
            KindCell::text_or_absent(node_selector.as_deref()),
            KindCell::age(set.created_at),
        ],
        sections,
        event: None,
        related_pods: controller_owner(&set.namespace, DAEMON_SET_KIND, &set.name),
        labels: chips(&set.labels),
        object: KindObject::Plain,
    }
}

pub(crate) fn replica_set_row(set: &ReplicaSetSummary) -> KindRow {
    let owner = owner_text(set.owner.as_ref());
    KindRow {
        namespace: Some(set.namespace.clone()),
        name: set.name.clone(),
        created_at: set.created_at,
        status: replicas_status(set.ready, set.desired),
        cells: vec![
            KindCell::count(set.desired),
            KindCell::count(set.current),
            toned_number(set.ready, replica_tone(set.ready, set.desired)),
            KindCell::text_or_absent(owner.as_deref()),
            KindCell::age(set.created_at),
        ],
        sections: vec![
            DetailSection {
                title: "Replicas",
                rows: vec![
                    DetailRow::field("Desired", KindCell::count(set.desired)),
                    DetailRow::field("Current", KindCell::count(set.current)),
                    DetailRow::field("Ready", KindCell::count(set.ready)),
                    owner_row(&set.namespace, set.owner.as_ref()),
                    DetailRow::field(
                        "Revision",
                        KindCell::text_or_absent(set.revision.as_deref()),
                    ),
                ],
            },
            selector_section(&set.selector),
            containers_section(&set.containers),
        ],
        event: None,
        related_pods: controller_owner(&set.namespace, REPLICA_SET_KIND, &set.name),
        labels: chips(&set.labels),
        object: KindObject::Plain,
    }
}

/// The pods of a StatefulSet in ordinal order; a pod without an ordinal sorts last.
pub(crate) fn sort_by_ordinal(pods: &mut [&PodSummary], set_name: &str) {
    pods.sort_by_key(|pod| {
        ordinal(&pod.name, set_name).map_or((true, 0), |ordinal| (false, ordinal))
    });
}

/// The `-{n}` suffix of a StatefulSet pod name.
fn ordinal(pod_name: &str, set_name: &str) -> Option<u32> {
    let digits = pod_name.strip_prefix(set_name)?.strip_prefix('-')?;
    // `parse` alone would accept a leading `+`.
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

/// The Owner row: a link to the owner's drawer when k8sBoard has a screen for its kind, else
/// its text, else a dash.
pub(crate) fn owner_row(namespace: &str, owner: Option<&ControllerRef>) -> DetailRow {
    let Some(owner) = owner else {
        return DetailRow::field("Owner", KindCell::Absent);
    };
    let text = owner_text(Some(owner)).unwrap_or_default();
    match ResourceKey::of_owner(namespace, owner) {
        Some(target) => DetailRow::Link {
            label: "Owner".into(),
            text: text.into(),
            target,
        },
        None => DetailRow::field("Owner", KindCell::Text(text.into())),
    }
}

/// kubectl style: `{kind lowercased}/{name}`.
pub(crate) fn owner_text(owner: Option<&ControllerRef>) -> Option<String> {
    owner.map(|owner| format!("{}/{}", owner.kind.to_lowercase(), owner.name))
}

pub(crate) fn controller_owner(
    namespace: &str,
    kind: &'static str,
    name: &str,
) -> Option<PodOwner> {
    Some(PodOwner::Controller {
        namespace: namespace.to_owned(),
        kind,
        name: name.to_owned(),
    })
}

/// `{ready}/{desired}` toned by `replica_tone`.
fn ready_cell(ready: u32, desired: u32) -> KindCell {
    toned(format!("{ready}/{desired}"), replica_tone(ready, desired))
}

pub(crate) fn toned_number(count: u32, tone: StatusTone) -> KindCell {
    toned(count.to_string(), tone)
}

fn toned(text: String, tone: StatusTone) -> KindCell {
    KindCell::Toned(StatusLabel {
        text: text.into(),
        tone,
    })
}

/// `{ready}/{desired} ready` toned by `replica_tone`.
fn ready_status(ready: u32, desired: u32) -> StatusLabel {
    StatusLabel {
        text: format!("{ready}/{desired} ready").into(),
        tone: replica_tone(ready, desired),
    }
}

/// StatefulSets and ReplicaSets read "Scaled to zero" when nothing is desired.
fn replicas_status(ready: u32, desired: u32) -> StatusLabel {
    if desired == 0 {
        return StatusLabel {
            text: "Scaled to zero".into(),
            tone: StatusTone::Done,
        };
    }
    ready_status(ready, desired)
}

pub(crate) fn optional_count(count: Option<u32>) -> KindCell {
    count.map_or(KindCell::Absent, KindCell::count)
}

/// `{storage} · {class} · {modes}`, leaving out the parts the claim does not set.
fn claim_text(claim: &ClaimTemplate) -> KindCell {
    let modes = (!claim.access_modes.is_empty()).then(|| claim.access_modes.join(","));
    let parts: Vec<&str> = claim
        .storage
        .as_deref()
        .into_iter()
        .chain(claim.storage_class.as_deref())
        .chain(modes.as_deref())
        .collect();
    if parts.is_empty() {
        return KindCell::Absent;
    }
    KindCell::Text(parts.join(" · ").into())
}

fn selector_section(selector: &[String]) -> DetailSection {
    DetailSection {
        title: "Selector",
        rows: vec![DetailRow::Chips(chips(selector))],
    }
}

pub(crate) fn containers_section(containers: &[TemplateContainer]) -> DetailSection {
    DetailSection {
        title: "Containers",
        rows: container_rows(containers),
    }
}

/// `None` when no container declares a port.
fn ports_section(containers: &[TemplateContainer]) -> Option<DetailSection> {
    let rows = port_rows(containers);
    (!rows.is_empty()).then_some(DetailSection {
        title: "Ports",
        rows,
    })
}

#[cfg(test)]
#[path = "workload_rows_tests.rs"]
mod workload_rows_tests;
