use cluster::{ByteAmount, ContainerResource, CpuAmount, PodCondition, StatusReason, Termination};

use super::*;

fn container(
    name: &str,
    kind: ContainerKind,
    state: ContainerState,
    is_ready: bool,
) -> ContainerSummary {
    ContainerSummary {
        terminal: cluster::ContainerTerminal::None,
        name: name.to_owned(),
        image: "img".to_owned(),
        kind,
        state,
        is_ready,
        restart_count: 0,
        last_termination: None,
        image_digest: None,
        pull_policy: None,
        is_started: None,
        ports: Vec::new(),
        resources: Vec::new(),
        probes: cluster::ContainerProbes::default(),
        env: Vec::new(),
        env_from: Vec::new(),
        mounts: Vec::new(),
    }
}

fn running() -> ContainerState {
    ContainerState::Running { started_at: None }
}

fn termination(exit_code: i32) -> Termination {
    Termination {
        reason: Some(StatusReason::Error),
        exit_code,
        signal: None,
        started_at: None,
        finished_at: None,
    }
}

#[test]
fn default_container_is_first_main_that_is_not_ready() {
    let containers = [
        container("init", ContainerKind::Init, running(), false),
        container("ready", ContainerKind::Main, running(), true),
        container("broken", ContainerKind::Main, running(), false),
    ];
    assert_eq!(default_container(&containers), Some(2));
}

#[test]
fn default_container_falls_back_to_first_main_then_first() {
    let all_ready = [
        container("init", ContainerKind::Init, running(), true),
        container("a", ContainerKind::Main, running(), true),
        container("b", ContainerKind::Main, running(), true),
    ];
    assert_eq!(default_container(&all_ready), Some(1));
    let only_init = [container("init", ContainerKind::Init, running(), true)];
    assert_eq!(default_container(&only_init), Some(0));
    assert_eq!(default_container(&[]), None);
}

#[test]
fn group_titles_count_progress_per_kind() {
    let done = ContainerState::Terminated(termination(0));
    let failed = ContainerState::Terminated(termination(1));
    let init_done = container("a", ContainerKind::Init, done, false);
    let init_failed = container("b", ContainerKind::Init, failed, false);
    assert_eq!(
        group_title(ContainerKind::Init, &[&init_done, &init_failed]),
        "Init · ran in order 1/2"
    );
    let sidecar = container("s", ContainerKind::Sidecar, running(), true);
    assert_eq!(
        group_title(ContainerKind::Sidecar, &[&sidecar]),
        "Sidecars 1"
    );
    let ready = container("m", ContainerKind::Main, running(), true);
    let not_ready = container("n", ContainerKind::Main, running(), false);
    assert_eq!(
        group_title(ContainerKind::Main, &[&ready, &not_ready]),
        "Containers 1/2"
    );
}

#[test]
fn condition_tooltip_joins_reason_and_message() {
    let condition = |reason: Option<&str>, message: Option<&str>| PodCondition {
        name: "Ready".to_owned(),
        is_true: false,
        reason: reason.map(str::to_owned),
        message: message.map(str::to_owned),
        changed_at: None,
    };
    assert_eq!(
        condition_tooltip(&condition(
            Some("ContainersNotReady"),
            Some("containers with unready status: [api]")
        ))
        .as_deref(),
        Some("ContainersNotReady: containers with unready status: [api]")
    );
    assert_eq!(
        condition_tooltip(&condition(Some("Unschedulable"), None)).as_deref(),
        Some("Unschedulable")
    );
    assert_eq!(
        condition_tooltip(&condition(None, Some("no nodes"))).as_deref(),
        Some("no nodes")
    );
    assert_eq!(condition_tooltip(&condition(None, None)), None);
}

fn resource(name: &str, request: Option<&str>, limit: Option<&str>) -> ContainerResource {
    ContainerResource {
        name: name.to_owned(),
        request: request.map(str::to_owned),
        limit: limit.map(str::to_owned),
    }
}

fn usage(millicores: u64, mebibytes: u64) -> ResourceUsage {
    ResourceUsage {
        cpu: CpuAmount::from_nanocores(millicores * 1_000_000),
        memory: ByteAmount::from_bytes(mebibytes << 20),
    }
}

