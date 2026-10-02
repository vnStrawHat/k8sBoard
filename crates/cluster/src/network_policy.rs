use futures::Stream;
use k8s_openapi::api::networking::v1::{
    NetworkPolicy, NetworkPolicyPeer, NetworkPolicyPort, NetworkPolicySpec,
};

use crate::connection::ClusterConnection;
use crate::namespace::NamespaceScope;
use crate::resource_watch::{WatchUpdate, summary_watch};
use crate::selector::Selector;
use crate::workload::{int_or_string_text, label_terms, non_empty};

const INGRESS: &str = "Ingress";
const EGRESS: &str = "Egress";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetworkPolicySummary {
    pub namespace: String,
    pub name: String,
    pub created_at: Option<jiff::Timestamp>,
    /// `key=value` terms in key order.
    pub labels: Vec<String>,
    /// An absent `spec.podSelector` selects every pod of the namespace.
    pub pod_selector: Selector,
    pub ingress: PolicyDirection,
    pub egress: PolicyDirection,
}

/// What a policy does to one traffic direction of the selected pods.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PolicyDirection {
    NotIsolated,
    /// `Allowed(vec![])` denies all traffic of the direction.
    Allowed(Vec<PolicyRule>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PolicyRule {
    /// Empty means any peer.
    pub peers: Vec<PolicyPeer>,
    /// Empty means all ports.
    pub ports: Vec<PolicyPort>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PolicyPeer {
    /// `namespaces: None` is the policy's own namespace; `pods: None` is every pod there.
    Pods {
        namespaces: Option<Selector>,
        pods: Option<Selector>,
    },
    IpBlock {
        cidr: String,
        except: Vec<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PolicyPort {
    /// Defaults to `TCP`.
    pub protocol: String,
    /// A number or a container port name.
    pub port: Option<String>,
    pub end_port: Option<u16>,
}

impl ClusterConnection {
    /// Watches network policies in `scope`. Yields batched snapshots ordered by
    /// (namespace, name).
    pub fn watch_network_policies(
        &self,
        scope: NamespaceScope,
    ) -> impl Stream<Item = WatchUpdate<NetworkPolicySummary>> + Send + 'static {
        summary_watch(
            self,
            self.scoped_apis(&scope),
            "watching network policies",
            network_policy_summary,
        )
    }
}

pub(crate) fn network_policy_summary(policy: &NetworkPolicy) -> NetworkPolicySummary {
    let spec = policy.spec.as_ref();
    let pod_selector = spec
        .and_then(|spec| spec.pod_selector.as_ref())
        .map_or_else(Selector::everything, Selector::of);
    NetworkPolicySummary {
        namespace: policy.metadata.namespace.clone().unwrap_or_default(),
        name: policy.metadata.name.clone().unwrap_or_default(),
        created_at: policy
            .metadata
            .creation_timestamp
            .as_ref()
            .map(|time| time.0),
        labels: label_terms(&policy.metadata),
        pod_selector,
        ingress: direction(spec, INGRESS),
        egress: direction(spec, EGRESS),
    }
}

/// Whether the policy isolates `direction`, by the API defaulting rule: with no
/// `policyTypes`, Ingress is always isolated and Egress only when egress rules exist.
fn is_isolated(spec: &NetworkPolicySpec, direction: &str) -> bool {
    match spec.policy_types.as_deref() {
        Some(types) if !types.is_empty() => types.iter().any(|name| name == direction),
        _ => direction == INGRESS || spec.egress.as_ref().is_some_and(|rules| !rules.is_empty()),
    }
}

fn direction(spec: Option<&NetworkPolicySpec>, direction: &str) -> PolicyDirection {
    let Some(spec) = spec.filter(|spec| is_isolated(spec, direction)) else {
        return PolicyDirection::NotIsolated;
    };
    let rules: Vec<_> = if direction == INGRESS {
        spec.ingress
            .iter()
            .flatten()
            .map(|rule| (&rule.from, &rule.ports))
            .collect()
    } else {
        spec.egress
            .iter()
            .flatten()
            .map(|rule| (&rule.to, &rule.ports))
            .collect()
    };
    PolicyDirection::Allowed(
        rules
            .into_iter()
            .map(|(peers, ports)| PolicyRule {
                peers: peers.iter().flatten().filter_map(peer).collect(),
                ports: ports.iter().flatten().map(port).collect(),
            })
            .collect(),
    )
}

/// A peer with neither a selector nor an `ipBlock` matches nothing definable and is skipped.
fn peer(peer: &NetworkPolicyPeer) -> Option<PolicyPeer> {
    if let Some(block) = &peer.ip_block {
        return Some(PolicyPeer::IpBlock {
            cidr: block.cidr.clone(),
            except: block.except.clone().unwrap_or_default(),
        });
    }
    if peer.namespace_selector.is_none() && peer.pod_selector.is_none() {
        return None;
    }
    Some(PolicyPeer::Pods {
        namespaces: peer.namespace_selector.as_ref().map(Selector::of),
        pods: peer.pod_selector.as_ref().map(Selector::of),
    })
}

fn port(port: &NetworkPolicyPort) -> PolicyPort {
    PolicyPort {
        protocol: non_empty(port.protocol.as_deref()).unwrap_or_else(|| "TCP".to_owned()),
        port: port.port.as_ref().map(int_or_string_text),
        end_port: port.end_port.and_then(|port| u16::try_from(port).ok()),
    }
}

#[cfg(test)]
mod tests {
    use k8s_openapi::api::networking::v1::{
        IPBlock, NetworkPolicyEgressRule, NetworkPolicyIngressRule,
    };
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::LabelSelector;
    use k8s_openapi::apimachinery::pkg::util::intstr::IntOrString;

    use super::*;

    fn policy(spec: NetworkPolicySpec) -> NetworkPolicy {
        NetworkPolicy {
            spec: Some(spec),
            ..Default::default()
        }
    }

    fn match_labels(pairs: &[(&str, &str)]) -> LabelSelector {
        LabelSelector {
            match_labels: Some(
                pairs
                    .iter()
                    .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
                    .collect(),
            ),
            ..Default::default()
        }
    }

    #[test]
    fn absent_pod_selector_selects_everything() {
        let summary = network_policy_summary(&policy(NetworkPolicySpec::default()));
        assert!(summary.pod_selector.selects_everything());
    }

    #[test]
    fn default_types_isolate_ingress_and_egress_with_rules() {
        let ingress_only = network_policy_summary(&policy(NetworkPolicySpec::default()));
        assert_eq!(ingress_only.ingress, PolicyDirection::Allowed(Vec::new()));
        assert_eq!(ingress_only.egress, PolicyDirection::NotIsolated);

        let with_egress = network_policy_summary(&policy(NetworkPolicySpec {
            egress: Some(vec![NetworkPolicyEgressRule::default()]),
            ..Default::default()
        }));
        let open_rule = PolicyRule {
            peers: Vec::new(),
            ports: Vec::new(),
        };
        assert_eq!(
            with_egress.egress,
            PolicyDirection::Allowed(vec![open_rule])
        );
    }

    #[test]
    fn listed_type_without_rules_denies_all() {
        let summary = network_policy_summary(&policy(NetworkPolicySpec {
            policy_types: Some(vec![EGRESS.to_owned()]),
            ..Default::default()
        }));
        assert_eq!(summary.egress, PolicyDirection::Allowed(Vec::new()));
        assert_eq!(summary.ingress, PolicyDirection::NotIsolated);
    }

    #[test]
    fn peers_map_pods_namespaces_and_ip_blocks() {
        let summary = network_policy_summary(&policy(NetworkPolicySpec {
            ingress: Some(vec![NetworkPolicyIngressRule {
                from: Some(vec![
                    NetworkPolicyPeer {
                        pod_selector: Some(match_labels(&[("app", "worker")])),
                        namespace_selector: Some(match_labels(&[("team", "jobs")])),
                        ..Default::default()
                    },
                    NetworkPolicyPeer {
                        ip_block: Some(IPBlock {
                            cidr: "10.0.0.0/8".to_owned(),
                            except: Some(vec!["10.1.0.0/16".to_owned()]),
                        }),
                        ..Default::default()
                    },
                    NetworkPolicyPeer::default(),
                ]),
                ports: None,
            }]),
            ..Default::default()
        }));
        let PolicyDirection::Allowed(rules) = summary.ingress else {
            panic!("ingress must be isolated");
        };
        let [rule] = rules.as_slice() else {
            panic!("one rule");
        };
        // The empty peer is skipped.
        assert_eq!(rule.peers.len(), 2);
        let PolicyPeer::Pods { namespaces, pods } = &rule.peers[0] else {
            panic!("pods peer");
        };
        assert_eq!(
            pods.as_ref().map(Selector::terms),
            Some(vec!["app=worker".to_owned()])
        );
        assert_eq!(
            namespaces.as_ref().map(Selector::terms),
            Some(vec!["team=jobs".to_owned()])
        );
        assert_eq!(
            rule.peers[1],
            PolicyPeer::IpBlock {
                cidr: "10.0.0.0/8".to_owned(),
                except: vec!["10.1.0.0/16".to_owned()],
            }
        );
    }

    #[test]
    fn port_defaults_to_tcp_and_keeps_range() {
        let range = port(&NetworkPolicyPort {
            port: Some(IntOrString::Int(8000)),
            end_port: Some(9000),
            protocol: None,
        });
        assert_eq!(
            range,
            PolicyPort {
                protocol: "TCP".to_owned(),
                port: Some("8000".to_owned()),
                end_port: Some(9000),
            }
        );
        let named = port(&NetworkPolicyPort {
            port: Some(IntOrString::String("http".to_owned())),
            protocol: Some("UDP".to_owned()),
            end_port: None,
        });
        assert_eq!(named.port.as_deref(), Some("http"));
        assert_eq!(named.protocol, "UDP");
    }
}
