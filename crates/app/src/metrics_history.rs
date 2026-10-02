//! Recent pod and node usage, one tick per metrics poll. Every ring is bounded (see
//! `history_rings`), so memory stays flat however long the session runs.

use std::collections::BTreeMap;

use cluster::{ByteAmount, CpuAmount, NamespaceScope, NodeMetrics, PodMetrics, ResourceUsage};

use crate::history_rings::{Retention, Rings, Timeline};

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

fn saturating_u32(value: u64) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

/// The usage of every pod seen recently, by namespace and name.
#[derive(Default)]
pub(crate) struct PodUsageHistory {
    timeline: Timeline,
    tick_count: u64,
    pods: BTreeMap<String, BTreeMap<String, PodHistory>>,
}

#[derive(Default)]
struct PodHistory {
    containers: BTreeMap<String, Rings<UsagePoint>>,
}

impl PodUsageHistory {
    /// Adds one tick. A pod or container missing from `sample` reads `None` for it, and a
    /// series that stays missing is freed after an hour and dropped after a day.
    pub(crate) fn record(&mut self, at: jiff::Timestamp, sample: &[PodMetrics]) {
        self.timeline.push(at);
        self.tick_count += 1;
        let tick = self.tick_count;
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
            for container in &pod.containers {
                if let Some(rings) = slot(&mut history.containers, &container.name, Rings::new) {
                    rings.record(UsagePoint::of(container.usage), tick);
                }
            }
        }
        let timeline_len = self.timeline.len();
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
    }

    fn rings_mut(&mut self) -> impl Iterator<Item = &mut Rings<UsagePoint>> {
        self.pods
            .values_mut()
            .flat_map(BTreeMap::values_mut)
            .flat_map(|pod| pod.containers.values_mut())
    }

    /// Drops the pods outside `scope`. `All` keeps everything; the timeline stays.
    pub(crate) fn retain_scope(&mut self, scope: &NamespaceScope) {
        if matches!(scope, NamespaceScope::All) {
            return;
        }
        let namespaces = scope.namespaces();
        self.pods
            .retain(|namespace, _| namespaces.contains(namespace));
    }

    /// The pod's total at the newest tick; `None` when no container reported it.
    pub(crate) fn latest(&self, namespace: &str, pod: &str) -> Option<ResourceUsage> {
        let pod = self.pods.get(namespace)?.get(pod)?;
        pod.containers
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
        let rings = self
            .pods
            .get(namespace)?
            .get(pod)?
            .containers
            .get(container)?;
        rings.newest().map(UsagePoint::usage)
    }

    /// The ticks ever recorded; it never decreases.
    pub(crate) fn tick_count(&self) -> u64 {
        self.tick_count
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
    timeline: Timeline,
    tick_count: u64,
    nodes: BTreeMap<String, Rings<ResourceUsage>>,
}

impl NodeUsageHistory {
    /// Adds one tick, with the same freeing rules as the pods.
    pub(crate) fn record(&mut self, at: jiff::Timestamp, sample: &[NodeMetrics]) {
        self.timeline.push(at);
        self.tick_count += 1;
        let tick = self.tick_count;
        for rings in self.nodes.values_mut() {
            rings.begin_tick();
        }
        for node in sample {
            if let Some(rings) = slot(&mut self.nodes, &node.name, Rings::new) {
                rings.record(node.usage, tick);
            }
        }
        let timeline_len = self.timeline.len();
        self.nodes.retain(|_, rings| {
            debug_assert!(rings.len() <= timeline_len);
            rings.age(tick) == Retention::Keep
        });
    }

    pub(crate) fn latest(&self, node: &str) -> Option<ResourceUsage> {
        self.nodes.get(node)?.newest()
    }

    pub(crate) fn tick_count(&self) -> u64 {
        self.tick_count
    }
}

#[cfg(test)]
#[path = "metrics_history_tests.rs"]
mod metrics_history_tests;
