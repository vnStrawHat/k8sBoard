//! Client-side NetworkPolicy evaluation: would a connection from a source to a destination pod
//! on a port be allowed? Pure: it works on policy summaries and never talks to the cluster.

use std::net::IpAddr;

use crate::namespace::NamespaceSummary;
use crate::network_policy::{
    NetworkPolicySummary, PolicyDirection, PolicyPeer, PolicyPort, PolicyRule,
};
use crate::workload::ContainerPort;

/// A pod, or a stand-in made of namespace and labels, on one end of a connection.
#[derive(Clone, Copy, Debug)]
pub struct TrafficEndpoint<'a> {
    pub namespace: &'a str,
    /// `key=value` terms in key order.
    pub labels: &'a [String],
    pub ip: Option<&'a str>,
}

#[derive(Clone, Copy, Debug)]
pub enum TrafficSource<'a> {
    Workload(TrafficEndpoint<'a>),
    /// Outside the cluster: only `ipBlock` peers can match it, and its egress is not checked.
    External {
        ip: &'a str,
    },
}

#[derive(Clone, Copy, Debug)]
pub struct TrafficDestination<'a> {
    pub endpoint: TrafficEndpoint<'a>,
    /// The destination's container ports; named ports resolve against them.
    pub ports: &'a [ContainerPort],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RequestPort {
    Number(u16),
    Name(String),
}

#[derive(Clone, Debug)]
pub struct TrafficRequest<'a> {
    pub source: TrafficSource<'a>,
    pub destination: TrafficDestination<'a>,
    pub port: RequestPort,
    /// `TCP`, `UDP`, or `SCTP`.
    pub protocol: String,
}

