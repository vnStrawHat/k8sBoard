use std::net::{Ipv4Addr, SocketAddrV4};
use std::path::PathBuf;

use cluster::{
    ContainerProbes, ContainerState, ContainerSummary, DeploymentSummary, ForwardTraffic,
    PodStatus, PodSummary, ReadyCount, ServiceSummary, StatefulSetSummary, StatusReason,
};

use super::*;
use crate::environment::Environment;
use crate::network_rows::service_row;
use crate::port_forwards::{ForwardFixture, ForwardState, LocalPortSpec};
use crate::workload_rows::{deployment_row, stateful_set_row};

fn tcp(name: Option<&str>, port: u16) -> ContainerPort {
    ContainerPort {
        name: name.map(str::to_owned),
        port,
        protocol: "TCP".to_owned(),
        host_port: None,
    }
}

fn udp(port: u16) -> ContainerPort {
    ContainerPort {
        protocol: "UDP".to_owned(),
        ..tcp(None, port)
    }
}

fn container(name: &str, kind: ContainerKind, ports: Vec<ContainerPort>) -> ContainerSummary {
    ContainerSummary {
        terminal: cluster::ContainerTerminal::None,
        name: name.to_owned(),
        image: "img".to_owned(),
        kind,
        state: ContainerState::Running { started_at: None },
        is_ready: true,
        restart_count: 0,
        last_termination: None,
        image_digest: None,
        pull_policy: None,
        is_started: None,
        ports,
        resources: Vec::new(),
        probes: ContainerProbes::default(),
        env: Vec::new(),
        env_from: Vec::new(),
        mounts: Vec::new(),
    }
}

fn pod(containers: Vec<ContainerSummary>) -> PodSummary {
    PodSummary {
        is_finished: false,
        namespace: "shop".to_owned(),
        name: "api-0".to_owned(),
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
        containers,
        status_message: None,
        labels: Vec::new(),
        host_network: false,
        image_pull_secrets: Vec::new(),
        node_selector: Vec::new(),
        node_affinity: Vec::new(),
    }
}

fn service(
    service_type: &str,
    selector: &[&str],
    ports: Vec<ServicePortSummary>,
) -> ServiceSummary {
    ServiceSummary {
        namespace: "shop".to_owned(),
        name: "api".to_owned(),
        created_at: None,
        labels: Vec::new(),
        service_type: service_type.to_owned(),
        cluster_ips: Vec::new(),
        is_headless: false,
        external_addresses: Vec::new(),
        ports,
        selector: selector.iter().map(|term| (*term).to_owned()).collect(),
    }
}

fn service_port(name: Option<&str>, port: u16, protocol: &str) -> ServicePortSummary {
    ServicePortSummary {
        name: name.map(str::to_owned),
        port,
        target_port: None,
        node_port: None,
        protocol: protocol.to_owned(),
    }
}

fn template(ports: Vec<ContainerPort>) -> Vec<TemplateContainer> {
    vec![TemplateContainer {
        resources: Vec::new(),
        name: "web".to_owned(),
        image: "img".to_owned(),
        ports,
    }]
}

fn deployment(containers: Vec<TemplateContainer>) -> DeploymentSummary {
    DeploymentSummary {
        namespace: "shop".to_owned(),
        name: "web".to_owned(),
        created_at: None,
        labels: Vec::new(),
        desired: 1,
        ready: 1,
        up_to_date: 1,
        available: 1,
        strategy: String::new(),
        max_surge: None,
        max_unavailable: None,
        progress_deadline_seconds: 600,
        is_paused: false,
        generation: 1,
        observed_generation: 1,
        revision: None,
        selector: vec!["app=web".to_owned()],
        containers,
        conditions: Vec::new(),
        template_change: None,
    }
}

fn stateful_set(containers: Vec<TemplateContainer>) -> StatefulSetSummary {
    StatefulSetSummary {
        namespace: "shop".to_owned(),
        name: "db".to_owned(),
        created_at: None,
        labels: Vec::new(),
        desired: 1,
        ready: 1,
        current: 1,
        updated: 1,
        service_name: None,
        update_strategy: String::new(),
        pod_management_policy: String::new(),
        selector: vec!["app=db".to_owned()],
        containers,
        claim_templates: Vec::new(),
        claim_retention: None,
    }
}

