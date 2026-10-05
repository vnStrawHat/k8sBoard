//! Recent pod and node usage, one tick per metrics poll: fine rings for the last hour and coarse
//! ones for the last day. Every ring is bounded (see `history_rings`), so memory stays flat
//! however long the session runs.

use std::collections::BTreeMap;
use std::time::Duration;

use cluster::{
    ByteAmount, ContainerState, ContainerSummary, ControllerRef, CpuAmount, NamespaceScope,
    NodeMetrics, PodMetrics, PodSummary, ResourceUsage, StatusReason, Termination,
};

use crate::history_rings::{Resolution, Retention, RingPoint, Rings, Timelines};
use crate::kind_row::{PodOwner, owns};

/// How long an OOM mark is kept.
const OOM_KEPT: jiff::SignedDuration = jiff::SignedDuration::from_hours(24);
/// A pod keeps at most this many marks; the oldest drop first.
const MAX_OOM_MARKS: usize = 32;

/// The usage of one container at one tick. Saturating, since a container never nears
/// 4,294 cores or 4 TiB; a node is not, and keeps `ResourceUsage` (u64).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct UsagePoint {
    cpu_microcores: u32,
    memory_kib: u32,
}

impl UsagePoint {
    pub(crate) fn of(usage: ResourceUsage) -> Self {
        Self {
            cpu_microcores: saturating_u32(usage.cpu.nanocores() / 1_000),
            memory_kib: saturating_u32(usage.memory.bytes() / 1_024),
        }
    }

    pub(crate) fn usage(self) -> ResourceUsage {
        ResourceUsage {
            cpu: CpuAmount::from_nanocores(u64::from(self.cpu_microcores) * 1_000),
            memory: ByteAmount::from_bytes(u64::from(self.memory_kib) * 1_024),
        }
    }
}

impl RingPoint for UsagePoint {
    fn mean(points: &[Self]) -> Self {
        let count = points.len() as u64;
        let mean = |field: fn(&Self) -> u32| {
            saturating_u32(
                points
                    .iter()
                    .map(|point| u64::from(field(point)))
                    .sum::<u64>()
                    / count,
            )
        };
        Self {
            cpu_microcores: mean(|point| point.cpu_microcores),
            memory_kib: mean(|point| point.memory_kib),
        }
    }
}

impl RingPoint for ResourceUsage {
    fn mean(points: &[Self]) -> Self {
        let count = points.len() as u128;
        let mean = |field: fn(&Self) -> u64| {
            u64::try_from(
                points
                    .iter()
                    .map(|point| u128::from(field(point)))
                    .sum::<u128>()
                    / count,
            )
            .unwrap_or(u64::MAX)
        };
        Self {
            cpu: CpuAmount::from_nanocores(mean(|point| point.cpu.nanocores())),
            memory: ByteAmount::from_bytes(mean(|point| point.memory.bytes())),
        }
    }
}

/// A series cut to one resolution, ready for a chart or a table.
pub(crate) struct UsageSeries {
    /// Oldest first; `None` where the series reported nothing.
    pub(crate) points: Vec<(jiff::Timestamp, Option<ResourceUsage>)>,
    /// The spacing of the points, for splitting a line at a gap.
    pub(crate) step: Duration,
    /// When a container of the series was OOM killed, oldest first.
    pub(crate) oom: Vec<jiff::Timestamp>,
    /// Pods with a value in `points`.
    pub(crate) pod_count: usize,
    /// The newest server timestamp among the included items.
    pub(crate) sampled_at: Option<jiff::Timestamp>,
    /// How many polls in a row brought the same server timestamp, for the reporting item that
    /// changed most recently. Counted in ticks, so it does not depend on the clock or the range.
    pub(crate) stalled_ticks: u32,
}

/// The entry for `key`, created with `make` on a miss. A hit allocates nothing, unlike `entry`,
/// which needs an owned key on every call; `record` runs for every container every tick.
/// `None` cannot happen: the key was just inserted.
fn slot<'m, V>(
    map: &'m mut BTreeMap<String, V>,
    key: &str,
    make: impl FnOnce() -> V,
) -> Option<&'m mut V> {
    if !map.contains_key(key) {
        map.insert(key.to_owned(), make());
    }
    map.get_mut(key)
}

/// The count of polls in a row with the same server timestamp after a poll that brought `new`.
fn next_unchanged(count: u32, old: Option<jiff::Timestamp>, new: Option<jiff::Timestamp>) -> u32 {
    match (old, new) {
        (Some(old), Some(new)) if old == new => count.saturating_add(1),
        _ => 0,
    }
}

fn saturating_u32(value: u64) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

/// An OOM kill of one container, from its status.
struct OomMark {
    container: String,
    at: jiff::Timestamp,
}

