//! The NetworkPolicies row builder: rules rewritten as sentences. The Affects cell and the status
//! count are joined later from the pods list (`kind_join`).

use cluster::{
    NetworkPolicySummary, PolicyDirection, PolicyPeer, PolicyPort, PolicyRule, Selector,
};

use crate::kind_row::{
    DetailRow, DetailSection, KindCell, KindObject, KindRow, LiveContent, chips,
};
use crate::status_tone::{StatusLabel, StatusTone};

/// The label of the namespace that every namespace carries, so a selector on it names one.
const NAMESPACE_NAME_LABEL: &str = "kubernetes.io/metadata.name";

pub(crate) fn network_policy_row(policy: &NetworkPolicySummary) -> KindRow {
    let types = policy_types(policy);
    let status = network_policy_status(policy);
    let terms = policy.pod_selector.terms();
    let selector_cell = if policy.pod_selector.selects_everything() {
        KindCell::Toned(StatusLabel {
            text: "(all pods)".into(),
            tone: StatusTone::Done,
        })
    } else {
        KindCell::Mono(terms.join(", ").into())
    };
    let types_cell = if types.is_empty() {
        KindCell::Absent
    } else {
        KindCell::Text(types.into())
    };
    let applies_to = if policy.pod_selector.selects_everything() {
        DetailRow::Note(format!("All pods in {}", policy.namespace).into())
    } else {
        DetailRow::Chips(chips(&terms))
    };
    let mut sections = vec![
        DetailSection {
            title: "Applies to",
            rows: vec![applies_to],
        },
        DetailSection {
            title: "Pods",
            rows: vec![DetailRow::Live(LiveContent::SelectedPods)],
        },
    ];
    sections.extend(direction_section(
        "Allow ingress from",
        &policy.ingress,
        Direction::Ingress,
    ));
    sections.extend(direction_section(
        "Allow egress to",
        &policy.egress,
        Direction::Egress,
    ));
    KindRow {
        namespace: Some(policy.namespace.clone()),
        name: policy.name.clone(),
        created_at: policy.created_at,
        status,
        cells: vec![
            selector_cell,
            types_cell,
            // The pods join fills Affects.
            KindCell::Absent,
            KindCell::age(policy.created_at),
        ],
        sections,
        event: None,
        related_pods: None,
        labels: chips(&policy.labels),
        object: KindObject::NetworkPolicy(policy.clone()),
    }
}

/// The status before the pods join: the isolated directions. Info would count as unhealthy in the
/// table chips, so this stays Ok.
pub(crate) fn network_policy_status(policy: &NetworkPolicySummary) -> StatusLabel {
    let types = policy_types(policy);
    if types.is_empty() {
        return StatusLabel {
            text: "No isolation".into(),
            tone: StatusTone::Done,
        };
    }
    StatusLabel {
        text: types.into(),
        tone: StatusTone::Ok,
    }
}

/// `Ingress`, `Egress`, `Ingress, Egress`, or empty when the policy isolates nothing.
fn policy_types(policy: &NetworkPolicySummary) -> &'static str {
    let is_isolated = |direction: &PolicyDirection| *direction != PolicyDirection::NotIsolated;
    match (is_isolated(&policy.ingress), is_isolated(&policy.egress)) {
        (true, true) => "Ingress, Egress",
        (true, false) => "Ingress",
        (false, true) => "Egress",
        (false, false) => "",
    }
}

#[derive(Clone, Copy)]
enum Direction {
    Ingress,
    Egress,
}

