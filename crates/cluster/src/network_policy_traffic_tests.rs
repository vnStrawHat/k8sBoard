use super::*;
use crate::namespace::NamespacePhase;
use crate::selector::Selector;

fn terms(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| (*item).to_owned()).collect()
}

/// An empty list is the selector that selects everything.
fn selector(items: &[&str]) -> Selector {
    Selector::of_labels(&terms(items)).unwrap_or_else(Selector::everything)
}

fn pods_peer(pods: Option<&[&str]>) -> PolicyPeer {
    PolicyPeer::Pods {
        namespaces: None,
        pods: pods.map(selector),
    }
}

fn namespace_peer(namespaces: &[&str], pods: Option<&[&str]>) -> PolicyPeer {
    PolicyPeer::Pods {
        namespaces: Some(selector(namespaces)),
        pods: pods.map(selector),
    }
}

fn ip_peer(cidr: &str, except: &[&str]) -> PolicyPeer {
    PolicyPeer::IpBlock {
        cidr: cidr.to_owned(),
        except: terms(except),
    }
}

fn rule(peers: Vec<PolicyPeer>, ports: Vec<PolicyPort>) -> PolicyRule {
    PolicyRule { peers, ports }
}

fn port(protocol: &str, number: Option<&str>, end: Option<u16>) -> PolicyPort {
    PolicyPort {
        protocol: protocol.to_owned(),
        port: number.map(str::to_owned),
        end_port: end,
    }
}

fn tcp(number: &str) -> PolicyPort {
    port("TCP", Some(number), None)
}

fn allow(rules: Vec<PolicyRule>) -> PolicyDirection {
    PolicyDirection::Allowed(rules)
}

fn policy(
    namespace: &str,
    name: &str,
    pods: &[&str],
    ingress: PolicyDirection,
    egress: PolicyDirection,
) -> NetworkPolicySummary {
    NetworkPolicySummary {
        namespace: namespace.to_owned(),
        name: name.to_owned(),
        created_at: None,
        labels: Vec::new(),
        pod_selector: selector(pods),
        ingress,
        egress,
    }
}

fn ingress_policy(name: &str, pods: &[&str], rules: Vec<PolicyRule>) -> NetworkPolicySummary {
    policy(
        "shop",
        name,
        pods,
        allow(rules),
        PolicyDirection::NotIsolated,
    )
}

fn egress_policy(name: &str, pods: &[&str], rules: Vec<PolicyRule>) -> NetworkPolicySummary {
    policy(
        "shop",
        name,
        pods,
        PolicyDirection::NotIsolated,
        allow(rules),
    )
}

fn namespace(name: &str, labels: &[&str]) -> NamespaceSummary {
    NamespaceSummary {
        name: name.to_owned(),
        phase: NamespacePhase::Active,
        labels: terms(labels),
        created_at: None,
        deleting_since: None,
        deletion_conditions: Vec::new(),
    }
}

fn container_port(name: Option<&str>, number: u16, protocol: &str) -> ContainerPort {
    ContainerPort {
        name: name.map(str::to_owned),
        port: number,
        protocol: protocol.to_owned(),
        host_port: None,
    }
}

/// A client pod `shop/client` calling a server pod `shop/web` on TCP 80.
struct Scene {
    source_namespace: &'static str,
    source_labels: Vec<String>,
    source_ip: Option<&'static str>,
    external_ip: Option<&'static str>,
    destination_labels: Vec<String>,
    destination_ip: Option<&'static str>,
    destination_ports: Vec<ContainerPort>,
    port: RequestPort,
    protocol: &'static str,
    source_policies: Vec<NetworkPolicySummary>,
    destination_policies: Vec<NetworkPolicySummary>,
    namespaces: Vec<NamespaceSummary>,
}

impl Scene {
    fn new() -> Self {
        Self {
            source_namespace: "shop",
            source_labels: terms(&["app=client"]),
            source_ip: Some("10.0.0.1"),
            external_ip: None,
            destination_labels: terms(&["app=web"]),
            destination_ip: Some("10.0.0.2"),
            destination_ports: vec![container_port(Some("http"), 80, "TCP")],
            port: RequestPort::Number(80),
            protocol: "TCP",
            source_policies: Vec::new(),
            destination_policies: Vec::new(),
            namespaces: vec![
                namespace("shop", &["team=shop"]),
                namespace("ops", &["team=ops"]),
            ],
        }
    }

