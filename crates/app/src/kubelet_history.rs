//! Rates of the kubelet counters (network, disk I/O), one tick per kubelet round, in the fine
//! and coarse rings of `history_rings`, plus the newest PVC usage. A counter is turned into
//! bytes per second between two kubelet sample times; a reset or a replaced pod leaves a gap.

use std::collections::BTreeMap;
use std::time::Duration;

use cluster::{
    ControllerRef, DiskIoSample, NamespaceScope, NodeKubeletStats, PodKubeletStats, PodSummary,
    PvcUsage,
};

use crate::history_rings::{
    DROP_AFTER_TICKS, FINE_TICKS, Resolution, Retention, RingPoint, Rings, Timelines,
};
use crate::kind_row::{PodOwner, owns};

/// A smaller step between two samples is the same scrape, not a new one.
const MIN_SAMPLE_GAP_MILLIS: i64 = 1_000;

/// Bytes per second: receive and transmit (network) or read and write (disk).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct RatePair<T> {
    pub(crate) first: T,
    pub(crate) second: T,
}

impl From<RatePair<u32>> for RatePair<u64> {
    fn from(rate: RatePair<u32>) -> Self {
        Self {
            first: u64::from(rate.first),
            second: u64::from(rate.second),
        }
    }
}

impl RatePair<u64> {
    /// Saturates at 4 GiB/s, far above what one pod or container does.
    fn saturating_u32(self) -> RatePair<u32> {
        RatePair {
            first: u32::try_from(self.first).unwrap_or(u32::MAX),
            second: u32::try_from(self.second).unwrap_or(u32::MAX),
        }
    }
}

/// The mean of `values`, which are never empty.
fn mean_of(values: impl Iterator<Item = u64>, count: usize) -> u64 {
    let sum: u128 = values.map(u128::from).sum();
    u64::try_from(sum / count as u128).unwrap_or(u64::MAX)
}

impl RingPoint for RatePair<u64> {
    fn mean(points: &[Self]) -> Self {
        Self {
            first: mean_of(points.iter().map(|point| point.first), points.len()),
            second: mean_of(points.iter().map(|point| point.second), points.len()),
        }
    }
}

impl RingPoint for RatePair<u32> {
    fn mean(points: &[Self]) -> Self {
        let mean = |field: fn(&Self) -> u32| {
            let mean = mean_of(
                points.iter().map(|point| u64::from(field(point))),
                points.len(),
            );
            u32::try_from(mean).unwrap_or(u32::MAX)
        };
        Self {
            first: mean(|point| point.first),
            second: mean(|point| point.second),
        }
    }
}

/// The last counter sample of a series and the rate it produced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CounterState {
    at: jiff::Timestamp,
    first: u64,
    second: u64,
    last_rate: Option<RatePair<u64>>,
}

/// The rate since `previous` and the state to keep (decisions 16 and 17).
fn next_rate(
    previous: Option<&CounterState>,
    at: jiff::Timestamp,
    first: u64,
    second: u64,
) -> (Option<RatePair<u64>>, CounterState) {
    let current = CounterState {
        at,
        first,
        second,
        last_rate: None,
    };
    let Some(previous) = previous else {
        return (None, current);
    };
    let millis = at
        .as_millisecond()
        .saturating_sub(previous.at.as_millisecond());
    // The same scrape again, or a clock that stepped back: nothing new to say.
    if millis < MIN_SAMPLE_GAP_MILLIS {
        return (previous.last_rate, *previous);
    }
    if first < previous.first || second < previous.second {
        return (None, current);
    }
    let per_second = |delta: u64| {
        let millis = millis as u128;
        let rate = (u128::from(delta) * 1_000 + millis / 2) / millis;
        u64::try_from(rate).unwrap_or(u64::MAX)
    };
    let rate = RatePair {
        first: per_second(first - previous.first),
        second: per_second(second - previous.second),
    };
    let state = CounterState {
        last_rate: Some(rate),
        ..current
    };
    (Some(rate), state)
}

/// One counter pair and the rings of its rates. The rings exist from the first rate on.
struct CounterRings<P> {
    counter: Option<CounterState>,
    rings: Option<Rings<P>>,
}

impl<P: RingPoint> CounterRings<P> {
    fn new() -> Self {
        Self {
            counter: None,
            rings: None,
        }
    }

    fn begin_tick(&mut self) {
        if let Some(rings) = &mut self.rings {
            rings.begin_tick();
        }
    }