/// `None` for a direction the policy does not restrict.
fn direction_section(
    title: &'static str,
    direction: &PolicyDirection,
    side: Direction,
) -> Option<DetailSection> {
    let PolicyDirection::Allowed(rules) = direction else {
        return None;
    };
    let (denies_all, any_peer) = match side {
        Direction::Ingress => ("Denies all ingress to the selected pods", "any source"),
        Direction::Egress => (
            "Denies all egress from the selected pods",
            "any destination",
        ),
    };
    if rules.is_empty() {
        return Some(DetailSection {
            title,
            rows: vec![DetailRow::Note(denies_all.into())],
        });
    }
    Some(DetailSection {
        title,
        rows: rules
            .iter()
            .flat_map(|rule| rule_rows(rule, any_peer))
            .collect(),
    })
}

/// One row per peer: the peer sentence and the ports; a rule with no peers reads `any_peer`.
fn rule_rows(rule: &PolicyRule, any_peer: &str) -> Vec<DetailRow> {
    let ports = KindCell::Text(ports_text(&rule.ports).into());
    if rule.peers.is_empty() {
        return vec![DetailRow::stacked(any_peer.to_owned(), ports)];
    }
    rule.peers
        .iter()
        .map(|peer| DetailRow::stacked(peer_text(peer), ports.clone()))
        .collect()
}

/// A peer as a sentence, such as `pods app=worker in namespace jobs`.
pub(crate) fn peer_text(peer: &PolicyPeer) -> String {
    match peer {
        PolicyPeer::IpBlock { cidr, except } if except.is_empty() => cidr.clone(),
        PolicyPeer::IpBlock { cidr, except } => format!("{cidr} except {}", except.join(", ")),
        PolicyPeer::Pods {
            namespaces: None,
            pods: Some(pods),
        } => match selector_terms(pods) {
            Some(terms) => format!("pods {terms}"),
            None => "all pods in this namespace".to_owned(),
        },
        PolicyPeer::Pods {
            namespaces: Some(namespaces),
            pods: None,
        } => namespaces_text(namespaces),
        PolicyPeer::Pods {
            namespaces: Some(namespaces),
            pods: Some(pods),
        } => {
            let pods = match selector_terms(pods) {
                Some(terms) => format!("pods {terms}"),
                None => "all pods".to_owned(),
            };
            format!("{pods} in {}", namespaces_text(namespaces))
        }
        // A peer without either selector is dropped by the cluster crate.
        PolicyPeer::Pods {
            namespaces: None,
            pods: None,
        } => "all pods in this namespace".to_owned(),
    }
}

/// The terms joined, or `None` when the selector selects everything.
fn selector_terms(selector: &Selector) -> Option<String> {
    (!selector.selects_everything()).then(|| selector.terms().join(", "))
}

fn namespaces_text(namespaces: &Selector) -> String {
    let terms = namespaces.terms();
    match terms.as_slice() {
        [] => "all namespaces".to_owned(),
        [term] => match term
            .strip_prefix(NAMESPACE_NAME_LABEL)
            .and_then(|rest| rest.strip_prefix('='))
        {
            Some(name) => format!("namespace {name}"),
            None => format!("namespaces {term}"),
        },
        _ => format!("namespaces {}", terms.join(", ")),
    }
}

/// `port 8080/TCP`, `ports 8000-9000/TCP`, `ports 80/TCP, 443/TCP`, `all TCP ports`, or
/// `all ports` for no restriction.
pub(crate) fn ports_text(ports: &[PolicyPort]) -> String {
    match ports {
        [] => "all ports".to_owned(),
        [only] => match (&only.port, only.end_port) {
            (None, _) => format!("all {} ports", only.protocol),
            (Some(_), Some(_)) => format!("ports {}", port_text(only)),
            (Some(_), None) => format!("port {}", port_text(only)),
        },
        several => {
            let texts: Vec<String> = several.iter().map(port_text).collect();
            format!("ports {}", texts.join(", "))
        }
    }
}

