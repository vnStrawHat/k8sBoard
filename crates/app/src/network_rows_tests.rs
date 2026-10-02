use cluster::{IngressPath, IngressTls};

use super::*;
use crate::resource_kind::ResourceKind;

fn service(service_type: &str) -> ServiceSummary {
    ServiceSummary {
        namespace: "team-a".to_owned(),
        name: "api".to_owned(),
        created_at: None,
        labels: Vec::new(),
        service_type: service_type.to_owned(),
        cluster_ips: vec!["10.0.0.5".to_owned()],
        is_headless: false,
        external_addresses: Vec::new(),
        ports: vec![ServicePortSummary {
            name: Some("http".to_owned()),
            port: 80,
            target_port: Some("8080".to_owned()),
            node_port: Some(30080),
            protocol: "TCP".to_owned(),
        }],
        selector: vec!["app=api".to_owned()],
    }
}

fn ingress() -> IngressSummary {
    IngressSummary {
        namespace: "team-a".to_owned(),
        name: "web".to_owned(),
        created_at: None,
        labels: Vec::new(),
        class: Some("nginx".to_owned()),
        hosts: vec!["a.example.com".to_owned(), "b.example.com".to_owned()],
        addresses: vec!["10.1.1.1".to_owned()],
        rules: vec![IngressPath {
            host: Some("a.example.com".to_owned()),
            path: Some("/api".to_owned()),
            backend: "api:80".to_owned(),
            service: Some("api".to_owned()),
        }],
        default_backend: None,
        default_service: None,
        tls: Vec::new(),
    }
}

fn tls(host: &str, secret: &str) -> IngressTls {
    IngressTls {
        hosts: vec![host.to_owned()],
        secret_name: Some(secret.to_owned()),
    }
}

fn toned(text: &str, tone: StatusTone) -> KindCell {
    KindCell::Toned(StatusLabel {
        text: text.to_owned().into(),
        tone,
    })
}

#[test]
fn network_row_cells_match_column_count() {
    assert_eq!(
        service_row(&service("ClusterIP")).cells.len(),
        ResourceKind::Services.columns().len()
    );
    assert_eq!(
        ingress_row(&ingress()).cells.len(),
        ResourceKind::Ingresses.columns().len()
    );
}

#[test]
fn service_row_shows_none_for_headless_and_pending_for_load_balancer() {
    let mut headless = service("ClusterIP");
    headless.is_headless = true;
    headless.cluster_ips.clear();
    assert_eq!(
        service_row(&headless).cells.get(1),
        Some(&toned("None", StatusTone::Done))
    );

    let balancer = service_row(&service("LoadBalancer"));
    assert_eq!(
        balancer.cells.get(2),
        Some(&toned("<pending>", StatusTone::Warn))
    );
    assert_eq!(balancer.status.text, "Address pending");
    assert_eq!(balancer.status.tone, StatusTone::Warn);
}

#[test]
fn service_with_an_address_is_ok_and_lists_it() {
    let mut balancer = service("LoadBalancer");
    balancer.external_addresses = vec!["203.0.113.7".to_owned()];
    let row = service_row(&balancer);
    assert_eq!(
        row.cells.get(2),
        Some(&KindCell::Mono("203.0.113.7".into()))
    );
    assert_eq!(row.status.tone, StatusTone::Ok);
    assert_eq!(row.status.text, "LoadBalancer");
}

#[test]
fn service_ports_cell_follows_kubectl_and_drawer_lists_targets() {
    let row = service_row(&service("NodePort"));
    assert_eq!(
        row.cells.get(3),
        Some(&KindCell::Mono("80:30080/TCP".into()))
    );
    let ports = row.section("Ports").expect("ports section");
    assert_eq!(
        ports.rows,
        [DetailRow::Port {
            text: "80 → 8080/TCP · http · node 30080".into()
        }]
    );
}

