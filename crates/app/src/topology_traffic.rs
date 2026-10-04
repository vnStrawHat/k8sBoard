//! The Traffic overlay of the Topology (spec 0049): which Resources edge carries which flow, the
//! extra `Calls` edges for pairs that talk with no Resources edge, the width, tone, and label of
//! every edge, and the traffic text of every node. Pure: no GPUI context, no logging. The graph is
//! never rebuilt and no card moves; this module only reads the graph and a sample.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::rc::Rc;

use cluster::{
    MetricsError, PodSummary, TrafficEnd, TrafficMetricSource, TrafficRate, TrafficReading,
    TrafficSourceKind,
};
use gpui_kit::SharedString;

use crate::status_tone::StatusTone;
use crate::topology_graph::{
    NodeId, NodeLook, Relation, TopologyEdge, TopologyGraph, TopologyKind, TopologyNode,
    controller_of,
};
use crate::topology_layout::{GraphPoint, TopologyLayout};
use crate::topology_route::{EdgeRoute, EdgeShape};
use crate::usage_format::Measure;

/// The thinnest flow, in graph units, and how much more the busiest one adds (decision 6).
const MIN_WIDTH: f32 = 1.5;
const WIDTH_RANGE: f32 = 4.5;
/// The share of 5xx answers from which an edge turns Warn and Bad (decision 7).
const WARN_SHARE: f64 = 0.01;
const BAD_SHARE: f64 = 0.05;
/// The share of 5xx answers from which a label or a node text names it.
const TEXT_SHARE: f64 = 0.001;
/// Peers outside the namespace that are kept for the chip tooltip.
const OUTSIDE_LIMIT: usize = 20;
/// The kinds a `TrafficEnd::Workload` can name, in the order they are tried. Istio's
/// `source_workload` carries no kind, so a Deployment with the same name wins over a StatefulSet
/// or a DaemonSet.
const WORKLOAD_KINDS: [TopologyKind; 3] = [
    TopologyKind::Deployment,
    TopologyKind::StatefulSet,
    TopologyKind::DaemonSet,
];