fn enabled() -> ActionAvailability {
    ActionAvailability::Enabled
}

fn disabled(reason: &str) -> ActionAvailability {
    ActionAvailability::Disabled {
        reason: reason.to_owned().into(),
    }
}

fn labels(subject: &ForwardSubject) -> Vec<&str> {
    subject
        .ports
        .iter()
        .map(|port| port.label.as_str())
        .collect()
}

#[test]
fn pod_menu_lists_tcp_ports_with_tags() {
    let subject = pod_subject(&pod(vec![
        container("init", ContainerKind::Init, vec![tcp(None, 1)]),
        container(
            "proxy",
            ContainerKind::Sidecar,
            vec![tcp(Some("admin"), 15000)],
        ),
        container(
            "web",
            ContainerKind::Main,
            vec![tcp(Some("http"), 8080), udp(5353)],
        ),
    ]));
    assert_eq!(subject.namespace, "shop");
    assert_eq!(subject.target, TargetSpec::pod("api-0"));
    // The init container is not offered; each port names its container and its tag.
    assert_eq!(
        labels(&subject),
        [
            "proxy · admin 15000/TCP · SIDECAR",
            "web · http 8080/TCP · MAIN",
            "web · 5353/UDP · MAIN",
        ]
    );
    let ports: Vec<(u16, bool)> = subject
        .ports
        .iter()
        .map(|port| (port.remote_port, port.is_tcp))
        .collect();
    assert_eq!(ports, [(15000, true), (8080, true), (5353, false)]);
}

#[test]
fn the_menu_state_follows_the_gate_the_target_and_its_ports() {
    let one = pod_subject(&pod(vec![container(
        "web",
        ContainerKind::Main,
        vec![tcp(None, 8080)],
    )]));
    assert!(matches!(
        menu_state(&one, enabled()),
        MenuState::Direct(port) if port.remote_port == 8080
    ));
    // The gate comes first, whatever the ports are.
    assert_eq!(
        menu_state(
            &one,
            disabled("Not permitted: get and create pods/portforward")
        ),
        MenuState::Disabled("Not permitted: get and create pods/portforward".into())
    );

    let none = pod_subject(&pod(vec![container("web", ContainerKind::Main, vec![])]));
    assert_eq!(menu_state(&none, enabled()), MenuState::NewForward);

    let several = pod_subject(&pod(vec![container(
        "web",
        ContainerKind::Main,
        vec![tcp(None, 8080), tcp(None, 9090)],
    )]));
    assert!(matches!(
        menu_state(&several, enabled()),
        MenuState::Pick(ports) if ports.len() == 2
    ));

    // One TCP port next to a UDP one is a choice, with the UDP entry disabled by the menu.
    let mixed = pod_subject(&pod(vec![container(
        "dns",
        ContainerKind::Main,
        vec![tcp(None, 53), udp(53)],
    )]));
    assert!(matches!(menu_state(&mixed, enabled()), MenuState::Pick(_)));

    for ports in [vec![udp(53)], vec![udp(53), udp(5353)]] {
        let udp_only = pod_subject(&pod(vec![container("dns", ContainerKind::Main, ports)]));
        assert_eq!(
            menu_state(&udp_only, enabled()),
            MenuState::Disabled("UDP ports cannot be forwarded".into())
        );
    }
}

#[test]
fn a_service_without_a_selector_or_an_external_name_cannot_be_forwarded_to() {
    let ports = || vec![service_port(Some("http"), 80, "TCP")];
    let fine = row_subject(&service_row(&service("ClusterIP", &["app=api"], ports())))
        .expect("a Service subject");
    assert_eq!(fine.target.kind, TargetKind::Service);
    assert_eq!(fine.blocked, None);
    assert_eq!(labels(&fine), ["http 80/TCP"]);

    for row in [
        service_row(&service("ClusterIP", &[], ports())),
        service_row(&service("ExternalName", &["app=api"], ports())),
    ] {
        let subject = row_subject(&row).expect("a Service subject");
        assert_eq!(
            menu_state(&subject, enabled()),
            MenuState::Disabled("Forward a pod: this Service selects no pods".into())
        );
    }
}