/// The usage of every pod seen recently, by namespace and name.
#[derive(Default)]
pub(crate) struct PodUsageHistory {
    timelines: Timelines,
    tick_count: u64,
    pods: BTreeMap<String, BTreeMap<String, PodHistory>>,
}

#[derive(Default)]
struct PodHistory {
    /// Filled from the pods list whenever it is still `None`, so a workload's chart keeps the
    /// pods a rollout replaced, which the list no longer holds.
    controller: Option<ControllerRef>,
    containers: BTreeMap<String, Rings<UsagePoint>>,
    oom: Vec<OomMark>,
    /// The server's scrape time of the newest sample.
    sampled_at: Option<jiff::Timestamp>,
    /// Consecutive polls that brought the same `sampled_at`.
    unchanged_ticks: u32,
}

impl PodUsageHistory {
    /// Adds one tick. A pod or container missing from `sample` reads `None` for it, and a
    /// series that stays missing is freed after an hour and dropped after a day. `pods` is the
    /// current pods list, for the controllers and the OOM kills; it may lag `sample`.
    #[cfg_attr(
        feature = "hotpath-profiling",
        hotpath::measure(impl_type = "PodUsageHistory")
    )]
    pub(crate) fn record(
        &mut self,
        at: jiff::Timestamp,
        sample: &[PodMetrics],
        pods: &[PodSummary],
    ) {
        self.tick_count += 1;
        let tick = self.tick_count;
        let closes_coarse = self.timelines.push(at, tick);
        for rings in self.rings_mut() {
            rings.begin_tick();
        }
        for pod in sample {
            let Some(namespace) = slot(&mut self.pods, &pod.namespace, BTreeMap::new) else {
                continue;
            };
            let Some(history) = slot(namespace, &pod.name, PodHistory::default) else {
                continue;
            };
            history.unchanged_ticks =
                next_unchanged(history.unchanged_ticks, history.sampled_at, pod.sampled_at);
            history.sampled_at = pod.sampled_at.or(history.sampled_at);
            for container in &pod.containers {
                if let Some(rings) = slot(&mut history.containers, &container.name, Rings::new) {
                    rings.record(UsagePoint::of(container.usage), tick);
                }
            }
        }
        if closes_coarse {
            for rings in self.rings_mut() {
                rings.fold();
            }
        }
        let timeline_len = self.timelines.fine_len();
        for pod in self.pods.values_mut().flat_map(BTreeMap::values_mut) {
            pod.containers.retain(|_, rings| {
                debug_assert!(rings.len() <= timeline_len);
                rings.age(tick) == Retention::Keep
            });
        }
        for namespace in self.pods.values_mut() {
            namespace.retain(|_, pod| !pod.containers.is_empty());
        }
        self.pods.retain(|_, namespace| !namespace.is_empty());
        self.note_pods(at, pods);
    }

    /// Fills the missing controllers and records the OOM kills the pods report.
    fn note_pods(&mut self, at: jiff::Timestamp, pods: &[PodSummary]) {
        for pod in pods {
            let Some(history) = self
                .pods
                .get_mut(&pod.namespace)
                .and_then(|namespace| namespace.get_mut(&pod.name))
            else {
                continue;
            };
            if history.controller.is_none() {
                history.controller.clone_from(&pod.controller);
            }
            for container in &pod.containers {
                for finished_at in oom_kills(container) {
                    history.note_oom(&container.name, finished_at, at);
                }
            }
        }
        for history in self.pods.values_mut().flat_map(BTreeMap::values_mut) {
            history.oom.retain(|mark| is_recent(mark.at, at));
        }
    }

    fn rings_mut(&mut self) -> impl Iterator<Item = &mut Rings<UsagePoint>> {
        self.pods
            .values_mut()
            .flat_map(BTreeMap::values_mut)
            .flat_map(|pod| pod.containers.values_mut())
    }

    /// Drops the pods outside `scope`. `All` keeps everything; the timelines stay.
    pub(crate) fn retain_scope(&mut self, scope: &NamespaceScope) {
        if matches!(scope, NamespaceScope::All) {
            return;
        }
        let namespaces = scope.namespaces();
        self.pods
            .retain(|namespace, _| namespaces.contains(namespace));
    }

    fn pod(&self, namespace: &str, pod: &str) -> Option<&PodHistory> {
        self.pods.get(namespace)?.get(pod)
    }

    /// The pod's total at the newest tick; `None` when no container reported it.
    pub(crate) fn latest(&self, namespace: &str, pod: &str) -> Option<ResourceUsage> {
        self.pod(namespace, pod)?
            .containers
            .values()
            .filter_map(Rings::newest)
            .map(UsagePoint::usage)
            .reduce(sum_usage)
    }

    /// One container's usage at the newest tick.
    pub(crate) fn latest_container(
        &self,
        namespace: &str,
        pod: &str,
        container: &str,
    ) -> Option<ResourceUsage> {
        let rings = self.pod(namespace, pod)?.containers.get(container)?;
        rings.newest().map(UsagePoint::usage)
    }

    /// One container's usage at the last two ticks, oldest first; `None` unless both reported.
    pub(crate) fn latest_container_pair(
        &self,
        namespace: &str,
        pod: &str,
        container: &str,
    ) -> Option<[ResourceUsage; 2]> {
        let rings = self.pod(namespace, pod)?.containers.get(container)?;
        rings.newest_pair().map(|pair| pair.map(UsagePoint::usage))
    }

    /// The pod (`container` `None`) or one of its containers.
    pub(crate) fn pod_series(
        &self,
        namespace: &str,
        pod: &str,
        container: Option<&str>,
        resolution: Resolution,
    ) -> UsageSeries {
        let mut builder = SeriesBuilder::new(&self.timelines, resolution);
        if let Some(history) = self.pod(namespace, pod) {
            builder.add_pod(history, container);
        }
        builder.finish()
    }

    /// The pods `owner` owns by namespace and controller, summed, or just `pod` among them.
    /// Pods a rollout replaced still count while their history is kept.
    pub(crate) fn owner_series(
        &self,
        owner: &PodOwner,
        pod: Option<&str>,
        resolution: Resolution,
    ) -> UsageSeries {
        let mut builder = SeriesBuilder::new(&self.timelines, resolution);
        for (namespace, pods) in &self.pods {
            for (name, history) in pods {
                let is_wanted = pod.is_none_or(|wanted| wanted == name);
                if is_wanted && owns(owner, namespace, history.controller.as_ref()) {
                    builder.add_pod(history, None);
                }
            }
        }
        builder.finish()
    }

    /// The ticks ever recorded; it never decreases.
    pub(crate) fn tick_count(&self) -> u64 {
        self.tick_count
    }

    /// The arrival time of the newest tick.
    pub(crate) fn newest_tick(&self) -> Option<jiff::Timestamp> {
        self.timelines.newest()
    }

    /// From the oldest kept point to the newest tick.
    pub(crate) fn span(&self) -> Option<Duration> {
        self.timelines.span()
    }
}

