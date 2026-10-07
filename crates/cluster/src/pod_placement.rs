//! Whether a node can take a pod (spec 0034, drain dialog): the scheduling constraints of a pod
//! that the scheduler checks against the labels and taints of one node alone, with no view of what
//! else runs there. Resources, ports, volumes, and pod (anti-)affinity are not checked.

use k8s_openapi::api::core::v1::{NodeSelectorRequirement, Pod, PodSpec};

use crate::node::{NodeSummary, NodeTaint};

/// The taint effects that keep a pod off a node; `PreferNoSchedule` only steers the scheduler.
const BLOCKING_EFFECTS: [&str; 2] = ["NoSchedule", "NoExecute"];
/// Set by the node lifecycle controller on a cordoned node; it follows `spec.unschedulable`, which
/// the caller decides, and it goes away on Uncordon.
const UNSCHEDULABLE_TAINT: &str = "node.kubernetes.io/unschedulable";

/// What the scheduler matches a pod against.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PodPlacement {
    pub tolerations: Vec<PodToleration>,
    pub node_selector: Vec<(String, String)>,
    /// Required node affinity: the terms are alternatives, the requirements of a term all hold.
    pub affinity_terms: Vec<Vec<AffinityRequirement>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PodToleration {
    /// Empty with `Exists` tolerates every taint.
    pub key: String,
    pub is_exists: bool,
    pub value: String,
    /// Empty tolerates every effect.
    pub effect: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AffinityRequirement {
    pub key: String,
    pub operator: String,
    pub values: Vec<String>,
}

/// Why a node cannot take a pod.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Misfit {
    /// The taints the pod does not tolerate, as `key=value` or `key`.
    Taints(Vec<String>),
    /// The pod's nodeSelector or required node affinity does not match the node's labels.
    Selector,
}

impl PodPlacement {
    pub(crate) fn of(pod: &Pod) -> Self {
        let Some(spec) = pod.spec.as_ref() else {
            return Self::default();
        };
        Self {
            tolerations: spec
                .tolerations
                .iter()
                .flatten()
                .map(|toleration| PodToleration {
                    key: toleration.key.clone().unwrap_or_default(),
                    is_exists: toleration.operator.as_deref() == Some("Exists"),
                    value: toleration.value.clone().unwrap_or_default(),
                    effect: toleration.effect.clone().unwrap_or_default(),
                })
                .collect(),
            node_selector: spec
                .node_selector
                .iter()
                .flatten()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
            affinity_terms: affinity_terms(spec),
        }
    }

    /// Why `node` cannot take the pod, `None` when it can as far as these checks see.
    pub fn misfit(&self, node: &NodeSummary) -> Option<Misfit> {
        let untolerated: Vec<String> = node
            .taints
            .iter()
            .filter(|taint| BLOCKING_EFFECTS.contains(&taint.effect.as_str()))
            .filter(|taint| taint.key != UNSCHEDULABLE_TAINT)
            .filter(|taint| {
                !self
                    .tolerations
                    .iter()
                    .any(|toleration| toleration.covers(taint))
            })
            .map(taint_text)
            .collect();
        if !untolerated.is_empty() {
            return Some(Misfit::Taints(untolerated));
        }
        let has_label = |key: &str, value: &str| {
            node.labels
                .iter()
                .any(|term| term.split_once('=') == Some((key, value)))
        };
        if !self
            .node_selector
            .iter()
            .all(|(key, value)| has_label(key, value))
        {
            return Some(Misfit::Selector);
        }
        let matches_term = |term: &Vec<AffinityRequirement>| {
            term.iter()
                .all(|requirement| requirement.holds(&node.labels))
        };
        (!self.affinity_terms.is_empty() && !self.affinity_terms.iter().any(matches_term))
            .then_some(Misfit::Selector)
    }
}

impl PodToleration {
    fn covers(&self, taint: &NodeTaint) -> bool {
        if !self.effect.is_empty() && self.effect != taint.effect {
            return false;
        }
        if self.key.is_empty() {
            return self.is_exists;
        }
        self.key == taint.key
            && (self.is_exists || self.value == taint.value.as_deref().unwrap_or_default())
    }
}

impl AffinityRequirement {
    fn holds(&self, labels: &[String]) -> bool {
        let value = labels.iter().find_map(|term| {
            let (key, value) = term.split_once('=')?;
            (key == self.key).then_some(value)
        });
        match self.operator.as_str() {
            "In" => value.is_some_and(|value| self.values.iter().any(|wanted| wanted == value)),
            "NotIn" => value.is_none_or(|value| self.values.iter().all(|wanted| wanted != value)),
            "Exists" => value.is_some(),
            "DoesNotExist" => value.is_none(),
            "Gt" | "Lt" => value.is_some_and(|value| self.compares(value)),
            // An operator this check does not know is not held against the node.
            _ => true,
        }
    }

    fn compares(&self, value: &str) -> bool {
        let (Ok(value), Some(Ok(bound))) = (
            value.parse::<i64>(),
            self.values.first().map(|bound| bound.parse::<i64>()),
        ) else {
            return false;
        };
        if self.operator == "Gt" {
            value > bound
        } else {
            value < bound
        }
    }
}

fn affinity_terms(spec: &PodSpec) -> Vec<Vec<AffinityRequirement>> {
    let required = spec
        .affinity
        .as_ref()
        .and_then(|affinity| affinity.node_affinity.as_ref())
        .and_then(|affinity| {
            affinity
                .required_during_scheduling_ignored_during_execution
                .as_ref()
        });
    required
        .into_iter()
        .flat_map(|selector| &selector.node_selector_terms)
        // `matchFields` names the node, which the other-node check never needs.
        .map(|term| {
            term.match_expressions
                .iter()
                .flatten()
                .map(requirement)
                .collect()
        })
        .collect()
}

fn requirement(requirement: &NodeSelectorRequirement) -> AffinityRequirement {
    AffinityRequirement {
        key: requirement.key.clone(),
        operator: requirement.operator.clone(),
        values: requirement.values.clone().unwrap_or_default(),
    }
}

/// `workload=data`, or `workload` for a taint without a value; the effect is left out.
fn taint_text(taint: &NodeTaint) -> String {
    match taint.value.as_deref() {
        Some(value) if !value.is_empty() => format!("{}={value}", taint.key),
        _ => taint.key.clone(),
    }
}

#[cfg(test)]
#[path = "pod_placement_tests.rs"]
mod pod_placement_tests;
