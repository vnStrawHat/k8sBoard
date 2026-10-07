use cluster::{PodCondition, PodStatus, ReadyCount, ServicePortSummary, StatusReason};

use super::*;

fn ingress(rules: &[(&str, &str, &str)]) -> IngressSummary {
    IngressSummary {
        namespace: "shop".to_owned(),
        name: "shop".to_owned(),
        created_at: None,
        labels: Vec::new(),
        class: None,
        hosts: Vec::new(),
        addresses: Vec::new(),
        rules: rules
            .iter()
            .map(|(path, service, backend)| IngressPath {
                host: None,
                path: Some((*path).to_owned()),
                backend: (*backend).to_owned(),
                service: Some((*service).to_owned()),
            })
            .collect(),
        default_backend: None,
        default_service: None,
        tls: Vec::new(),
    }
}

fn service(name: &str, ports: &[(Option<&str>, u16)], selector: &[&str]) -> ServiceSummary {
    ServiceSummary {
        namespace: "shop".to_owned(),
        name: name.to_owned(),
        created_at: None,
        labels: Vec::new(),
        service_type: "ClusterIP".to_owned(),
        cluster_ips: Vec::new(),
        is_headless: false,
        external_addresses: Vec::new(),
        ports: ports
            .iter()
            .map(|(name, port)| ServicePortSummary {
                name: name.map(str::to_owned),
                port: *port,
                target_port: None,
                node_port: None,
                protocol: "TCP".to_owned(),
            })
            .collect(),
        selector: selector.iter().map(|term| (*term).to_owned()).collect(),
    }
}

fn pod(label: &str, is_ready: bool) -> PodSummary {
    PodSummary {
        annotations: cluster::AnnotationTerms::default(),
        namespace: "shop".to_owned(),
        name: format!("{label}-0"),
        status: PodStatus::Reason(StatusReason::Running),
        ready: ReadyCount { ready: 0, total: 1 },
        restarts: 0,
        node_name: None,
        created_at: None,
        pod_ip: None,
        qos_class: None,
        service_account: None,
        controller: None,
        conditions: vec![PodCondition {
            name: "Ready".to_owned(),
            is_true: is_ready,
            reason: None,
            message: None,
            changed_at: None,
        }],
        containers: Vec::new(),
        status_message: None,
        labels: vec![label.to_owned()],
        host_network: false,
        image_pull_secrets: Vec::new(),
        node_selector: Vec::new(),
        node_affinity: Vec::new(),
        is_finished: false,
    }
}

fn problems(
    ingress: &IngressSummary,
    services: &[ServiceSummary],
    pods: &[PodSummary],
) -> Vec<String> {
    backend_problems(ingress, &IngressBackends { services, pods }).0
}

#[test]
fn a_missing_service_is_named() {
    assert_eq!(
        problems(&ingress(&[("/v2", "web-v2", "web-v2:80")]), &[], &[]),
        ["Rule /v2 → web-v2:80: Service web-v2 does not exist in shop"]
    );
}

#[test]
fn a_port_the_service_does_not_expose_lists_the_ports_it_has() {
    let services = [service(
        "web-v2",
        &[(None, 80), (Some("metrics"), 9090)],
        &["app=web"],
    )];
    assert_eq!(
        problems(
            &ingress(&[("/v2", "web-v2", "web-v2:9999")]),
            &services,
            &[pod("app=web", true)]
        ),
        ["Rule /v2 → web-v2:9999: Service web-v2 has no port 9999 (has 80, 9090 (metrics))"]
    );
}

#[test]
fn a_named_port_matches_the_service_port_name() {
    let services = [service("web", &[(Some("http"), 80)], &["app=web"])];
    let pods = [pod("app=web", true)];
    assert!(problems(&ingress(&[("/", "web", "web:http")]), &services, &pods).is_empty());
    assert_eq!(
        problems(&ingress(&[("/", "web", "web:grpc")]), &services, &pods),
        ["Rule / → web:grpc: Service web has no port grpc (has 80 (http))"]
    );
}

#[test]
fn a_service_without_a_ready_pod_has_no_ready_endpoints() {
    let services = [service("web", &[(None, 80)], &["app=web"])];
    let rule = ingress(&[("/", "web", "web:80")]);
    assert_eq!(
        problems(&rule, &services, &[]),
        ["Rule / → web:80: Service web has 0 ready endpoints (no pod matches app=web)"]
    );
    assert_eq!(
        problems(
            &rule,
            &services,
            &[pod("app=web", false), pod("app=web", false)]
        ),
        ["Rule / → web:80: Service web has 0 ready endpoints (2 pods match, none ready)"]
    );
    assert!(
        problems(
            &rule,
            &services,
            &[pod("app=web", false), pod("app=web", true)]
        )
        .is_empty()
    );
}

#[test]
fn a_service_without_a_selector_or_an_external_name_is_not_checked_for_pods() {
    let mut external = service("web", &[(None, 80)], &["app=web"]);
    external.service_type = "ExternalName".to_owned();
    let rule = ingress(&[("/", "web", "web:80")]);
    assert!(problems(&rule, &[external], &[]).is_empty());
    assert!(problems(&rule, &[service("web", &[(None, 80)], &[])], &[]).is_empty());
}

#[test]
fn the_default_backend_and_the_host_are_named_and_the_service_link_is_the_first_problem() {
    let mut rule = ingress(&[("/ok", "web", "web:80"), ("/x", "web", "web:81")]);
    rule.rules[1].host = Some("shop.lab".to_owned());
    rule.default_service = Some("ghost".to_owned());
    rule.default_backend = Some("ghost:80".to_owned());
    let services = [service("web", &[(None, 80)], &["app=web"])];
    let pods = [pod("app=web", true)];
    let (lines, link) = backend_problems(
        &rule,
        &IngressBackends {
            services: &services,
            pods: &pods,
        },
    );
    assert_eq!(
        lines,
        [
            "Rule shop.lab/x → web:81: Service web has no port 81 (has 80)",
            "Default backend → ghost:80: Service ghost does not exist in shop"
        ]
    );
    assert_eq!(link.map(|service| service.name.as_str()), Some("web"));
}

#[test]
fn a_resource_backend_has_no_service_to_check() {
    let mut rule = ingress(&[]);
    rule.rules.push(IngressPath {
        host: None,
        path: Some("/".to_owned()),
        backend: "StorageBucket/assets".to_owned(),
        service: None,
    });
    assert!(problems(&rule, &[], &[]).is_empty());
}
