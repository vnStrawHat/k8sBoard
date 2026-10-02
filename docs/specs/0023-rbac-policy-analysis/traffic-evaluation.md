# 0023 · Cluster crate: NetworkPolicy traffic evaluation

[Back to index](README.md) · Step 1b · Module `network_policy_traffic.rs` (new) + `network_policy_traffic_tests.rs`. Pure; reuses 0013 `NetworkPolicySummary`, `PolicyDirection`, `PolicyRule`, `PolicyPeer`, `PolicyPort`, 0012 `Selector`, 0005 `NamespaceSummary`, `ContainerPort`.

## Types

```rust
pub struct TrafficEndpoint<'a> { pub namespace: &'a str, pub labels: &'a [String] /* key-ordered terms */,
    pub ip: Option<&'a str> }   // host network is warned by the app from `PodSummary`
pub enum TrafficSource<'a> { Workload(TrafficEndpoint<'a>) /* a pod or a labels-only stand-in */, External { ip: &'a str } }
pub struct TrafficDestination<'a> { pub endpoint: TrafficEndpoint<'a>, pub ports: &'a [ContainerPort] }
pub enum RequestPort { Number(u16), Name(String) }
pub struct TrafficRequest<'a> { pub source: TrafficSource<'a>, pub destination: TrafficDestination<'a>,
    pub port: RequestPort, pub protocol: String /* TCP, UDP, SCTP */ }
pub struct PolicyInputs<'a> { pub source_policies: &'a [NetworkPolicySummary] /* source namespace */,
    pub destination_policies: &'a [NetworkPolicySummary], pub namespaces: &'a [NamespaceSummary] }

pub struct TrafficVerdict { pub egress: DirectionVerdict, pub ingress: DirectionVerdict,
    pub port: u16 /* resolved */, pub unknown_namespaces: Vec<String> /* labels needed, namespace not listed */ }
pub enum DirectionVerdict {
    NotIsolated,                                                    // no policy selects the pod for this direction
    NotApplicable,                                                  // egress of an External source
    Allowed { isolating: Vec<String>, allowing: Vec<RuleRef> },
    Denied { isolating: Vec<String> },                              // policy names
}
pub struct RuleRef { pub policy: String, pub rule: usize /* 0-based index in ingress or egress rules */,
    pub named_port: Option<String> /* the rule matched only through this named port */ }
pub enum TrafficError { UnknownPortName(String) }                  // thiserror: "the destination has no port named {0}"
impl TrafficVerdict { pub fn is_allowed(&self) -> bool; }           // neither direction Denied
pub fn evaluate_traffic(request: &TrafficRequest, inputs: &PolicyInputs) -> Result<TrafficVerdict, TrafficError>;
```

## Semantics (NetworkPolicy v1, decision 12)

1. **Port**: `Number(n)` as given; `Name(x)` → the first destination container port named `x` with the request protocol; none → `UnknownPortName`.
2. **Ingress** at the destination: policies of `destination_policies` (all in the destination namespace) whose `pod_selector` matches the destination labels and whose `ingress` is `Allowed(rules)` are **isolating**. None → `NotIsolated`. Else `Allowed` with every `(policy, rule index)` whose rule admits the source on the port; none → `Denied`.
3. **Egress** at the source: `External` → `NotApplicable`. Else the same over `source_policies` (`egress` direction), peers matched against the destination endpoint.
4. A rule admits when (`peers` empty **or** any peer matches the other end) **and** (`ports` empty **or** any port matches).
5. `is_allowed` = neither direction is `Denied`.

## Peers

| Peer | Matches endpoint `e` (policy namespace `p`) |
|---|---|
| `Pods { namespaces: None, pods: Some(s) }` | `e.namespace == p` and `s.matches(e.labels)` |
| `Pods { namespaces: Some(n), pods: None }` | `n` matches the labels of `e.namespace` |
| `Pods { namespaces: Some(n), pods: Some(s) }` | both of the above |
| `IpBlock { cidr, except }` | `e.ip` inside `cidr` and inside no `except`; no IP → no match |
| any `Pods` peer vs `External` | no match |

- Namespace labels: the `NamespaceSummary` with that name. `n.selects_everything()` needs no lookup. A needed but missing namespace → no match and its name is added to `unknown_namespaces` (once).
- CIDR: private `cidr_contains(cidr: &str, ip: &str) -> Option<bool>` with `std::net::IpAddr` and prefix masks (v4 as `u32`, v6 as `u128`); an unparsable CIDR or IP, or mixed families → `None` = no match. No new dependency.

## Ports

| Rule port | Matches resolved request port `n`, protocol `p` |
|---|---|
| protocol | rule `protocol` (TCP default from 0013) equals `p`, case-sensitive like the API |
| `port: None` | every port of that protocol |
| numeric, no `end_port` | equal to `n` |
| numeric with `end_port` | `port <= n <= end_port` |
| named `x` | the destination's container port named `x` with protocol `p` equals `n` (also for egress rules: a named port names the destination's port); recorded in `RuleRef.named_port` when no other port of the rule matched |
| named with `end_port`, or unparsable | no match |

## Not evaluated (returned as caveats by the app)

Host-network pods and endpoints without an IP are evaluated as written and warned by the app; CNI-specific CRDs (Calico, Cilium), AdminNetworkPolicy and BaselineAdminNetworkPolicy, Service IP → pod translation, secondary pod IPs, SNAT, and whether the pod listens on the port.

## Policy fetch (`network_policy.rs`)

```rust
impl ClusterConnection {
    pub async fn read_network_policies(&self, namespace: &str) -> Result<Vec<NetworkPolicySummary>, ClusterError>;
}
```

One `list_all` of the namespace with the 0013 summarizer; action `listing network policies`. Called once per distinct namespace of a check (one or two, concurrently in the app's runtime task).