/// One refresh: what each source answered at `at`.
pub(crate) struct TrafficSample {
    pub(crate) at: jiff::Timestamp,
    pub(crate) readings: Vec<(TrafficMetricSource, Result<TrafficReading, MetricsError>)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TrafficUnit {
    Requests,
    Bytes,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct EdgeFlow {
    /// Requests or bytes per second.
    pub(crate) rate: f64,
    pub(crate) unit: TrafficUnit,
    /// 5xx answers over requests; `None` for bytes and for a source without the errors query.
    pub(crate) error_share: Option<f64>,
    /// In graph units.
    pub(crate) width: f32,
    pub(crate) tone: Option<StatusTone>,
    pub(crate) label: Option<SharedString>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum EdgeTraffic {
    /// Mounts and Access edges are not drawn in Traffic mode.
    Hidden,
    /// No flow, or a rate of 0.
    Idle,
    Flow(EdgeFlow),
}

/// What the cards of one node show: the traffic that arrives at it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeTraffic {
    pub(crate) requests: Option<f64>,
    pub(crate) error_share: Option<f64>,
    pub(crate) receive: Option<f64>,
    pub(crate) transmit: Option<f64>,
    pub(crate) text: SharedString,
}

pub(crate) struct TrafficOverlay {
    /// `graph.edges`, then the `Calls` edges, in the same order.
    pub(crate) edges: Vec<EdgeTraffic>,
    /// One per graph node.
    pub(crate) nodes: Vec<Option<NodeTraffic>>,
    /// The sources that answered, for the chip.
    pub(crate) sources: Vec<TrafficSourceKind>,
    /// Peers that are not nodes of the graph, deduplicated, at most `OUTSIDE_LIMIT` kept.
    pub(crate) outside: Vec<String>,
    /// Failed or cut readings.
    pub(crate) notes: Vec<String>,
}

/// Where a `TrafficEnd` lands in the graph.
enum Resolved {
    Node(usize),
    /// A name that adds nothing: a host-network pod, or a pod, Service, or workload of the
    /// namespace that is not a node (ended, not delivered yet, hidden by a filter).
    Skipped,
    /// Another namespace or an unknown peer.
    Outside(String),
}

struct Resolver<'a> {
    ids: HashMap<&'a NodeId, usize>,
    pods: HashMap<&'a str, &'a PodSummary>,
}

impl<'a> Resolver<'a> {
    /// `pods` are the pods of the Topology namespace.
    fn new(graph: &'a TopologyGraph, pods: &'a [&'a PodSummary]) -> Self {
        Self {
            ids: graph
                .nodes
                .iter()
                .enumerate()
                .map(|(index, node)| (&node.id, index))
                .collect(),
            pods: pods.iter().map(|pod| (pod.name.as_str(), *pod)).collect(),
        }
    }

    fn object(&self, kind: TopologyKind, name: &str) -> Option<usize> {
        let id = NodeId::Object {
            kind,
            name: name.to_owned(),
        };
        self.ids.get(&id).copied()
    }

    /// Both queries are scoped to the Topology namespace, so a name that is not a node is a pod
    /// that just ended, an object not delivered yet, or one the filters hide: it is skipped. Only
    /// `TrafficEnd::Outside` (an Istio peer of another namespace or an unknown caller) is outside.
    fn end(&self, end: &TrafficEnd) -> Resolved {
        let found = |index: Option<usize>| index.map_or(Resolved::Skipped, Resolved::Node);
        match end {
            TrafficEnd::Workload(name) => found(
                WORKLOAD_KINDS
                    .iter()
                    .find_map(|kind| self.object(*kind, name)),
            ),
            TrafficEnd::Service(name) => found(self.object(TopologyKind::Service, name)),
            TrafficEnd::Pod(name) => self.pod(name),
            TrafficEnd::Outside(label) => Resolved::Outside(label.clone()),
        }
    }

    /// The pod's own node, else the group node that holds it.
    fn pod(&self, name: &str) -> Resolved {
        let Some(pod) = self.pods.get(name) else {
            return Resolved::Skipped;
        };
        if pod.host_network {
            return Resolved::Skipped;
        }
        if let Some(index) = self.object(TopologyKind::Pod, name) {
            return Resolved::Node(index);
        }
        let group = controller_of(pod).and_then(|(owner_kind, owner)| {
            self.ids
                .get(&NodeId::PodGroup {
                    owner_kind,
                    owner: owner.to_owned(),
                })
                .copied()
        });
        group.map_or(Resolved::Skipped, Resolved::Node)
    }
}

/// Requests and 5xx answers per second, added up over the rows of one edge or node.
#[derive(Clone, Copy)]
struct RequestSum {
    requests: f64,
    /// `None` when any row lacks the errors query.
    errors: Option<f64>,
}

impl RequestSum {
    fn of(requests: f64, errors: Option<f64>) -> Self {
        Self { requests, errors }
    }

    fn add(&mut self, requests: f64, errors: Option<f64>) {
        self.requests += requests;
        self.errors = self.errors.zip(errors).map(|(sum, more)| sum + more);
    }

