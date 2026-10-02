//! The Capacity panel of Overview: CPU, Memory, Pods, and Volumes summed over the cluster. Every
//! figure goes through `node_usage.rs`, so the numbers agree with the Nodes screen and drawer. Pure:
//! the inputs are the Ready snapshots.

use std::collections::{HashMap, HashSet};

use cluster::{NodeSummary, PodSummary, PvcUsage};

use crate::kind_join::is_shared_filesystem;
use crate::kind_row::KindObject;
use crate::metrics_history::NodeUsageHistory;
use crate::node_usage::{node_allocatable, node_pod_limit, requests_of, takes_room};
use crate::status_tone::StatusTone;
use crate::usage_format::{Measure, group_digits, usage_tone};

const ALLOCATABLE_CEILING: &str = "Allocatable includes NotReady and cordoned nodes.";
const COMPUTE_CEILING: &str =
    "Init-container requests are not counted. Allocatable includes NotReady and cordoned nodes.";
const REQUESTS_NOTE: &str = "Requests need all namespaces";
const POD_COUNTS_NOTE: &str = "Pod counts need all namespaces";
const UNCHECKED_VOLUMES_NOTE: &str = "Totals may include node filesystems";

/// What a figure that comes from the pods list has to show. The list is scoped, so the figure
/// needs the All scope; and it is unknown while the list loads or has failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FromPods<T> {
    Known(T),
    NeedsAllNamespaces,
    Pending,
}

impl<T: Copy> FromPods<T> {
    pub(crate) fn known(self) -> Option<T> {
        match self {
            Self::Known(value) => Some(value),
            Self::NeedsAllNamespaces | Self::Pending => None,
        }
    }

    fn map<U>(self, convert: impl FnOnce(T) -> U) -> FromPods<U> {
        match self {
            Self::Known(value) => FromPods::Known(convert(value)),
            Self::NeedsAllNamespaces => FromPods::NeedsAllNamespaces,
            Self::Pending => FromPods::Pending,
        }
    }
}

/// The three layers of one compute resource, in cores or bytes.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Layers {
    /// `None`: no node feed, or no node has a sample.
    pub(crate) used: Option<f64>,
    pub(crate) requested: FromPods<f64>,
    /// Greater than zero, else the row is omitted.
    pub(crate) allocatable: f64,
    /// The listed nodes, for the `used from {k} of {n} nodes` note.
    pub(crate) nodes: usize,
    /// Nodes without a metrics sample.
    pub(crate) unsampled_nodes: usize,
}

/// The newest kubelet sample of every claim that has a volume of its own and reports a capacity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct VolumeTotals {
    pub(crate) used: u64,
    pub(crate) capacity: u64,
    pub(crate) claims: usize,
    /// Claims left out because they report the node's disk (hostPath and local volumes).
    pub(crate) shared_claims: usize,
    /// The claim list was not loaded, so shared disks could not be told apart.
    pub(crate) is_sharing_unchecked: bool,
    /// Set when the kubelet poll covers part of the cluster only.
    pub(crate) limited: Option<String>,
}

/// The kubelet feed behind the Volumes row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum VolumeFeed {
    /// Not polling; `reason` is the feed's own explanation, when it has one.
    Unavailable {
        reason: Option<String>,
    },
    Live(VolumeTotals),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum CapacityRow {
    Cpu(Layers),
    Memory(Layers),
    Pods {
        taking_room: FromPods<usize>,
        allocatable: u64,
    },
    Volumes(VolumeFeed),
}

/// A piece of a row's right-hand label, toned when it is a used or requested figure.
pub(crate) struct LabelPart {
    pub(crate) text: String,
    pub(crate) tone: Option<StatusTone>,
}