impl PodHistory {
    /// Records one kill unless it is known or older than a day. Past the mark limit the oldest
    /// mark drops.
    fn note_oom(&mut self, container: &str, finished_at: jiff::Timestamp, now: jiff::Timestamp) {
        if !is_recent(finished_at, now)
            || self
                .oom
                .iter()
                .any(|mark| mark.container == container && mark.at == finished_at)
        {
            return;
        }
        self.oom.push(OomMark {
            container: container.to_owned(),
            at: finished_at,
        });
        self.oom.sort_by_key(|mark| mark.at);
        let excess = self.oom.len().saturating_sub(MAX_OOM_MARKS);
        self.oom.drain(..excess);
    }
}

fn is_recent(at: jiff::Timestamp, now: jiff::Timestamp) -> bool {
    now.duration_since(at) <= OOM_KEPT
}

/// When `container` was OOM killed, per its current state and its last termination.
fn oom_kills(container: &ContainerSummary) -> impl Iterator<Item = jiff::Timestamp> {
    let current = match &container.state {
        ContainerState::Terminated(termination) => Some(termination),
        _ => None,
    };
    current
        .into_iter()
        .chain(container.last_termination.as_ref())
        .filter_map(oom_finished_at)
}

fn oom_finished_at(termination: &Termination) -> Option<jiff::Timestamp> {
    (termination.reason == Some(StatusReason::OomKilled))
        .then_some(termination.finished_at)
        .flatten()
}

/// Sums the series of one or more pods point by point.
struct SeriesBuilder<'a> {
    timelines: &'a Timelines,
    resolution: Resolution,
    values: Vec<Option<ResourceUsage>>,
    oom: Vec<jiff::Timestamp>,
    pod_count: usize,
    sampled_at: Option<jiff::Timestamp>,
    stalled_ticks: Option<u32>,
}

impl<'a> SeriesBuilder<'a> {
    fn new(timelines: &'a Timelines, resolution: Resolution) -> Self {
        Self {
            timelines,
            resolution,
            values: timelines.blank(resolution),
            oom: Vec::new(),
            pod_count: 0,
            sampled_at: None,
            stalled_ticks: None,
        }
    }