#[test]
fn workloads_offer_their_template_ports() {
    let deployment = row_subject(&deployment_row(&deployment(template(vec![tcp(
        Some("http"),
        8080,
    )]))))
    .expect("a Deployment subject");
    assert_eq!(deployment.target.kind, TargetKind::Deployment);
    assert_eq!(deployment.target.name, "web");
    assert_eq!(deployment.namespace, "shop");
    assert_eq!(labels(&deployment), ["web · http 8080/TCP"]);

    let set = row_subject(&stateful_set_row(&stateful_set(template(vec![tcp(
        None, 5432,
    )]))))
    .expect("a StatefulSet subject");
    assert_eq!(set.target.kind, TargetKind::StatefulSet);
    assert!(matches!(menu_state(&set, enabled()), MenuState::Direct(_)));
}

#[test]
fn other_kinds_have_no_subject() {
    // A row of a kind that is not forwardable has none, namespaced or not.
    let mut row = service_row(&service("ClusterIP", &["app=api"], vec![]));
    row.object = KindObject::Plain;
    assert_eq!(row_subject(&row), None);
}

#[test]
fn the_dialog_starts_from_the_first_tcp_port() {
    let subject = pod_subject(&pod(vec![container(
        "dns",
        ContainerKind::Main,
        vec![udp(53), tcp(None, 8080), tcp(None, 9090)],
    )]));
    assert_eq!(first_tcp_port(&subject), Some(8080));
    let none = pod_subject(&pod(vec![container("web", ContainerKind::Main, vec![])]));
    assert_eq!(first_tcp_port(&none), None);
}

fn cluster(context: &str) -> ClusterRef {
    ClusterRef {
        kubeconfig: PathBuf::from("a.yaml"),
        context: context.to_owned(),
    }
}

fn running_forward(forwards: &mut PortForwards, context: &str, port: u16, local: u16) -> ForwardId {
    forwards.insert_fixture(ForwardFixture {
        cluster: cluster(context),
        cluster_label: context.to_owned().into(),
        environment: Environment::STAGING,
        spec: crate::port_forwards::ForwardSpec {
            namespace: "shop".to_owned(),
            target: TargetSpec::pod("api-0"),
            remote_port: port,
            local_port: LocalPortSpec::Auto,
        },
        state: ForwardState::Active,
        local: Some(SocketAddrV4::new(Ipv4Addr::LOCALHOST, local)),
        pod: None,
        traffic: ForwardTraffic::default(),
        events: Vec::new(),
        started_at: None,
        is_preset: false,
    })
}

#[test]
fn forward_button_states() {
    let subject = pod_subject(&pod(vec![container(
        "web",
        ContainerKind::Main,
        vec![tcp(None, 9090), udp(5353)],
    )]));
    let (http, dns) = (subject.ports[0].clone(), subject.ports[1].clone());
    let mut forwards = PortForwards::new();
    let id = running_forward(&mut forwards, "prod-a", 9090, 19090);
    let here = cluster("prod-a");
    let buttons = PortButtons {
        forwards: &forwards,
        cluster: &here,
        gate: enabled(),
    };
    // A running forward of the port is live, with the port it listens on.
    assert_eq!(
        buttons.button(&subject, &http),
        PortButton::Live {
            id,
            local_port: 19090
        }
    );
    assert_eq!(
        buttons.button(&subject, &dns),
        PortButton::Disabled("UDP ports cannot be forwarded".into())
    );
    let other = PortChoice {
        label: "x".to_owned(),
        remote_port: 9091,
        is_tcp: true,
    };
    assert_eq!(buttons.button(&subject, &other), PortButton::Offer);

    // The gate of the drawer's cluster disables an offer; a live forward stays live so it can stop.
    let gated = PortButtons {
        forwards: &forwards,
        cluster: &here,
        gate: disabled("prod-a is read-only"),
    };
    assert_eq!(
        gated.button(&subject, &other),
        PortButton::Disabled("prod-a is read-only".into())
    );
    assert!(matches!(
        gated.button(&subject, &http),
        PortButton::Live { .. }
    ));

    let blocked = ForwardSubject {
        blocked: Some("Forward a pod: this Service selects no pods"),
        ..subject.clone()
    };
    assert_eq!(
        buttons.button(&blocked, &other),
        PortButton::Disabled("Forward a pod: this Service selects no pods".into())
    );
}