    /// The next sample starts a new counter; the rings keep their points.
    fn reset_counter(&mut self) {
        self.counter = None;
    }

    /// Reads the counters at `at` and records the rate, if there is one, in the open tick.
    fn observe(
        &mut self,
        at: jiff::Timestamp,
        first: u64,
        second: u64,
        tick: u64,
        convert: fn(RatePair<u64>) -> P,
    ) {
        let (rate, state) = next_rate(self.counter.as_ref(), at, first, second);
        self.counter = Some(state);
        if let Some(rate) = rate {
            self.rings
                .get_or_insert_with(Rings::new)
                .record(convert(rate), tick);
        }
    }

    fn fold(&mut self) {
        if let Some(rings) = &mut self.rings {
            rings.fold();
        }
    }

    /// Frees the fine ring after an hour without a rate; the rings go after a day.
    fn age(&mut self, tick: u64) {
        let is_dropped = self
            .rings
            .as_mut()
            .is_some_and(|rings| rings.age(tick) == Retention::Drop);
        if is_dropped {
            self.rings = None;
        }
    }
}

/// What a node's newest cAdvisor read said.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DiskIoState {
    /// Not read in the newest round: not a disk target, failed, or no round yet.
    NotSampled,
    /// Read; `has_root` is whether the root cgroup (`id="/"`) had a series.
    Sampled { has_root: bool },
}

/// Which counter pair a series reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RateKind {
    /// Receive and transmit.
    Network,
    /// Read and write.
    DiskIo,
}

/// Rates cut to one resolution, ready for a chart or a table.
pub(crate) struct RateSeries {
    /// Oldest first; `None` where the series has no rate.
    pub(crate) points: Vec<(jiff::Timestamp, Option<RatePair<u64>>)>,
    /// The spacing of the points, for splitting a line at a gap.
    pub(crate) step: Duration,
}

struct NodeKubelet {
    network: CounterRings<RatePair<u64>>,
    disk: CounterRings<RatePair<u64>>,
    disk_io: DiskIoState,
    last_seen: u64,
}

impl NodeKubelet {
    fn new(tick: u64) -> Self {
        Self {
            network: CounterRings::new(),
            disk: CounterRings::new(),
            disk_io: DiskIoState::NotSampled,
            last_seen: tick,
        }
    }
}

struct ContainerKubelet {
    disk: CounterRings<RatePair<u32>>,
    last_seen: u64,
}

struct PodKubelet {
    /// Unknown while only a disk series has named the pod.
    uid: Option<String>,
    /// The first sight fills it, so a workload's history keeps the pods a rollout replaced.
    controller: Option<ControllerRef>,
    /// `None` for a host-network pod, whose counters are the node's.
    network: Option<CounterRings<RatePair<u32>>>,
    containers: BTreeMap<String, ContainerKubelet>,
    last_seen: u64,
}

impl PodKubelet {
    fn new(tick: u64) -> Self {
        Self {
            uid: None,
            controller: None,
            network: None,
            containers: BTreeMap::new(),
            last_seen: tick,
        }
    }

