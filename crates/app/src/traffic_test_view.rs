//! The Test traffic dialog: would a connection from a pod, a set of labels, or an IP address to a
//! destination pod on a port be allowed? The NetworkPolicies of the namespaces involved are
//! listed on the runtime, then evaluated on the GPUI thread (`cluster::evaluate_traffic`).

use std::net::IpAddr;

use cluster::{
    ClusterError, ContainerKind, ContainerPort, DirectionVerdict, NamespaceSummary,
    NetworkPolicySummary, PodSummary, PolicyDirection, PolicyInputs, RequestPort,
    TrafficDestination, TrafficEndpoint, TrafficError, TrafficRequest, TrafficSource,
    TrafficVerdict, evaluate_traffic,
};
use futures::future::try_join_all;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::select::{Select, SelectState};
use gpui_kit::component::{
    ActiveTheme as _, IndexPath, Sizable as _, StyledExt as _, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _,
    Subscription, WeakEntity, Window, div, px,
};

use crate::cluster_runtime::ClusterRuntime;
use crate::cluster_session::{ClusterSession, error_text};
use crate::network_policy_rows::{peer_text, ports_text};
use crate::permissions_view::RequestState;
use crate::resource_kind::ResourceKind;
use crate::status_tone::{StatusTone, tone_color};
use crate::table_selection::{DialogOrigin, ResourceKey};
use crate::who_can_view::clock_text;

/// The policies of each listed namespace, as `(namespace, policies)`.
type ListedPolicies = Vec<(String, Vec<NetworkPolicySummary>)>;

const PROTOCOLS: [&str; 3] = ["TCP", "UDP", "SCTP"];
const DEFAULT_PORT: &str = "80";
const CAVEATS: &str = "Not covered: CNI-specific policies (Calico, Cilium), AdminNetworkPolicy, Services, and whether the pod listens on the port. ipBlock rules against pod IPs depend on the network plugin.";

/// The defaults the dialog opens with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TrafficForm {
    pub(crate) source: Option<ResourceKey>,
    pub(crate) destination: Option<ResourceKey>,
    pub(crate) port: String,
}

/// Destination: the first pod (by namespace, name) the policy selects, else the first pod.
/// Source: the first other pod of the destination's namespace, else the first other pod. Port:
/// the destination's first main-container port, else 80.
pub(crate) fn traffic_defaults(
    pods: &[PodSummary],
    policy: Option<&NetworkPolicySummary>,
) -> TrafficForm {
    let mut sorted: Vec<&PodSummary> = pods.iter().collect();
    sorted.sort_by(|a, b| (&a.namespace, &a.name).cmp(&(&b.namespace, &b.name)));
    let destination = policy
        .and_then(|policy| {
            sorted.iter().copied().find(|pod| {
                pod.namespace == policy.namespace && policy.pod_selector.matches(&pod.labels)
            })
        })
        .or_else(|| sorted.first().copied());
    let is_other = |pod: &&PodSummary| {
        destination.is_none_or(|dest| (&pod.namespace, &pod.name) != (&dest.namespace, &dest.name))
    };
    let source = destination.and_then(|dest| {
        let others = || sorted.iter().copied().filter(is_other);
        others()
            .find(|pod| pod.namespace == dest.namespace)
            .or_else(|| others().next())
    });
    let port = destination
        .and_then(|dest| {
            dest.containers
                .iter()
                .filter(|container| container.kind == ContainerKind::Main)
                .flat_map(|container| &container.ports)
                .map(|port| port.port.to_string())
                .next()
        })
        .unwrap_or_else(|| DEFAULT_PORT.to_owned());
    TrafficForm {
        source: source.map(ResourceKey::of_pod),
        destination: destination.map(ResourceKey::of_pod),
        port,
    }
}

/// `app=web, tier=front` as trimmed `key=value` terms in key order, as pod labels are kept.
/// `None` for a term without `=`, with an empty key, or for a key given twice: a stand-in with
/// such labels would silently match something else than what was typed.
fn label_terms(text: &str) -> Option<Vec<String>> {
    let mut pairs: Vec<(&str, &str)> = Vec::new();
    for term in text
        .split(',')
        .map(str::trim)
        .filter(|term| !term.is_empty())
    {
        let (key, value) = term.split_once('=')?;
        let (key, value) = (key.trim(), value.trim());
        if key.is_empty() || pairs.iter().any(|(known, _)| *known == key) {
            return None;
        }
        pairs.push((key, value));
    }
    pairs.sort_by_key(|(key, _)| *key);
    Some(
        pairs
            .into_iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect(),
    )
}

