//! What a namespace ResourceQuota leaves for more pods: the line of a Scale confirm that says how
//! many new pods will not start, and the exact numbers of an "exceeded quota" failure. Pure.

use cluster::{
    QuotaCheck, QuotaResource, QuotaShortfall, ResourceQuotaSummary, TemplateContainer,
    quota_check, scale_demand,
};

use crate::edit_quota::{Round, amount_text};

/// The quota and the resources an "exceeded quota" admission failure names.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct QuotaExceeded {
    pub(crate) quota: String,
    /// `limits.memory 600Mi of 640Mi used, needs 150Mi`, one per resource.
    pub(crate) lines: Vec<String>,
}

/// Reads `exceeded quota: team-quota, requested: limits.memory=150Mi, used: limits.memory=600Mi,
/// limited: limits.memory=640Mi`, the text the API server gives a pod it refuses. `None` for any
/// other message.
pub(crate) fn quota_exceeded(message: &str) -> Option<QuotaExceeded> {
    let (_, rest) = message.split_once("exceeded quota: ")?;
    let (quota, rest) = rest.split_once(", requested: ")?;
    let (requested, rest) = rest.split_once(", used: ")?;
    let (used, limited) = rest.split_once(", limited: ")?;
    let (requested, used, limited) = (pairs(requested), pairs(used), pairs(limited));
    let lines = requested
        .iter()
        .filter_map(|(resource, needs)| {
            Some(format!(
                "{resource} {} of {} used, needs {needs}",
                value_of(&used, resource)?,
                value_of(&limited, resource)?
            ))
        })
        .collect::<Vec<_>>();
    (!lines.is_empty()).then(|| QuotaExceeded {
        quota: quota.to_owned(),
        lines,
    })
}

fn value_of<'a>(list: &[(&'a str, &'a str)], resource: &str) -> Option<&'a str> {
    list.iter()
        .find_map(|(name, value)| (*name == resource).then_some(*value))
}

/// `a=1,b=2` as pairs; an entry without `=` is dropped.
fn pairs(text: &str) -> Vec<(&str, &str)> {
    text.split(',')
        .filter_map(|entry| entry.trim().split_once('='))
        .collect()
}

/// One line for each quota item the new pods of a Scale from `from` to `to` replicas would overrun,
/// such as `needs 150Mi limits.memory per pod, team-quota has 40Mi left: the new pod will not
/// start`. The numbers come from `quota_check`, so scoped quotas and unsynced items are skipped
/// there; LimitRange defaults and init containers are not counted.
pub(crate) fn quota_scale_warnings(
    quotas: &[ResourceQuotaSummary],
    containers: &[TemplateContainer],
    from: u32,
    to: u32,
) -> Vec<String> {
    let extra = to.saturating_sub(from);
    if extra == 0 {
        return Vec::new();
    }
    let QuotaCheck::Exceeds(shortfalls) = quota_check(&scale_demand(containers, from, to), quotas)
    else {
        return Vec::new();
    };
    shortfalls
        .iter()
        .map(|shortfall| overrun_text(shortfall, extra))
        .collect()
}

fn overrun_text(shortfall: &QuotaShortfall, extra: u32) -> String {
    let extra_pods = u64::from(extra);
    // The growth is linear in the pod count, so each new pod takes an equal share.
    let per_pod = shortfall.needed.div_ceil(extra_pods).max(1);
    let fit = shortfall.left / per_pod;
    let head = match shortfall.resource {
        QuotaResource::Pods => format!(
            "{} allows {} more pods",
            shortfall.quota,
            amount_text(shortfall.resource, shortfall.left, Round::Down)
        ),
        resource => format!(
            "needs {} {} per pod, {} has {} left",
            amount_text(resource, per_pod, Round::Up),
            resource.name(),
            shortfall.quota,
            amount_text(resource, shortfall.left, Round::Down)
        ),
    };
    let outcome = match (extra, fit) {
        (1, _) => "the new pod will not start".to_owned(),
        (_, 0) => format!("none of the {extra} new pods will start"),
        _ => format!(
            "{} of the {extra} new pods will not start",
            extra_pods - fit
        ),
    };
    format!("{head}: {outcome}")
}

#[cfg(test)]
#[path = "quota_room_tests.rs"]
mod quota_room_tests;
