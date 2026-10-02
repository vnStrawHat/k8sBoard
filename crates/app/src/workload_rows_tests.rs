use cluster::{
    ClaimTemplate, ContainerPort, ControllerRef, DaemonSetSummary, DeploymentSummary, PodStatus,
    PodSummary, ReadyCount, ReplicaSetSummary, StatefulSetSummary, StatusReason, TemplateContainer,
    WorkloadCondition,
};

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

#[test]
fn workload_row_cells_match_column_count() {
    let rows = [
        (ResourceKind::Deployments, deployment_row(&deployment())),
        (
            ResourceKind::StatefulSets,
            stateful_set_row(&stateful_set()),
        ),
        (ResourceKind::DaemonSets, daemon_set_row(&daemon_set())),
        (ResourceKind::ReplicaSets, replica_set_row(&replica_set())),
    ];
    for (kind, row) in rows {
        assert_eq!(row.cells.len(), kind.columns().len(), "{kind:?}");
        assert_eq!(row.namespace.as_deref(), Some("team-a"));
    }
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
}

#[test]
fn deployment_recreate_strategy_names_no_surge() {
    let mut recreate = deployment();
    recreate.strategy = "Recreate".to_owned();
    recreate.max_surge = None;
    recreate.max_unavailable = None;
    assert_eq!(strategy_text(&recreate).as_deref(), Some("Recreate"));
}

#[test]
fn deployment_without_strategy_shows_absent() {
    let mut unset = deployment();
    unset.strategy.clear();
    assert_eq!(strategy_text(&unset), None);
    assert_eq!(deployment_row(&unset).cells.get(3), Some(&KindCell::Absent));
}

#[test]
fn deployment_ports_section_lists_named_and_unnamed_ports() {
    let row = deployment_row(&deployment());
    let ports = row.section("Ports").expect("ports section");
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
    assert!(deployment_row(&portless).section("Ports").is_none());
}