/// `8080/TCP`, `8000-9000/TCP`, or `TCP` for a port-less entry.
fn port_text(port: &PolicyPort) -> String {
    match (&port.port, port.end_port) {
        (None, _) => port.protocol.clone(),
        (Some(start), Some(end)) => format!("{start}-{end}/{}", port.protocol),
        (Some(start), None) => format!("{start}/{}", port.protocol),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resource_kind::ResourceKind;

    fn labels(terms: &[&str]) -> Selector {
        let terms: Vec<String> = terms.iter().map(|term| (*term).to_owned()).collect();
        Selector::of_labels(&terms).expect("terms are not empty")
    }

    fn policy(ingress: PolicyDirection, egress: PolicyDirection) -> NetworkPolicySummary {
        NetworkPolicySummary {
            namespace: "team-a".to_owned(),
            name: "web".to_owned(),
            created_at: None,
            labels: Vec::new(),
            pod_selector: labels(&["app=web"]),
            ingress,
            egress,
        }
    }

    fn port(port: Option<&str>, end_port: Option<u16>) -> PolicyPort {
        PolicyPort {
            protocol: "TCP".to_owned(),
            port: port.map(str::to_owned),
            end_port,
        }
    }

    fn rule(peers: Vec<PolicyPeer>, ports: Vec<PolicyPort>) -> PolicyRule {
        PolicyRule { peers, ports }
    }

    #[test]
    fn network_policy_row_cells_match_column_count() {
        let row = network_policy_row(&policy(
            PolicyDirection::Allowed(Vec::new()),
            PolicyDirection::NotIsolated,
        ));
        assert_eq!(
            row.cells.len(),
            ResourceKind::NetworkPolicies.columns().len()
        );
        assert_eq!(row.cells[1], KindCell::Text("Ingress".into()));
        assert_eq!(row.status.text.as_ref(), "Ingress");
        assert_eq!(row.status.tone, StatusTone::Ok);
    }

    #[test]
    fn pods_section_follows_applies_to() {
        let row = network_policy_row(&policy(
            PolicyDirection::Allowed(Vec::new()),
            PolicyDirection::NotIsolated,
        ));
        let titles: Vec<&str> = row.sections.iter().map(|section| section.title).collect();
        assert_eq!(&titles[..2], ["Applies to", "Pods"]);
        let pods = row.section("Pods").expect("a Pods section");
        assert_eq!(pods.rows, [DetailRow::Live(LiveContent::SelectedPods)]);
    }

    #[test]
    fn all_pods_selector_reads_all_pods() {
        let mut everything = policy(
            PolicyDirection::Allowed(Vec::new()),
            PolicyDirection::NotIsolated,
        );
        everything.pod_selector = Selector::everything();
        let row = network_policy_row(&everything);
        assert_eq!(
            row.cells[0],
            KindCell::Toned(StatusLabel {
                text: "(all pods)".into(),
                tone: StatusTone::Done,
            })
        );
        let applies = row.section("Applies to").expect("section");
        assert_eq!(applies.rows, [DetailRow::Note("All pods in team-a".into())]);
    }

    #[test]
    fn peer_text_namespace_by_name_label() {
        let by_name = PolicyPeer::Pods {
            namespaces: Some(labels(&["kubernetes.io/metadata.name=ingress"])),
            pods: None,
        };
        assert_eq!(peer_text(&by_name), "namespace ingress");
        let by_label = PolicyPeer::Pods {
            namespaces: Some(labels(&["team=jobs", "tier=batch"])),
            pods: Some(labels(&["app=worker"])),
        };
        assert_eq!(
            peer_text(&by_label),
            "pods app=worker in namespaces team=jobs, tier=batch"
        );
        let in_namespace = PolicyPeer::Pods {
            namespaces: Some(labels(&["kubernetes.io/metadata.name=jobs"])),
            pods: Some(labels(&["app=worker"])),
        };
        assert_eq!(
            peer_text(&in_namespace),
            "pods app=worker in namespace jobs"
        );
    }

    #[test]
    fn peer_text_pods_in_all_namespaces() {
        let everywhere = PolicyPeer::Pods {
            namespaces: Some(Selector::everything()),
            pods: Some(Selector::everything()),
        };
        assert_eq!(peer_text(&everywhere), "all pods in all namespaces");
        let all_namespaces = PolicyPeer::Pods {
            namespaces: Some(Selector::everything()),
            pods: None,
        };
        assert_eq!(peer_text(&all_namespaces), "all namespaces");
    }

    #[test]
    fn peer_text_pods_in_own_namespace() {
        let same_namespace = PolicyPeer::Pods {
            namespaces: None,
            pods: Some(labels(&["app=db"])),
        };
        assert_eq!(peer_text(&same_namespace), "pods app=db");
        let any_pod = PolicyPeer::Pods {
            namespaces: None,
            pods: Some(Selector::everything()),
        };
        assert_eq!(peer_text(&any_pod), "all pods in this namespace");
        // A peer with no selector at all is dropped by the cluster crate; the text is defensive.
        let neither = PolicyPeer::Pods {
            namespaces: None,
            pods: None,
        };
        assert_eq!(peer_text(&neither), "all pods in this namespace");
    }

    #[test]
    fn peer_text_ip_block_with_except() {
        let plain = PolicyPeer::IpBlock {
            cidr: "10.0.0.0/8".to_owned(),
            except: Vec::new(),
        };
        assert_eq!(peer_text(&plain), "10.0.0.0/8");
        let with_except = PolicyPeer::IpBlock {
            cidr: "10.0.0.0/8".to_owned(),
            except: vec!["10.1.0.0/16".to_owned(), "10.2.0.0/16".to_owned()],
        };
        assert_eq!(
            peer_text(&with_except),
            "10.0.0.0/8 except 10.1.0.0/16, 10.2.0.0/16"
        );
    }

    #[test]
    fn ports_text_single_range_named_and_all() {
        assert_eq!(ports_text(&[]), "all ports");
        assert_eq!(ports_text(&[port(Some("8080"), None)]), "port 8080/TCP");
        assert_eq!(ports_text(&[port(Some("http"), None)]), "port http/TCP");
        assert_eq!(
            ports_text(&[port(Some("8000"), Some(9000))]),
            "ports 8000-9000/TCP"
        );
        assert_eq!(ports_text(&[port(None, None)]), "all TCP ports");
        assert_eq!(
            ports_text(&[port(Some("80"), None), port(Some("443"), None)]),
            "ports 80/TCP, 443/TCP"
        );
    }

    #[test]
    fn deny_all_ingress_note() {
        let row = network_policy_row(&policy(
            PolicyDirection::Allowed(Vec::new()),
            PolicyDirection::NotIsolated,
        ));
        let ingress = row.section("Allow ingress from").expect("section");
        assert_eq!(
            ingress.rows,
            [DetailRow::Note(
                "Denies all ingress to the selected pods".into()
            )]
        );
    }

    #[test]
    fn not_isolated_egress_has_no_section() {
        let row = network_policy_row(&policy(
            PolicyDirection::Allowed(Vec::new()),
            PolicyDirection::NotIsolated,
        ));
        assert!(row.section("Allow egress to").is_none());
    }

    #[test]
    fn rule_without_peers_reads_any_source() {
        let row = network_policy_row(&policy(
            PolicyDirection::Allowed(vec![rule(Vec::new(), vec![port(Some("53"), None)])]),
            PolicyDirection::Allowed(vec![rule(Vec::new(), Vec::new())]),
        ));
        let ingress = row.section("Allow ingress from").expect("section");
        assert_eq!(
            ingress.rows,
            [DetailRow::stacked(
                "any source",
                KindCell::Text("port 53/TCP".into())
            )]
        );
        let egress = row.section("Allow egress to").expect("section");
        assert_eq!(
            egress.rows,
            [DetailRow::stacked(
                "any destination",
                KindCell::Text("all ports".into())
            )]
        );
    }
}
