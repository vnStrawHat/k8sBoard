use cluster::{
    ContainerProbes, ContainerState, ContainerSummary, PodStatus, PolicyPeer, PolicyPort,
    PolicyRule, ReadyCount, RuleRef, Selector, StatusReason,
};

use super::*;

fn terms(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| (*item).to_owned()).collect()
}

fn container(kind: ContainerKind, ports: &[(Option<&str>, u16)]) -> ContainerSummary {
    ContainerSummary {
        terminal: cluster::ContainerTerminal::None,
        name: "c".to_owned(),
        image: "img".to_owned(),
        kind,
        state: ContainerState::NotReported,
        is_ready: false,
        restart_count: 0,
        last_termination: None,
        image_digest: None,
        pull_policy: None,
        is_started: None,
        ports: ports
            .iter()
            .map(|(name, port)| ContainerPort {
                name: name.map(str::to_owned),
                port: *port,
                protocol: "TCP".to_owned(),
                host_port: None,
            })
            .collect(),
        resources: Vec::new(),
        probes: ContainerProbes::default(),
        env: Vec::new(),
        env_from: Vec::new(),
        mounts: Vec::new(),
    }
}

fn pod(
    namespace: &str,
    name: &str,
    labels: &[&str],
    containers: Vec<ContainerSummary>,
) -> PodSummary {
    PodSummary {
        annotations: cluster::AnnotationTerms::default(),
        is_finished: false,
        namespace: namespace.to_owned(),
        name: name.to_owned(),
        status: PodStatus::Reason(StatusReason::Running),
        ready: ReadyCount { ready: 0, total: 0 },
        restarts: 0,
        node_name: None,
        created_at: None,
        pod_ip: Some("10.0.0.1".to_owned()),
        qos_class: None,
        service_account: None,
        controller: None,
        conditions: Vec::new(),
        status_message: None,
        labels: terms(labels),
        host_network: false,
        image_pull_secrets: Vec::new(),
        node_selector: Vec::new(),
        node_affinity: Vec::new(),
        containers,
    }
}

fn policy(namespace: &str, name: &str, selector: &[&str]) -> NetworkPolicySummary {
    NetworkPolicySummary {
        namespace: namespace.to_owned(),
        name: name.to_owned(),
        created_at: None,
        labels: Vec::new(),
        pod_selector: Selector::of_labels(&terms(selector)).unwrap_or_else(Selector::everything),
        ingress: PolicyDirection::NotIsolated,
        egress: PolicyDirection::NotIsolated,
    }
}

fn key(namespace: &str, name: &str) -> Option<ResourceKey> {
    Some(ResourceKey::Pod {
        namespace: namespace.to_owned(),
        name: name.to_owned(),
    })
}

#[test]
fn defaults_destination_selected_by_policy() {
    let pods = [
        pod("shop", "a-client", &["app=client"], Vec::new()),
        pod("shop", "b-web", &["app=web"], Vec::new()),
        pod("shop", "c-web", &["app=web"], Vec::new()),
    ];
    let form = traffic_defaults(&pods, Some(&policy("shop", "web-only", &["app=web"])));
    assert_eq!(form.destination, key("shop", "b-web"));
    // The first other pod of that namespace.
    assert_eq!(form.source, key("shop", "a-client"));
}

#[test]
fn defaults_without_policy_use_first_two_pods() {
    let pods = [
        pod("shop", "z", &[], Vec::new()),
        pod("ops", "b", &[], Vec::new()),
        pod("ops", "a", &[], Vec::new()),
    ];
    let form = traffic_defaults(&pods, None);
    assert_eq!(form.destination, key("ops", "a"));
    assert_eq!(form.source, key("ops", "b"));
}

#[test]
fn defaults_source_falls_back_to_another_namespace() {
    let pods = [
        pod("ops", "a", &[], Vec::new()),
        pod("shop", "b", &[], Vec::new()),
    ];
    let form = traffic_defaults(&pods, None);
    assert_eq!(form.destination, key("ops", "a"));
    assert_eq!(form.source, key("shop", "b"));
}

#[test]
fn defaults_with_one_pod_have_no_source() {
    let pods = [pod("ops", "a", &[], Vec::new())];
    assert_eq!(traffic_defaults(&pods, None).source, None);
}

#[test]
fn defaults_without_pods_have_no_destination() {
    assert_eq!(traffic_defaults(&[], None).destination, None);
}

#[test]
fn defaults_port_first_main_container_port_or_80() {
    let with_ports = pod(
        "shop",
        "web",
        &[],
        vec![
            container(ContainerKind::Init, &[(None, 9999)]),
            container(ContainerKind::Main, &[(Some("http"), 8080), (None, 9090)]),
        ],
    );
    assert_eq!(traffic_defaults(&[with_ports], None).port, "8080");
    let without = pod(
        "shop",
        "web",
        &[],
        vec![container(ContainerKind::Main, &[])],
    );
    assert_eq!(traffic_defaults(&[without], None).port, "80");
}