    fn evaluate(&self) -> Result<TrafficVerdict, TrafficError> {
        let source = match self.external_ip {
            Some(ip) => TrafficSource::External { ip },
            None => TrafficSource::Workload(TrafficEndpoint {
                namespace: self.source_namespace,
                labels: &self.source_labels,
                ip: self.source_ip,
            }),
        };
        let request = TrafficRequest {
            source,
            destination: TrafficDestination {
                endpoint: TrafficEndpoint {
                    namespace: "shop",
                    labels: &self.destination_labels,
                    ip: self.destination_ip,
                },
                ports: &self.destination_ports,
            },
            port: self.port.clone(),
            protocol: self.protocol.to_owned(),
        };
        let inputs = PolicyInputs {
            source_policies: &self.source_policies,
            destination_policies: &self.destination_policies,
            namespaces: &self.namespaces,
        };
        evaluate_traffic(&request, &inputs)
    }

    fn verdict(&self) -> TrafficVerdict {
        self.evaluate().expect("evaluates")
    }

    /// Both lists hold the same policies, as when source and destination share a namespace.
    fn with_policies(mut self, policies: Vec<NetworkPolicySummary>) -> Self {
        self.source_policies.clone_from(&policies);
        self.destination_policies = policies;
        self
    }

    fn ingress(&self) -> DirectionVerdict {
        self.verdict().ingress
    }

    fn egress(&self) -> DirectionVerdict {
        self.verdict().egress
    }
}

fn is_allowing(verdict: &DirectionVerdict) -> bool {
    matches!(verdict, DirectionVerdict::Allowed { .. })
}

fn is_denied(verdict: &DirectionVerdict) -> bool {
    matches!(verdict, DirectionVerdict::Denied { .. })
}

// Isolation.

#[test]
fn no_policies_allow_both_directions() {
    let verdict = Scene::new().verdict();
    assert_eq!(verdict.ingress, DirectionVerdict::NotIsolated);
    assert_eq!(verdict.egress, DirectionVerdict::NotIsolated);
    assert!(verdict.is_allowed());
    assert_eq!(verdict.port, 80);
}

#[test]
fn policy_not_selecting_pod_does_not_isolate() {
    let scene = Scene::new().with_policies(vec![ingress_policy("db", &["app=db"], Vec::new())]);
    assert_eq!(scene.ingress(), DirectionVerdict::NotIsolated);
}

#[test]
fn empty_ingress_rules_deny_all() {
    let scene = Scene::new().with_policies(vec![ingress_policy("deny", &[], Vec::new())]);
    assert_eq!(
        scene.ingress(),
        DirectionVerdict::Denied {
            isolating: terms(&["deny"])
        }
    );
    assert!(!scene.verdict().is_allowed());
}

#[test]
fn ingress_only_policy_does_not_isolate_egress() {
    let scene = Scene::new().with_policies(vec![ingress_policy("deny", &[], Vec::new())]);
    assert_eq!(scene.egress(), DirectionVerdict::NotIsolated);
}

#[test]
fn egress_isolation_denies_without_rule() {
    let scene = Scene::new().with_policies(vec![egress_policy(
        "lockdown",
        &["app=client"],
        vec![rule(vec![pods_peer(Some(&["app=db"]))], Vec::new())],
    )]);
    assert!(is_denied(&scene.egress()));
    assert_eq!(scene.ingress(), DirectionVerdict::NotIsolated);
}

#[test]
fn union_of_policies_any_rule_allows() {
    let scene = Scene::new().with_policies(vec![
        ingress_policy("none", &[], Vec::new()),
        ingress_policy(
            "clients",
            &["app=web"],
            vec![rule(vec![pods_peer(Some(&["app=client"]))], Vec::new())],
        ),
    ]);
    assert!(is_allowing(&scene.ingress()));
}

