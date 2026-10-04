use super::*;

fn fields() -> MetricsSourceFields {
    MetricsSourceFields {
        namespace: "monitoring".to_owned(),
        service: "vmselect-vm-victoria-metrics-k8s-stack".to_owned(),
        port: "8481".to_owned(),
        scheme: MetricsScheme::Http,
        prefix: "/select/0/prometheus".to_owned(),
    }
}

fn with(
    change: impl FnOnce(&mut MetricsSourceFields),
) -> Result<MetricsSource, MetricsSourceError> {
    let mut fields = fields();
    change(&mut fields);
    MetricsSource::new(&fields)
}

fn port(name: Option<&str>, number: u16) -> ServicePortSummary {
    ServicePortSummary {
        name: name.map(str::to_owned),
        port: number,
        target_port: None,
        node_port: None,
        protocol: "TCP".to_owned(),
    }
}

fn service(
    namespace: &str,
    name: &str,
    labels: &[&str],
    ports: Vec<ServicePortSummary>,
) -> ServiceSummary {
    ServiceSummary {
        namespace: namespace.to_owned(),
        name: name.to_owned(),
        created_at: None,
        labels: labels.iter().map(|label| (*label).to_owned()).collect(),
        service_type: "ClusterIP".to_owned(),
        cluster_ips: Vec::new(),
        is_headless: false,
        external_addresses: Vec::new(),
        ports,
        selector: Vec::new(),
    }
}

#[test]
fn source_validation_per_field() {
    assert!(MetricsSource::new(&fields()).is_ok());
    assert_eq!(
        with(|f| f.namespace = "Monitoring".to_owned()).unwrap_err(),
        MetricsSourceError::Namespace
    );
    assert_eq!(
        with(|f| f.namespace = String::new()).unwrap_err(),
        MetricsSourceError::Namespace
    );
    assert_eq!(
        with(|f| f.service = "a.b".to_owned()).unwrap_err(),
        MetricsSourceError::Service
    );
    assert_eq!(
        with(|f| f.service = "a".repeat(64)).unwrap_err(),
        MetricsSourceError::Service
    );
    assert!(with(|f| f.service = "a".repeat(63)).is_ok());
    for valid in [
        "1",
        "80",
        "65535",
        "http",
        "http-metrics",
        "a",
        "web2",
        "9p",
    ] {
        assert!(with(|f| f.port = valid.to_owned()).is_ok(), "{valid}");
    }
    for invalid in [
        "",
        "0",
        "65536",
        "08481",
        "-1",
        "http-",
        "-http",
        "ht--tp",
        "HTTP",
        "a_b",
        "a/b",
        "1234567890123456",
        "123a456789012345",
        "80 ",
        "8 0",
    ] {
        assert_eq!(
            with(|f| f.port = invalid.to_owned()).unwrap_err(),
            MetricsSourceError::Port,
            "{invalid:?}"
        );
    }
    assert!(with(|f| f.prefix = String::new()).is_ok());
    assert_eq!(
        with(|f| f.prefix = format!("/{}", "a".repeat(128))).unwrap_err(),
        MetricsSourceError::Prefix
    );
}

#[test]
fn prefix_rejects_dot_segments_and_query_chars() {
    for invalid in [
        "/a/../b", "/..", "/.", "/a/./b", "/a?x", "/a#", "/a%2f", "/a/", "select/0", "/a//b", "/",
        "//", "/a b", "/a\\b", "/a:b", "/é",
    ] {
        assert_eq!(
            with(|f| f.prefix = invalid.to_owned()).unwrap_err(),
            MetricsSourceError::Prefix,
            "{invalid:?}"
        );
    }
    for valid in [
        "/select/0/prometheus",
        "/a.b/c_d/e~f-g",
        "/prometheus",
        "/..a",
    ] {
        assert!(with(|f| f.prefix = valid.to_owned()).is_ok(), "{valid:?}");
    }
}