/// The ports a pod serves on. Init containers are done before traffic flows, so main and sidecar
/// containers count (a sidecar can serve a port, such as a mesh proxy).
fn serving_ports(pod: &PodSummary) -> Vec<ContainerPort> {
    pod.containers
        .iter()
        .filter(|container| container.kind != ContainerKind::Init)
        .flat_map(|container| container.ports.clone())
        .collect()
}

/// The address in canonical form, or `None` when the text is not an IP address.
fn parse_ip(text: &str) -> Option<String> {
    text.trim().parse::<IpAddr>().ok().map(|ip| ip.to_string())
}

fn parse_port(text: &str) -> Result<RequestPort, &'static str> {
    let text = text.trim();
    if text.is_empty() {
        return Err("Enter a port number or a port name");
    }
    if text.bytes().all(|byte| byte.is_ascii_digit()) {
        return match text.parse::<u16>() {
            Ok(port) if port > 0 => Ok(RequestPort::Number(port)),
            _ => Err("Port numbers run from 1 to 65535"),
        };
    }
    Ok(RequestPort::Name(text.to_owned()))
}

/// One end of the connection, owned so it outlives the pod list.
#[derive(Clone, Debug)]
struct EndpointData {
    /// `shop/web`, or the words for a labels stand-in.
    label: String,
    namespace: String,
    labels: Vec<String>,
    ip: Option<String>,
    is_host_network: bool,
}

impl EndpointData {
    fn endpoint(&self) -> TrafficEndpoint<'_> {
        TrafficEndpoint {
            namespace: &self.namespace,
            labels: &self.labels,
            ip: self.ip.as_deref(),
        }
    }
}

#[derive(Clone, Debug)]
enum SourceData {
    Workload(EndpointData),
    External(String),
}

impl SourceData {
    fn label(&self) -> String {
        match self {
            Self::Workload(endpoint) => endpoint.label.clone(),
            Self::External(ip) => ip.clone(),
        }
    }
}

#[derive(Clone, Debug)]
struct Asked {
    source: SourceData,
    destination: EndpointData,
    destination_ports: Vec<ContainerPort>,
    port: RequestPort,
    protocol: String,
}

impl Asked {
    /// The namespaces whose policies matter: the source's (a workload) and the destination's.
    fn policy_namespaces(&self) -> Vec<String> {
        let mut namespaces = Vec::new();
        if let SourceData::Workload(source) = &self.source {
            namespaces.push(source.namespace.clone());
        }
        if !namespaces.contains(&self.destination.namespace) {
            namespaces.push(self.destination.namespace.clone());
        }
        namespaces
    }
}

/// One rule that allows the traffic.
#[derive(Clone, Debug, PartialEq, Eq)]
struct RuleLine {
    policy: ResourceKey,
    policy_text: String,
    text: String,
}

/// What one direction says.
#[derive(Clone, Debug, PartialEq, Eq)]
struct DirectionView {
    title: String,
    /// The sentence for a direction without allowing rules.
    note: Option<String>,
    tone: Option<StatusTone>,
    rules: Vec<RuleLine>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    Egress,
    Ingress,
}

impl Side {
    fn word(self) -> &'static str {
        match self {
            Self::Egress => "egress",
            Self::Ingress => "ingress",
        }
    }

    fn any_peer(self) -> &'static str {
        match self {
            Self::Egress => "any destination",
            Self::Ingress => "any source",
        }
    }
}

struct TrafficOutcome {
    is_allowed: bool,
    port_note: Option<String>,
    egress: DirectionView,
    ingress: DirectionView,
    warnings: Vec<String>,
    source_line: String,
}