#[test]
fn container_usage_row_shows_usage_of_limit() {
    let row = container_usage_row(
        &resource("memory", Some("256Mi"), Some("512Mi")),
        Some(usage(0, 498)),
    )
    .expect("a usage row");
    assert_eq!(row.value, "498 of 512Mi");
    assert_eq!(row.tone, Some(StatusTone::Bad));
    assert_eq!(row.note.as_deref(), Some("request 256Mi"));
    let bar = row.bar.expect("a bar");
    assert!((bar.fill - 0.972).abs() < 0.001, "{}", bar.fill);
    assert_eq!(bar.marker, Some(0.5));
    assert_eq!(bar.tone, Some(StatusTone::Bad));

    let cpu = container_usage_row(&resource("cpu", None, Some("1")), Some(usage(310, 0)))
        .expect("a usage row");
    assert_eq!(cpu.value, "310m of 1 core");
    assert_eq!(cpu.tone, None);
    assert_eq!(cpu.note, None);
    assert_eq!(cpu.bar.expect("a bar").marker, None);
}

#[test]
fn container_usage_row_without_limit_has_no_bar() {
    let row = container_usage_row(&resource("cpu", Some("250m"), None), Some(usage(310, 0)))
        .expect("a usage row");
    assert_eq!(row.value, "310m used");
    assert_eq!(row.bar, None);
    assert_eq!(row.tone, None);
    assert_eq!(row.note.as_deref(), Some("request 250m · no limit"));
    let bare = container_usage_row(&resource("memory", None, None), Some(usage(0, 498)))
        .expect("a usage row");
    assert_eq!(bare.note.as_deref(), Some("no request · no limit"));
    let zero = container_usage_row(&resource("cpu", None, Some("0")), Some(usage(10, 0)))
        .expect("a usage row");
    assert_eq!(zero.bar, None);
}

#[test]
fn container_usage_row_keeps_the_text_without_usage_or_for_other_resources() {
    let memory = resource("memory", Some("1Mi"), Some("2Mi"));
    assert_eq!(container_usage_row(&memory, None), None);
    let storage = resource("ephemeral-storage", Some("1Gi"), None);
    assert_eq!(container_usage_row(&storage, Some(usage(1, 1))), None);
}

#[test]
fn container_display_order_groups_init_sidecar_main() {
    let containers = [
        container("main-a", ContainerKind::Main, running(), true),
        container("side", ContainerKind::Sidecar, running(), true),
        container("init", ContainerKind::Init, running(), true),
        container("main-b", ContainerKind::Main, running(), true),
        container("init-b", ContainerKind::Init, running(), true),
    ];
    // Init, Sidecar, Main; spec order inside each group.
    assert_eq!(container_display_order(&containers), [2, 4, 1, 0, 3]);
    assert!(container_display_order(&[]).is_empty());
}

#[test]
fn restart_label_is_hidden_for_zero_and_warns_above() {
    assert_eq!(restart_label(0), None);
    let one = restart_label(1).expect("one restart is labelled");
    assert_eq!(
        (one.text.as_ref(), one.tone),
        ("1 restart", StatusTone::Warn)
    );
    let many = restart_label(10).expect("ten restarts are labelled");
    assert_eq!(many.text.as_ref(), "10 restarts");
}

fn mounting(name: &str, mounts: Vec<(&str, VolumeSource)>) -> ContainerSummary {
    let mut summary = container(name, ContainerKind::Main, running(), true);
    summary.mounts = mounts
        .into_iter()
        .map(|(volume, source)| cluster::MountEntry {
            path: format!("/mnt/{volume}"),
            volume: volume.to_owned(),
            source,
            is_read_only: false,
            sub_path: None,
        })
        .collect();
    summary
}

fn secret(name: &str) -> VolumeSource {
    VolumeSource::Secret {
        name: name.to_owned(),
    }
}

