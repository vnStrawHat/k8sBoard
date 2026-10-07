//! The Recent changes panel of Overview: Deployment rollouts and HPA rescales (selected by the
//! server), node readiness transitions, joined nodes, and new namespaces, newest first. Pure: it
//! takes the Ready snapshots and a clock. Event messages are arbitrary text, so nothing here logs.

use std::collections::HashMap;

use cluster::{
    ConditionStatus, EventSummary, FieldWriter, NamespaceSummary, NodeSummary, ReplicaSetSummary,
};
use jiff::tz::TimeZone;
use jiff::{SignedDuration, Timestamp};

use crate::event_rows::message_line;
use crate::kind_row::KindObject;
use crate::port_forwards::is_dns_subdomain;
use crate::table_selection::ResourceKey;
use crate::workload_actions::image_tag;

/// The rows the panel shows; the rest stay in the Events screen.
pub(crate) const CHANGE_ROWS: usize = 8;

const NODE_READY_CONDITION: &str = "Ready";
const KUBELET: &str = "kubelet";
const NODE_CONTROLLER: &str = "node-controller";

/// How far back the panel looks. The API server keeps events about an hour, so the 24 h range adds
/// the rollouts that the creation times of the Deployments' ReplicaSets still tell.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ChangeWindow {
    #[default]
    FifteenMinutes,
    OneHour,
    TwentyFourHours,
}

impl ChangeWindow {
    pub(crate) const ALL: [Self; 3] = [Self::FifteenMinutes, Self::OneHour, Self::TwentyFourHours];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::FifteenMinutes => "Last 15 min",
            Self::OneHour => "Last 1 h",
            Self::TwentyFourHours => "Last 24 h",
        }
    }

    pub(crate) fn span(self) -> SignedDuration {
        match self {
            Self::FifteenMinutes => SignedDuration::from_mins(15),
            Self::OneHour => SignedDuration::from_hours(1),
            Self::TwentyFourHours => SignedDuration::from_hours(24),
        }
    }

    /// Whether the range reaches past the events the API server keeps, so the ReplicaSets are read.
    pub(crate) fn reads_replica_sets(self) -> bool {
        self == Self::TwentyFourHours
    }
}

/// How the rows write their time: the clock time, with the date when a change is not from today
/// (the 24 h range reaches back to yesterday, where a bare `22:10` would read as tonight).
#[derive(Clone, Copy)]
pub(crate) struct ChangeClock<'a> {
    pub(crate) zone: &'a TimeZone,
    pub(crate) now: Timestamp,
    pub(crate) window: ChangeWindow,
}

impl ChangeClock<'_> {
    pub(crate) fn label(&self, at: Timestamp) -> String {
        let (at, now) = (
            at.to_zoned(self.zone.clone()),
            self.now.to_zoned(self.zone.clone()),
        );
        let format = if at.date() == now.date() || !self.window.reads_replica_sets() {
            "%H:%M"
        } else {
            "%m-%d %H:%M"
        };
        at.strftime(format).to_string()
    }

    /// The width of the time column: room for the date on the 24 h range.
    pub(crate) fn width(&self) -> f32 {
        if self.window.reads_replica_sets() {
            92.
        } else {
            44.
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

/// Where the actor of a row comes from. A field manager is inferred, not recorded: Kubernetes
/// stores no user on an object.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ActorSource {
    /// The newest manager of the Deployment's pod template, near the rollout (spec 0041).
    FieldManager,
    /// The `source` of the event.
    EventSource,
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
    pub(crate) actor_source: ActorSource,
    /// The ReplicaSet a Deployment event names, which the click diffs against its predecessor.
    pub(crate) replica_set: Option<String>,
    pub(crate) target: Option<ResourceKey>,
}

impl ChangeEntry {
    /// The row tooltip. A Deployment row says where its actor came from, so a field manager is
    /// never read as proof of who did it.
    pub(crate) fn tooltip(&self) -> String {
        let base = format!("{} {} {}", self.kind.label(), self.object, self.text);
        match (&self.actor, self.actor_source, self.kind) {
            (Some(manager), ActorSource::FieldManager, _) => {
                format!("{base} · probably {manager} · last pod-template writer (field manager)")
            }
            (Some(_), ActorSource::EventSource, ChangeKind::Deployment) => {
                format!("{base} · event source")
            }
            _ => base,
        }
    }
}

/// What the model reads; `None` means that list has not loaded and contributes nothing.
pub(crate) struct ChangeInputs<'a> {
    pub(crate) rollouts: Option<&'a [EventSummary]>,
    pub(crate) rescales: Option<&'a [EventSummary]>,
    pub(crate) nodes: Option<&'a [NodeSummary]>,
    pub(crate) namespaces: Option<&'a [NamespaceSummary]>,
    /// The ReplicaSets of the scope, read only by the 24 h range; `None` while that list has not
    /// loaded or is not asked for.
    pub(crate) replica_sets: Option<&'a [ReplicaSetSummary]>,
    /// The Deployments condition feed, for the field manager of a rollout; `None` while the feed is
    /// off or loading, which leaves the event source as the actor.
    pub(crate) deployments: Option<&'a [KindObject]>,
    pub(crate) window: ChangeWindow,
    pub(crate) now: Timestamp,
}

