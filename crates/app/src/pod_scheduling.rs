//! The exact selector, affinity, and request a Pending pod's scheduler message talks about. The
//! message only counts nodes ("1 node(s) didn't match Pod's node affinity/selector"), so this
//! reads the pod and the node list and says which label or size is missing. Pure.

use std::collections::BTreeSet;

use cluster::{NodeSummary, PodSummary};

use crate::node_usage::{node_allocatable, requests_of};
use crate::usage_format::Measure;

/// The scheduler's wording for a node that misses the selector or the required affinity.
const SELECTOR_MISMATCH: &str = "node affinity/selector";

/// One line for each cause the scheduler `message` names and the pod and nodes can pin down.
pub(crate) fn scheduling_hints(
    pod: &PodSummary,
    nodes: &[NodeSummary],
    message: &str,
) -> Vec<String> {
    let mut hints = Vec::new();
    if message.contains(SELECTOR_MISMATCH) {
        hints.extend(selector_hint(pod, nodes));
        // ponytail: affinity terms are quoted, not evaluated; evaluate them when a lab case needs it.
        hints.extend(
            pod.node_affinity
                .iter()
                .map(|term| format!("required node affinity {term}")),
        );
    }
    for resource in ["cpu", "memory"] {
        if message.contains(&format!("Insufficient {resource}")) {
            hints.extend(oversized_request_hint(pod, nodes, resource));
        }
    }
    hints
}

/// The selector terms no node can satisfy, with the values the nodes do carry for the key.
fn selector_hint(pod: &PodSummary, nodes: &[NodeSummary]) -> Option<String> {
    if pod.node_selector.is_empty() {
        return None;
    }
    let has_term = |node: &NodeSummary, term: &String| node.labels.contains(term);
    if nodes
        .iter()
        .any(|node| pod.node_selector.iter().all(|term| has_term(node, term)))
    {
        return None;
    }
    let missing: Vec<&String> = pod
        .node_selector
        .iter()
        .filter(|term| !nodes.iter().any(|node| has_term(node, term)))
        .collect();
    if missing.is_empty() {
        return Some(format!(
            "nodeSelector {} — no single node has all of them",
            pod.node_selector.join(", ")
        ));
    }
    Some(
        missing
            .into_iter()
            .map(|term| format!("nodeSelector {term} — {}", node_values_text(term, nodes)))
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

/// `no node has it (nodes have pool: web, data)`, or `no node carries the key` when no node has
/// the key at all.
fn node_values_text(term: &str, nodes: &[NodeSummary]) -> String {
    let key = term.split_once('=').map_or(term, |(key, _)| key);
    let prefix = format!("{key}=");
    let values: BTreeSet<&str> = nodes
        .iter()
        .flat_map(|node| &node.labels)
        .filter_map(|label| label.strip_prefix(&prefix))
        .collect();
    if values.is_empty() {
        return format!("no node carries the key {key}");
    }
    let values: Vec<&str> = values.into_iter().collect();
    format!("no node has it (nodes have {key}: {})", values.join(", "))
}

/// `requests cpu 64 cores — the largest node allocates 4 cores`, only when no node is big enough
/// even when empty; a request that fits an empty node is a matter of what runs there now.
fn oversized_request_hint(
    pod: &PodSummary,
    nodes: &[NodeSummary],
    resource: &str,
) -> Option<String> {
    let (cpu, memory) = requests_of(std::iter::once(pod));
    let allocatable: Vec<_> = nodes.iter().map(node_allocatable).collect();
    let (measure, requested, largest) = if resource == "cpu" {
        let largest = allocatable.iter().filter_map(|(cpu, _)| *cpu).max()?;
        (Measure::Cpu, cpu.cores(), largest.cores())
    } else {
        let largest = allocatable.iter().filter_map(|(_, memory)| *memory).max()?;
        (
            Measure::Bytes,
            memory.bytes() as f64,
            largest.bytes() as f64,
        )
    };
    (requested > largest).then(|| {
        format!(
            "requests {resource} {} — the largest node allocates {}",
            measure.format(requested),
            measure.format(largest)
        )
    })
}

#[cfg(test)]
#[path = "pod_scheduling_tests.rs"]
mod pod_scheduling_tests;