#[test]
fn source_fields_round_trip_json() {
    let json = r#"{"namespace":"monitoring","service":"vmselect-vm-victoria-metrics-k8s-stack","port":"8481","scheme":"http","prefix":"/select/0/prometheus"}"#;
    let parsed: MetricsSourceFields = serde_json::from_str(json).expect("parses");
    assert_eq!(parsed, fields());
    assert_eq!(serde_json::to_string(&parsed).expect("serializes"), json);
    let https: MetricsSourceFields =
        serde_json::from_str(r#"{"namespace":"a","service":"b","port":"web","scheme":"https"}"#)
            .expect("a missing prefix is allowed");
    assert_eq!(https.scheme, MetricsScheme::Https);
    assert_eq!(https.prefix, "");
}

#[test]
fn source_debug_is_display() {
    let source = MetricsSource::new(&fields()).expect("valid");
    let expected = "monitoring/vmselect-vm-victoria-metrics-k8s-stack:8481 /select/0/prometheus";
    assert_eq!(source.display(), expected);
    assert_eq!(format!("{source:?}"), expected);
    let bare = with(|f| f.prefix = String::new()).expect("valid");
    assert_eq!(
        bare.display(),
        "monitoring/vmselect-vm-victoria-metrics-k8s-stack:8481"
    );
    assert_eq!(source.fields(), fields());
}

#[test]
fn proxy_path_names_the_service_and_prefix() {
    let source = MetricsSource::new(&fields()).expect("valid");
    assert_eq!(
        source.proxy_path(),
        "/api/v1/namespaces/monitoring/services/http:vmselect-vm-victoria-metrics-k8s-stack:8481/proxy/select/0/prometheus"
    );
    let https = with(|f| {
        f.scheme = MetricsScheme::Https;
        f.port = "web".to_owned();
        f.prefix = String::new();
    })
    .expect("valid");
    assert_eq!(
        https.proxy_path(),
        "/api/v1/namespaces/monitoring/services/https:vmselect-vm-victoria-metrics-k8s-stack:web/proxy"
    );
}

fn uat_services() -> Vec<ServiceSummary> {
    let http = |number| vec![port(Some("http"), number)];
    let labels = |name: &str| format!("app.kubernetes.io/name={name}");
    vec![
        service(
            "monitoring",
            "vm-prometheus-node-exporter",
            &[&labels("prometheus-node-exporter")],
            http(9101),
        ),
        service(
            "monitoring",
            "vmquery-vm-victoria-metrics-k8s-stack",
            &[&labels("vmquery")],
            http(8481),
        ),
        service(
            "monitoring",
            "vminsert-vm-victoria-metrics-k8s-stack",
            &[&labels("vminsert")],
            http(8480),
        ),
        service(
            "monitoring",
            "vmstorage-vm-victoria-metrics-k8s-stack",
            &[&labels("vmstorage")],
            http(8482),
        ),
        service(
            "monitoring",
            "vmselect-vm-victoria-metrics-k8s-stack",
            &[&labels("vmselect")],
            http(8481),
        ),
        service("monitoring", "vm-grafana", &[&labels("grafana")], http(80)),
        service(
            "monitoring",
            "vm-kube-state-metrics",
            &[&labels("kube-state-metrics")],
            http(8080),
        ),
    ]
}

#[test]
fn candidates_rank_vmselect_before_vmquery() {
    let candidates = metrics_candidates(&uat_services());
    let found: Vec<(MetricsFlavor, &str, &str, &str)> = candidates
        .iter()
        .map(|c| {
            (
                c.flavor,
                c.fields.service.as_str(),
                c.fields.port.as_str(),
                c.fields.prefix.as_str(),
            )
        })
        .collect();
    assert_eq!(
        found,
        [
            (
                MetricsFlavor::VictoriaMetricsCluster,
                "vmselect-vm-victoria-metrics-k8s-stack",
                "8481",
                "/select/0/prometheus"
            ),
            (
                MetricsFlavor::VictoriaMetricsQuery,
                "vmquery-vm-victoria-metrics-k8s-stack",
                "8481",
                "/select/0/prometheus"
            ),
        ]
    );
}

#[test]
fn candidates_per_flavor() {
    let named = |name: &str, port_name: &str, number: u16, label: &str| {
        service("obs", name, &[label], vec![port(Some(port_name), number)])
    };
    let services = vec![
        named("single", "http", 8428, "app.kubernetes.io/name=vmsingle"),
        named("alt-single", "http", 8428, "app=victoria-metrics-single"),
        named("prom", "web", 9090, "app.kubernetes.io/name=prometheus"),
        named("prometheus-k8s", "web", 9090, "other=x"),
        named(
            "thanos",
            "http",
            10902,
            "app.kubernetes.io/name=thanos-query",
        ),
        named("querier", "http", 9090, "app=thanos-querier"),
        named(
            "mimir-qf",
            "http-metrics",
            8080,
            "app.kubernetes.io/name=mimir",
        ),
        named(
            "secure",
            "https-web",
            9091,
            "app.kubernetes.io/name=prometheus",
        ),
    ];
    let mut with_component = services;
    with_component[6]
        .labels
        .push("app.kubernetes.io/component=query-frontend".to_owned());
    let candidates = metrics_candidates(&with_component);
    let summary: Vec<(MetricsFlavor, &str, &str, MetricsScheme, &str)> = candidates
        .iter()
        .map(|c| {
            (
                c.flavor,
                c.fields.service.as_str(),
                c.fields.port.as_str(),
                c.fields.scheme,
                c.fields.prefix.as_str(),
            )
        })
        .collect();
    assert_eq!(
        summary,
        [
            (
                MetricsFlavor::VictoriaMetricsSingle,
                "alt-single",
                "8428",
                MetricsScheme::Http,
                ""
            ),
            (
                MetricsFlavor::VictoriaMetricsSingle,
                "single",
                "8428",
                MetricsScheme::Http,
                ""
            ),
            (
                MetricsFlavor::Prometheus,
                "prom",
                "9090",
                MetricsScheme::Http,
                ""
            ),
            (
                MetricsFlavor::Prometheus,
                "prometheus-k8s",
                "9090",
                MetricsScheme::Http,
                ""
            ),
            (
                MetricsFlavor::ThanosQuery,
                "querier",
                "9090",
                MetricsScheme::Http,
                ""
            ),
            (
                MetricsFlavor::ThanosQuery,
                "thanos",
                "10902",
                MetricsScheme::Http,
                ""
            ),
            (
                MetricsFlavor::Mimir,
                "mimir-qf",
                "8080",
                MetricsScheme::Http,
                "/prometheus"
            ),
        ],
        "the https-web service has no candidate port and is skipped"
    );
}

#[test]
fn mimir_needs_the_query_frontend_component() {
    let ports = vec![port(Some("http-metrics"), 8080)];
    let distributor = service("obs", "mimir-d", &["app.kubernetes.io/name=mimir"], ports);
    assert!(metrics_candidates(&[distributor]).is_empty());
}

#[test]
fn https_scheme_follows_the_port_name() {
    let services = [service(
        "obs",
        "prom",
        &["app.kubernetes.io/name=prometheus"],
        vec![port(Some("web"), 9090), port(Some("https-web"), 9091)],
    )];
    assert_eq!(
        metrics_candidates(&services)[0].fields.scheme,
        MetricsScheme::Http
    );
    // `https` is taken from the chosen port's own name.
    let secure = Detection {
        flavor: MetricsFlavor::Prometheus,
        prefix: "",
        port_names: &["https-web"],
        port_numbers: &[],
    };
    let candidate = candidate_of(&secure, &services[0]).expect("port exists");
    assert_eq!(candidate.fields.scheme, MetricsScheme::Https);
    assert_eq!(candidate.fields.port, "9091");
}

#[test]
fn candidate_port_prefers_names_then_number() {
    let label = ["app.kubernetes.io/name=vmselect"];
    let named_elsewhere = service(
        "obs",
        "a",
        &label,
        vec![port(Some("other"), 8481), port(Some("http"), 9999)],
    );
    let by_number = service("obs", "b", &label, vec![port(Some("metrics"), 8481)]);
    let none = service("obs", "c", &label, vec![port(Some("metrics"), 7000)]);
    let candidates = metrics_candidates(&[named_elsewhere, by_number, none]);
    let ports: Vec<(&str, &str)> = candidates
        .iter()
        .map(|c| (c.fields.service.as_str(), c.fields.port.as_str()))
        .collect();
    assert_eq!(ports, [("a", "9999"), ("b", "8481")]);
}

#[test]
fn candidates_cap_at_twenty() {
    let services: Vec<ServiceSummary> = (0..25)
        .map(|index| {
            service(
                "obs",
                &format!("vmselect-{index:02}"),
                &["app.kubernetes.io/name=vmselect"],
                vec![port(Some("http"), 8481)],
            )
        })
        .collect();
    let candidates = metrics_candidates(&services);
    assert_eq!(candidates.len(), 20);
    assert_eq!(candidates[0].fields.service, "vmselect-00");
    assert_eq!(candidates[19].fields.service, "vmselect-19");
}

#[test]
fn app_label_is_the_fallback_for_the_name_label() {
    let both = service(
        "obs",
        "x",
        &["app=vmquery", "app.kubernetes.io/name=vmselect"],
        vec![port(Some("http"), 8481)],
    );
    let only_app = service("obs", "y", &["app=vmquery"], vec![port(Some("http"), 8481)]);
    let flavors: Vec<MetricsFlavor> = metrics_candidates(&[both, only_app])
        .iter()
        .map(|c| c.flavor)
        .collect();
    assert_eq!(
        flavors,
        [
            MetricsFlavor::VictoriaMetricsCluster,
            MetricsFlavor::VictoriaMetricsQuery
        ]
    );
}

#[test]
fn flavor_labels() {
    assert_eq!(
        MetricsFlavor::VictoriaMetricsCluster.label(),
        "VictoriaMetrics cluster"
    );
    assert_eq!(MetricsFlavor::ThanosQuery.label(), "Thanos Query");
}