#[test]
fn allowing_lists_every_matching_rule() {
    let scene = Scene::new().with_policies(vec![
        ingress_policy(
            "first",
            &[],
            vec![
                rule(vec![pods_peer(Some(&["app=other"]))], Vec::new()),
                rule(Vec::new(), vec![tcp("80")]),
            ],
        ),
        ingress_policy("second", &["app=web"], vec![rule(Vec::new(), Vec::new())]),
    ]);
    let DirectionVerdict::Allowed {
        isolating,
        allowing,
    } = scene.ingress()
    else {
        panic!("allowed");
    };
    assert_eq!(isolating, terms(&["first", "second"]));
    let rules: Vec<_> = allowing
        .iter()
        .map(|reference| (reference.policy.as_str(), reference.rule))
        .collect();
    assert_eq!(rules, [("first", 1), ("second", 0)]);
}

#[test]
fn denied_lists_isolating_policies() {
    let scene = Scene::new().with_policies(vec![
        ingress_policy("a", &[], Vec::new()),
        ingress_policy("b", &["app=db"], Vec::new()),
        ingress_policy("c", &["app=web"], Vec::new()),
    ]);
    assert_eq!(
        scene.ingress(),
        DirectionVerdict::Denied {
            isolating: terms(&["a", "c"])
        }
    );
}

#[test]
fn allowed_needs_both_directions() {
    let mut scene = Scene::new();
    scene.source_policies = vec![egress_policy(
        "egress",
        &[],
        vec![rule(Vec::new(), Vec::new())],
    )];
    scene.destination_policies = vec![ingress_policy("ingress", &[], Vec::new())];
    let verdict = scene.verdict();
    assert!(is_allowing(&verdict.egress));
    assert!(is_denied(&verdict.ingress));
    assert!(!verdict.is_allowed());
}

// Peers.

#[test]
fn rule_without_peers_allows_any_source() {
    let scene = Scene::new().with_policies(vec![ingress_policy(
        "open",
        &[],
        vec![rule(Vec::new(), Vec::new())],
    )]);
    assert!(is_allowing(&scene.ingress()));
}

#[test]
fn pod_selector_peer_same_namespace_only() {
    let policies = vec![ingress_policy(
        "p",
        &[],
        vec![rule(vec![pods_peer(Some(&["app=client"]))], Vec::new())],
    )];
    let mut scene = Scene::new().with_policies(policies);
    assert!(is_allowing(&scene.ingress()));
    // The same labels in another namespace are a different pod.
    scene.source_namespace = "ops";
    assert!(is_denied(&scene.ingress()));
}

#[test]
fn namespace_selector_peer_uses_namespace_labels() {
    let policies = vec![ingress_policy(
        "p",
        &[],
        vec![rule(vec![namespace_peer(&["team=ops"], None)], Vec::new())],
    )];
    let mut scene = Scene::new().with_policies(policies);
    scene.source_namespace = "ops";
    assert!(is_allowing(&scene.ingress()));
    scene.source_namespace = "shop";
    assert!(is_denied(&scene.ingress()));
}

#[test]
fn namespace_and_pod_selector_both_required() {
    let policies = vec![ingress_policy(
        "p",
        &[],
        vec![rule(
            vec![namespace_peer(&["team=ops"], Some(&["app=client"]))],
            Vec::new(),
        )],
    )];
    let mut scene = Scene::new().with_policies(policies);
    scene.source_namespace = "ops";
    assert!(is_allowing(&scene.ingress()));
}

#[test]
fn namespace_and_pod_selector_pod_mismatch_denies() {
    let policies = vec![ingress_policy(
        "p",
        &[],
        vec![rule(
            vec![namespace_peer(&["team=ops"], Some(&["app=client"]))],
            Vec::new(),
        )],
    )];
    let mut scene = Scene::new().with_policies(policies);
    scene.source_namespace = "ops";
    scene.source_labels = terms(&["app=other"]);
    assert!(is_denied(&scene.ingress()));
}