    fn error_share(self) -> Option<f64> {
        let errors = self.errors?;
        (self.requests > 0.).then(|| errors / self.requests)
    }
}

fn add_request(sums: &mut BTreeMap<usize, RequestSum>, key: usize, rate: &TrafficRate) {
    let Some(requests) = rate.requests else {
        return;
    };
    sums.entry(key)
        .and_modify(|sum| sum.add(requests, rate.errors))
        .or_insert_with(|| RequestSum::of(requests, rate.errors));
}

/// What the Istio readings of a sample say, resolved to nodes.
#[derive(Default)]
struct IstioFlows {
    /// Per ordered pair of nodes.
    pairs: BTreeMap<(usize, usize), RequestSum>,
    /// Per target node: every row that reaches it, whoever the caller is.
    inbound: BTreeMap<usize, RequestSum>,
    outside: Vec<String>,
}

fn istio_flows(resolver: &Resolver, sample: &TrafficSample) -> IstioFlows {
    let mut flows = IstioFlows::default();
    for rate in ok_rates(sample, TrafficSourceKind::Istio) {
        let Some(requests) = rate.requests else {
            continue;
        };
        let to = match resolver.end(&rate.to) {
            Resolved::Node(to) => to,
            Resolved::Outside(name) => {
                push_outside(&mut flows.outside, name);
                continue;
            }
            Resolved::Skipped => continue,
        };
        add_request(&mut flows.inbound, to, rate);
        let Some(from) = &rate.from else {
            continue;
        };
        match resolver.end(from) {
            Resolved::Node(from) if from != to => {
                flows
                    .pairs
                    .entry((from, to))
                    .and_modify(|sum| sum.add(requests, rate.errors))
                    .or_insert_with(|| RequestSum::of(requests, rate.errors));
            }
            Resolved::Outside(name) => push_outside(&mut flows.outside, name),
            Resolved::Node(_) | Resolved::Skipped => {}
        }
    }
    flows
}

/// The rows of every answered reading of `kind`.
fn ok_rates(sample: &TrafficSample, kind: TrafficSourceKind) -> impl Iterator<Item = &TrafficRate> {
    sample
        .readings
        .iter()
        .filter(move |(source, _)| source.kind() == kind)
        .filter_map(|(_, reading)| reading.as_ref().ok())
        .flat_map(|reading| reading.rates.iter())
}

fn push_outside(outside: &mut Vec<String>, name: String) {
    if outside.len() < OUTSIDE_LIMIT && !outside.contains(&name) {
        outside.push(name);
    }
}

/// The Resources edges that can carry a flow, by their ends.
fn flow_edges(graph: &TopologyGraph) -> HashMap<(usize, usize), usize> {
    let mut found = HashMap::new();
    for (index, edge) in graph.edges.iter().enumerate() {
        match edge.relation {
            Relation::Owns | Relation::RoutesTo => {
                found.entry((edge.from, edge.to)).or_insert(index);
            }
            Relation::Mounts | Relation::Access | Relation::Calls => {}
        }
    }
    found
}

/// The pairs that talk with no Resources edge between their nodes in that direction, as `Calls`
/// edges sorted by `(from, to)`. `pods` are the pods of the Topology namespace.
pub(crate) fn call_edges(
    graph: &TopologyGraph,
    pods: &[&PodSummary],
    sample: &TrafficSample,
) -> Vec<TopologyEdge> {
    let existing = flow_edges(graph);
    istio_flows(&Resolver::new(graph, pods), sample)
        .pairs
        .into_keys()
        .filter(|pair| !existing.contains_key(pair))
        .map(|(from, to)| TopologyEdge {
            from,
            to,
            relation: Relation::Calls,
        })
        .collect()
}

#[derive(Clone, Copy, Default)]
struct ByteSum {
    receive: f64,
    transmit: f64,
}

/// The bytes per second of every node: the sum over the distinct pods that can be reached from it
/// along `Owns` and `RoutesTo` edges, so a pod counts once however many paths lead to it.
fn byte_sums(
    graph: &TopologyGraph,
    resolver: &Resolver,
    sample: &TrafficSample,
    outside: &mut Vec<String>,
) -> Vec<Option<ByteSum>> {
    let mut parents: Vec<Vec<usize>> = vec![Vec::new(); graph.nodes.len()];
    for edge in &graph.edges {
        let carries = match edge.relation {
            Relation::Owns | Relation::RoutesTo => true,
            Relation::Mounts | Relation::Access | Relation::Calls => false,
        };
        // A scaler is not in the data path.
        let is_scaler = graph.nodes[edge.from].kind == TopologyKind::HorizontalPodAutoscaler;
        if carries && !is_scaler {
            parents[edge.to].push(edge.from);
        }
    }
    let mut sums: Vec<Option<ByteSum>> = vec![None; graph.nodes.len()];
    let mut ancestors: HashMap<usize, Vec<usize>> = HashMap::new();
    for rate in ok_rates(sample, TrafficSourceKind::PodNetwork) {
        let pod = match resolver.end(&rate.to) {
            Resolved::Node(pod) => pod,
            Resolved::Outside(name) => {
                push_outside(outside, name);
                continue;
            }
            Resolved::Skipped => continue,
        };
        let reachable = ancestors
            .entry(pod)
            .or_insert_with(|| ancestors_of(pod, &parents));
        for node in reachable {
            let sum = sums[*node].get_or_insert_with(ByteSum::default);
            sum.receive += rate.receive.unwrap_or(0.);
            sum.transmit += rate.transmit.unwrap_or(0.);
        }
    }
    sums
}

/// `node` and every node that reaches it.
fn ancestors_of(node: usize, parents: &[Vec<usize>]) -> Vec<usize> {
    let mut seen = BTreeSet::from([node]);
    let mut pending = vec![node];
    while let Some(next) = pending.pop() {
        for parent in &parents[next] {
            if seen.insert(*parent) {
                pending.push(*parent);
            }
        }
    }
    seen.into_iter().collect()
}

/// An edge before widths: what flows on it.
enum RawEdge {
    Hidden,
    Idle,
    Flow {
        rate: f64,
        unit: TrafficUnit,
        error_share: Option<f64>,
        relation: Relation,
    },
}

fn raw_edge(
    graph: &TopologyGraph,
    edge: &TopologyEdge,
    istio: &IstioFlows,
    bytes: &[Option<ByteSum>],
) -> RawEdge {
    match edge.relation {
        Relation::Mounts | Relation::Access => return RawEdge::Hidden,
        Relation::Owns | Relation::RoutesTo | Relation::Calls => {}
    }
    let flow = |rate: f64, unit, error_share| {
        if rate > 0. {
            RawEdge::Flow {
                rate,
                unit,
                error_share,
                relation: edge.relation,
            }
        } else {
            RawEdge::Idle
        }
    };
    // Istio wins over the bytes of the same edge.
    if let Some(sum) = istio.pairs.get(&(edge.from, edge.to)) {
        return flow(sum.requests, TrafficUnit::Requests, sum.error_share());
    }
    let is_scaler = graph.nodes[edge.from].kind == TopologyKind::HorizontalPodAutoscaler;
    match bytes[edge.to] {
        Some(sum) if edge.relation != Relation::Calls && !is_scaler => {
            flow(sum.receive, TrafficUnit::Bytes, None)
        }
        Some(_) | None => RawEdge::Idle,
    }
}

/// The overlay of `sample`: `calls` are the edges `call_edges` made for it (or for a sample with
/// the same pairs). `pods` are the pods of the Topology namespace.
pub(crate) fn traffic_overlay(
    graph: &TopologyGraph,
    calls: &[TopologyEdge],
    pods: &[&PodSummary],
    sample: &TrafficSample,
) -> TrafficOverlay {
    let resolver = Resolver::new(graph, pods);
    let istio = istio_flows(&resolver, sample);
    let mut outside = istio.outside.clone();
    let bytes = byte_sums(graph, &resolver, sample, &mut outside);
    let raw: Vec<RawEdge> = graph
        .edges
        .iter()
        .chain(calls)
        .map(|edge| raw_edge(graph, edge, &istio, &bytes))
        .collect();
    let max_of = |wanted: TrafficUnit| {
        raw.iter()
            .filter_map(|edge| match edge {
                RawEdge::Flow { rate, unit, .. } if *unit == wanted => Some(*rate),
                RawEdge::Flow { .. } | RawEdge::Hidden | RawEdge::Idle => None,
            })
            .fold(0., f64::max)
    };
    let (max_requests, max_bytes) = (max_of(TrafficUnit::Requests), max_of(TrafficUnit::Bytes));
    let edges = raw
        .into_iter()
        .map(|edge| match edge {
            RawEdge::Hidden => EdgeTraffic::Hidden,
            RawEdge::Idle => EdgeTraffic::Idle,
            RawEdge::Flow {
                rate,
                unit,
                error_share,
                relation,
            } => {
                let max = match unit {
                    TrafficUnit::Requests => max_requests,
                    TrafficUnit::Bytes => max_bytes,
                };
                EdgeTraffic::Flow(EdgeFlow {
                    rate,
                    unit,
                    error_share,
                    width: flow_width(rate, max),
                    tone: error_share.and_then(error_tone),
                    label: flow_label(rate, unit, error_share, relation),
                })
            }
        })
        .collect();
    TrafficOverlay {
        edges,
        nodes: node_traffic(graph, &istio, &bytes),
        sources: answered_sources(sample),
        outside,
        notes: sample_notes(sample),
    }
}

/// `1.5 + 4.5 × √(rate / max)`.
fn flow_width(rate: f64, max: f64) -> f32 {
    if max <= 0. {
        return MIN_WIDTH;
    }
    MIN_WIDTH + WIDTH_RANGE * (rate / max).sqrt() as f32
}

fn error_tone(share: f64) -> Option<StatusTone> {
    if share >= BAD_SHARE {
        Some(StatusTone::Bad)
    } else if share >= WARN_SHARE {
        Some(StatusTone::Warn)
    } else {
        None
    }
}

/// Requests: `35 req/s · 6% 5xx`. Bytes: `1.2 MB/s`, only on `RoutesTo` edges.
fn flow_label(
    rate: f64,
    unit: TrafficUnit,
    error_share: Option<f64>,
    relation: Relation,
) -> Option<SharedString> {
    match unit {
        TrafficUnit::Requests => Some(request_text(rate, error_share).into()),
        TrafficUnit::Bytes => match relation {
            Relation::RoutesTo => Some(Measure::Rate.format(rate).into()),
            Relation::Owns | Relation::Mounts | Relation::Access | Relation::Calls => None,
        },
    }
}

/// `35 req/s`, one decimal below 10, plus the 5xx share when it is at least 0.1 %.
fn request_text(rate: f64, error_share: Option<f64>) -> String {
    let mut text = format!("{} req/s", compact(rate));
    if let Some(share) = error_share.filter(|share| *share >= TEXT_SHARE) {
        text.push_str(&format!(" \u{b7} {}% 5xx", percent(share)));
    }
    text
}

/// One decimal below 10, whole numbers from there.
fn compact(value: f64) -> String {
    if value < 10. {
        format!("{value:.1}")
    } else {
        format!("{value:.0}")
    }
}

/// A percentage of a share, to a tenth, without a trailing `.0`: `6`, `0.4`, `12`.
fn percent(share: f64) -> String {
    let tenths = (share * 1_000.).round() / 10.;
    if tenths.fract() == 0. {
        format!("{tenths:.0}")
    } else {
        format!("{tenths:.1}")
    }
}

fn node_traffic(
    graph: &TopologyGraph,
    istio: &IstioFlows,
    bytes: &[Option<ByteSum>],
) -> Vec<Option<NodeTraffic>> {
    graph
        .nodes
        .iter()
        .enumerate()
        .map(|(index, node)| {
            if node.look != NodeLook::Plain {
                return None;
            }
            let requests = istio.inbound.get(&index).copied();
            let bytes = bytes[index];
            let text = match (requests, bytes) {
                (Some(sum), _) => request_text(sum.requests, sum.error_share()),
                (None, Some(sum)) => format!(
                    "\u{2193} {}  \u{2191} {}",
                    Measure::Rate.format(sum.receive),
                    Measure::Rate.format(sum.transmit)
                ),
                (None, None) => return None,
            };
            Some(NodeTraffic {
                requests: requests.map(|sum| sum.requests),
                error_share: requests.and_then(RequestSum::error_share),
                receive: bytes.map(|sum| sum.receive),
                transmit: bytes.map(|sum| sum.transmit),
                text: text.into(),
            })
        })
        .collect()
}

fn answered_sources(sample: &TrafficSample) -> Vec<TrafficSourceKind> {
    let answered: BTreeSet<TrafficSourceKind> = sample
        .readings
        .iter()
        .filter(|(_, reading)| reading.is_ok())
        .map(|(source, _)| source.kind())
        .collect();
    answered.into_iter().collect()
}

fn sample_notes(sample: &TrafficSample) -> Vec<String> {
    let mut notes = Vec::new();
    for (source, reading) in &sample.readings {
        let label = source.kind().label();
        match reading {
            Ok(reading) if reading.was_cut => {
                notes.push(format!(
                    "{label}: more than 2,000 series, the first 2,000 are read"
                ));
            }
            Ok(_) => {}
            Err(error) => notes.push(format!("{label}: {error}")),
        }
    }
    notes
}

/// The Traffic layer of the graph shown: the sample, the `Calls` edges it made with their routes,
/// and what every edge and node carries. Derived from the graph, its layout, and the sample; kept
/// beside the layout, which it never changes.
pub(crate) struct TrafficLayer {
    pub(crate) sample: Rc<TrafficSample>,
    /// Beyond `graph.edges`, in the order of `overlay.edges` after them.
    pub(crate) calls: Vec<TopologyEdge>,
    pub(crate) call_routes: Vec<EdgeRoute>,
    pub(crate) overlay: Rc<TrafficOverlay>,
}

impl TrafficLayer {
    /// `pods` are the pods of the Topology namespace. The layout is not computed again: the
    /// `Calls` edges are routed over its cards.
    pub(crate) fn build(
        graph: &TopologyGraph,
        layout: &TopologyLayout,
        shape: EdgeShape,
        pods: &[&PodSummary],
        sample: Rc<TrafficSample>,
    ) -> Self {
        let calls = call_edges(graph, pods, &sample);
        let call_routes = layout.route_extra(&calls, shape);
        let overlay = Rc::new(traffic_overlay(graph, &calls, pods, &sample));
        Self {
            sample,
            calls,
            call_routes,
            overlay,
        }
    }