    fn reset_counters(&mut self) {
        if let Some(network) = &mut self.network {
            network.reset_counter();
        }
        for container in self.containers.values_mut() {
            container.disk.reset_counter();
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct PodKey {
    namespace: String,
    name: String,
}

struct SeenPvc {
    usage: PvcUsage,
    last_seen: u64,
}

/// The kubelet rates of the nodes and pods of a session, and the newest PVC usage.
#[derive(Default)]
pub(crate) struct KubeletHistory {
    timelines: Timelines,
    tick_count: u64,
    nodes: BTreeMap<String, NodeKubelet>,
    pods: BTreeMap<PodKey, PodKubelet>,
    /// By namespace, then claim, so a lookup needs no owned key.
    pvcs: BTreeMap<String, BTreeMap<String, SeenPvc>>,
}

fn is_in_scope(scope: &NamespaceScope, namespace: &str) -> bool {
    matches!(scope, NamespaceScope::All) || scope.namespaces().iter().any(|name| name == namespace)
}

/// `pods` is ordered by (namespace, name), like every pods snapshot.
fn find_pod<'a>(pods: &'a [PodSummary], namespace: &str, name: &str) -> Option<&'a PodSummary> {
    let index = pods
        .binary_search_by(|pod| (pod.namespace.as_str(), pod.name.as_str()).cmp(&(namespace, name)))
        .ok()?;
    pods.get(index)
}

impl KubeletHistory {
    /// Adds one tick for the nodes of `round`. A series with no rate this tick reads `None`; a
    /// node that failed adds nothing. Only pods in `scope` are stored. `pods` is the current pods
    /// list, for host-network flags and controllers; it may lag `round`.
    #[cfg_attr(
        feature = "hotpath-profiling",
        hotpath::measure(impl_type = "KubeletHistory")
    )]
    pub(crate) fn record(
        &mut self,
        at: jiff::Timestamp,
        round: &[NodeKubeletStats],
        pods: &[PodSummary],
        scope: &NamespaceScope,
    ) {
        debug_assert!(
            pods.is_sorted_by(
                |left, right| (&left.namespace, &left.name) <= (&right.namespace, &right.name)
            ),
            "the pods list is ordered by namespace and name"
        );
        self.tick_count += 1;
        let tick = self.tick_count;
        let closes_coarse = self.timelines.push(at, tick);
        self.begin_tick();
        for stats in round {
            if let Ok(summary) = &stats.summary {
                let node = self
                    .nodes
                    .entry(stats.node.clone())
                    .or_insert_with(|| NodeKubelet::new(tick));
                node.last_seen = tick;
                if let Some(network) = &summary.network {
                    let sampled_at = network.sampled_at.unwrap_or(at);
                    node.network.observe(
                        sampled_at,
                        network.rx_bytes,
                        network.tx_bytes,
                        tick,
                        identity,
                    );
                }
                for pod in &summary.pods {
                    self.record_pod(at, tick, pod, pods, scope);
                }
            }
            if let Some(Ok(sample)) = &stats.disk_io {
                self.record_disk_io(at, tick, &stats.node, sample, scope);
            }
        }
        if closes_coarse {
            self.fold();
        }
        self.age(tick);
        self.note_controllers(pods);
    }

    fn begin_tick(&mut self) {
        for node in self.nodes.values_mut() {
            node.disk_io = DiskIoState::NotSampled;
            node.network.begin_tick();
            node.disk.begin_tick();
        }
        for pod in self.pods.values_mut() {
            if let Some(network) = &mut pod.network {
                network.begin_tick();
            }
            for container in pod.containers.values_mut() {
                container.disk.begin_tick();
            }
        }
    }

    fn fold(&mut self) {
        for node in self.nodes.values_mut() {
            node.network.fold();
            node.disk.fold();
        }
        for pod in self.pods.values_mut() {
            if let Some(network) = &mut pod.network {
                network.fold();
            }
            for container in pod.containers.values_mut() {
                container.disk.fold();
            }
        }
    }

    /// Frees the rings that stopped reporting and forgets what has been unseen for a day.
    fn age(&mut self, tick: u64) {
        let is_stale = |last_seen: u64| tick.saturating_sub(last_seen) >= DROP_AFTER_TICKS;
        self.nodes.retain(|_, node| !is_stale(node.last_seen));
        for node in self.nodes.values_mut() {
            node.network.age(tick);
            node.disk.age(tick);
        }
        self.pods.retain(|_, pod| !is_stale(pod.last_seen));
        for pod in self.pods.values_mut() {
            if let Some(network) = &mut pod.network {
                network.age(tick);
            }
            pod.containers
                .retain(|_, container| !is_stale(container.last_seen));
            for container in pod.containers.values_mut() {
                container.disk.age(tick);
            }
        }
        for claims in self.pvcs.values_mut() {
            claims.retain(|_, pvc| tick.saturating_sub(pvc.last_seen) < FINE_TICKS as u64);
        }
        self.pvcs.retain(|_, claims| !claims.is_empty());
    }

    fn record_pod(
        &mut self,
        at: jiff::Timestamp,
        tick: u64,
        stats: &PodKubeletStats,
        pods: &[PodSummary],
        scope: &NamespaceScope,
    ) {
        if !is_in_scope(scope, &stats.namespace) {
            return;
        }
        let key = PodKey {
            namespace: stats.namespace.clone(),
            name: stats.name.clone(),
        };
        let pod = self
            .pods
            .entry(key)
            .or_insert_with(|| PodKubelet::new(tick));
        // A StatefulSet pod reuses its name: the new pod's counters start again, while the
        // rings and the controller stay with the name.
        if pod.uid.as_ref().is_some_and(|uid| *uid != stats.uid) {
            pod.reset_counters();
        }
        pod.uid = Some(stats.uid.clone());
        pod.last_seen = tick;
        let is_host_network =
            find_pod(pods, &stats.namespace, &stats.name).is_some_and(|pod| pod.host_network);
        if is_host_network {
            pod.network = None;
        } else if let Some(network) = &stats.network {
            pod.network.get_or_insert_with(CounterRings::new).observe(
                network.sampled_at.unwrap_or(at),
                network.rx_bytes,
                network.tx_bytes,
                tick,
                RatePair::saturating_u32,
            );
        }
        for usage in &stats.volumes {
            self.keep_pvc(usage, tick);
        }
    }

    /// An RWX claim appears under several pods; the newest sample wins.
    fn keep_pvc(&mut self, usage: &PvcUsage, tick: u64) {
        let claims = self.pvcs.entry(usage.namespace.clone()).or_default();
        match claims.get_mut(&usage.claim) {
            Some(seen) => {
                seen.last_seen = tick;
                if seen.usage.sampled_at <= usage.sampled_at {
                    seen.usage = usage.clone();
                }
            }
            None => {
                let seen = SeenPvc {
                    usage: usage.clone(),
                    last_seen: tick,
                };
                claims.insert(usage.claim.clone(), seen);
            }
        }
    }

    fn record_disk_io(
        &mut self,
        at: jiff::Timestamp,
        tick: u64,
        node: &str,
        sample: &DiskIoSample,
        scope: &NamespaceScope,
    ) {
        let entry = self
            .nodes
            .entry(node.to_owned())
            .or_insert_with(|| NodeKubelet::new(tick));
        entry.last_seen = tick;
        entry.disk_io = DiskIoState::Sampled {
            has_root: sample.node.is_some(),
        };
        if let Some(counters) = &sample.node {
            let sampled_at = counters.sampled_at.unwrap_or(at);
            entry.disk.observe(
                sampled_at,
                counters.read_bytes,
                counters.write_bytes,
                tick,
                identity,
            );
        }
        for container in &sample.containers {
            if !is_in_scope(scope, &container.namespace) {
                continue;
            }
            let key = PodKey {
                namespace: container.namespace.clone(),
                name: container.pod.clone(),
            };
            let pod = self
                .pods
                .entry(key)
                .or_insert_with(|| PodKubelet::new(tick));
            pod.last_seen = tick;
            let series = pod
                .containers
                .entry(container.container.clone())
                .or_insert_with(|| ContainerKubelet {
                    disk: CounterRings::new(),
                    last_seen: tick,
                });
            series.last_seen = tick;
            let counters = &container.counters;
            series.disk.observe(
                counters.sampled_at.unwrap_or(at),
                counters.read_bytes,
                counters.write_bytes,
                tick,
                RatePair::saturating_u32,
            );
        }
    }

    /// Fills the controllers that are still unknown.
    fn note_controllers(&mut self, pods: &[PodSummary]) {
        for (key, history) in &mut self.pods {
            if history.controller.is_some() {
                continue;
            }
            if let Some(pod) = find_pod(pods, &key.namespace, &key.name) {
                history.controller.clone_from(&pod.controller);
            }
        }
    }

    /// The newest kubelet sample of a claim.
    pub(crate) fn pvc_usage(&self, namespace: &str, claim: &str) -> Option<&PvcUsage> {
        self.pvcs.get(namespace)?.get(claim).map(|seen| &seen.usage)
    }

    /// The newest sample of every claim.
    pub(crate) fn pvc_usages(&self) -> impl Iterator<Item = &PvcUsage> {
        self.pvcs
            .values()
            .flat_map(BTreeMap::values)
            .map(|seen| &seen.usage)
    }

    /// Drops the pods and claims outside `scope`. `All` keeps everything; the timelines stay.
    pub(crate) fn retain_scope(&mut self, scope: &NamespaceScope) {
        if matches!(scope, NamespaceScope::All) {
            return;
        }
        self.pods
            .retain(|key, _| is_in_scope(scope, &key.namespace));
        self.pvcs
            .retain(|namespace, _| is_in_scope(scope, namespace));
    }

    /// The ticks ever recorded; it never decreases.
    pub(crate) fn tick_count(&self) -> u64 {
        self.tick_count
    }

    /// The newest disk read of `node`.
    pub(crate) fn disk_io_state(&self, node: &str) -> DiskIoState {
        self.nodes
            .get(node)
            .map_or(DiskIoState::NotSampled, |node| node.disk_io)
    }

    /// Whether the kubelet has reported a disk series for the pod's `container`, or for any
    /// container of the pod.
    pub(crate) fn has_disk_series(
        &self,
        namespace: &str,
        pod: &str,
        container: Option<&str>,
    ) -> bool {
        self.pod(namespace, pod).is_some_and(|pod| match container {
            Some(container) => pod.containers.contains_key(container),
            None => !pod.containers.is_empty(),
        })
    }

    fn pod(&self, namespace: &str, name: &str) -> Option<&PodKubelet> {
        // The lookup key is owned, which is fine for a read that happens once per redraw.
        let key = PodKey {
            namespace: namespace.to_owned(),
            name: name.to_owned(),
        };
        self.pods.get(&key)
    }

    /// One pod. Network is the pod's own, whatever `container` says: its containers share the
    /// network namespace. Disk I/O is the sum of its containers, or just `container`.
    pub(crate) fn pod_rates(
        &self,
        kind: RateKind,
        namespace: &str,
        pod: &str,
        container: Option<&str>,
        resolution: Resolution,
    ) -> RateSeries {
        let mut sum = RateSum::new(&self.timelines, resolution);
        if let Some(pod) = self.pod(namespace, pod) {
            sum.add_pod(pod, kind, container);
        }
        sum.finish()
    }

    /// The pods `owner` owns by namespace and controller, summed, or just `pod` among them.
    /// Host-network pods have no network rings, so they add nothing.
    pub(crate) fn owner_rates(
        &self,
        kind: RateKind,
        owner: &PodOwner,
        pod: Option<&str>,
        resolution: Resolution,
    ) -> RateSeries {
        let mut sum = RateSum::new(&self.timelines, resolution);
        for (key, entry) in &self.pods {
            let is_wanted = pod.is_none_or(|wanted| wanted == key.name);
            if is_wanted && owns(owner, &key.namespace, entry.controller.as_ref()) {
                sum.add_pod(entry, kind, None);
            }
        }
        sum.finish()
    }

    pub(crate) fn node_rates(
        &self,
        kind: RateKind,
        node: &str,
        resolution: Resolution,
    ) -> RateSeries {
        let mut sum = RateSum::new(&self.timelines, resolution);
        if let Some(node) = self.nodes.get(node) {
            let series = match kind {
                RateKind::Network => &node.network,
                RateKind::DiskIo => &node.disk,
            };
            sum.add(series.rings.as_ref());
        }
        sum.finish()
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

/// Sums rate series point by point on the kubelet timeline.
struct RateSum<'a> {
    timelines: &'a Timelines,
    resolution: Resolution,
    values: Vec<Option<RatePair<u64>>>,
}