impl CapacityRow {
    pub(crate) fn name(&self) -> &'static str {
        match self {
            Self::Cpu(_) => "CPU",
            Self::Memory(_) => "Memory",
            Self::Pods { .. } => "Pods",
            Self::Volumes(_) => "Volumes",
        }
    }

    /// The right-hand figures, such as `104 used · 131 req · 168 cores`, split so the used and
    /// requested figures can carry a tone.
    pub(crate) fn label_parts(&self) -> Vec<LabelPart> {
        match self {
            Self::Cpu(layers) => compute_label(Measure::Cpu, layers),
            Self::Memory(layers) => compute_label(Measure::Bytes, layers),
            Self::Pods {
                taking_room,
                allocatable,
            } => {
                let used = taking_room
                    .known()
                    .map_or_else(|| "—".to_owned(), group_digits);
                vec![plain(format!(
                    "{used} / {}",
                    group_digits_u64(*allocatable)
                ))]
            }
            Self::Volumes(VolumeFeed::Live(totals)) if totals.claims == 0 => {
                vec![plain("—".to_owned())]
            }
            Self::Volumes(VolumeFeed::Live(totals)) => {
                let pair =
                    Measure::Bytes.format_pair(totals.used as f64, totals.capacity as f64, " / ");
                let unit = if totals.claims == 1 { "PVC" } else { "PVCs" };
                vec![plain(format!("{pair} · {} {unit}", totals.claims))]
            }
            Self::Volumes(VolumeFeed::Unavailable { .. }) => vec![plain("—".to_owned())],
        }
    }

    /// The muted line under the bar.
    pub(crate) fn note(&self) -> Option<String> {
        let mut notes = Vec::new();
        match self {
            Self::Cpu(layers) | Self::Memory(layers) => {
                if layers.used.is_some() && layers.unsampled_nodes > 0 {
                    notes.push(format!(
                        "used from {} of {} nodes",
                        layers.nodes - layers.unsampled_nodes,
                        layers.nodes
                    ));
                }
                if layers.requested == FromPods::NeedsAllNamespaces {
                    notes.push(REQUESTS_NOTE.to_owned());
                }
            }
            Self::Pods { taking_room, .. } => {
                if *taking_room == FromPods::NeedsAllNamespaces {
                    notes.push(POD_COUNTS_NOTE.to_owned());
                }
            }
            Self::Volumes(VolumeFeed::Live(totals)) => {
                if totals.shared_claims > 0 {
                    notes.push(shared_claims_note(totals.shared_claims));
                }
                if totals.is_sharing_unchecked {
                    notes.push(UNCHECKED_VOLUMES_NOTE.to_owned());
                }
                if let Some(limited) = &totals.limited {
                    notes.push(format!("Volume usage: {limited}."));
                }
            }
            Self::Volumes(VolumeFeed::Unavailable { .. }) => {}
        }
        (!notes.is_empty()).then(|| notes.join(" · "))
    }

    /// What the row leaves out, for its tooltip.
    pub(crate) fn ceiling(&self) -> Option<&'static str> {
        match self {
            Self::Cpu(_) | Self::Memory(_) => Some(COMPUTE_CEILING),
            Self::Pods { .. } => Some(ALLOCATABLE_CEILING),
            Self::Volumes(_) => None,
        }
    }

    /// Why a Volumes row without a feed shows `—`.
    pub(crate) fn unavailable_reason(&self) -> Option<&str> {
        match self {
            Self::Volumes(VolumeFeed::Unavailable { reason }) => reason.as_deref(),
            _ => None,
        }
    }

    /// Whether the bar draws a requested layer.
    pub(crate) fn has_requested(&self) -> bool {
        matches!(self, Self::Cpu(layers) | Self::Memory(layers) if layers.requested.known().is_some())
    }

    /// The used and requested shares of the bar's total, `None` where a layer is unknown.
    pub(crate) fn ratios(&self) -> (Option<f64>, Option<f64>) {
        match self {
            Self::Cpu(layers) | Self::Memory(layers) => (
                layers.used.map(|used| used / layers.allocatable),
                layers
                    .requested
                    .known()
                    .map(|requested| requested / layers.allocatable),
            ),
            Self::Pods {
                taking_room,
                allocatable,
            } => (
                taking_room
                    .known()
                    .map(|count| count as f64 / (*allocatable).max(1) as f64),
                None,
            ),
            Self::Volumes(VolumeFeed::Live(totals)) if totals.claims > 0 => (
                Some(totals.used as f64 / totals.capacity.max(1) as f64),
                None,
            ),
            Self::Volumes(VolumeFeed::Live(_)) => (None, None),
            Self::Volumes(VolumeFeed::Unavailable { .. }) => (None, None),
        }
    }
}