#[derive(Clone, Copy, Debug)]
pub struct PolicyInputs<'a> {
    /// The policies of the source's namespace.
    pub source_policies: &'a [NetworkPolicySummary],
    /// The policies of the destination's namespace.
    pub destination_policies: &'a [NetworkPolicySummary],
    /// Namespace labels for namespace selectors.
    pub namespaces: &'a [NamespaceSummary],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrafficVerdict {
    pub egress: DirectionVerdict,
    pub ingress: DirectionVerdict,
    /// The request port after resolving a name.
    pub port: u16,
    /// Namespaces whose labels a namespace selector needed but the namespaces list lacked.
    pub unknown_namespaces: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DirectionVerdict {
    /// No policy selects the pod for this direction.
    NotIsolated,
    /// The egress of an external source.
    NotApplicable,
    Allowed {
        /// Policy names.
        isolating: Vec<String>,
        allowing: Vec<RuleRef>,
    },
    Denied {
        /// Policy names.
        isolating: Vec<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuleRef {
    pub policy: String,
    /// 0-based index in the policy's ingress or egress rules.
    pub rule: usize,
    /// Set when the rule matched only through this named port.
    pub named_port: Option<String>,
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum TrafficError {
    #[error("the destination has no port named {0}")]
    UnknownPortName(String),
}

impl TrafficVerdict {
    /// Neither direction is denied.
    pub fn is_allowed(&self) -> bool {
        let is_denied =
            |verdict: &DirectionVerdict| matches!(verdict, DirectionVerdict::Denied { .. });
        !is_denied(&self.egress) && !is_denied(&self.ingress)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Direction {
    Ingress,
    Egress,
}

/// The far end of a rule's peers.
#[derive(Clone, Copy)]
enum Remote<'a> {
    Pod(TrafficEndpoint<'a>),
    External(&'a str),
}

impl Remote<'_> {
    fn ip(&self) -> Option<&str> {
        match self {
            Self::Pod(endpoint) => endpoint.ip,
            Self::External(ip) => Some(ip),
        }
    }
}

/// How a rule's ports matched the request.
#[derive(PartialEq, Eq)]
enum PortMatch {
    No,
    Number,
    Named(String),
}

struct Evaluation<'a> {
    port: u16,
    protocol: &'a str,
    destination_ports: &'a [ContainerPort],
    namespaces: &'a [NamespaceSummary],
    unknown_namespaces: Vec<String>,
}

/// NetworkPolicy v1 semantics: a direction is isolated when a policy of the pod's namespace
/// selects the pod for it; then the union of the rules decides. Traffic is allowed only when
/// the source's egress and the destination's ingress both allow it.
pub fn evaluate_traffic(
    request: &TrafficRequest,
    inputs: &PolicyInputs,
) -> Result<TrafficVerdict, TrafficError> {
    let destination = &request.destination;
    let port = resolve_port(request)?;
    let mut evaluation = Evaluation {
        port,
        protocol: &request.protocol,
        destination_ports: destination.ports,
        namespaces: inputs.namespaces,
        unknown_namespaces: Vec::new(),
    };
    let egress = match request.source {
        TrafficSource::External { .. } => DirectionVerdict::NotApplicable,
        TrafficSource::Workload(source) => direction_verdict(
            inputs.source_policies,
            Direction::Egress,
            &source,
            Remote::Pod(destination.endpoint),
            &mut evaluation,
        ),
    };
    let source_remote = match request.source {
        TrafficSource::Workload(source) => Remote::Pod(source),
        TrafficSource::External { ip } => Remote::External(ip),
    };
    let ingress = direction_verdict(
        inputs.destination_policies,
        Direction::Ingress,
        &destination.endpoint,
        source_remote,
        &mut evaluation,
    );
    Ok(TrafficVerdict {
        egress,
        ingress,
        port,
        unknown_namespaces: evaluation.unknown_namespaces,
    })
}

fn resolve_port(request: &TrafficRequest) -> Result<u16, TrafficError> {
    match &request.port {
        RequestPort::Number(number) => Ok(*number),
        RequestPort::Name(name) => request
            .destination
            .ports
            .iter()
            .find(|port| port.name.as_deref() == Some(name) && port.protocol == request.protocol)
            .map(|port| port.port)
            .ok_or_else(|| TrafficError::UnknownPortName(name.clone())),
    }
}

fn direction_verdict(
    policies: &[NetworkPolicySummary],
    direction: Direction,
    local: &TrafficEndpoint,
    remote: Remote,
    evaluation: &mut Evaluation,
) -> DirectionVerdict {
    let isolating: Vec<(&NetworkPolicySummary, &[PolicyRule])> = policies
        .iter()
        .filter(|policy| {
            policy.namespace == local.namespace && policy.pod_selector.matches(local.labels)
        })
        .filter_map(|policy| match direction_of(policy, direction) {
            PolicyDirection::NotIsolated => None,
            PolicyDirection::Allowed(rules) => Some((policy, rules.as_slice())),
        })
        .collect();
    if isolating.is_empty() {
        return DirectionVerdict::NotIsolated;
    }
    let mut allowing = Vec::new();
    for (policy, rules) in &isolating {
        for (index, rule) in rules.iter().enumerate() {
            let Some(named_port) = admits(rule, policy, remote, evaluation) else {
                continue;
            };
            allowing.push(RuleRef {
                policy: policy.name.clone(),
                rule: index,
                named_port,
            });
        }
    }
    let isolating = isolating
        .into_iter()
        .map(|(policy, _)| policy.name.clone())
        .collect();
    if allowing.is_empty() {
        DirectionVerdict::Denied { isolating }
    } else {
        DirectionVerdict::Allowed {
            isolating,
            allowing,
        }
    }
}

fn direction_of(policy: &NetworkPolicySummary, direction: Direction) -> &PolicyDirection {
    match direction {
        Direction::Ingress => &policy.ingress,
        Direction::Egress => &policy.egress,
    }
}

/// `Some` when the rule admits the traffic; the inner value is the named port it matched
/// through, when no other port of the rule matched.
fn admits(
    rule: &PolicyRule,
    policy: &NetworkPolicySummary,
    remote: Remote,
    evaluation: &mut Evaluation,
) -> Option<Option<String>> {
    let is_peer_match = rule.peers.is_empty()
        || rule
            .peers
            .iter()
            .any(|peer| peer_matches(peer, &policy.namespace, remote, evaluation));
    if !is_peer_match {
        return None;
    }
    if rule.ports.is_empty() {
        return Some(None);
    }
    let mut named = None;
    for port in &rule.ports {
        match port_match(port, evaluation) {
            PortMatch::No => {}
            PortMatch::Number => return Some(None),
            PortMatch::Named(name) => {
                named.get_or_insert(name);
            }
        }
    }
    named.map(Some)
}

fn peer_matches(
    peer: &PolicyPeer,
    policy_namespace: &str,
    remote: Remote,
    evaluation: &mut Evaluation,
) -> bool {
    match peer {
        PolicyPeer::IpBlock { cidr, except } => {
            let Some(ip) = remote.ip() else {
                return false;
            };
            cidr_contains(cidr, ip) == Some(true)
                && !except
                    .iter()
                    .any(|excluded| cidr_contains(excluded, ip) == Some(true))
        }
        PolicyPeer::Pods { namespaces, pods } => {
            let Remote::Pod(endpoint) = remote else {
                return false;
            };
            let is_namespace_match = match namespaces {
                None => endpoint.namespace == policy_namespace,
                Some(selector) if selector.selects_everything() => true,
                Some(selector) => {
                    let Some(namespace) = evaluation
                        .namespaces
                        .iter()
                        .find(|namespace| namespace.name == endpoint.namespace)
                    else {
                        let name = endpoint.namespace.to_owned();
                        if !evaluation.unknown_namespaces.contains(&name) {
                            evaluation.unknown_namespaces.push(name);
                        }
                        return false;
                    };
                    selector.matches(&namespace.labels)
                }
            };
            is_namespace_match
                && pods
                    .as_ref()
                    .is_none_or(|selector| selector.matches(endpoint.labels))
        }
    }
}

fn port_match(port: &PolicyPort, evaluation: &Evaluation) -> PortMatch {
    if port.protocol != evaluation.protocol {
        return PortMatch::No;
    }
    let Some(text) = &port.port else {
        return PortMatch::Number;
    };
    if let Ok(start) = text.parse::<u16>() {
        let end = port.end_port.unwrap_or(start);
        return if (start..=end).contains(&evaluation.port) {
            PortMatch::Number
        } else {
            PortMatch::No
        };
    }
    // A named port names the destination's port, also in egress rules.
    let is_named_match = port.end_port.is_none()
        && evaluation.destination_ports.iter().any(|container_port| {
            container_port.name.as_deref() == Some(text.as_str())
                && container_port.protocol == evaluation.protocol
                && container_port.port == evaluation.port
        });
    if is_named_match {
        PortMatch::Named(text.clone())
    } else {
        PortMatch::No
    }
}

/// `None` for an unparsable CIDR or address, or mixed address families: no match.
fn cidr_contains(cidr: &str, ip: &str) -> Option<bool> {
    let (network, length) = cidr.split_once('/')?;
    let length: u32 = length.parse().ok()?;
    let network: IpAddr = network.parse().ok()?;
    let ip: IpAddr = ip.parse().ok()?;
    match (network, ip) {
        (IpAddr::V4(network), IpAddr::V4(ip)) => {
            if length > 32 {
                return None;
            }
            let mask = u32::MAX.checked_shl(32 - length).unwrap_or(0);
            Some(u32::from(network) & mask == u32::from(ip) & mask)
        }
        (IpAddr::V6(network), IpAddr::V6(ip)) => {
            if length > 128 {
                return None;
            }
            let mask = u128::MAX.checked_shl(128 - length).unwrap_or(0);
            Some(u128::from(network) & mask == u128::from(ip) & mask)
        }
        _ => None,
    }
}

#[cfg(test)]
#[path = "network_policy_traffic_tests.rs"]
mod network_policy_traffic_tests;