    /// Adds the pod's containers, or just `container`.
    fn add_pod(&mut self, history: &PodHistory, container: Option<&str>) {
        let mut has_value = false;
        for (name, rings) in &history.containers {
            if container.is_some_and(|wanted| wanted != name) {
                continue;
            }
            let points = self.timelines.values(rings, self.resolution);
            for (total, point) in self.values.iter_mut().zip(points) {
                let Some(point) = point else {
                    continue;
                };
                has_value = true;
                let usage = point.usage();
                *total = Some(total.map_or(usage, |sum| sum_usage(sum, usage)));
            }
        }
        self.pod_count += usize::from(has_value);
        self.oom.extend(
            history
                .oom
                .iter()
                .filter(|mark| container.is_none_or(|wanted| mark.container == wanted))
                .map(|mark| mark.at),
        );
        self.sampled_at = self.sampled_at.max(history.sampled_at);
        // A pod that did not report the newest tick is not running, so it is not "stale".
        if history
            .containers
            .values()
            .any(|rings| rings.newest().is_some())
        {
            let stalled = history.unchanged_ticks;
            self.stalled_ticks = Some(
                self.stalled_ticks
                    .map_or(stalled, |count| count.min(stalled)),
            );
        }
    }

    fn finish(mut self) -> UsageSeries {
        self.oom.sort();
        UsageSeries {
            points: self
                .timelines
                .times(self.resolution)
                .into_iter()
                .zip(self.values)
                .collect(),
            step: Timelines::step(self.resolution),
            oom: self.oom,
            pod_count: self.pod_count,
            sampled_at: self.sampled_at,
            stalled_ticks: self.stalled_ticks.unwrap_or(0),
        }
    }
}

fn sum_usage(left: ResourceUsage, right: ResourceUsage) -> ResourceUsage {
    ResourceUsage {
        cpu: CpuAmount::from_nanocores(left.cpu.nanocores().saturating_add(right.cpu.nanocores())),
        memory: ByteAmount::from_bytes(left.memory.bytes().saturating_add(right.memory.bytes())),
    }
}

/// The usage of every node seen recently, by name.
#[derive(Default)]
pub(crate) struct NodeUsageHistory {
    timelines: Timelines,
    tick_count: u64,
    nodes: BTreeMap<String, NodeHistory>,
}

struct NodeHistory {
    rings: Rings<ResourceUsage>,
    sampled_at: Option<jiff::Timestamp>,
    unchanged_ticks: u32,
}

impl NodeUsageHistory {
    /// Adds one tick, with the same freeing rules as the pods.
    pub(crate) fn record(&mut self, at: jiff::Timestamp, sample: &[NodeMetrics]) {
        self.tick_count += 1;
        let tick = self.tick_count;
        let closes_coarse = self.timelines.push(at, tick);
        for node in self.nodes.values_mut() {
            node.rings.begin_tick();
        }
        for node in sample {
            let make = || NodeHistory {
                rings: Rings::new(),
                sampled_at: None,
                unchanged_ticks: 0,
            };
            if let Some(history) = slot(&mut self.nodes, &node.name, make) {
                history.rings.record(node.usage, tick);
                history.unchanged_ticks =
                    next_unchanged(history.unchanged_ticks, history.sampled_at, node.sampled_at);
                history.sampled_at = node.sampled_at.or(history.sampled_at);
            }
        }
        if closes_coarse {
            for node in self.nodes.values_mut() {
                node.rings.fold();
            }
        }
        let timeline_len = self.timelines.fine_len();
        self.nodes.retain(|_, node| {
            debug_assert!(node.rings.len() <= timeline_len);
            node.rings.age(tick) == Retention::Keep
        });
    }

    pub(crate) fn latest(&self, node: &str) -> Option<ResourceUsage> {
        self.nodes.get(node)?.rings.newest()
    }

    pub(crate) fn node_series(&self, node: &str, resolution: Resolution) -> UsageSeries {
        let history = self.nodes.get(node);
        let values = match history {
            Some(history) => self.timelines.values(&history.rings, resolution),
            None => self.timelines.blank(resolution),
        };
        UsageSeries {
            points: self
                .timelines
                .times(resolution)
                .into_iter()
                .zip(values)
                .collect(),
            step: Timelines::step(resolution),
            oom: Vec::new(),
            pod_count: 0,
            sampled_at: history.and_then(|history| history.sampled_at),
            stalled_ticks: history
                .filter(|history| history.rings.newest().is_some())
                .map_or(0, |history| history.unchanged_ticks),
        }
    }

    pub(crate) fn tick_count(&self) -> u64 {
        self.tick_count
    }

    pub(crate) fn newest_tick(&self) -> Option<jiff::Timestamp> {
        self.timelines.newest()
    }

    pub(crate) fn span(&self) -> Option<Duration> {
        self.timelines.span()
    }
}

#[cfg(test)]
#[path = "metrics_history_tests.rs"]
mod metrics_history_tests;