#[test]
fn volume_rows_dedupe_by_volume_name_in_first_seen_order() {
    let containers = [
        mounting(
            "init",
            vec![
                ("certs", secret("tls")),
                ("scratch", VolumeSource::EmptyDir),
            ],
        ),
        mounting(
            "app",
            vec![
                ("scratch", VolumeSource::EmptyDir),
                ("certs", secret("tls")),
            ],
        ),
        mounting("side", vec![("cache", VolumeSource::EmptyDir)]),
    ];
    let names: Vec<String> = volume_rows("shop", &containers)
        .into_iter()
        .map(|row| row.name)
        .collect();
    assert_eq!(names, ["certs", "scratch", "cache"]);
    assert!(volume_rows("shop", &[]).is_empty());
}

#[test]
fn volume_rows_name_sources_and_link_only_those_with_a_screen() {
    let containers = [mounting(
        "app",
        vec![
            (
                "config",
                VolumeSource::ConfigMap {
                    name: "api".to_owned(),
                },
            ),
            ("key", secret("api-key")),
            (
                "data",
                VolumeSource::PersistentVolumeClaim {
                    claim: "data-0".to_owned(),
                },
            ),
            ("tmp", VolumeSource::EmptyDir),
            (
                "logs",
                VolumeSource::HostPath {
                    path: "/var/log".to_owned(),
                },
            ),
            ("labels", VolumeSource::DownwardApi),
            (
                "token",
                VolumeSource::Projected {
                    config_maps: vec!["ca".to_owned()],
                    secrets: vec!["sa-token".to_owned()],
                },
            ),
            (
                "bare",
                VolumeSource::Projected {
                    config_maps: Vec::new(),
                    secrets: Vec::new(),
                },
            ),
            ("odd", VolumeSource::Other),
        ],
    )];
    let rows = volume_rows("shop", &containers);
    let listed: Vec<(&str, &str, bool)> = rows
        .iter()
        .map(|row| (row.name.as_str(), row.source.as_str(), row.target.is_some()))
        .collect();
    assert_eq!(
        listed,
        [
            ("config", "configmap/api", true),
            ("key", "secret/api-key", true),
            ("data", "pvc/data-0", true),
            ("tmp", "emptyDir", false),
            ("logs", "hostPath /var/log", false),
            ("labels", "downwardAPI", false),
            ("token", "projected · configmap/ca, secret/sa-token", false),
            ("bare", "projected", false),
            ("odd", "volume", false),
        ]
    );
    assert_eq!(
        rows[1].target,
        ResourceKey::of_object("Secret", Some("shop"), "api-key")
    );
    assert_eq!(
        rows[2].target,
        ResourceKey::of_object("PersistentVolumeClaim", Some("shop"), "data-0")
    );
}

fn service(ports: &[(u16, &str)]) -> ServiceSummary {
    ServiceSummary {
        namespace: "team-a".to_owned(),
        name: "api".to_owned(),
        created_at: None,
        labels: Vec::new(),
        service_type: "ClusterIP".to_owned(),
        cluster_ips: Vec::new(),
        is_headless: false,
        external_addresses: Vec::new(),
        ports: ports
            .iter()
            .map(|(port, protocol)| cluster::ServicePortSummary {
                name: None,
                port: *port,
                target_port: None,
                node_port: None,
                protocol: (*protocol).to_owned(),
            })
            .collect(),
        selector: Vec::new(),
    }
}

#[test]
fn service_summary_text_names_the_type_and_ports() {
    assert_eq!(
        service_summary_text(&service(&[(80, "TCP"), (443, "TCP")])),
        "ClusterIP · 80/TCP, 443/TCP"
    );
    assert_eq!(service_summary_text(&service(&[])), "ClusterIP");
}

#[test]
fn container_list_fits_the_longest_name_within_its_limits() {
    let named = |name: &str| container(name, ContainerKind::Main, running(), true);
    assert_eq!(container_list_width(&[]), px(240.));
    assert_eq!(container_list_width(&[named("web")]), px(240.));
    assert_eq!(
        container_list_width(&[named("web"), named("ingress-controller-nginx-extra")]),
        px(30. * 8. + 96.)
    );
    assert_eq!(container_list_width(&[named(&"x".repeat(80))]), px(360.));
}
