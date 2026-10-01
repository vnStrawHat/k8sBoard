use k8s_openapi::api::core::v1::{LoadBalancerStatus, ServiceSpec, ServiceStatus};
use k8s_openapi::apimachinery::pkg::util::intstr::IntOrString;

use super::*;

fn service_with_spec(spec: ServiceSpec) -> Service {
    Service {
        spec: Some(spec),
        ..Default::default()
    }
}

fn summary_port(port: u16, node_port: Option<u16>, protocol: &str) -> ServicePortSummary {
    ServicePortSummary {
        name: None,
        port,
        target_port: None,
        node_port,
        protocol: protocol.to_owned(),
    }
}

#[test]
fn service_port_display_matches_kubectl() {
    assert_eq!(summary_port(80, None, "TCP").to_string(), "80/TCP");
    assert_eq!(
        summary_port(80, Some(30080), "TCP").to_string(),
        "80:30080/TCP"
    );
    assert_eq!(summary_port(53, None, "UDP").to_string(), "53/UDP");
}

#[test]
fn service_summary_reads_type_ports_and_selector() {
    let service = service_with_spec(ServiceSpec {
        cluster_ip: Some("10.0.0.7".to_owned()),
        ports: Some(vec![
            ServicePort {
                name: Some("http".to_owned()),
                port: 80,
                target_port: Some(IntOrString::String("web".to_owned())),
                node_port: Some(30080),
                ..Default::default()
            },
            ServicePort {
                port: 70_000,
                ..Default::default()
            },
        ]),
        selector: Some([("app".to_owned(), "api".to_owned())].into_iter().collect()),
        ..Default::default()
    });
    let summary = service_summary(&service);
    assert_eq!(summary.service_type, "ClusterIP");
    assert_eq!(summary.cluster_ips, ["10.0.0.7"]);
    assert!(!summary.is_headless);
    assert_eq!(summary.selector, ["app=api"]);
    assert_eq!(
        summary.ports,
        [ServicePortSummary {
            name: Some("http".to_owned()),
            port: 80,
            target_port: Some("web".to_owned()),
            node_port: Some(30080),
            protocol: "TCP".to_owned(),
        }]
    );
}

#[test]
fn cluster_ips_prefer_the_list_over_the_single_ip() {
    let service = service_with_spec(ServiceSpec {
        cluster_ip: Some("10.0.0.7".to_owned()),
        cluster_ips: Some(vec!["10.0.0.7".to_owned(), "fd00::7".to_owned()]),
        ..Default::default()
    });
    assert_eq!(
        service_summary(&service).cluster_ips,
        ["10.0.0.7", "fd00::7"]
    );
}

#[test]
fn headless_service_has_no_cluster_ips() {
    let service = service_with_spec(ServiceSpec {
        cluster_ip: Some("None".to_owned()),
        cluster_ips: Some(vec!["None".to_owned()]),
        ..Default::default()
    });
    let summary = service_summary(&service);
    assert!(summary.is_headless);
    assert!(summary.cluster_ips.is_empty());
}

#[test]
fn load_balancer_addresses_precede_external_ips() {
    let ingress = |ip: Option<&str>, hostname: Option<&str>| LoadBalancerIngress {
        ip: ip.map(str::to_owned),
        hostname: hostname.map(str::to_owned),
        ..Default::default()
    };
    let service = Service {
        spec: Some(ServiceSpec {
            type_: Some("LoadBalancer".to_owned()),
            external_ips: Some(vec!["203.0.113.9".to_owned()]),
            ..Default::default()
        }),
        status: Some(ServiceStatus {
            load_balancer: Some(LoadBalancerStatus {
                ingress: Some(vec![
                    ingress(Some("198.51.100.4"), Some("ignored.example")),
                    ingress(None, Some("lb.example")),
                ]),
            }),
            ..Default::default()
        }),
        ..Default::default()
    };
    assert_eq!(
        service_summary(&service).external_addresses,
        ["198.51.100.4", "lb.example", "203.0.113.9"]
    );
}

#[test]
fn external_name_service_reports_its_name() {
    let service = service_with_spec(ServiceSpec {
        type_: Some("ExternalName".to_owned()),
        external_name: Some("db.example.com".to_owned()),
        cluster_ip: Some(String::new()),
        ..Default::default()
    });
    let summary = service_summary(&service);
    assert_eq!(summary.external_addresses, ["db.example.com"]);
    assert!(summary.cluster_ips.is_empty());

    let cluster_ip = service_with_spec(ServiceSpec {
        external_name: Some("ignored.example.com".to_owned()),
        ..Default::default()
    });
    assert!(service_summary(&cluster_ip).external_addresses.is_empty());
}