/// The sentence for one direction. `local_policies` are the policies of the namespace of the pod
/// they isolate (the source for egress, the destination for ingress); `other` is the far end.
fn direction_view(
    side: Side,
    verdict: &DirectionVerdict,
    local_policies: &[NetworkPolicySummary],
    title: String,
    other: &str,
    port_text: &str,
) -> DirectionView {
    let (note, tone, rules) = match verdict {
        DirectionVerdict::NotIsolated => {
            let end = match side {
                Side::Egress => "from the source",
                Side::Ingress => "to the destination",
            };
            (
                Some(format!("No policy limits {} {end}.", side.word())),
                None,
                Vec::new(),
            )
        }
        DirectionVerdict::NotApplicable => (
            Some("The source is outside the cluster; egress is not checked.".to_owned()),
            None,
            Vec::new(),
        ),
        DirectionVerdict::Allowed {
            isolating,
            allowing,
        } => (
            Some(format!("Isolated by {}; allowed by:", isolating.join(", "))),
            None,
            allowing
                .iter()
                .map(|reference| rule_line(side, local_policies, &reference.policy, reference.rule))
                .collect(),
        ),
        DirectionVerdict::Denied { isolating } => (
            Some(format!(
                "Isolated by {}; no {} rule allows {other} on {port_text}.",
                isolating.join(", "),
                side.word()
            )),
            Some(StatusTone::Bad),
            Vec::new(),
        ),
    };
    DirectionView {
        title,
        note,
        tone,
        rules,
    }
}

/// `argo · egress rule 2: pods app=db, port 5432/TCP`.
fn rule_line(side: Side, policies: &[NetworkPolicySummary], name: &str, index: usize) -> RuleLine {
    let policy = policies.iter().find(|policy| policy.name == name);
    let namespace = policy.map(|policy| policy.namespace.clone());
    let direction = policy.map(|policy| match side {
        Side::Egress => &policy.egress,
        Side::Ingress => &policy.ingress,
    });
    let rule = match direction {
        Some(PolicyDirection::Allowed(rules)) => rules.get(index),
        _ => None,
    };
    let detail = match rule {
        Some(rule) => {
            let peers = if rule.peers.is_empty() {
                side.any_peer().to_owned()
            } else {
                rule.peers
                    .iter()
                    .map(peer_text)
                    .collect::<Vec<_>>()
                    .join(" or ")
            };
            format!("{peers}, {}", ports_text(&rule.ports))
        }
        None => String::new(),
    };
    RuleLine {
        policy: ResourceKey::Kind {
            kind: ResourceKind::NetworkPolicies,
            namespace,
            name: name.to_owned(),
        },
        policy_text: name.to_owned(),
        text: format!(" · {} rule {}: {detail}", side.word(), index + 1),
    }
}

/// The Warn lines of one verdict.
fn traffic_warnings(asked: &Asked, verdict: &TrafficVerdict) -> Vec<String> {
    let mut warnings = Vec::new();
    let mut ends = vec![&asked.destination];
    if let SourceData::Workload(source) = &asked.source {
        ends.insert(0, source);
    }
    for end in &ends {
        if end.is_host_network {
            warnings.push(format!(
                "{} uses the host network; most network plugins do not apply NetworkPolicy to it.",
                end.label
            ));
        }
    }
    for end in &ends {
        if end.ip.is_none() {
            warnings.push(format!(
                "{} has no IP; ipBlock rules were treated as not matching.",
                end.label
            ));
        }
    }
    if let DirectionVerdict::Allowed { allowing, .. } = &verdict.egress {
        for reference in allowing {
            let Some(port) = &reference.named_port else {
                continue;
            };
            warnings.push(format!(
                "Egress rule {} of {} matched by port name {port}, resolved on the destination; some network plugins do not support named egress ports.",
                reference.rule + 1,
                reference.policy
            ));
        }
    }
    for namespace in &verdict.unknown_namespaces {
        warnings.push(format!(
            "Labels of namespace {namespace} are unknown; its namespace selectors were treated as not matching."
        ));
    }
    warnings
}

