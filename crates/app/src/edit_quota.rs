//! The advisory quota line of Edit YAML (spec 0041, wireframe W10 side panel): what the namespace
//! quotas say about the pods, CPU, and memory a workload change adds. The numbers come from the
//! session's ResourceQuotas feed; nothing here sends a request, blocks Apply, or logs a value.

use cluster::{DemandChange, QuotaCheck, QuotaResource, ResourceQuotaSummary, quota_check};
use gpui_kit::SharedString;

use crate::usage_format::Measure;

/// What the session knows of the quotas of the edited namespace.
pub(crate) enum QuotaInput {
    /// The feed does not run, with the reason it gives.
    Off(String),
    Loading,
    /// The quotas of the edited namespace.
    Quotas(Vec<ResourceQuotaSummary>),
}

/// The quota part of the Checks of a passed dry-run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum QuotaLine {
    /// Nothing grows, or no quota limits what grows.
    None,
    /// The feed could not answer; muted.
    NotChecked(SharedString),
    /// The tightest item still fits; success tone.
    Fits(SharedString),
    /// One line per item the change exceeds; warning tone, also dialog warnings.
    Exceeds(Vec<SharedString>),
}

impl QuotaLine {
    /// The lines the confirm dialog repeats: only an exceeded quota warns there.
    pub(crate) fn warnings(&self) -> &[SharedString] {
        match self {
            Self::Exceeds(lines) => lines,
            Self::None | Self::NotChecked(_) | Self::Fits(_) => &[],
        }
    }
}

/// The line for `demand` against `input`. Pure.
pub(crate) fn quota_line(demand: Option<&DemandChange>, input: &QuotaInput) -> QuotaLine {
    let Some(change) = demand.filter(|change| change.grows()) else {
        return QuotaLine::None;
    };
    let quotas = match input {
        QuotaInput::Off(reason) => {
            return QuotaLine::NotChecked(format!("Quota not checked: {reason}").into());
        }
        QuotaInput::Loading => {
            return QuotaLine::NotChecked("Quota not checked: quotas are still loading".into());
        }
        QuotaInput::Quotas(quotas) => quotas,
    };
    match quota_check(change, quotas) {
        QuotaCheck::NotAffected => QuotaLine::None,
        QuotaCheck::Fits { resource, left, .. } => QuotaLine::Fits(
            format!(
                "Namespace quota OK ({} {} left)",
                amount_text(resource, left),
                resource.name()
            )
            .into(),
        ),
        QuotaCheck::Exceeds(shortfalls) => QuotaLine::Exceeds(
            shortfalls
                .into_iter()
                .map(|shortfall| {
                    format!(
                        "Quota {}: {} needs {} more, {} left",
                        shortfall.quota,
                        shortfall.resource.name(),
                        amount_text(shortfall.resource, shortfall.needed),
                        amount_text(shortfall.resource, shortfall.left),
                    )
                    .into()
                })
                .collect(),
        ),
    }
}

/// Memory as binary bytes (`22Gi`), CPU as cores or millicores (`1.5`, `250m`), pods as a count.
fn amount_text(resource: QuotaResource, value: u64) -> String {
    match resource {
        QuotaResource::Pods => value.to_string(),
        QuotaResource::RequestsMemory | QuotaResource::LimitsMemory => {
            Measure::Bytes.format(value as f64)
        }
        QuotaResource::RequestsCpu | QuotaResource::LimitsCpu => {
            let text = Measure::Cpu.format(value as f64 / 1e9);
            text.trim_end_matches(" cores")
                .trim_end_matches(" core")
                .to_owned()
        }
    }
}

/// `--screen edit-yaml-diff`: the quota line of W10, `Namespace quota OK (22Gi requests.memory
/// left)`, computed from a fixed quota of 64Gi with 41Gi used and a change that adds 1Gi.
#[cfg(feature = "screenshot")]
pub(crate) fn fixture_line() -> QuotaLine {
    use cluster::{QuotaItem, WorkloadDemand};
    const GI: u64 = 1 << 30;
    let change = DemandChange {
        before: WorkloadDemand::default(),
        after: WorkloadDemand {
            requests_memory: GI,
            ..WorkloadDemand::default()
        },
    };
    let quota = ResourceQuotaSummary {
        namespace: "payments".to_owned(),
        name: "compute-quota".to_owned(),
        created_at: None,
        labels: Vec::new(),
        items: vec![QuotaItem {
            resource: "requests.memory".to_owned(),
            hard: "64Gi".to_owned(),
            used: Some("41Gi".to_owned()),
        }],
        scopes: Vec::new(),
    };
    quota_line(Some(&change), &QuotaInput::Quotas(vec![quota]))
}

#[cfg(test)]
#[path = "edit_quota_tests.rs"]
mod edit_quota_tests;