#[test]
fn deployment_paused_field_appears_only_when_paused() {
    let has_paused = |deployment: &DeploymentSummary| {
        let row = deployment_row(deployment);
        row.section("Replicas")
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
    let conditions = row.section("Conditions").expect("conditions section");
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

fn stateful_set() -> StatefulSetSummary {
    StatefulSetSummary {
        namespace: "team-a".to_owned(),
        name: "web".to_owned(),
        created_at: None,
        labels: Vec::new(),
        desired: 3,
        ready: 2,
        current: 3,
        updated: 3,
        service_name: Some("web-headless".to_owned()),
        update_strategy: "RollingUpdate".to_owned(),
        pod_management_policy: "OrderedReady".to_owned(),
        selector: vec!["app=web".to_owned()],
        containers: Vec::new(),
        claim_templates: vec![ClaimTemplate {
            name: "data".to_owned(),
            storage: Some("10Gi".to_owned()),
            storage_class: Some("fast".to_owned()),
            access_modes: vec!["ReadWriteOnce".to_owned()],
        }],
    }
}

fn daemon_set() -> DaemonSetSummary {
    DaemonSetSummary {
        namespace: "team-a".to_owned(),
        name: "agent".to_owned(),
        created_at: None,
        labels: Vec::new(),
        desired: 4,
        current: 4,
        ready: 4,
        up_to_date: 4,
        available: 4,
        misscheduled: 0,
        node_selector: vec!["disk=ssd".to_owned(), "zone=a".to_owned()],
        update_strategy: "RollingUpdate".to_owned(),
        selector: Vec::new(),
        containers: Vec::new(),
    }
}

fn replica_set() -> ReplicaSetSummary {
    ReplicaSetSummary {
        namespace: "team-a".to_owned(),
        name: "api-7d9f8c".to_owned(),
        created_at: None,
        labels: Vec::new(),
        desired: 2,
        current: 2,
        ready: 2,
        owner: Some(ControllerRef {
            kind: "Deployment".to_owned(),
            name: "api".to_owned(),
        }),
        revision: Some("3".to_owned()),
        selector: Vec::new(),
        containers: Vec::new(),
    }
}

fn pod_named(name: &str) -> PodSummary {
    PodSummary {
        namespace: "team-a".to_owned(),
        name: name.to_owned(),
        status: PodStatus::Reason(StatusReason::Running),
        ready: ReadyCount { ready: 1, total: 1 },
        restarts: 0,
        node_name: None,
        created_at: None,
        pod_ip: None,
        qos_class: None,
        service_account: None,
        controller: None,
        conditions: Vec::new(),
        status_message: None,
        labels: Vec::new(),
        host_network: false,
        containers: Vec::new(),
    }
}

#[test]
fn replica_and_stateful_set_status_say_scaled_to_zero_when_nothing_is_desired() {
    let mut stateful = stateful_set();
    stateful.desired = 0;
    stateful.ready = 0;
    assert_eq!(stateful_set_row(&stateful).status.text, "Scaled to zero");
    let mut replicas = replica_set();
    replicas.desired = 0;
    replicas.ready = 0;
    let status = replica_set_row(&replicas).status;
    assert_eq!(status.text, "Scaled to zero");
    assert_eq!(status.tone, StatusTone::Done);
}

#[test]
fn stateful_set_status_reads_ready_over_desired() {
    let status = stateful_set_row(&stateful_set()).status;
    assert_eq!(status.text, "2/3 ready");
    assert_eq!(status.tone, StatusTone::Warn);
}

#[test]
fn stateful_set_service_cell_is_mono() {
    let row = stateful_set_row(&stateful_set());
    assert_eq!(
        row.cells.get(1),
        Some(&KindCell::Mono("web-headless".into()))
    );
}

#[test]
fn stateful_set_lists_volume_claim_templates() {
    let row = stateful_set_row(&stateful_set());
    let claims = row
        .section("Volume claim templates")
        .expect("claims section");
    assert_eq!(
        claims.rows,
        [DetailRow::field(
            "data",
            KindCell::Text("10Gi · fast · ReadWriteOnce".into())
        )]
    );
}

#[test]
fn claim_text_skips_unset_parts() {
    let claim = ClaimTemplate {
        name: "data".to_owned(),
        storage: Some("1Gi".to_owned()),
        storage_class: None,
        access_modes: Vec::new(),
    };
    assert_eq!(claim_text(&claim), KindCell::Text("1Gi".into()));
    let empty = ClaimTemplate {
        storage: None,
        ..claim
    };
    assert_eq!(claim_text(&empty), KindCell::Absent);
}

#[test]
fn daemon_set_status_reads_ready_over_desired() {
    assert_eq!(daemon_set_row(&daemon_set()).status.text, "4/4 ready");
}

#[test]
fn daemon_set_node_selector_cell_joins_terms() {
    assert_eq!(
        daemon_set_row(&daemon_set()).cells.get(5),
        Some(&KindCell::Text("disk=ssd, zone=a".into()))
    );
}

#[test]
fn idle_daemon_set_has_no_nodes_and_no_selector() {
    let mut idle = daemon_set();
    idle.desired = 0;
    idle.ready = 0;
    idle.node_selector.clear();
    let row = daemon_set_row(&idle);
    assert_eq!(row.status.text, "No nodes scheduled");
    assert_eq!(row.status.tone, StatusTone::Done);
    assert_eq!(row.cells.get(5), Some(&KindCell::Absent));
}

#[test]
fn daemon_set_misscheduled_field_appears_only_when_positive() {
    let has_misscheduled = |set: &DaemonSetSummary| {
        let row = daemon_set_row(set);
        row.section("Rollout")
            .expect("rollout section")
            .rows
            .iter()
            .any(|row| matches!(row, DetailRow::Field { label, .. } if label == "Misscheduled"))
    };
    let mut set = daemon_set();
    assert!(!has_misscheduled(&set));
    set.misscheduled = 1;
    assert!(has_misscheduled(&set));
}

#[test]
fn owner_cells_use_lowercase_kind() {
    let replicas = replica_set_row(&replica_set());
    assert_eq!(
        replicas.cells.get(3),
        Some(&KindCell::Text("deployment/api".into()))
    );
}

#[test]
fn replica_set_without_owner_shows_absent() {
    let mut orphan = replica_set();
    orphan.owner = None;
    assert_eq!(
        replica_set_row(&orphan).cells.get(3),
        Some(&KindCell::Absent)
    );
}

#[test]
fn stateful_set_pods_sort_by_ordinal() {
    let pods = [
        pod_named("web-10"),
        pod_named("web-2"),
        pod_named("web-abc"),
        pod_named("web-0"),
        pod_named("web-+1"),
    ];
    let mut sorted: Vec<&PodSummary> = pods.iter().collect();
    sort_by_ordinal(&mut sorted, "web");
    let names: Vec<&str> = sorted.iter().map(|pod| pod.name.as_str()).collect();
    assert_eq!(names, ["web-0", "web-2", "web-10", "web-abc", "web-+1"]);
}

#[test]
fn workload_related_pods_name_their_controller_kind() {
    let owner = |row: KindRow| match row.related_pods {
        Some(PodOwner::Controller { kind, name, .. }) => Some((kind, name)),
        _ => None,
    };
    assert_eq!(
        owner(stateful_set_row(&stateful_set())),
        Some((STATEFUL_SET_KIND, "web".to_owned()))
    );
    assert_eq!(
        owner(daemon_set_row(&daemon_set())),
        Some((DAEMON_SET_KIND, "agent".to_owned()))
    );
    assert_eq!(
        owner(replica_set_row(&replica_set())),
        Some(("ReplicaSet", "api-7d9f8c".to_owned()))
    );
}