fn shared_claims_note(count: usize) -> String {
    if count == 1 {
        "1 claim shares a node disk".to_owned()
    } else {
        format!("{count} claims share node disks")
    }
}

fn plain(text: String) -> LabelPart {
    LabelPart { text, tone: None }
}

fn group_digits_u64(number: u64) -> String {
    group_digits(usize::try_from(number).unwrap_or(usize::MAX))
}

/// `104 used · 131 req · 168 cores`. A missing used prints `—`. A request that needs another scope
/// drops its part; one that is still loading prints `—`.
fn compute_label(measure: Measure, layers: &Layers) -> Vec<LabelPart> {
    let figures: Vec<f64> = [layers.used, layers.requested.known()]
        .into_iter()
        .flatten()
        .chain([layers.allocatable])
        .collect();
    let mut shared = measure.format_shared(&figures).into_iter();
    let mut part = |is_known: bool, suffix: &str, value: Option<f64>| {
        let text = if is_known {
            shared.next().unwrap_or_default()
        } else {
            "—".to_owned()
        };
        LabelPart {
            text: format!("{text} {suffix}"),
            tone: value.and_then(|value| usage_tone(value / layers.allocatable)),
        }
    };
    let mut parts = vec![part(layers.used.is_some(), "used", layers.used)];
    match layers.requested {
        FromPods::Known(requested) => {
            parts.push(plain(" · ".to_owned()));
            parts.push(part(true, "req", Some(requested)));
        }
        FromPods::Pending => {
            parts.push(plain(" · ".to_owned()));
            parts.push(part(false, "req", None));
        }
        FromPods::NeedsAllNamespaces => {}
    }
    parts.push(plain(" · ".to_owned()));
    parts.push(plain(shared.next().unwrap_or_default()));
    parts
}

pub(crate) struct CapacityInputs<'a> {
    pub(crate) nodes: &'a [NodeSummary],
    /// The pods list when the scope is All and the list is Ready.
    pub(crate) pods: FromPods<&'a [PodSummary]>,
    /// `None` unless the node feed is Live or Interrupted.
    pub(crate) node_usage: Option<&'a NodeUsageHistory>,
    pub(crate) volumes: VolumeFeed,
}

/// CPU, Memory, Pods, and Volumes, each only when it has something to show.
pub(crate) fn cluster_capacity(inputs: &CapacityInputs) -> Vec<CapacityRow> {
    let mut rows = Vec::new();
    let (cpu, memory, taking_room) = sum_nodes(inputs);
    rows.extend(cpu.map(CapacityRow::Cpu));
    rows.extend(memory.map(CapacityRow::Memory));
    let allocatable: u64 = inputs.nodes.iter().filter_map(node_pod_limit).sum();
    if allocatable > 0 {
        rows.push(CapacityRow::Pods {
            taking_room,
            allocatable,
        });
    }
    match &inputs.volumes {
        // A feed whose claims all share node disks still shows its row, with the note saying so.
        VolumeFeed::Live(totals) if totals.claims == 0 && totals.shared_claims == 0 => {}
        volumes => rows.push(CapacityRow::Volumes(volumes.clone())),
    }
    rows
}