#[test]
fn service_without_selector_gets_a_manual_endpoints_note() {
    let mut selectorless = service("ClusterIP");
    selectorless.selector.clear();
    let row = service_row(&selectorless);
    let details = row.section("Service").expect("service section");
    assert!(
        details
            .rows
            .iter()
            .any(|row| matches!(row, DetailRow::Note(_)))
    );
    let with_selector = service_row(&service("ClusterIP"));
    let details = with_selector.section("Service").expect("service section");
    assert!(
        !details
            .rows
            .iter()
            .any(|row| matches!(row, DetailRow::Note(_)))
    );
}

#[test]
fn service_has_no_related_pods() {
    assert_eq!(service_row(&service("ClusterIP")).related_pods, None);
}

#[test]
fn ingress_row_hosts_star_when_empty_and_tls_ports() {
    let mut open = ingress();
    open.hosts.clear();
    assert_eq!(
        ingress_row(&open).cells.get(1),
        Some(&KindCell::Text("*".into()))
    );
    assert_eq!(
        ingress_row(&open).cells.get(3),
        Some(&KindCell::Text("80".into()))
    );
    open.tls = vec![tls("a.example.com", "a-tls")];
    assert_eq!(
        ingress_row(&open).cells.get(3),
        Some(&KindCell::Text("80, 443".into()))
    );
}

#[test]
fn ingress_tls_section_names_hosts_and_secrets() {
    let mut secured = ingress();
    secured.tls = vec![tls("a.example.com", "a-tls")];
    let row = ingress_row(&secured);
    let tls = row.section("TLS").expect("tls section");
    assert_eq!(
        tls.rows,
        [DetailRow::stacked(
            "a.example.com",
            KindCell::Text("secret a-tls".into())
        )]
    );
    assert!(ingress_row(&ingress()).section("TLS").is_none());
}

#[test]
fn ingress_status_follows_the_address() {
    assert_eq!(ingress_row(&ingress()).status.text, "Address assigned");
    let mut pending = ingress();
    pending.addresses.clear();
    let status = ingress_row(&pending).status;
    assert_eq!(status.text, "No address yet");
    assert_eq!(status.tone, StatusTone::Warn);
}

#[test]
fn ingress_rules_label_host_and_path_with_star_for_missing_host() {
    let mut catch_all = ingress();
    catch_all.rules.push(IngressPath {
        host: None,
        path: None,
        backend: "fallback:80".to_owned(),
        service: Some("fallback".to_owned()),
    });
    let row = ingress_row(&catch_all);
    let rules = row.section("Rules").expect("rules section");
    assert_eq!(
        rules.rows,
        [
            service_link_row("a.example.com/api", "api:80", "api"),
            service_link_row("*", "fallback:80", "fallback"),
        ]
    );
}

fn port(name: Option<&str>, target: Option<&str>, node_port: Option<u16>) -> ServicePortSummary {
    ServicePortSummary {
        name: name.map(str::to_owned),
        port: 80,
        target_port: target.map(str::to_owned),
        node_port,
        protocol: "TCP".to_owned(),
    }
}

#[test]
fn port_text_without_target_name_or_node_port_is_just_the_port() {
    assert_eq!(port_text(&port(None, None, None)), "80/TCP");
}

#[test]
fn port_text_adds_target_name_and_node_port_in_that_order() {
    assert_eq!(port_text(&port(None, Some("8080"), None)), "80 → 8080/TCP");
    assert_eq!(port_text(&port(Some("http"), None, None)), "80/TCP · http");
    assert_eq!(
        port_text(&port(None, None, Some(30080))),
        "80/TCP · node 30080"
    );
    assert_eq!(
        port_text(&port(Some("http"), Some("web"), Some(30080))),
        "80 → web/TCP · http · node 30080"
    );
}

#[test]
fn ingress_addresses_join_with_a_comma_like_hosts() {
    let mut balanced = ingress();
    balanced.addresses = vec!["10.1.1.1".to_owned(), "10.1.1.2".to_owned()];
    assert_eq!(
        ingress_row(&balanced).cells.get(2),
        Some(&KindCell::Mono("10.1.1.1,10.1.1.2".into()))
    );
}

#[test]
fn service_row_has_endpoints_placeholder() {
    let service = service("ClusterIP");
    let row = service_row(&service);
    assert_eq!(
        row.cells.get(crate::kind_join::SERVICE_ENDPOINTS),
        Some(&KindCell::Absent)
    );
    let endpoints = row.section("Endpoints").expect("an Endpoints section");
    assert_eq!(endpoints.rows, [DetailRow::Live(LiveContent::Endpoints)]);
}

