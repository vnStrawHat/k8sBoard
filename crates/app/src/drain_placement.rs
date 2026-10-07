//! Where the replacements of a drain's pods can go (spec 0034, drain dialog): a controller makes a
//! replacement for every evicted pod, and the scheduler places it on another node. When no other
//! node takes it (a taint it does not tolerate, a selector that matches nothing else), it stays
//! Pending, and the dialog says so before the drain starts.

use cluster::{DrainPod, Misfit, NodeReadiness, NodeScheduling, NodeSummary};

use crate::drain_plan::{NodePlan, PlannedPod};

/// How many nodes the reason names before it says `and 2 more nodes`.
const NAMED_NODES: usize = 2;

/// `No other node fits catalog-db-0 (taints on worker2: workload=data): its replacement will stay
/// Pending.` for the evicted pods that have a controller and no other node to go to. `None` when
/// every such pod fits somewhere, or when the cluster's nodes could not be read (`nodes` empty).
pub(crate) fn placement_note(plans: &[NodePlan], nodes: &[NodeSummary]) -> Option<String> {
    if nodes.is_empty() {
        return None;
    }
    let candidates: Vec<&NodeSummary> = nodes
        .iter()
        .filter(|node| plans.iter().all(|plan| plan.node != node.name))
        .filter(|node| node.status.scheduling == NodeScheduling::Enabled)
        .filter(|node| node.status.readiness == NodeReadiness::Ready)
        .collect();
    let stranded: Vec<(&PlannedPod, String)> = plans
        .iter()
        .flat_map(|plan| plan.evictions())
        .filter(|planned| has_replacement(&planned.pod) && planned.pinned_volume().is_none())
        .filter_map(|planned| Some((planned, stranded_reason(&planned.pod, &candidates)?)))
        .collect();
    Some(match stranded.as_slice() {
        [] => return None,
        [(planned, reason)] => format!(
            "No other node fits {} ({reason}): its replacement will stay Pending.",
            planned.pod.name
        ),
        [(first, reason), more @ ..] => format!(
            "No other node fits {} and {} more ({reason}): their replacements will stay Pending.",
            first.pod.name,
            more.len()
        ),
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

/// Why no candidate takes `pod`, `None` when one does.
fn stranded_reason(pod: &DrainPod, candidates: &[&NodeSummary]) -> Option<String> {
    if candidates.is_empty() {
        return Some("no other node is schedulable".to_owned());
    }
    let mut misfits = Vec::new();
    for node in candidates {
        misfits.push((node.name.as_str(), pod.placement.misfit(node)?));
    }
    let hidden = misfits.len().saturating_sub(NAMED_NODES);
    misfits.truncate(NAMED_NODES);
    // `taints on a: x, b: y` when taints are all there is; else each node names its own cause.
    let is_all_taints = misfits
        .iter()
        .all(|(_, misfit)| matches!(misfit, Misfit::Taints(_)));
    let mut parts: Vec<String> = misfits
        .iter()
        .map(|(node, misfit)| match misfit {
            Misfit::Taints(taints) if is_all_taints => format!("{node}: {}", taints.join(", ")),
            Misfit::Taints(taints) => format!("taints on {node}: {}", taints.join(", ")),
            Misfit::Selector => format!("selector on {node}"),
        })
        .collect();
    if hidden > 0 {
        parts.push(format!("and {hidden} more nodes"));
    }
    let lead = if is_all_taints { "taints on " } else { "" };
    Some(format!(
        "{lead}{}",
        parts.join(if is_all_taints { ", " } else { "; " })
    ))
}

#[cfg(test)]
#[path = "drain_placement_tests.rs"]
mod drain_placement_tests;