#[test]
fn namespace_and_pod_selector_namespace_mismatch_denies() {
    let policies = vec![ingress_policy(
        "p",
        &[],
        vec![rule(
            vec![namespace_peer(&["team=ops"], Some(&["app=client"]))],
            Vec::new(),
        )],
    )];
    let scene = Scene::new().with_policies(policies);
    // The pod labels match but the source namespace carries `team=shop`.
    assert!(is_denied(&scene.ingress()));
}

#[test]
fn empty_namespace_selector_needs_no_lookup() {
    let policies = vec![ingress_policy(
        "p",
        &[],
        vec![rule(vec![namespace_peer(&[], None)], Vec::new())],
    )];
    let mut scene = Scene::new().with_policies(policies);
    scene.namespaces = Vec::new();
    scene.source_namespace = "anywhere";
    let verdict = scene.verdict();
    assert!(is_allowing(&verdict.ingress));
    assert!(verdict.unknown_namespaces.is_empty());
}

#[test]
fn unknown_namespace_labels_reported_once() {
    let policies = vec![ingress_policy(
        "p",
        &[],
        vec![
            rule(vec![namespace_peer(&["team=ops"], None)], Vec::new()),
            rule(vec![namespace_peer(&["team=dev"], None)], Vec::new()),
        ],
    )];
    let mut scene = Scene::new().with_policies(policies);
    scene.namespaces = Vec::new();
    scene.source_namespace = "ghost";
    let verdict = scene.verdict();
    assert!(is_denied(&verdict.ingress));
    assert_eq!(verdict.unknown_namespaces, terms(&["ghost"]));
}

#[test]
fn ip_block_matches_cidr_and_except() {
    let policies = vec![ingress_policy(
        "p",
        &[],
        vec![rule(
            vec![ip_peer("10.0.0.0/24", &["10.0.0.0/30"])],
            Vec::new(),
        )],
    )];
    let mut scene = Scene::new().with_policies(policies);
    scene.source_ip = Some("10.0.0.9");
    assert!(is_allowing(&scene.ingress()));
}

#[test]
fn ip_block_except_range_denies() {
    let policies = vec![ingress_policy(
        "p",
        &[],
        vec![rule(
            vec![ip_peer("10.0.0.0/24", &["10.0.0.0/30"])],
            Vec::new(),
        )],
    )];
    let mut scene = Scene::new().with_policies(policies);
    scene.source_ip = Some("10.0.0.1");
    assert!(is_denied(&scene.ingress()));
}

#[test]
fn ip_block_outside_cidr_denies() {
    let policies = vec![ingress_policy(
        "p",
        &[],
        vec![rule(vec![ip_peer("10.0.0.0/24", &[])], Vec::new())],
    )];
    let mut scene = Scene::new().with_policies(policies);
    scene.source_ip = Some("10.0.1.9");
    assert!(is_denied(&scene.ingress()));
}

#[test]
fn ip_block_without_ip_never_matches() {
    let policies = vec![ingress_policy(
        "p",
        &[],
        vec![rule(vec![ip_peer("0.0.0.0/0", &[])], Vec::new())],
    )];
    let mut scene = Scene::new().with_policies(policies);
    scene.source_ip = None;
    assert!(is_denied(&scene.ingress()));
}

#[test]
fn ipv6_cidr_matches() {
    assert_eq!(cidr_contains("fd00::/8", "fd12:3456::1"), Some(true));
    assert_eq!(cidr_contains("fd00::/16", "fd12::1"), Some(false));
    assert_eq!(cidr_contains("::/0", "2001:db8::1"), Some(true));
}

#[test]
fn ipv4_cidr_matches() {
    assert_eq!(cidr_contains("10.0.0.0/8", "10.9.9.9"), Some(true));
    assert_eq!(cidr_contains("10.0.0.0/8", "11.0.0.1"), Some(false));
    assert_eq!(cidr_contains("0.0.0.0/0", "10.9.9.9"), Some(true));
}

#[test]
fn ipv4_host_prefix_matches_only_that_address() {
    assert_eq!(cidr_contains("10.0.0.1/32", "10.0.0.1"), Some(true));
    assert_eq!(cidr_contains("10.0.0.1/32", "10.0.0.2"), Some(false));
    assert_eq!(cidr_contains("fd00::1/128", "fd00::1"), Some(true));
    assert_eq!(cidr_contains("fd00::1/128", "fd00::2"), Some(false));
}