/// Evaluates `asked` against the listed policies. `policies` is `(namespace, policies)` per
/// listed namespace.
fn outcome(
    asked: &Asked,
    policies: &[(String, Vec<NetworkPolicySummary>)],
    namespaces: &[NamespaceSummary],
    listed_at: jiff::Timestamp,
) -> Result<TrafficOutcome, TrafficError> {
    let of = |namespace: &str| -> &[NetworkPolicySummary] {
        policies
            .iter()
            .find(|(listed, _)| listed == namespace)
            .map_or(&[][..], |(_, list)| list.as_slice())
    };
    let source_policies = match &asked.source {
        SourceData::Workload(source) => of(&source.namespace),
        SourceData::External(_) => &[],
    };
    let destination_policies = of(&asked.destination.namespace);
    let source = match &asked.source {
        SourceData::Workload(data) => TrafficSource::Workload(data.endpoint()),
        SourceData::External(ip) => TrafficSource::External { ip },
    };
    let request = TrafficRequest {
        source,
        destination: TrafficDestination {
            endpoint: asked.destination.endpoint(),
            ports: &asked.destination_ports,
        },
        port: asked.port.clone(),
        protocol: asked.protocol.clone(),
    };
    let inputs = PolicyInputs {
        source_policies,
        destination_policies,
        namespaces,
    };
    let verdict = evaluate_traffic(&request, &inputs)?;
    let port_text = format!("{}/{}", verdict.port, asked.protocol);
    let source_label = asked.source.label();
    let destination_label = asked.destination.label.clone();
    let egress = direction_view(
        Side::Egress,
        &verdict.egress,
        source_policies,
        format!("Egress from {source_label}"),
        &destination_label,
        &port_text,
    );
    let ingress = direction_view(
        Side::Ingress,
        &verdict.ingress,
        destination_policies,
        format!("Ingress to {destination_label}"),
        &source_label,
        &port_text,
    );
    let port_note = match &asked.port {
        RequestPort::Name(name) => Some(format!("Port {name} is {}", verdict.port)),
        RequestPort::Number(_) => None,
    };
    let namespaces_text = asked.policy_namespaces().join(" and ");
    Ok(TrafficOutcome {
        is_allowed: verdict.is_allowed(),
        port_note,
        warnings: traffic_warnings(asked, &verdict),
        egress,
        ingress,
        source_line: format!(
            "Computed from NetworkPolicies of {namespaces_text} listed at {}",
            clock_text(listed_at)
        ),
    })
}