    /// The cards moved (a drag) or the shape changed: only the routes of the `Calls` edges change.
    pub(crate) fn rerouted(&self, layout: &TopologyLayout, shape: EdgeShape) -> Self {
        Self {
            sample: Rc::clone(&self.sample),
            calls: self.calls.clone(),
            call_routes: layout.route_extra(&self.calls, shape),
            overlay: Rc::clone(&self.overlay),
        }
    }
}

/// The caption a card shows: the traffic text, unless the node is Warn or Bad, whose caption
/// stays visible (decision 11).
pub(crate) fn shown_caption(node: &TopologyNode, traffic: Option<&NodeTraffic>) -> SharedString {
    let keeps_caption = match node.tone {
        Some(StatusTone::Warn | StatusTone::Bad) => true,
        Some(StatusTone::Ok | StatusTone::Info | StatusTone::Done) | None => false,
    };
    match traffic {
        Some(traffic) if !keeps_caption => traffic.text.clone(),
        Some(_) | None => node.caption.clone(),
    }
}

/// The tooltip of a card: `base`, then the traffic text on its own line.
pub(crate) fn tooltip_with_traffic(
    base: &SharedString,
    traffic: Option<&NodeTraffic>,
) -> SharedString {
    match traffic {
        Some(traffic) => format!("{base}\n{}", traffic.text).into(),
        None => base.clone(),
    }
}

/// The point at half the arc length of the route: the label of a curve and of an elbow both sit
/// on the line.
pub(crate) fn label_anchor(route: &EdgeRoute) -> GraphPoint {
    point_along(route, 0.5)
}

/// The point at `share` (0 to 1) of the arc length of the route.
pub(crate) fn point_along(route: &EdgeRoute, share: f32) -> GraphPoint {
    let length = |pair: &[GraphPoint]| (pair[1].x - pair[0].x).hypot(pair[1].y - pair[0].y);
    let wanted = route.points.windows(2).map(length).sum::<f32>() * share;
    let mut walked = 0.;
    for pair in route.points.windows(2) {
        let segment = length(pair);
        if segment > 0. && walked + segment >= wanted {
            let part = (wanted - walked) / segment;
            return GraphPoint {
                x: pair[0].x + (pair[1].x - pair[0].x) * part,
                y: pair[0].y + (pair[1].y - pair[0].y) * part,
            };
        }
        walked += segment;
    }
    route.start()
}

#[cfg(test)]
#[path = "topology_traffic_tests.rs"]
mod topology_traffic_tests;