#[test]
fn mixed_family_or_invalid_cidr_no_match() {
    assert_eq!(cidr_contains("10.0.0.0/8", "fd00::1"), None);
    assert_eq!(cidr_contains("fd00::/8", "10.0.0.1"), None);
}

#[test]
fn invalid_cidr_or_address_no_match() {
    assert_eq!(cidr_contains("10.0.0.0", "10.0.0.1"), None);
    assert_eq!(cidr_contains("10.0.0.0/33", "10.0.0.1"), None);
    assert_eq!(cidr_contains("nonsense/8", "10.0.0.1"), None);
    assert_eq!(cidr_contains("10.0.0.0/8", "nonsense"), None);
}

#[test]
fn external_source_skips_egress() {
    let mut scene = Scene::new();
    scene.external_ip = Some("203.0.113.7");
    scene.source_policies = vec![egress_policy("lockdown", &[], Vec::new())];
    assert_eq!(scene.egress(), DirectionVerdict::NotApplicable);
}

#[test]
fn external_source_matches_only_ip_blocks() {
    let policies = vec![ingress_policy(
        "p",
        &[],
        vec![
            rule(vec![pods_peer(None)], Vec::new()),
            rule(vec![namespace_peer(&[], None)], Vec::new()),
            rule(vec![ip_peer("203.0.113.0/24", &[])], Vec::new()),
        ],
    )];
    let mut scene = Scene::new();
    scene.destination_policies = policies;
    scene.external_ip = Some("203.0.113.7");
    let DirectionVerdict::Allowed { allowing, .. } = scene.ingress() else {
        panic!("allowed");
    };
    let indexes: Vec<_> = allowing.iter().map(|reference| reference.rule).collect();
    assert_eq!(indexes, [2]);
    scene.external_ip = Some("198.51.100.1");
    assert!(is_denied(&scene.ingress()));
}

// Ports.

#[test]
fn rule_without_ports_allows_any_port() {
    let mut scene = Scene::new().with_policies(vec![ingress_policy(
        "p",
        &[],
        vec![rule(Vec::new(), Vec::new())],
    )]);
    scene.port = RequestPort::Number(5432);
    assert!(is_allowing(&scene.ingress()));
}

#[test]
fn port_number_and_protocol_must_match() {
    let policies = vec![ingress_policy(
        "p",
        &[],
        vec![rule(Vec::new(), vec![tcp("80")])],
    )];
    let mut scene = Scene::new().with_policies(policies);
    assert!(is_allowing(&scene.ingress()));
    scene.port = RequestPort::Number(81);
    assert!(is_denied(&scene.ingress()));
}

#[test]
fn port_protocol_must_match() {
    let policies = vec![ingress_policy(
        "p",
        &[],
        vec![rule(Vec::new(), vec![tcp("80")])],
    )];
    let mut scene = Scene::new().with_policies(policies);
    scene.protocol = "UDP";
    assert!(is_denied(&scene.ingress()));
}

#[test]
fn port_none_matches_every_port_of_protocol() {
    let policies = vec![ingress_policy(
        "p",
        &[],
        vec![rule(Vec::new(), vec![port("UDP", None, None)])],
    )];
    let mut scene = Scene::new().with_policies(policies);
    scene.protocol = "UDP";
    scene.port = RequestPort::Number(5353);
    assert!(is_allowing(&scene.ingress()));
    scene.protocol = "TCP";
    assert!(is_denied(&scene.ingress()));
}

#[test]
fn port_range_with_end_port() {
    let policies = vec![ingress_policy(
        "p",
        &[],
        vec![rule(
            Vec::new(),
            vec![port("TCP", Some("8000"), Some(9000))],
        )],
    )];
    let mut scene = Scene::new().with_policies(policies);
    for (number, expected) in [
        (7999, false),
        (8000, true),
        (8500, true),
        (9000, true),
        (9001, false),
    ] {
        scene.port = RequestPort::Number(number);
        assert_eq!(is_allowing(&scene.ingress()), expected, "{number}");
    }
}

