use cluster::{
    ClaimTemplate, ContainerPort, ControllerRef, DaemonSetSummary, DeploymentSummary, PodStatus,
    PodSummary, ReadyCount, ReplicaSetSummary, StatefulSetSummary, StatusReason, TemplateContainer,
    WorkloadCondition,
};

use super::*;
use crate::resource_kind::ResourceKind;
use crate::table_selection::ResourceKey;

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
        progress_deadline_seconds: 600,
        is_paused: false,
        generation: 1,
        observed_generation: 1,
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
                    host_port: None,
                },
                ContainerPort {
                    name: None,
                    port: 9090,
                    protocol: "UDP".to_owned(),
                    host_port: None,
                },
            ],
        }],
        conditions: Vec::new(),
        template_change: None,
    }
}

fn condition(name: &str, is_true: bool, reason: Option<&str>) -> WorkloadCondition {
    WorkloadCondition {
        name: name.to_owned(),
        is_true,
        reason: reason.map(str::to_owned),
        message: None,
        last_transition: None,
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
                text: "8080/TCP · http · web".into(),
                port: 8080,
                is_tcp: true,
            },
            DetailRow::Port {
                text: "9090/UDP · web".into(),
                port: 9090,
                is_tcp: false,
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
            DetailRow::Condition { status, .. } => Some((status.text.to_string(), status.tone)),
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
        claim_retention: None,
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
        is_finished: false,
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
        image_pull_secrets: Vec::new(),
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
        row.section("Rollout by node")
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
fn replica_set_revision_cell_sorts_as_a_number() {
    let revision_of = |revision: Option<&str>| {
        let mut set = replica_set();
        set.revision = revision.map(str::to_owned);
        replica_set_row(&set).cells.get(4).cloned()
    };
    let columns = ResourceKind::ReplicaSets.columns();
    assert_eq!(columns[4].name, "Revision");
    assert_eq!(
        revision_of(Some("10")),
        Some(KindCell::Quantity {
            text: "10".into(),
            value: 10,
            tone: None,
        })
    );
    assert_eq!(revision_of(Some("n/a")), Some(KindCell::Absent));
    assert_eq!(revision_of(None), Some(KindCell::Absent));
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

#[test]
fn deployment_row_has_revisions_section() {
    let row = deployment_row(&deployment());
    let titles: Vec<&str> = row.sections.iter().map(|section| section.title).collect();
    // Revisions follow Conditions; the pods and labels sections are added by the drawer.
    assert_eq!(
        titles,
        [
            "Replicas",
            "Selector",
            "Containers",
            "Ports",
            "Conditions",
            "Revisions"
        ]
    );
    assert_eq!(
        row.section("Revisions").map(|section| section.rows.clone()),
        Some(vec![DetailRow::Live(LiveContent::Revisions)])
    );
}

#[test]
fn replica_set_owner_is_a_link() {
    let row = replica_set_row(&replica_set());
    let replicas = row.section("Replicas").expect("replicas section");
    assert!(replicas.rows.contains(&DetailRow::Link {
        label: "Owner".into(),
        text: "deployment/api".into(),
        target: ResourceKey::Kind {
            kind: ResourceKind::Deployments,
            namespace: Some("team-a".to_owned()),
            name: "api".to_owned(),
        },
    }));
    let mut orphan = replica_set();
    orphan.owner = None;
    let row = replica_set_row(&orphan);
    let replicas = row.section("Replicas").expect("replicas section");
    assert!(
        replicas
            .rows
            .contains(&DetailRow::field("Owner", KindCell::Absent))
    );
}

#[test]
fn deployment_rows_keep_their_object() {
    let summary = deployment();
    assert_eq!(
        deployment_row(&summary).object,
        KindObject::Deployment(summary)
    );
    let set = replica_set();
    assert_eq!(replica_set_row(&set).object, KindObject::ReplicaSet(set));
    assert_eq!(
        stateful_set_row(&stateful_set()).object,
        KindObject::StatefulSet(stateful_set())
    );
    assert_eq!(
        daemon_set_row(&daemon_set()).object,
        KindObject::DaemonSet(daemon_set())
    );
}

#[test]
fn daemon_set_rollout_by_node_starts_with_bars() {
    let mut set = daemon_set();
    set.ready = 3;
    set.up_to_date = 4;
    let row = daemon_set_row(&set);
    let titles: Vec<&str> = row.sections.iter().map(|section| section.title).collect();
    assert_eq!(
        titles,
        [
            "Rollout by node",
            "Not ready",
            "Node selector",
            "Selector",
            "Containers"
        ]
    );
    let rollout = row.section("Rollout by node").expect("rollout section");
    assert_eq!(
        rollout.rows[0],
        DetailRow::Bar {
            label: "Ready".into(),
            percent: 75,
            text: "3 / 4".into(),
            tone: Some(StatusTone::Warn),
        }
    );
    assert_eq!(
        rollout.rows[1],
        DetailRow::Bar {
            label: "Up-to-date".into(),
            percent: 100,
            text: "4 / 4".into(),
            tone: Some(StatusTone::Ok),
        }
    );
    // The count fields follow the bars.
    assert_eq!(
        rollout.rows[2],
        DetailRow::field("Desired", KindCell::count(4))
    );
    assert_eq!(
        row.section("Not ready").map(|section| section.rows.clone()),
        Some(vec![DetailRow::Live(LiveContent::NotReadyPods)])
    );
    // Nothing desired: empty bars, not a division by zero.
    let mut idle = daemon_set();
    (idle.desired, idle.ready, idle.up_to_date) = (0, 0, 0);
    let rollout = daemon_set_row(&idle)
        .section("Rollout by node")
        .cloned()
        .expect("rollout section");
    assert!(matches!(
        &rollout.rows[0],
        DetailRow::Bar { percent: 0, text, tone: Some(StatusTone::Done), .. } if text == "0 / 0"
    ));
}

#[test]
fn daemon_set_port_shows_host_port() {
    let mut set = daemon_set();
    set.containers = vec![TemplateContainer {
        name: "agent".to_owned(),
        image: "agent:1".to_owned(),
        ports: vec![
            ContainerPort {
                name: Some("metrics".to_owned()),
                port: 9100,
                protocol: "TCP".to_owned(),
                host_port: Some(9100),
            },
            ContainerPort {
                name: None,
                port: 8080,
                protocol: "TCP".to_owned(),
                host_port: None,
            },
        ],
    }];
    let row = daemon_set_row(&set);
    let ports = row.section("Ports").expect("ports section");
    assert_eq!(
        ports.rows,
        [
            DetailRow::Port {
                text: "9100/TCP · metrics · agent · host 9100".into(),
                port: 9100,
                is_tcp: true,
            },
            DetailRow::Port {
                text: "8080/TCP · agent".into(),
                port: 8080,
                is_tcp: true,
            },
        ]
    );
}

#[test]
fn stateful_set_claims_show_retention() {
    let mut set = stateful_set();
    set.claim_retention = Some("whenDeleted Retain \u{b7} whenScaled Delete".to_owned());
    let row = stateful_set_row(&set);
    let claims = row
        .section("Volume claim templates")
        .expect("claims section");
    assert_eq!(
        claims.rows.last(),
        Some(&DetailRow::field(
            "Retention",
            KindCell::Hinted {
                text: "Retain / Delete".into(),
                tooltip: "whenDeleted Retain \u{b7} whenScaled Delete".into(),
            }
        ))
    );
    // A set without claim templates has nothing to retain: no Retention row.
    let mut claimless = set.clone();
    claimless.claim_templates = Vec::new();
    let row = stateful_set_row(&claimless);
    let claims = row
        .section("Volume claim templates")
        .expect("claims section");
    assert!(claims.rows.is_empty());
    // Unset: no Retention row.
    let row = stateful_set_row(&stateful_set());
    let claims = row
        .section("Volume claim templates")
        .expect("claims section");
    assert!(
        !claims
            .rows
            .iter()
            .any(|row| matches!(row, DetailRow::Field { label, .. } if label == "Retention"))
    );
}

#[test]
fn replica_set_template_shows_hash_and_image() {
    let container = |name: &str, image: &str| TemplateContainer {
        name: name.to_owned(),
        image: image.to_owned(),
        ports: Vec::new(),
    };
    let mut set = replica_set();
    set.labels = vec!["app=api".to_owned(), "pod-template-hash=7d9f8c".to_owned()];
    set.containers = vec![container("web", "registry/api:1.4")];
    let row = replica_set_row(&set);
    // Template replaces Containers.
    assert!(row.section("Containers").is_none());
    let template = row.section("Template").expect("template section");
    assert_eq!(
        template.rows,
        [
            DetailRow::field("pod-template-hash", KindCell::Mono("7d9f8c".into())),
            DetailRow::copyable_field("Image", KindCell::Mono("registry/api:1.4".into())),
        ]
    );
    // Several containers: one row per container, no hash label: a dash.
    set.labels = Vec::new();
    set.containers = vec![
        container("web", "registry/api:1.4"),
        container("sidecar", "proxy:2"),
    ];
    let row = replica_set_row(&set);
    let template = row.section("Template").expect("template section");
    assert_eq!(
        template.rows,
        [
            DetailRow::field("pod-template-hash", KindCell::Absent),
            DetailRow::copyable_field("web", KindCell::Mono("registry/api:1.4".into())),
            DetailRow::copyable_field("sidecar", KindCell::Mono("proxy:2".into())),
        ]
    );
}

#[test]
fn retention_short_form_drops_the_field_names() {
    assert_eq!(
        retention_short("whenDeleted Retain \u{b7} whenScaled Delete"),
        "Retain / Delete"
    );
    // Any other shape is shown as it is.
    assert_eq!(retention_short("something else"), "something else");
}

#[test]
fn deployment_containers_offer_to_copy_their_images() {
    let row = deployment_row(&deployment());
    let containers = row.section("Containers").expect("containers section");
    assert_eq!(
        containers.rows,
        [DetailRow::copyable_field(
            "web",
            KindCell::Mono("nginx:1.27".into())
        )]
    );
    assert!(matches!(
        containers.rows[0],
        DetailRow::CopyField { ref text, .. } if text.as_ref() == "nginx:1.27"
    ));
}

#[test]
fn a_long_container_name_goes_above_its_image() {
    let container = |name: &str, image: &str| TemplateContainer {
        name: name.to_owned(),
        image: image.to_owned(),
        ports: Vec::new(),
    };
    let rows = container_rows(&[
        container("web", "registry/api:1.4"),
        container("argocd-applicationset-controller", "registry/argocd:2"),
    ]);
    assert_eq!(
        rows,
        [
            DetailRow::copyable_field("web", KindCell::Mono("registry/api:1.4".into())),
            DetailRow::stacked(
                "argocd-applicationset-controller",
                KindCell::Mono("registry/argocd:2".into())
            ),
        ]
    );
}

#[test]
fn workload_kinds_share_one_vocabulary() {
    let field_labels = |row: &KindRow| -> Vec<String> {
        row.sections
            .iter()
            .flat_map(|section| section.rows.iter())
            .filter_map(|row| match row {
                DetailRow::Field { label, .. } => Some(label.to_string()),
                _ => None,
            })
            .collect()
    };
    let stateful = field_labels(&stateful_set_row(&stateful_set()));
    let daemon = field_labels(&daemon_set_row(&daemon_set()));
    let deployment = field_labels(&deployment_row(&deployment()));
    for labels in [&stateful, &daemon, &deployment] {
        assert!(labels.iter().any(|label| label == "Strategy"));
        assert!(!labels.iter().any(|label| label == "Update strategy"));
        assert!(!labels.iter().any(|label| label == "Updated"));
    }
    let columns = |kind: ResourceKind| -> Vec<&'static str> {
        kind.columns().iter().map(|column| column.name).collect()
    };
    assert!(columns(ResourceKind::StatefulSets).contains(&"Strategy"));
    assert!(columns(ResourceKind::CronJobs).contains(&"Last run"));
}

#[test]
fn a_condition_keeps_its_reason_message_and_transition_time() {
    let at = "2026-10-01T10:00:00Z".parse().expect("a valid timestamp");
    let mut available = condition("Progressing", true, Some("NewReplicaSetAvailable"));
    available.message = Some("ReplicaSet \"api-7d9\" has successfully progressed.".to_owned());
    available.last_transition = Some(at);
    assert_eq!(
        condition_row(&available),
        DetailRow::Condition {
            name: "Progressing".into(),
            status: StatusLabel {
                text: "True · NewReplicaSetAvailable".into(),
                tone: StatusTone::Ok,
            },
            since: Some(at),
            message: Some("ReplicaSet \"api-7d9\" has successfully progressed.".into()),
        }
    );
}

#[test]
fn a_true_failure_condition_is_bad_and_a_true_completion_is_ok() {
    let tone = |name: &str| match condition_row(&condition(name, true, None)) {
        DetailRow::Condition { status, .. } => status.tone,
        other => panic!("not a condition row: {other:?}"),
    };
    for name in [
        "Failed",
        "ReplicaFailure",
        "ScalingLimited",
        "Unschedulable",
    ] {
        assert_eq!(tone(name), StatusTone::Bad, "{name}");
    }
    assert_eq!(tone("Complete"), StatusTone::Ok);
}

#[test]
fn deployment_and_stateful_set_lead_with_rollout_bars_like_a_daemon_set() {
    let leads = |row: &KindRow, title: &str| -> Vec<DetailRow> {
        row.section(title).expect("section").rows.clone()
    };
    let mut rolling = deployment();
    (rolling.desired, rolling.ready, rolling.up_to_date) = (4, 2, 4);
    let rows = leads(&deployment_row(&rolling), "Replicas");
    assert_eq!(
        rows[0],
        DetailRow::Bar {
            label: "Ready".into(),
            percent: 50,
            text: "2 / 4".into(),
            tone: Some(StatusTone::Warn),
        }
    );
    assert!(matches!(&rows[1], DetailRow::Bar { label, text, .. }
        if label == "Up-to-date" && text == "4 / 4"));
    let repeated = |rows: &[DetailRow]| {
        rows.iter().any(|row| {
            matches!(row, DetailRow::Field { label, .. }
            if label == "Ready" || label == "Up-to-date")
        })
    };
    assert!(!repeated(&rows));
    let rows = leads(&stateful_set_row(&stateful_set()), "Replicas");
    assert!(matches!(&rows[0], DetailRow::Bar { label, .. } if label == "Ready"));
    assert!(matches!(&rows[1], DetailRow::Bar { label, .. } if label == "Up-to-date"));
    assert!(!repeated(&rows));
    let rollout = leads(&daemon_set_row(&daemon_set()), "Rollout by node");
    assert!(!repeated(&rollout));
}
