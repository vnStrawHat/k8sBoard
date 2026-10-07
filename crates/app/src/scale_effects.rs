//! What a scale-down does besides removing pods (UX round 3, P20): the volume claims it leaves
//! behind and the PodDisruptionBudgets that block evictions at the new count. Pure: the lists the
//! session already holds go in, the warning lines of the Scale confirm come out.

use cluster::{PodDisruptionBudgetSummary, Selector};
use gpui_kit::SharedString;

use crate::kind_row::KindObject;

/// How many claim names the line spells out before it counts the rest.
const LISTED_CLAIMS: usize = 3;

/// The claims the pods of a StatefulSet own: `{template}-{set}-{ordinal}` for each template.
#[derive(Clone, Debug, PartialEq, Eq)]
struct StatefulClaims {
    set: String,
    templates: Vec<String>,
    /// `persistentVolumeClaimRetentionPolicy.whenScaled` is `Delete`.
    is_deleted_on_scale: bool,
}

impl StatefulClaims {
    /// The claims of the pods with ordinals `replicas..desired`, the ones a scale-down removes.
    fn of_removed_pods(&self, desired: u32, replicas: u32) -> Vec<String> {
        (replicas..desired)
            .flat_map(|ordinal| {
                self.templates
                    .iter()
                    .map(move |template| format!("{template}-{}-{ordinal}", self.set))
            })
            .collect()
    }
}

/// How much a budget asks for: a count, or a percentage of the pods the budget covers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Amount {
    Count(u32),
    Percent(u32),
}

impl Amount {
    /// `2` or `50%`; anything else reads as no rule.
    fn parse(text: &str) -> Option<Self> {
        match text.strip_suffix('%') {
            Some(percent) => percent.parse().ok().map(Self::Percent),
            None => text.parse().ok().map(Self::Count),
        }
    }

    /// The pods this amount stands for out of `pods`, rounded up as the budget controller does.
    fn of(self, pods: u32) -> u32 {
        match self {
            Self::Count(count) => count,
            Self::Percent(percent) => (u64::from(pods) * u64::from(percent)).div_ceil(100) as u32,
        }
    }

    fn text(self) -> String {
        match self {
            Self::Count(count) => count.to_string(),
            Self::Percent(percent) => format!("{percent}%"),
        }
    }
}

/// One limit of a PodDisruptionBudget.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Limit {
    MinAvailable(Amount),
    MaxUnavailable(Amount),
}

impl Limit {
    /// How many pods may be evicted at once when `pods` run.
    fn allowed(self, pods: u32) -> u32 {
        match self {
            Self::MinAvailable(amount) => pods.saturating_sub(amount.of(pods)),
            Self::MaxUnavailable(amount) => amount.of(pods),
        }
    }

    /// `minAvailable 2`, as the budget's spec writes it.
    fn text(self) -> String {
        match self {
            Self::MinAvailable(amount) => format!("minAvailable {}", amount.text()),
            Self::MaxUnavailable(amount) => format!("maxUnavailable {}", amount.text()),
        }
    }
}

/// A budget that covers the workload being scaled.
#[derive(Clone, Debug, PartialEq, Eq)]
struct CoveringBudget {
    name: String,
    limit: Limit,
}

impl CoveringBudget {
    fn of(budget: &PodDisruptionBudgetSummary) -> Option<Self> {
        let limit = match (&budget.min_available, &budget.max_unavailable) {
            (Some(min), _) => Limit::MinAvailable(Amount::parse(min)?),
            (None, Some(max)) => Limit::MaxUnavailable(Amount::parse(max)?),
            (None, None) => return None,
        };
        Some(Self {
            name: budget.name.clone(),
            limit,
        })
    }
}

/// The side effects of scaling one workload down, from the lists the session holds.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ScaleEffects {
    claims: Option<StatefulClaims>,
    /// The pod labels the workload's selector holds, which a budget's selector must accept.
    selector: Vec<String>,
    budgets: Vec<CoveringBudget>,
}

impl ScaleEffects {
    /// The claims of a StatefulSet and the selector of a Deployment or StatefulSet; no budgets yet.
    pub(crate) fn of(object: &KindObject) -> Self {
        match object {
            KindObject::StatefulSet(set) => Self {
                claims: (!set.claim_templates.is_empty()).then(|| StatefulClaims {
                    set: set.name.clone(),
                    templates: set
                        .claim_templates
                        .iter()
                        .map(|template| template.name.clone())
                        .collect(),
                    is_deleted_on_scale: set
                        .claim_retention
                        .as_deref()
                        .is_some_and(|retention| retention.contains("whenScaled Delete")),
                }),
                selector: set.selector.clone(),
                budgets: Vec::new(),
            },
            KindObject::Deployment(deployment) => Self {
                claims: None,
                selector: deployment.selector.clone(),
                budgets: Vec::new(),
            },
            _ => Self::default(),
        }
    }

    /// The budgets among `budgets` (the PDBs the session holds) of `namespace` whose selector
    /// accepts the workload's selector terms.
    // ponytail: the workload's own selector stands for its pods' labels, so a budget that selects
    // on a label only the pod template adds is missed; read the pods' labels if that matters.
    pub(crate) fn with_budgets(mut self, namespace: &str, budgets: &[KindObject]) -> Self {
        self.budgets = budgets
            .iter()
            .filter_map(|object| match object {
                KindObject::PodDisruptionBudget(budget) if budget.namespace == namespace => budget
                    .selector
                    .as_ref()
                    .filter(|selector: &&Selector| selector.matches(&self.selector))
                    .and_then(|_| CoveringBudget::of(budget)),
                _ => None,
            })
            .collect();
        self
    }

    /// The warning lines of scaling from `desired` to `replicas`; none unless pods go away.
    pub(crate) fn lines(&self, desired: u32, replicas: u32) -> Vec<SharedString> {
        if replicas >= desired {
            return Vec::new();
        }
        let mut lines: Vec<String> = Vec::new();
        if let Some(claims) = &self.claims {
            lines.extend(claim_line(claims, desired, replicas));
        }
        // No pod left means nothing to evict, so nothing to block.
        lines.extend(
            self.budgets
                .iter()
                .filter(|budget| replicas > 0 && budget.limit.allowed(replicas) == 0)
                .map(|budget| blocked_line(budget, replicas)),
        );
        lines.into_iter().map(Into::into).collect()
    }
}

/// `Keeps the PersistentVolumeClaims of the removed pods (data-db-2, data-db-3 and 1 more)`.
fn claim_line(claims: &StatefulClaims, desired: u32, replicas: u32) -> Option<String> {
    let names = claims.of_removed_pods(desired, replicas);
    if names.is_empty() {
        return None;
    }
    let shown = names[..names.len().min(LISTED_CLAIMS)].join(", ");
    let listed = match names.len().saturating_sub(LISTED_CLAIMS) {
        0 => shown,
        more => format!("{shown} and {more} more"),
    };
    let verb = if claims.is_deleted_on_scale {
        "Deletes"
    } else {
        "Keeps"
    };
    Some(format!(
        "{verb} the PersistentVolumeClaims of the removed pods ({listed})"
    ))
}

/// `PDB catalog-db minAvailable 2: evictions will be blocked at 1 replica`.
fn blocked_line(budget: &CoveringBudget, replicas: u32) -> String {
    let noun = if replicas == 1 { "replica" } else { "replicas" };
    format!(
        "PDB {} {}: evictions will be blocked at {replicas} {noun}",
        budget.name,
        budget.limit.text()
    )
}

#[cfg(test)]
#[path = "scale_effects_tests.rs"]
mod scale_effects_tests;
