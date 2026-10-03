//! The Recent changes panel of Overview: Deployment rollouts and HPA rescales (selected by the
//! server), node readiness transitions, joined nodes, and new namespaces, newest first. Pure: it
//! takes the Ready snapshots and a clock. Event messages are arbitrary text, so nothing here logs.

use cluster::{ConditionStatus, EventSummary, NamespaceSummary, NodeSummary};
use jiff::{SignedDuration, Timestamp};

use crate::event_rows::message_line;
use crate::table_selection::ResourceKey;

/// The rows the panel shows; the rest stay in the Events screen.
pub(crate) const CHANGE_ROWS: usize = 8;

const NODE_READY_CONDITION: &str = "Ready";
const KUBELET: &str = "kubelet";
const NODE_CONTROLLER: &str = "node-controller";

/// How far back the panel looks. The API server keeps events about an hour, so that is the
/// longest range.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ChangeWindow {
    #[default]
    FifteenMinutes,
    OneHour,
}

impl ChangeWindow {
    pub(crate) const ALL: [Self; 2] = [Self::FifteenMinutes, Self::OneHour];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::FifteenMinutes => "Last 15 min",
            Self::OneHour => "Last 1 h",
        }
    }

    pub(crate) fn span(self) -> SignedDuration {
        match self {
            Self::FifteenMinutes => SignedDuration::from_mins(15),
            Self::OneHour => SignedDuration::from_hours(1),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ChangeKind {
    Deployment,
    Autoscaler,
    Node,
    Namespace,
}

impl ChangeKind {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Deployment => "Deployment",
            Self::Autoscaler => "HPA",
            Self::Node => "Node",
            Self::Namespace => "Namespace",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ChangeEntry {
    pub(crate) at: Timestamp,
    pub(crate) kind: ChangeKind,
    /// `payments/api` or `ip-10-0-3-17`.
    pub(crate) object: String,
    /// `Scaled up replica set api-7d9f8c to 3`, `became NotReady`.
    pub(crate) text: String,
    /// The event count; 1 for state rows.
    pub(crate) count: u32,
    pub(crate) actor: Option<String>,
    pub(crate) target: Option<ResourceKey>,
}

/// What the model reads; `None` means that list has not loaded and contributes nothing.
pub(crate) struct ChangeInputs<'a> {
    pub(crate) rollouts: Option<&'a [EventSummary]>,
    pub(crate) rescales: Option<&'a [EventSummary]>,
    pub(crate) nodes: Option<&'a [NodeSummary]>,
    pub(crate) namespaces: Option<&'a [NamespaceSummary]>,
    pub(crate) window: ChangeWindow,
    pub(crate) now: Timestamp,
}

/// Every change newer than `now − span` (so a time slightly ahead of `now` counts), newest first;
/// ties are broken by object.
pub(crate) fn recent_changes(inputs: &ChangeInputs) -> Vec<ChangeEntry> {
    // An event stamped slightly ahead of this machine's clock is recent, not dropped.
    let is_recent = |at: Timestamp| inputs.now.duration_since(at) < inputs.window.span();
    let mut entries = Vec::new();
    for (events, kind) in [
        (inputs.rollouts, ChangeKind::Deployment),
        (inputs.rescales, ChangeKind::Autoscaler),
    ] {
        entries.extend(
            events
                .into_iter()
                .flatten()
                .filter_map(|event| event_entry(event, kind, &is_recent)),
        );
    }
    for node in inputs.nodes.into_iter().flatten() {
        entries.extend(node_entry(node, &is_recent));
    }
    for namespace in inputs.namespaces.into_iter().flatten() {
        let Some(at) = namespace.created_at.filter(|at| is_recent(*at)) else {
            continue;
        };
        entries.push(ChangeEntry {
            at,
            kind: ChangeKind::Namespace,
            object: namespace.name.clone(),
            text: "created".to_owned(),
            count: 1,
            actor: None,
            target: ResourceKey::of_object("Namespace", None, &namespace.name),
        });
    }
    entries.sort_by(|left, right| {
        right
            .at
            .cmp(&left.at)
            .then_with(|| left.object.cmp(&right.object))
    });
    entries
}

fn event_entry(
    event: &EventSummary,
    kind: ChangeKind,
    is_recent: &impl Fn(Timestamp) -> bool,
) -> Option<ChangeEntry> {
    // An aggregated event moves to its newest occurrence; one without a time cannot be placed.
    let at = event.last_seen.filter(|at| is_recent(*at))?;
    let object = &event.object;
    Some(ChangeEntry {
        at,
        kind,
        object: match &object.namespace {
            Some(namespace) => format!("{namespace}/{}", object.name),
            None => object.name.clone(),
        },
        text: message_line(&event.message),
        count: event.count,
        actor: event.source.as_deref().and_then(actor_of),
        target: ResourceKey::of_object(&object.kind, object.namespace.as_deref(), &object.name),
    })
}

/// The event source up to its first ` on `: `kubelet on ip-10-0-1-23` is `kubelet`.
fn actor_of(source: &str) -> Option<String> {
    let actor = source.split_once(" on ").map_or(source, |(actor, _)| actor);
    (!actor.is_empty()).then(|| actor.to_owned())
}

/// A node that joined in the window says so; one that did not may have changed readiness in it.
fn node_entry(node: &NodeSummary, is_recent: &impl Fn(Timestamp) -> bool) -> Option<ChangeEntry> {
    let entry = |at, text: &str, actor: Option<&str>| ChangeEntry {
        at,
        kind: ChangeKind::Node,
        object: node.name.clone(),
        text: text.to_owned(),
        count: 1,
        actor: actor.map(str::to_owned),
        target: Some(ResourceKey::of_node(node)),
    };
    if let Some(at) = node.created_at.filter(|at| is_recent(*at)) {
        return Some(entry(at, "joined the cluster", None));
    }
    let ready = node
        .conditions
        .iter()
        .find(|condition| condition.name == NODE_READY_CONDITION)?;
    let at = ready.changed_at.filter(|at| is_recent(*at))?;
    Some(match ready.status {
        ConditionStatus::True => entry(at, "became Ready", Some(KUBELET)),
        ConditionStatus::False => entry(at, "became NotReady", Some(KUBELET)),
        ConditionStatus::Unknown => entry(at, "stopped reporting (Unknown)", Some(NODE_CONTROLLER)),
    })
}

#[cfg(test)]
#[path = "recent_changes_tests.rs"]
mod recent_changes_tests;