#[test]
fn labels_input_sorted_into_terms() {
    assert_eq!(
        label_terms("tier=front, app=web ,").as_deref(),
        Some(&terms(&["app=web", "tier=front"])[..])
    );
}

#[test]
fn empty_labels_input_is_no_labels() {
    assert_eq!(label_terms("  "), Some(Vec::new()));
}

#[test]
fn label_keys_and_values_are_trimmed() {
    assert_eq!(
        label_terms(" app = web , tier=front ").as_deref(),
        Some(&terms(&["app=web", "tier=front"])[..])
    );
    // An empty value is a valid label value.
    assert_eq!(
        label_terms("debug=").as_deref(),
        Some(&terms(&["debug="])[..])
    );
}

#[test]
fn label_without_equals_is_rejected() {
    assert_eq!(label_terms("app=web, tier"), None);
}

#[test]
fn label_with_empty_key_is_rejected() {
    assert_eq!(label_terms("=web"), None);
    assert_eq!(label_terms("  =web"), None);
}

#[test]
fn duplicate_label_keys_are_rejected() {
    assert_eq!(label_terms("app=web,app=db"), None);
    assert_eq!(label_terms("app=web, app =web"), None);
}

#[test]
fn destination_ports_include_sidecar_ports() {
    let web = pod(
        "shop",
        "web",
        &[],
        vec![
            container(ContainerKind::Init, &[(None, 1111)]),
            container(ContainerKind::Sidecar, &[(Some("proxy"), 15001)]),
            container(ContainerKind::Main, &[(Some("http"), 8080)]),
        ],
    );
    let numbers: Vec<u16> = serving_ports(&web).iter().map(|port| port.port).collect();
    assert_eq!(numbers, [15001, 8080]);
}

#[test]
fn ip_is_read_in_canonical_form() {
    assert_eq!(parse_ip(" 10.0.0.5 ").as_deref(), Some("10.0.0.5"));
    assert_eq!(parse_ip("fd00:0:0::1").as_deref(), Some("fd00::1"));
}

#[test]
fn invalid_ip_is_rejected() {
    for text in ["", "10.0.0", "10.0.0.256", "web", "10.0.0.0/8"] {
        assert_eq!(parse_ip(text), None, "{text}");
    }
}

#[test]
fn port_input_reads_numbers() {
    assert_eq!(parse_port("8080"), Ok(RequestPort::Number(8080)));
}

#[test]
fn port_input_reads_names() {
    assert_eq!(
        parse_port(" http "),
        Ok(RequestPort::Name("http".to_owned()))
    );
}

#[test]
fn port_input_rejects_empty_and_out_of_range() {
    assert!(parse_port("").is_err());
    assert!(parse_port("0").is_err());
    assert!(parse_port("70000").is_err());
}

fn endpoint(label: &str, namespace: &str, ip: Option<&str>, is_host_network: bool) -> EndpointData {
    EndpointData {
        label: label.to_owned(),
        namespace: namespace.to_owned(),
        labels: terms(&["app=x"]),
        ip: ip.map(str::to_owned),
        is_host_network,
    }
}

fn asked(source: EndpointData, destination: EndpointData) -> Asked {
    Asked {
        source: SourceData::Workload(source),
        destination,
        destination_ports: vec![ContainerPort {
            name: Some("http".to_owned()),
            port: 80,
            protocol: "TCP".to_owned(),
            host_port: None,
        }],
        port: RequestPort::Number(80),
        protocol: "TCP".to_owned(),
    }
}

fn open_verdict() -> TrafficVerdict {
    TrafficVerdict {
        egress: DirectionVerdict::NotIsolated,
        ingress: DirectionVerdict::NotIsolated,
        port: 80,
        unknown_namespaces: Vec::new(),
    }
}

fn two_rule_policy() -> NetworkPolicySummary {
    let mut allowing = policy("shop", "db-access", &[]);
    allowing.egress = PolicyDirection::Allowed(vec![
        PolicyRule {
            peers: Vec::new(),
            ports: Vec::new(),
        },
        PolicyRule {
            peers: vec![PolicyPeer::Pods {
                namespaces: None,
                pods: Selector::of_labels(&terms(&["app=db"])),
            }],
            ports: vec![PolicyPort {
                protocol: "TCP".to_owned(),
                port: Some("5432".to_owned()),
                end_port: None,
            }],
        },
    ]);
    allowing
}

fn view_of(side: Side, verdict: &DirectionVerdict) -> DirectionView {
    direction_view(
        side,
        verdict,
        &[two_rule_policy()],
        "title".to_owned(),
        "shop/db",
        "5432/TCP",
    )
}

#[test]
fn verdict_rows_for_a_direction_no_policy_limits() {
    let egress = view_of(Side::Egress, &DirectionVerdict::NotIsolated);
    assert_eq!(
        egress.note.as_deref(),
        Some("No policy limits egress from the source.")
    );
    let ingress = view_of(Side::Ingress, &DirectionVerdict::NotIsolated);
    assert_eq!(
        ingress.note.as_deref(),
        Some("No policy limits ingress to the destination.")
    );
}