#[test]
fn service_sections_end_with_endpoints() {
    let row = service_row(&service("ClusterIP"));
    let titles: Vec<&str> = row.sections.iter().map(|section| section.title).collect();
    assert_eq!(titles, ["Service", "Ports", "Selector", "Endpoints"]);
}

#[test]
fn service_row_keeps_builder_status_and_object_until_joined() {
    let service = service("ClusterIP");
    let row = service_row(&service);
    assert_eq!(row.status.text, "ClusterIP");
    assert_eq!(row.object, KindObject::Service(service));
}

fn service_target(name: &str) -> ResourceKey {
    ResourceKey::of_object("Service", Some("team-a"), name).expect("a Service key")
}

fn service_link_row(label: &str, text: &str, service: &str) -> DetailRow {
    DetailRow::StackedLink {
        label: label.to_owned().into(),
        text: text.to_owned().into(),
        target: service_target(service),
    }
}

#[test]
fn ingress_backend_is_a_link() {
    let mut routed = ingress();
    routed.default_backend = Some("fallback:80".to_owned());
    routed.default_service = Some("fallback".to_owned());
    // A resource backend names no Service, so it stays plain text.
    routed.rules.push(IngressPath {
        host: Some("b.example.com".to_owned()),
        path: None,
        backend: "StorageBucket/assets".to_owned(),
        service: None,
    });
    let row = ingress_row(&routed);
    let rules = row.section("Rules").expect("rules section");
    assert_eq!(
        rules.rows,
        [
            service_link_row("a.example.com/api", "api:80", "api"),
            DetailRow::stacked(
                "b.example.com",
                KindCell::Mono("StorageBucket/assets".into())
            ),
        ]
    );
    let ingress_section = row.section("Ingress").expect("ingress section");
    assert!(ingress_section.rows.contains(&DetailRow::Link {
        label: "Default backend".into(),
        text: "fallback:80".into(),
        target: service_target("fallback"),
    }));
}

fn rule(host: Option<&str>, path: Option<&str>) -> IngressPath {
    IngressPath {
        host: host.map(str::to_owned),
        path: path.map(str::to_owned),
        backend: "api:80".to_owned(),
        service: Some("api".to_owned()),
    }
}

fn urls_of(rules: Vec<IngressPath>, tls: Vec<IngressTls>) -> Vec<String> {
    let mut summary = ingress();
    summary.rules = rules;
    summary.tls = tls;
    ingress_urls(&summary)
}

#[test]
fn ingress_urls_https_for_tls_and_wildcard() {
    let tls = vec![
        tls("secure.example.com", "a"),
        tls("*.apps.example.com", "b"),
    ];
    let rules = vec![
        rule(Some("secure.example.com"), Some("/")),
        rule(Some("web.apps.example.com"), Some("/ui")),
        rule(Some("plain.example.com"), None),
        // A wildcard covers one label only.
        rule(Some("a.b.apps.example.com"), None),
    ];
    assert_eq!(
        urls_of(rules, tls),
        [
            "https://secure.example.com/",
            "https://web.apps.example.com/ui",
            "http://plain.example.com/",
            "http://a.b.apps.example.com/",
        ]
    );
}

#[test]
fn ingress_urls_skip_wildcard_hosts_and_regex_paths() {
    let rules = vec![
        rule(Some("*.example.com"), Some("/")),
        rule(None, Some("/")),
        rule(Some("bad host.example.com"), Some("/")),
        rule(Some("api.example.com"), Some("/v1(/|$)(.*)")),
    ];
    assert_eq!(urls_of(rules, Vec::new()), ["http://api.example.com/"]);
}

#[test]
fn ingress_urls_repeat_nothing() {
    let rules = vec![
        rule(Some("a.example.com"), Some("/x")),
        rule(Some("a.example.com"), Some("/x")),
    ];
    assert_eq!(urls_of(rules, Vec::new()), ["http://a.example.com/x"]);
}