#[test]
fn named_ingress_port_resolves_on_destination() {
    let policies = vec![ingress_policy(
        "p",
        &[],
        vec![rule(Vec::new(), vec![tcp("http")])],
    )];
    let scene = Scene::new().with_policies(policies);
    assert!(is_allowing(&scene.ingress()));
}

#[test]
fn named_ingress_port_denied_when_destination_port_differs() {
    let policies = vec![ingress_policy(
        "p",
        &[],
        vec![rule(Vec::new(), vec![tcp("http")])],
    )];
    let mut scene = Scene::new().with_policies(policies);
    // The rule names `http`, which the destination maps to 80, not 8080.
    scene.port = RequestPort::Number(8080);
    assert!(is_denied(&scene.ingress()));
}

#[test]
fn named_egress_port_resolves_on_destination() {
    let policies = vec![egress_policy(
        "p",
        &["app=client"],
        vec![rule(Vec::new(), vec![tcp("http")])],
    )];
    let scene = Scene::new().with_policies(policies);
    assert!(is_allowing(&scene.egress()));
}

#[test]
fn named_egress_port_denied_when_destination_port_differs() {
    let policies = vec![egress_policy(
        "p",
        &["app=client"],
        vec![rule(Vec::new(), vec![tcp("http")])],
    )];
    let mut scene = Scene::new().with_policies(policies);
    scene.destination_ports = vec![container_port(Some("http"), 8080, "TCP")];
    assert!(is_denied(&scene.egress()));
}

#[test]
fn named_port_recorded_on_rule_ref() {
    let policies = vec![ingress_policy(
        "p",
        &[],
        vec![
            rule(Vec::new(), vec![tcp("http")]),
            rule(Vec::new(), vec![tcp("http"), tcp("80")]),
        ],
    )];
    let scene = Scene::new().with_policies(policies);
    let DirectionVerdict::Allowed { allowing, .. } = scene.ingress() else {
        panic!("allowed");
    };
    assert_eq!(allowing[0].named_port.as_deref(), Some("http"));
    // A numeric port of the same rule matched too, so no name decides.
    assert_eq!(allowing[1].named_port, None);
}

#[test]
fn request_port_name_resolves() {
    let mut scene = Scene::new();
    scene.destination_ports = vec![
        container_port(Some("metrics"), 9090, "TCP"),
        container_port(Some("dns"), 53, "UDP"),
    ];
    scene.port = RequestPort::Name("metrics".to_owned());
    assert_eq!(scene.verdict().port, 9090);
}

#[test]
fn request_port_name_resolves_with_protocol() {
    let mut scene = Scene::new();
    scene.destination_ports = vec![
        container_port(Some("metrics"), 9090, "TCP"),
        container_port(Some("dns"), 53, "UDP"),
    ];
    scene.port = RequestPort::Name("dns".to_owned());
    scene.protocol = "UDP";
    assert_eq!(scene.verdict().port, 53);
}

#[test]
fn unknown_request_port_name_errors() {
    let mut scene = Scene::new();
    scene.port = RequestPort::Name("nope".to_owned());
    let error = scene.evaluate().expect_err("unknown name");
    assert_eq!(error, TrafficError::UnknownPortName("nope".to_owned()));
    assert_eq!(error.to_string(), "the destination has no port named nope");
}

#[test]
fn request_port_name_is_per_protocol() {
    let mut scene = Scene::new();
    scene.port = RequestPort::Name("http".to_owned());
    scene.protocol = "UDP";
    assert!(scene.evaluate().is_err());
}

#[test]
fn policy_from_other_namespace_does_not_isolate() {
    // A policy listed under another namespace selects nothing in the pod's namespace.
    let foreign = policy("ops", "deny-all", &[], allow(Vec::new()), allow(Vec::new()));
    let mut scene = Scene::new();
    scene.source_policies = vec![foreign.clone()];
    scene.destination_policies = vec![foreign];
    let verdict = scene.verdict();
    assert_eq!(verdict.ingress, DirectionVerdict::NotIsolated);
    assert_eq!(verdict.egress, DirectionVerdict::NotIsolated);
}