#[test]
fn verdict_rows_for_an_external_source() {
    let external = view_of(Side::Egress, &DirectionVerdict::NotApplicable);
    assert_eq!(
        external.note.as_deref(),
        Some("The source is outside the cluster; egress is not checked.")
    );
}

#[test]
fn verdict_rows_per_direction_state() {
    let denied = view_of(
        Side::Ingress,
        &DirectionVerdict::Denied {
            isolating: terms(&["a", "b"]),
        },
    );
    assert_eq!(
        denied.note.as_deref(),
        Some("Isolated by a, b; no ingress rule allows shop/db on 5432/TCP.")
    );
    assert_eq!(denied.tone, Some(StatusTone::Bad));
    let reference = |rule| RuleRef {
        policy: "db-access".to_owned(),
        rule,
        named_port: None,
    };
    let allowed = view_of(
        Side::Egress,
        &DirectionVerdict::Allowed {
            isolating: terms(&["db-access"]),
            allowing: vec![reference(0), reference(1)],
        },
    );
    let texts: Vec<_> = allowed
        .rules
        .iter()
        .map(|rule| rule.text.as_str())
        .collect();
    assert_eq!(
        texts,
        [
            " · egress rule 1: any destination, all ports",
            " · egress rule 2: pods app=db, port 5432/TCP"
        ]
    );
    assert_eq!(allowed.rules[0].policy_text, "db-access");
}

#[test]
fn unknown_port_name_text() {
    assert_eq!(
        error_line(&TrafficError::UnknownPortName("nope".to_owned())),
        "The destination has no port named nope"
    );
}

#[test]
fn host_network_warning() {
    let data = asked(
        endpoint("shop/agent", "shop", Some("10.0.0.1"), true),
        endpoint("shop/web", "shop", Some("10.0.0.2"), false),
    );
    assert_eq!(
        traffic_warnings(&data, &open_verdict()),
        [
            "shop/agent uses the host network; most network plugins do not apply NetworkPolicy to it."
        ]
    );
}

#[test]
fn missing_ip_warning() {
    let data = asked(
        endpoint("pods app=x in shop", "shop", None, false),
        endpoint("shop/web", "shop", Some("10.0.0.2"), false),
    );
    assert_eq!(
        traffic_warnings(&data, &open_verdict()),
        ["pods app=x in shop has no IP; ipBlock rules were treated as not matching."]
    );
}

#[test]
fn named_egress_port_warning() {
    let data = asked(
        endpoint("shop/a", "shop", Some("10.0.0.1"), false),
        endpoint("shop/web", "shop", Some("10.0.0.2"), false),
    );
    let mut verdict = open_verdict();
    verdict.egress = DirectionVerdict::Allowed {
        isolating: terms(&["p"]),
        allowing: vec![RuleRef {
            policy: "p".to_owned(),
            rule: 1,
            named_port: Some("http".to_owned()),
        }],
    };
    assert_eq!(
        traffic_warnings(&data, &verdict),
        [
            "Egress rule 2 of p matched by port name http, resolved on the destination; some network plugins do not support named egress ports."
        ]
    );
}

#[test]
fn unknown_namespace_warning() {
    let data = asked(
        endpoint("shop/a", "shop", Some("10.0.0.1"), false),
        endpoint("shop/web", "shop", Some("10.0.0.2"), false),
    );
    let mut verdict = open_verdict();
    verdict.unknown_namespaces = terms(&["ghost"]);
    assert_eq!(
        traffic_warnings(&data, &verdict),
        [
            "Labels of namespace ghost are unknown; its namespace selectors were treated as not matching."
        ]
    );
}

#[test]
fn outcome_denies_without_an_ingress_rule_and_names_the_namespace() {
    let mut isolate = policy("shop", "deny-all", &[]);
    isolate.ingress = PolicyDirection::Allowed(Vec::new());
    let data = asked(
        endpoint("shop/a", "shop", Some("10.0.0.1"), false),
        endpoint("shop/web", "shop", Some("10.0.0.2"), false),
    );
    let result = outcome(
        &data,
        &[("shop".to_owned(), vec![isolate])],
        &[],
        jiff::Timestamp::UNIX_EPOCH,
    )
    .expect("evaluates");
    assert!(!result.is_allowed);
    assert!(
        result
            .source_line
            .starts_with("Computed from NetworkPolicies of shop listed at ")
    );
    assert_eq!(result.port_note, None);
}

#[test]
fn outcome_with_two_namespaces_and_a_port_name() {
    let mut data = asked(
        endpoint("ops/a", "ops", Some("10.0.0.1"), false),
        endpoint("shop/web", "shop", Some("10.0.0.2"), false),
    );
    data.port = RequestPort::Name("http".to_owned());
    let result = outcome(&data, &[], &[], jiff::Timestamp::UNIX_EPOCH).expect("evaluates");
    assert!(result.is_allowed);
    assert_eq!(result.port_note.as_deref(), Some("Port http is 80"));
    assert!(
        result
            .source_line
            .starts_with("Computed from NetworkPolicies of ops and shop listed at ")
    );
}