/// Every change newer than `now − span` (so a time slightly ahead of `now` counts), newest first;
/// ties are broken by object.
pub(crate) fn recent_changes(inputs: &ChangeInputs) -> Vec<ChangeEntry> {
    // An event stamped slightly ahead of this machine's clock is recent, not dropped.
    let is_recent = |at: Timestamp| inputs.now.duration_since(at) < inputs.window.span();
    let writers = template_writers(inputs.deployments);
    let mut entries = Vec::new();
    for (events, kind) in [
        (inputs.rollouts, ChangeKind::Deployment),
        (inputs.rescales, ChangeKind::Autoscaler),
    ] {
        entries.extend(
            events
                .into_iter()
                .flatten()
                .filter_map(|event| event_entry(event, kind, &writers, &is_recent)),
        );
    }
    if inputs.window.reads_replica_sets() {
        let created = replica_set_entries(inputs.replica_sets.unwrap_or_default(), &is_recent);
        // The rollout row says what the scale-up event that made its ReplicaSet only hints at, so
        // that event is dropped. Scale-downs stay, and so does a scale-up long after the creation: a
        // rollback reuses its old ReplicaSet, and only the event has the time of that rollout.
        entries.retain(|entry| !is_scale_up_of(entry, &created));
        entries.extend(created);
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
            actor_source: ActorSource::EventSource,
            replica_set: None,
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

/// A rollout from the creation of a Deployment's ReplicaSet: `rev 8 · 1.26-alpine · release test`.
/// A ReplicaSet without a Deployment owner or a revision number is no rollout of its own.
fn replica_set_entries(
    replica_sets: &[ReplicaSetSummary],
    is_recent: &impl Fn(Timestamp) -> bool,
) -> Vec<ChangeEntry> {
    replica_sets
        .iter()
        .filter_map(|set| {
            let owner = set
                .owner
                .as_ref()
                .filter(|owner| owner.kind == "Deployment")?;
            let revision = set.revision.as_deref()?;
            let at = set.created_at.filter(|at| is_recent(*at))?;
            let mut text = format!("rev {revision}");
            let tag = set
                .containers
                .first()
                .map(|container| image_tag(&container.image))
                .filter(|tag| !tag.is_empty());
            if let Some(tag) = tag {
                text.push_str(&format!(" · {tag}"));
            }
            if let Some(cause) = &set.change_cause {
                text.push_str(&format!(" · {cause}"));
            }
            Some(ChangeEntry {
                at,
                kind: ChangeKind::Deployment,
                object: format!("{}/{}", set.namespace, owner.name),
                text,
                count: 1,
                actor: None,
                actor_source: ActorSource::EventSource,
                replica_set: Some(set.name.clone()),
                target: ResourceKey::of_object("Deployment", Some(&set.namespace), &owner.name),
            })
        })
        .collect()
}

/// How far an event may be from the creation of its ReplicaSet and still be the scale-up that made it.
const CREATION_SKEW: SignedDuration = SignedDuration::from_secs(120);

/// Whether `entry` is the `Scaled up replica set X to N` event that made a ReplicaSet which has a
/// rollout row of its own.
fn is_scale_up_of(entry: &ChangeEntry, rollouts: &[ChangeEntry]) -> bool {
    entry.kind == ChangeKind::Deployment
        && entry.text.starts_with("Scaled up replica set ")
        && entry.replica_set.as_ref().is_some_and(|name| {
            rollouts.iter().any(|rollout| {
                rollout.replica_set.as_ref() == Some(name)
                    && entry.at.duration_since(rollout.at).abs() <= CREATION_SKEW
            })
        })
}

/// How far before the event's last occurrence a template write may be and still count as its
/// cause, and how far after it (clock skew between the API server and the controller).
const WRITER_WINDOW_BEFORE: SignedDuration = SignedDuration::from_mins(30);
const WRITER_WINDOW_AFTER: SignedDuration = SignedDuration::from_secs(60);

fn event_entry(
    event: &EventSummary,
    kind: ChangeKind,
    writers: &TemplateWriters,
    is_recent: &impl Fn(Timestamp) -> bool,
) -> Option<ChangeEntry> {
    // An aggregated event moves to its newest occurrence; one without a time cannot be placed.
    let at = event.last_seen.filter(|at| is_recent(*at))?;
    let object = &event.object;
    let is_deployment = kind == ChangeKind::Deployment;
    let manager = is_deployment
        .then(|| template_writer(event, at, writers))
        .flatten();
    let (actor, actor_source) = match manager {
        Some(manager) => (Some(manager), ActorSource::FieldManager),
        None => (
            event.source.as_deref().and_then(actor_of),
            ActorSource::EventSource,
        ),
    };
    Some(ChangeEntry {
        at,
        kind,
        object: match &object.namespace {
            Some(namespace) => format!("{namespace}/{}", object.name),
            None => object.name.clone(),
        },
        text: message_line(&event.message),
        count: event.count,
        actor,
        actor_source,
        replica_set: is_deployment
            .then(|| named_replica_set(&event.message))
            .flatten(),
        target: ResourceKey::of_object(&object.kind, object.namespace.as_deref(), &object.name),
    })
}

/// The newest manager of the event's Deployment's pod template when its write is at most 30 min
/// before and 60 s after the event (a rollout starts right after the template write; a later
/// write cannot have caused it).
/// The template writers of the Deployments feed by (namespace, name), built once per call.
type TemplateWriters<'a> = HashMap<(&'a str, &'a str), &'a FieldWriter>;

fn template_writers(deployments: Option<&[KindObject]>) -> TemplateWriters<'_> {
    deployments
        .into_iter()
        .flatten()
        .filter_map(|object| match object {
            KindObject::Deployment(deployment) => {
                deployment.template_change.as_ref().map(|writer| {
                    (
                        (deployment.namespace.as_str(), deployment.name.as_str()),
                        writer,
                    )
                })
            }
            _ => None,
        })
        .collect()
}

/// The newest manager of the event's Deployment's pod template when its write is at most 30 min
/// before and 60 s after the event (a rollout starts right after the template write; a later
/// write cannot have caused it).
///
/// `ponytail:` a heuristic: an exact cause needs the audit log. An HPA scale of the same
/// Deployment inside the window (a `ScalingReplicaSet` event with no template change) is also
/// attributed to the template writer. The time is that of the manager's whole `managedFields`
/// entry, which any field the manager owns can refresh, so a later edit by the same manager of
/// another template field moves it too.
fn template_writer(
    event: &EventSummary,
    at: Timestamp,
    writers: &TemplateWriters,
) -> Option<String> {
    let namespace = event.object.namespace.as_deref()?;
    let writer = writers.get(&(namespace, event.object.name.as_str()))?;
    let lag = at.duration_since(writer.at);
    (lag <= WRITER_WINDOW_BEFORE && lag >= -WRITER_WINDOW_AFTER).then(|| writer.manager.clone())
}

/// The word after `replica set ` in `Scaled up replica set api-7d9f8c to 3`, when it is a name.
fn named_replica_set(message: &str) -> Option<String> {
    let (_, rest) = message.split_once("replica set ")?;
    let name = rest.split_whitespace().next()?;
    is_dns_subdomain(name).then(|| name.to_owned())
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
        actor_source: ActorSource::EventSource,
        replica_set: None,
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