#[test]
fn a_forward_of_another_cluster_does_not_make_the_button_live() {
    let subject = pod_subject(&pod(vec![container(
        "web",
        ContainerKind::Main,
        vec![tcp(None, 9090)],
    )]));
    let mut forwards = PortForwards::new();
    running_forward(&mut forwards, "prod-a", 9090, 19090);
    let there = cluster("stg-b");
    let buttons = PortButtons {
        forwards: &forwards,
        cluster: &there,
        gate: enabled(),
    };
    assert_eq!(
        buttons.button(&subject, &subject.ports[0]),
        PortButton::Offer
    );
}

#[test]
fn a_running_forward_shows_its_address_and_a_copy_tooltip_apart_from_stop() {
    // The address is a copy link of its own: its text and tooltip name the address, never Stop.
    assert_eq!(forward_address_text(19090), "localhost:19090");
    assert_eq!(copy_address_tooltip(19090), "Copy localhost:19090");
}

fn owned_pod(controller: Option<(&str, &str)>, labels: &[&str]) -> PodSummary {
    PodSummary {
        controller: controller.map(|(kind, name)| cluster::ControllerRef {
            kind: kind.to_owned(),
            name: name.to_owned(),
        }),
        labels: labels.iter().map(|term| (*term).to_owned()).collect(),
        ..pod(vec![container(
            "web",
            ContainerKind::Main,
            vec![tcp(None, 80)],
        )])
    }
}

#[test]
fn a_deployment_pod_forwards_to_its_deployment() {
    let pod = owned_pod(
        Some(("ReplicaSet", "web-6d9f7c")),
        &["app=web", "pod-template-hash=6d9f7c"],
    );
    let subject = pod_drawer_subject(&pod);
    assert_eq!(
        subject.target,
        TargetSpec {
            kind: TargetKind::Deployment,
            name: "web".to_owned()
        }
    );
    // The ports are the pod's own.
    assert_eq!(subject.ports.len(), 1);
}

#[test]
fn a_stateful_set_pod_forwards_to_its_stateful_set() {
    let pod = owned_pod(Some(("StatefulSet", "db")), &[]);
    assert_eq!(
        pod_drawer_subject(&pod).target,
        TargetSpec {
            kind: TargetKind::StatefulSet,
            name: "db".to_owned()
        }
    );
}

#[test]
fn a_pod_without_a_rollout_owner_forwards_by_name() {
    let by_name = TargetSpec::pod("api-0");
    // No owner, a Job, a DaemonSet, and a ReplicaSet that is not a Deployment's.
    for controller in [None, Some(("Job", "once")), Some(("DaemonSet", "agent"))] {
        let pod = owned_pod(controller, &["pod-template-hash=6d9f7c"]);
        assert_eq!(pod_drawer_subject(&pod).target, by_name, "{controller:?}");
    }
    let bare = owned_pod(Some(("ReplicaSet", "standalone")), &[]);
    assert_eq!(pod_drawer_subject(&bare).target, by_name);
    // A hash that does not end the ReplicaSet name is not a Deployment's ReplicaSet either.
    let mismatch = owned_pod(
        Some(("ReplicaSet", "web-6d9f7c")),
        &["pod-template-hash=aaaa"],
    );
    assert_eq!(pod_drawer_subject(&mismatch).target, by_name);
}

#[test]
fn the_table_menu_keeps_the_pod_target() {
    let pod = owned_pod(
        Some(("ReplicaSet", "web-6d9f7c")),
        &["pod-template-hash=6d9f7c"],
    );
    assert_eq!(pod_subject(&pod).target, TargetSpec::pod("api-0"));
}