fn error_line(error: &TrafficError) -> String {
    match error {
        TrafficError::UnknownPortName(name) => {
            format!("The destination has no port named {name}")
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SourceMode {
    Pod,
    Labels,
    Ip,
}

pub(crate) struct TrafficTestView {
    session: WeakEntity<ClusterSession>,
    origin: DialogOrigin,
    mode: SourceMode,
    source_pod: Entity<SelectState<Vec<String>>>,
    labels_namespace: Entity<SelectState<Vec<String>>>,
    labels: Entity<InputState>,
    ip: Entity<InputState>,
    destination: Entity<SelectState<Vec<String>>>,
    port: Entity<InputState>,
    protocol: Entity<SelectState<Vec<String>>>,
    verdict: RequestState<TrafficOutcome>,
    link_count: usize,
    _subscriptions: Vec<Subscription>,
}

fn pod_name(key: &ResourceKey) -> Option<String> {
    match key {
        ResourceKey::Pod { namespace, name } => Some(format!("{namespace}/{name}")),
        _ => None,
    }
}

impl TrafficTestView {
    pub(crate) fn new(
        origin: DialogOrigin,
        session: &Entity<ClusterSession>,
        form: TrafficForm,
        check_now: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let (mut pod_names, mut namespace_names) = (Vec::new(), Vec::new());
        if let Some(live) = session.read(cx).live() {
            pod_names = live
                .pods
                .items()
                .iter()
                .map(|pod| format!("{}/{}", pod.namespace, pod.name))
                .collect();
            pod_names.sort();
            namespace_names = live.namespaces.ready_items().map_or_else(
                || live.scope.namespaces().to_vec(),
                |items| items.iter().map(|item| item.name.clone()).collect(),
            );
        }
        let select_of = |names: Vec<String>,
                         picked: Option<String>,
                         is_searchable: bool,
                         window: &mut Window,
                         cx: &mut Context<Self>| {
            let selected = picked
                .and_then(|picked| names.iter().position(|name| *name == picked))
                .map(|row| IndexPath::default().row(row));
            cx.new(|cx| SelectState::new(names, selected, window, cx).searchable(is_searchable))
        };
        let source_pod = select_of(
            pod_names.clone(),
            form.source.as_ref().and_then(pod_name),
            true,
            window,
            cx,
        );
        let destination = select_of(
            pod_names,
            form.destination.as_ref().and_then(pod_name),
            true,
            window,
            cx,
        );
        let first_namespace = form.destination.as_ref().and_then(|key| match key {
            ResourceKey::Pod { namespace, .. } => Some(namespace.clone()),
            _ => None,
        });
        let labels_namespace = select_of(namespace_names, first_namespace, true, window, cx);
        let protocol = select_of(
            PROTOCOLS.map(str::to_owned).to_vec(),
            Some("TCP".to_owned()),
            false,
            window,
            cx,
        );
        let labels = cx.new(|cx| InputState::new(window, cx).placeholder("app=web,tier=front"));
        let ip = cx.new(|cx| InputState::new(window, cx).placeholder("10.0.0.5"));
        let port = cx.new(|cx| {
            let mut input = InputState::new(window, cx).placeholder("80 or a port name");
            input.set_value(form.port.clone(), window, cx);
            input
        });
        let subscriptions = [&labels, &ip, &port]
            .into_iter()
            .map(|input| {
                cx.subscribe_in(input, window, |view, _, event, _, cx| {
                    if matches!(event, InputEvent::PressEnter { .. }) {
                        view.check(cx);
                    }
                })
            })
            .collect();
        let mut view = Self {
            session: session.downgrade(),
            origin,
            mode: SourceMode::Pod,
            source_pod,
            labels_namespace,
            labels,
            ip,
            destination,
            port,
            protocol,
            verdict: RequestState::Idle,
            link_count: 0,
            _subscriptions: subscriptions,
        };
        if check_now {
            view.check(cx);
        }
        view
    }

    /// Whether the policies are still being listed: the capture waits on it.
    #[cfg(feature = "screenshot")]
    pub(crate) fn is_pending(&self) -> bool {
        matches!(self.verdict, RequestState::Loading { .. })
    }

    fn picked(select: &Entity<SelectState<Vec<String>>>, cx: &Context<Self>) -> Option<String> {
        select.read(cx).selected_value().cloned()
    }

    /// The pod a `namespace/name` pick names, read from the live pod list now.
    fn endpoint_of_pod(
        &self,
        picked: &str,
        cx: &Context<Self>,
    ) -> Result<(EndpointData, Vec<ContainerPort>), String> {
        let session = self.session.upgrade().ok_or("Not connected")?;
        let live = session.read(cx).live().ok_or("Not connected")?;
        let (namespace, name) = picked
            .split_once('/')
            .ok_or_else(|| "Pick a pod".to_owned())?;
        let pod = live
            .pods
            .items()
            .iter()
            .find(|pod| pod.namespace == namespace && pod.name == name)
            .ok_or_else(|| format!("Pod {namespace}/{name} no longer exists"))?;
        let ports = serving_ports(pod);
        Ok((
            EndpointData {
                label: picked.to_owned(),
                namespace: pod.namespace.clone(),
                labels: pod.labels.clone(),
                ip: pod.pod_ip.clone(),
                is_host_network: pod.host_network,
            },
            ports,
        ))
    }

    fn ask(&self, cx: &Context<Self>) -> Result<Asked, String> {
        let source = match self.mode {
            SourceMode::Pod => {
                let picked = Self::picked(&self.source_pod, cx).ok_or("Pick a source pod")?;
                SourceData::Workload(self.endpoint_of_pod(&picked, cx)?.0)
            }
            SourceMode::Labels => {
                let namespace = Self::picked(&self.labels_namespace, cx)
                    .ok_or("Pick the namespace of the source pods")?;
                let terms = label_terms(&self.labels.read(cx).value())
                    .ok_or("Labels are key=value pairs")?;
                let label = if terms.is_empty() {
                    format!("pods in {namespace}")
                } else {
                    format!("pods {} in {namespace}", terms.join(","))
                };
                SourceData::Workload(EndpointData {
                    label,
                    namespace,
                    labels: terms,
                    ip: None,
                    is_host_network: false,
                })
            }
            SourceMode::Ip => {
                let ip = parse_ip(&self.ip.read(cx).value()).ok_or("Not an IP address")?;
                SourceData::External(ip)
            }
        };
        let picked = Self::picked(&self.destination, cx).ok_or("Pick a destination pod")?;
        let (destination, destination_ports) = self.endpoint_of_pod(&picked, cx)?;
        let port = parse_port(&self.port.read(cx).value())?;
        let protocol = Self::picked(&self.protocol, cx).unwrap_or_else(|| "TCP".to_owned());
        Ok(Asked {
            source,
            destination,
            destination_ports,
            port,
            protocol,
        })
    }

    fn check(&mut self, cx: &mut Context<Self>) {
        let asked = match self.ask(cx) {
            Ok(asked) => asked,
            Err(message) => {
                self.verdict = RequestState::Failed(message);
                cx.notify();
                return;
            }
        };
        let Some(session) = self.session.upgrade() else {
            return;
        };
        let (connection, namespaces) = match session.read(cx).live() {
            Some(live) => (
                live.connection().clone(),
                live.namespaces
                    .ready_items()
                    .map(<[NamespaceSummary]>::to_vec)
                    .unwrap_or_default(),
            ),
            None => {
                self.verdict = RequestState::Failed("Not connected".to_owned());
                cx.notify();
                return;
            }
        };
        let runtime = cx.global::<ClusterRuntime>().clone();
        let wanted = asked.policy_namespaces();
        let listing = runtime.spawn(async move {
            try_join_all(wanted.into_iter().map(|namespace| {
                let connection = connection.clone();
                async move {
                    let policies = connection.read_network_policies(&namespace).await?;
                    Ok::<_, ClusterError>((namespace, policies))
                }
            }))
            .await
        });
        let task = cx.spawn(async move |this, cx| {
            let result = listing.await;
            let _ = this.update(cx, |view, cx| view.finish(&asked, &namespaces, result, cx));
        });
        self.verdict = RequestState::Loading { _task: task };
        cx.notify();
    }

    fn finish(
        &mut self,
        asked: &Asked,
        namespaces: &[NamespaceSummary],
        result: Result<Result<ListedPolicies, ClusterError>, tokio::task::JoinError>,
        cx: &mut Context<Self>,
    ) {
        self.verdict = match result {
            Ok(Ok(policies)) => {
                match outcome(asked, &policies, namespaces, jiff::Timestamp::now()) {
                    Ok(outcome) => RequestState::Ready(outcome),
                    Err(error) => RequestState::Failed(error_line(&error)),
                }
            }
            Ok(Err(error)) => RequestState::Failed(error_text(&error)),
            Err(_) => RequestState::Failed("the policy listing stopped unexpectedly".to_owned()),
        };
        cx.notify();
    }

    fn reveal(&mut self, key: ResourceKey, window: &mut Window, cx: &mut Context<Self>) {
        window.close_dialog(cx);
        self.origin.reveal(key, cx);
    }

    fn link(&mut self, text: &str, key: ResourceKey, cx: &mut Context<Self>) -> AnyElement {
        self.link_count += 1;
        let theme = cx.theme();
        div()
            .id(("traffic-link", self.link_count))
            .cursor_pointer()
            .font_family(theme.mono_font_family.clone())
            .text_color(theme.link)
            .underline()
            .on_click(cx.listener(move |view, _, window, cx| {
                view.reveal(key.clone(), window, cx);
            }))
            .child(text.to_owned())
            .into_any_element()
    }

    fn muted(&self, text: impl Into<SharedString>, cx: &Context<Self>) -> AnyElement {
        div()
            .text_sm()
            .text_color(cx.theme().muted_foreground)
            .child(text.into())
            .into_any_element()
    }

    fn toned(
        &self,
        text: impl Into<SharedString>,
        tone: StatusTone,
        cx: &Context<Self>,
    ) -> AnyElement {
        div()
            .text_color(tone_color(tone, cx))
            .child(text.into())
            .into_any_element()
    }

    fn mode_button(
        &self,
        id: &'static str,
        label: &'static str,
        mode: SourceMode,
        cx: &mut Context<Self>,
    ) -> Button {
        let is_on = self.mode == mode;
        Button::new(id)
            .small()
            .label(label)
            .when(is_on, |button| button.primary())
            .when(!is_on, |button| button.outline())
            .on_click(cx.listener(move |view, _, _, cx| {
                // A result belongs to the source it was computed for.
                view.verdict = RequestState::Idle;
                view.mode = mode;
                cx.notify();
            }))
    }

    fn render_source(&self, cx: &mut Context<Self>) -> AnyElement {
        let fields = match self.mode {
            SourceMode::Pod => {
                h_flex().child(div().flex_1().child(Select::new(&self.source_pod).small()))
            }
            SourceMode::Labels => h_flex()
                .gap_2()
                .child(
                    div()
                        .w(px(200.))
                        .child(Select::new(&self.labels_namespace).small()),
                )
                .child(div().flex_1().child(Input::new(&self.labels).small())),
            SourceMode::Ip => h_flex().child(div().flex_1().child(Input::new(&self.ip).small())),
        };
        h_flex()
            .gap_2()
            .items_center()
            .child(div().w(px(80.)).text_sm().child("Source"))
            .child(
                h_flex()
                    .gap_1()
                    .child(self.mode_button("traffic-pod", "Pod", SourceMode::Pod, cx))
                    .child(self.mode_button("traffic-labels", "Labels", SourceMode::Labels, cx))
                    .child(self.mode_button("traffic-ip", "IP address", SourceMode::Ip, cx)),
            )
            .child(div().flex_1().min_w_0().child(fields))
            .into_any_element()
    }

    fn render_direction(&mut self, view: &DirectionView, cx: &mut Context<Self>) -> AnyElement {
        let mut rows: Vec<AnyElement> = vec![
            div()
                .font_semibold()
                .child(view.title.clone())
                .into_any_element(),
        ];
        if let Some(note) = &view.note {
            rows.push(match view.tone {
                Some(tone) => self.toned(note.clone(), tone, cx),
                None => self.muted(note.clone(), cx),
            });
        }
        for rule in &view.rules {
            let policy = self.link(&rule.policy_text, rule.policy.clone(), cx);
            rows.push(
                h_flex()
                    .pl_4()
                    .text_sm()
                    .flex_wrap()
                    .child(policy)
                    .child(self.muted(rule.text.clone(), cx))
                    .into_any_element(),
            );
        }
        v_flex().gap_0p5().children(rows).into_any_element()
    }

    fn render_outcome(
        &mut self,
        outcome: &TrafficOutcome,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let (text, tone) = if outcome.is_allowed {
            ("Allowed", StatusTone::Ok)
        } else {
            ("Denied", StatusTone::Bad)
        };
        let mut body = vec![
            div()
                .text_xl()
                .font_semibold()
                .text_color(tone_color(tone, cx))
                .child(text)
                .into_any_element(),
        ];
        if let Some(note) = &outcome.port_note {
            body.push(self.muted(note.clone(), cx));
        }
        body.push(self.render_direction(&outcome.egress, cx));
        body.push(self.render_direction(&outcome.ingress, cx));
        for warning in &outcome.warnings {
            body.push(self.toned(warning.clone(), StatusTone::Warn, cx));
        }
        body.push(self.muted(outcome.source_line.clone(), cx));
        body.push(self.muted(CAVEATS, cx));
        body
    }
}

impl Render for TrafficTestView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.link_count = 0;
        let verdict = std::mem::replace(&mut self.verdict, RequestState::Idle);
        let body = match &verdict {
            RequestState::Ready(outcome) => self.render_outcome(outcome, cx),
            RequestState::Loading { .. } => vec![self.muted("Listing network policies…", cx)],
            RequestState::Failed(message) => vec![self.toned(message.clone(), StatusTone::Bad, cx)],
            RequestState::Idle => Vec::new(),
        };
        self.verdict = verdict;
        let source = self.render_source(cx);
        v_flex()
            .gap_3()
            .w_full()
            .child(source)
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(div().w(px(80.)).text_sm().child("Destination"))
                    .child(div().flex_1().child(Select::new(&self.destination).small())),
            )
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(div().w(px(80.)).text_sm().child("Port"))
                    .child(div().w(px(120.)).child(Input::new(&self.port).small()))
                    .child(div().w(px(100.)).child(Select::new(&self.protocol).small()))
                    .child(
                        Button::new("traffic-check")
                            .small()
                            .primary()
                            .label("Check")
                            .on_click(cx.listener(|view, _, _, cx| view.check(cx))),
                    ),
            )
            .children(body)
    }
}

#[cfg(test)]
#[path = "traffic_test_view_tests.rs"]
mod traffic_test_view_tests;
