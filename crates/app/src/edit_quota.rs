//! The advisory quota line of Edit YAML (spec 0041, wireframe W10 side panel): what the namespace
//! quotas say about the pods, CPU, and memory a workload change adds. The numbers come from the
//! session's ResourceQuotas feed; nothing here sends a request, blocks Apply, or logs a value.

use cluster::{DemandChange, QuotaCheck, QuotaResource, ResourceQuotaSummary, quota_check};
use gpui_kit::SharedString;

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
                amount_text(resource, left, Round::Down),
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
                        amount_text(shortfall.resource, shortfall.needed, Round::Up),
                        amount_text(shortfall.resource, shortfall.left, Round::Down),
                    )
                    .into()
                })
                .collect(),
        ),
    }
}

/// Which way a shown amount is rounded: a headroom must never read larger than it is, and a need
/// never smaller.
#[derive(Clone, Copy)]
pub(crate) enum Round {
    /// For what is left.
    Down,
    /// For what is needed.
    Up,
}

fn divide(numerator: u128, denominator: u128, round: Round) -> u128 {
    match round {
        Round::Down => numerator / denominator,
        Round::Up => numerator.div_ceil(denominator),
    }
}

/// Memory as binary bytes (`22Gi`, `1.2Gi`), CPU as cores or millicores (`1.5`, `250m`), pods as a
/// count. The text is cut to the shown precision in the direction `round` says, so `left` is never
/// overstated and `needed` never understated.
pub(crate) fn amount_text(resource: QuotaResource, value: u64, round: Round) -> String {
    match resource {
        QuotaResource::Pods => value.to_string(),
        QuotaResource::RequestsMemory | QuotaResource::LimitsMemory => memory_text(value, round),
        QuotaResource::RequestsCpu | QuotaResource::LimitsCpu => cpu_text(value, round),
    }
}

/// `value` nanocores: whole millicores below one core, tenths of a core from there on.
fn cpu_text(nanocores: u64, round: Round) -> String {
    let millicores = divide(u128::from(nanocores), 1_000_000, round);
    if millicores < 1000 {
        return format!("{millicores}m");
    }
    tenths_text(divide(u128::from(nanocores), 100_000_000, round))
}

/// The smallest binary unit that shows `bytes` below 1024, in whole numbers up to `Mi` and in tenths
/// from `Gi`.
fn memory_text(bytes: u64, round: Round) -> String {
    const UNITS: [&str; 7] = ["B", "Ki", "Mi", "Gi", "Ti", "Pi", "Ei"];
    const FIRST_TENTHS_UNIT: usize = 3;
    for (position, unit) in UNITS.iter().enumerate() {
        let size = 1u128 << (10 * position);
        let is_last = position == UNITS.len() - 1;
        if position < FIRST_TENTHS_UNIT {
            let whole = divide(u128::from(bytes), size, round);
            if whole < 1024 || is_last {
                return format!("{whole}{unit}");
            }
        } else {
            let tenths = divide(u128::from(bytes) * 10, size, round);
            if tenths < 10_240 || is_last {
                return format!("{}{unit}", tenths_text(tenths));
            }
        }
    }
    // The last unit always returns above.
    format!("{bytes}B")
}

/// `15` tenths is `1.5`, `20` is `2`.
fn tenths_text(tenths: u128) -> String {
    if tenths.is_multiple_of(10) {
        (tenths / 10).to_string()
    } else {
        format!("{}.{}", tenths / 10, tenths % 10)
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