/// Used, requested, and allocatable summed over the nodes for CPU and Memory, and the pods that
/// take room on them. The pods are read once: only those on a listed node count, so the sums
/// agree with the per-node numbers of the Nodes screen.
fn sum_nodes(inputs: &CapacityInputs) -> (Option<Layers>, Option<Layers>, FromPods<usize>) {
    let (mut cpu_allocatable, mut memory_allocatable) = (0., 0.);
    let mut samples = Vec::new();
    for node in inputs.nodes {
        let (cpu, memory) = node_allocatable(node);
        cpu_allocatable += cpu.map_or(0., |cpu| cpu.cores());
        memory_allocatable += memory.map_or(0., |memory| memory.bytes() as f64);
        if let Some(latest) = inputs.node_usage.and_then(|usage| usage.latest(&node.name)) {
            samples.push(latest);
        }
    }
    let node_names: HashSet<&str> = inputs.nodes.iter().map(|node| node.name.as_str()).collect();
    let requests = inputs.pods.map(|pods| {
        let on_listed_nodes = || {
            pods.iter().filter(|pod| {
                takes_room(pod)
                    && pod
                        .node_name
                        .as_deref()
                        .is_some_and(|node| node_names.contains(node))
            })
        };
        let (cpu, memory) = requests_of(on_listed_nodes());
        (
            cpu.cores(),
            memory.bytes() as f64,
            on_listed_nodes().count(),
        )
    });
    let has_feed = inputs.node_usage.is_some();
    let has_samples = !samples.is_empty();
    let unsampled_nodes = if has_feed {
        inputs.nodes.len() - samples.len()
    } else {
        0
    };
    let layers = |used: f64, requested: FromPods<f64>, allocatable: f64| {
        (allocatable > 0.).then(|| Layers {
            used: has_samples.then_some(used),
            requested,
            allocatable,
            nodes: inputs.nodes.len(),
            unsampled_nodes,
        })
    };
    let cpu_used = samples.iter().map(|usage| usage.cpu.cores()).sum();
    let memory_used = samples
        .iter()
        .map(|usage| usage.memory.bytes() as f64)
        .sum();
    (
        layers(cpu_used, requests.map(|(cpu, ..)| cpu), cpu_allocatable),
        layers(
            memory_used,
            requests.map(|(_, memory, _)| memory),
            memory_allocatable,
        ),
        requests.map(|(.., count)| count),
    )
}

/// The claims that report a capacity, summed. A claim whose kubelet numbers exceed its own size
/// reports the node's disk, so it is skipped and counted; `claims` is the list of PVC objects, or
/// `None` while it is not loaded. `limited` is for the caller to set.
pub(crate) fn volume_totals<'a>(
    usages: impl Iterator<Item = &'a PvcUsage>,
    claims: Option<&[KindObject]>,
) -> VolumeTotals {
    let sizes: HashMap<(&str, &str), Option<&str>> = claims
        .into_iter()
        .flatten()
        .filter_map(|object| match object {
            KindObject::PersistentVolumeClaim(claim) => Some((
                (claim.namespace.as_str(), claim.name.as_str()),
                claim.capacity.as_deref(),
            )),
            _ => None,
        })
        .collect();
    let mut totals = VolumeTotals {
        used: 0,
        capacity: 0,
        claims: 0,
        shared_claims: 0,
        is_sharing_unchecked: claims.is_none(),
        limited: None,
    };
    for usage in usages {
        let Some(capacity) = usage.capacity else {
            continue;
        };
        let claimed = sizes
            .get(&(usage.namespace.as_str(), usage.claim.as_str()))
            .copied()
            .flatten();
        if is_shared_filesystem(usage, claimed) {
            totals.shared_claims += 1;
            continue;
        }
        totals.capacity = totals.capacity.saturating_add(capacity.bytes());
        totals.used = totals
            .used
            .saturating_add(usage.used.map_or(0, |used| used.bytes()));
        totals.claims += 1;
    }
    totals
}

#[cfg(test)]
#[path = "cluster_capacity_tests.rs"]
mod cluster_capacity_tests;
