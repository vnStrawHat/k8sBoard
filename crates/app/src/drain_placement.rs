//! Where the replacements of a drain's pods can go (spec 0034, drain dialog): a controller makes a
//! replacement for every evicted pod, and the scheduler places it on another node. When no other
//! node takes it (a taint it does not tolerate, a selector that matches nothing else), it stays
//! Pending, and the dialog says so before the drain starts.

use cluster::{DrainPod, Misfit, NodeReadiness, NodeScheduling, NodeSummary};

use crate::drain_plan::{NodePlan, PlannedPod};

/// How many nodes the reason names before it says `and 2 more nodes`.
const NAMED_NODES: usize = 2;

/// How many pods the tooltip lists before it says `and 12 more`.
const LISTED_PODS: usize = 12;

/// The HEADS UP line of the pods no other node takes, and the tooltip behind it.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct PlacementNote {
    /// `No other node fits 14 pods (taints on worker, worker2): replacements stay Pending.`
    pub(crate) text: String,
    /// The pods, then the taint or selector each candidate node stops them with.
    pub(crate) detail: String,
}

/// The note for the evicted pods that have a controller and no other node to go to. `None` when
/// every such pod fits somewhere, or when the cluster's nodes could not be read (`nodes` empty).
pub(crate) fn placement_note(plans: &[NodePlan], nodes: &[NodeSummary]) -> Option<PlacementNote> {
    if nodes.is_empty() {
        return None;
    }
    let candidates: Vec<&NodeSummary> = nodes
        .iter()
        .filter(|node| plans.iter().all(|plan| plan.node != node.name))
        .filter(|node| node.status.scheduling == NodeScheduling::Enabled)
        .filter(|node| node.status.readiness == NodeReadiness::Ready)
        .collect();
    let stranded: Vec<(&PlannedPod, Stranded)> = plans
        .iter()
        .flat_map(|plan| plan.evictions())
        .filter(|planned| has_replacement(&planned.pod) && planned.pinned_volume().is_none())
        .filter_map(|planned| Some((planned, stranded_reason(&planned.pod, &candidates)?)))
        .collect();
    let (first, reason) = stranded.first()?;
    let text = match stranded.len() {
        1 => format!(
            "No other node fits {} ({}): its replacement stays Pending.",
            first.pod.name, reason.summary
        ),
        count => format!(
            "No other node fits {count} pods ({}): their replacements stay Pending.",
            reason.summary
        ),
    };
    let mut detail: Vec<String> = stranded
        .iter()
        .take(LISTED_PODS)
        .map(|(planned, _)| format!("{}/{}", planned.pod.namespace, planned.pod.name))
        .collect();
    if stranded.len() > LISTED_PODS {
        detail.push(format!("and {} more", stranded.len() - LISTED_PODS));
    }
    detail.extend(reason.nodes.iter().cloned());
    Some(PlacementNote {
        text,
        detail: detail.join("\n"),
    })
}

/// `taint workload=data is` or `taints a, b are`: the taints of `node` that `pod` does not tolerate,
/// which keep a volume-pinned replacement off its own node after Uncordon too.
pub(crate) fn own_taint_text(pod: &DrainPod, node: &str, nodes: &[NodeSummary]) -> Option<String> {
    let own = nodes.iter().find(|candidate| candidate.name == node)?;
    let Some(Misfit::Taints(taints)) = pod.placement.misfit(own) else {
        return None;
    };
    Some(match taints.as_slice() {
        [only] => format!("taint {only} is"),
        several => format!("taints {} are", several.join(", ")),
    })
}

fn has_replacement(pod: &DrainPod) -> bool {
    pod.controller.is_some() && !pod.is_finished
}

/// Why no candidate node takes a pod: the short form for the line, and one line per node for the
/// tooltip.
struct Stranded {
    summary: String,
    nodes: Vec<String>,
}

/// Why no candidate takes `pod`, `None` when one does.
fn stranded_reason(pod: &DrainPod, candidates: &[&NodeSummary]) -> Option<Stranded> {
    if candidates.is_empty() {
        return Some(Stranded {
            summary: "no other node is schedulable".to_owned(),
            nodes: Vec::new(),
        });
    }
    let mut misfits = Vec::new();
    for node in candidates {
        misfits.push((node.name.as_str(), pod.placement.misfit(node)?));
    }
    let nodes = misfits
        .iter()
        .map(|(node, misfit)| match misfit {
            Misfit::Taints(taints) => format!("{node}: taints {}", taints.join(", ")),
            Misfit::Selector => format!("{node}: selector does not match"),
        })
        .collect();
    let hidden = misfits.len().saturating_sub(NAMED_NODES);
    misfits.truncate(NAMED_NODES);
    // `taints on a, b` when taints are all there is; else each node names its own cause.
    let is_all_taints = misfits
        .iter()
        .all(|(_, misfit)| matches!(misfit, Misfit::Taints(_)));
    let mut parts: Vec<String> = misfits
        .iter()
        .map(|(node, misfit)| match misfit {
            Misfit::Taints(_) if is_all_taints => (*node).to_owned(),
            Misfit::Taints(_) => format!("taints on {node}"),
            Misfit::Selector => format!("selector on {node}"),
        })
        .collect();
    if hidden > 0 {
        parts.push(format!("and {hidden} more nodes"));
    }
    let lead = if is_all_taints { "taints on " } else { "" };
    let summary = format!(
        "{lead}{}",
        parts.join(if is_all_taints { ", " } else { "; " })
    );
    Some(Stranded { summary, nodes })
}

#[cfg(test)]
#[path = "drain_placement_tests.rs"]
mod drain_placement_tests;
