use std::collections::BTreeMap;

use k8s_openapi::api::core::v1::TypedLocalObjectReference;
use k8s_openapi::api::networking::v1::{
    HTTPIngressPath, HTTPIngressRuleValue, IngressLoadBalancerIngress, IngressLoadBalancerStatus,
    IngressRule, IngressServiceBackend, IngressSpec, IngressStatus, IngressTLS, ServiceBackendPort,
};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;

use super::*;

fn service_backend(name: &str, number: Option<i32>, port_name: Option<&str>) -> IngressBackend {
    IngressBackend {
        service: Some(IngressServiceBackend {
            name: name.to_owned(),
            port: Some(ServiceBackendPort {
                number,
                name: port_name.map(str::to_owned),
            }),
        }),
        ..Default::default()
    }
}

fn rule(host: Option<&str>, paths: Vec<HTTPIngressPath>) -> IngressRule {
    IngressRule {
        host: host.map(str::to_owned),
        http: Some(HTTPIngressRuleValue { paths }),
    }
}

fn path(path: Option<&str>, backend: IngressBackend) -> HTTPIngressPath {
    HTTPIngressPath {
        backend,
        path: path.map(str::to_owned),
        path_type: "Prefix".to_owned(),
    }
}

fn ingress_with_rules(rules: Vec<IngressRule>) -> Ingress {
    Ingress {
        spec: Some(IngressSpec {
            rules: Some(rules),
            ..Default::default()
        }),
        ..Default::default()
    }
}

#[test]
fn ingress_class_prefers_spec_over_annotation() {
    let annotations = BTreeMap::from([(CLASS_ANNOTATION.to_owned(), "legacy".to_owned())]);
    let mut ingress = Ingress {
        metadata: ObjectMeta {
            annotations: Some(annotations),
            ..Default::default()
        },
        ..Default::default()
    };
    assert_eq!(ingress_summary(&ingress).class.as_deref(), Some("legacy"));

    ingress.spec = Some(IngressSpec {
        ingress_class_name: Some("nginx".to_owned()),
        ..Default::default()
    });
    assert_eq!(ingress_summary(&ingress).class.as_deref(), Some("nginx"));
    assert_eq!(ingress_summary(&Ingress::default()).class, None);
}

#[test]
fn ingress_paths_flatten_host_path_backend() {
    let resource = IngressBackend {
        resource: Some(TypedLocalObjectReference {
            kind: "StorageBucket".to_owned(),
            name: "assets".to_owned(),
            ..Default::default()
        }),
        ..Default::default()
    };
    let mut ingress = ingress_with_rules(vec![
        rule(
            Some("shop.example.com"),
            vec![
                path(Some("/api"), service_backend("api", Some(8080), None)),
                path(Some("/static"), resource),
            ],
        ),
        rule(
            None,
            vec![path(None, service_backend("web", None, Some("http")))],
        ),
    ]);
    if let Some(spec) = ingress.spec.as_mut() {
        spec.default_backend = Some(service_backend("fallback", Some(80), None));
    }
    let summary = ingress_summary(&ingress);
    assert_eq!(
        summary.rules,
        [
            IngressPath {
                host: Some("shop.example.com".to_owned()),
                path: Some("/api".to_owned()),
                backend: "api:8080".to_owned(),
            },
            IngressPath {
                host: Some("shop.example.com".to_owned()),
                path: Some("/static".to_owned()),
                backend: "StorageBucket/assets".to_owned(),
            },
            IngressPath {
                host: None,
                path: None,
                backend: "web:http".to_owned(),
            },
        ]
    );
    assert_eq!(summary.default_backend.as_deref(), Some("fallback:80"));
}

#[test]
fn ingress_hosts_are_deduplicated_in_order() {
    let ingress = ingress_with_rules(vec![
        rule(Some("b.example.com"), Vec::new()),
        rule(None, Vec::new()),
        rule(Some("a.example.com"), Vec::new()),
        rule(Some("b.example.com"), Vec::new()),
    ]);
    assert_eq!(
        ingress_summary(&ingress).hosts,
        ["b.example.com", "a.example.com"]
    );
}

#[test]
fn ingress_reads_addresses_and_tls_names() {
    let ingress = Ingress {
        spec: Some(IngressSpec {
            tls: Some(vec![IngressTLS {
                hosts: Some(vec!["shop.example.com".to_owned()]),
                secret_name: Some("shop-tls".to_owned()),
            }]),
            ..Default::default()
        }),
        status: Some(IngressStatus {
            load_balancer: Some(IngressLoadBalancerStatus {
                ingress: Some(vec![
                    IngressLoadBalancerIngress {
                        ip: Some("198.51.100.4".to_owned()),
                        ..Default::default()
                    },
                    IngressLoadBalancerIngress {
                        hostname: Some("lb.example".to_owned()),
                        ..Default::default()
                    },
                ]),
            }),
        }),
        ..Default::default()
    };
    let summary = ingress_summary(&ingress);
    assert_eq!(summary.addresses, ["198.51.100.4", "lb.example"]);
    assert_eq!(
        summary.tls,
        [IngressTls {
            hosts: vec!["shop.example.com".to_owned()],
            secret_name: Some("shop-tls".to_owned()),
        }]
    );
}