impl<'a> RateSum<'a> {
    fn new(timelines: &'a Timelines, resolution: Resolution) -> Self {
        Self {
            timelines,
            resolution,
            values: timelines.blank(resolution),
        }
    }

    /// Adds one series; `None` (no rings yet) adds nothing.
    fn add<P: Copy + Into<RatePair<u64>>>(&mut self, rings: Option<&Rings<P>>) {
        let Some(rings) = rings else {
            return;
        };
        let points = self.timelines.values(rings, self.resolution);
        for (total, point) in self.values.iter_mut().zip(points) {
            let Some(point) = point else {
                continue;
            };
            let rate: RatePair<u64> = point.into();
            *total = Some(total.map_or(rate, |sum| RatePair {
                first: sum.first.saturating_add(rate.first),
                second: sum.second.saturating_add(rate.second),
            }));
        }
    }

    fn add_pod(&mut self, pod: &PodKubelet, kind: RateKind, container: Option<&str>) {
        match kind {
            RateKind::Network => {
                self.add(
                    pod.network
                        .as_ref()
                        .and_then(|network| network.rings.as_ref()),
                );
            }
            RateKind::DiskIo => {
                for (name, series) in &pod.containers {
                    if container.is_none_or(|wanted| wanted == name) {
                        self.add(series.disk.rings.as_ref());
                    }
                }
            }
        }
    }

    fn finish(self) -> RateSeries {
        RateSeries {
            points: self
                .timelines
                .times(self.resolution)
                .into_iter()
                .zip(self.values)
                .collect(),
            step: Timelines::step(self.resolution),
        }
    }
}

fn identity(rate: RatePair<u64>) -> RatePair<u64> {
    rate
}

#[cfg(test)]
#[path = "kubelet_history_tests.rs"]
mod kubelet_history_tests;
