//! What the volume edits of spec 0032b (UX round 3) ask the user to confirm: a Pending claim
//! recreated with another class, and the reclaim policy of a volume. Pure: it reads summaries and
//! builds the choices of the popover, the warnings, and the intents; the shell sends nothing from
//! here.

use cluster::{
    ObjectKind, ObjectRef, PersistentVolumeClaimSummary, PersistentVolumeSummary, ReclaimPolicy,
    StorageClassSummary, WriteOperation, WriteRequest,
};
use gpui_kit::SharedString;

use crate::app_shell::write_flow::WriteIntent;
use crate::audit_log::AuditField;
use crate::resource_actions::{ResourceAction, action_risk};
use crate::workload_actions::WorkloadScope;

/// One button of a Choice popover.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ChoiceOption {
    pub(crate) value: String,
    /// Said beside the value: `default`, `current`.
    pub(crate) hint: Option<String>,
}

// ---- Recreate a claim with another class ----

const ONLY_PENDING: &str = "Only a Pending claim that no volume is bound to can be recreated";
const CLAIM_BEING_DELETED: &str = "The claim is being deleted";

/// Why the claim cannot be recreated now, `None` when it can. A bound claim holds data (or a
/// volume that would be reclaimed), so only a Pending, unbound one goes.
pub(crate) fn recreate_block(claim: &PersistentVolumeClaimSummary) -> Option<SharedString> {
    if claim.is_terminating {
        return Some(CLAIM_BEING_DELETED.into());
    }
    if claim.phase != "Pending" || claim.volume.is_some() {
        return Some(ONLY_PENDING.into());
    }
    None
}

/// `Now class missing · Pending · 100Mi`, the muted line under the buttons.
pub(crate) fn class_state_text(claim: &PersistentVolumeClaimSummary) -> String {
    let class = claim.storage_class.as_deref().unwrap_or("none");
    let size = claim.requested.as_deref().unwrap_or("unknown size");
    format!("Now class {class} \u{b7} {} \u{b7} {size}", claim.phase)
}

/// The classes as buttons, and the one picked first: the default class, else the first one that is
/// not the claim's own. `None` when no class is listed.
pub(crate) fn class_choices(
    classes: &[&StorageClassSummary],
    current: Option<&str>,
) -> Option<(Vec<ChoiceOption>, usize)> {
    if classes.is_empty() {
        return None;
    }
    let options: Vec<ChoiceOption> = classes
        .iter()
        .map(|class| ChoiceOption {
            value: class.name.clone(),
            hint: if current == Some(class.name.as_str()) {
                Some("current".to_owned())
            } else {
                class.is_default.then(|| "default".to_owned())
            },
        })
        .collect();
    let is_other = |class: &StorageClassSummary| current != Some(class.name.as_str());
    let selected = classes
        .iter()
        .position(|class| class.is_default && is_other(class))
        .or_else(|| classes.iter().position(|class| is_other(class)))
        .unwrap_or(0);
    Some((options, selected))
}

/// What a recreate does, said before it is confirmed: the claim goes and comes back with the new
/// class and everything else it had.
pub(crate) fn recreate_warnings(
    claim: &PersistentVolumeClaimSummary,
    class: &str,
) -> Vec<SharedString> {
    let size = claim.requested.as_deref().unwrap_or("its size");
    let modes = if claim.access_modes.is_empty() {
        "its access modes".to_owned()
    } else {
        claim.access_modes.join(", ")
    };
    vec![
        format!(
            "Deletes claim {} and creates it again with class {class}; {size} and {modes} stay",
            claim.name
        )
        .into(),
        "It is Pending and no pod mounts it, so no data is lost".into(),
    ]
}

/// The intent of recreating `claim` with `class`; `uid` is the one the delete is pinned to. `None`
/// when the name or the class is not valid.
pub(crate) fn recreate_intent(
    scope: &WorkloadScope<'_>,
    claim: &PersistentVolumeClaimSummary,
    uid: &str,
    class: &str,
) -> Option<WriteIntent> {
    let target = ObjectRef::new(
        ObjectKind::PersistentVolumeClaim,
        Some(claim.namespace.clone()),
        claim.name.clone(),
    )?;
    let request = WriteRequest::new(
        target,
        WriteOperation::RecreateClaim {
            uid: uid.to_owned(),
            storage_class: class.to_owned(),
            previous_class: claim.storage_class.clone(),
        },
    )?;
    let action = ResourceAction::RecreateClaim;
    // What the new claim keeps: the request does not carry it, so the audit line does.
    let kept = [
        ("spec.resources.requests.storage", claim.requested.clone()),
        (
            "spec.accessModes",
            (!claim.access_modes.is_empty()).then(|| claim.access_modes.join(", ")),
        ),
    ];
    Some(WriteIntent {
        cluster: scope.cluster.clone(),
        cluster_name: scope.cluster_name.to_owned().into(),
        action,
        label: format!("Recreate claim {} with class {class}", claim.name).into(),
        button: "Recreate".into(),
        request,
        risk: action_risk(action),
        warnings: recreate_warnings(claim, class),
        change_lines: Vec::new(),
        audit_fields: kept
            .into_iter()
            .filter_map(|(path, value)| {
                Some(AuditField {
                    path: path.to_owned(),
                    value: Some(value?),
                    from: None,
                })
            })
            .collect(),
    })
}

// ---- Reclaim policy of a volume ----

/// Why the volume's policy cannot be set, `None` when it can.
pub(crate) fn reclaim_block(volume: &PersistentVolumeSummary) -> Option<SharedString> {
    if volume.is_terminating {
        return Some("The volume is being deleted".into());
    }
    ReclaimPolicy::parse(&volume.reclaim_policy)
        .is_none()
        .then(|| {
            format!(
                "Reclaim policy {} cannot be changed here",
                volume.reclaim_policy
            )
            .into()
        })
}

/// `Now Delete · claim lab-house/data`, the muted line under the buttons.
pub(crate) fn policy_state_text(volume: &PersistentVolumeSummary) -> String {
    match &volume.claim {
        Some(claim) => format!(
            "Now {} \u{b7} claim {}/{}",
            volume.reclaim_policy, claim.namespace, claim.name
        ),
        None => format!("Now {} \u{b7} no claim", volume.reclaim_policy),
    }
}

/// Retain and Delete as buttons, the one the volume does not have picked.
pub(crate) fn policy_choices(volume: &PersistentVolumeSummary) -> (Vec<ChoiceOption>, usize) {
    let options: Vec<ChoiceOption> = [ReclaimPolicy::Retain, ReclaimPolicy::Delete]
        .into_iter()
        .map(|policy| ChoiceOption {
            value: policy.to_string(),
            hint: (volume.reclaim_policy == policy.as_str()).then(|| "current".to_owned()),
        })
        .collect();
    let selected = options
        .iter()
        .position(|option| option.hint.is_none())
        .unwrap_or(0);
    (options, selected)
}

/// What the policy decides when the claim is deleted, said before it is confirmed.
pub(crate) fn reclaim_warnings(
    volume: &PersistentVolumeSummary,
    policy: &str,
) -> Vec<SharedString> {
    let claim = match &volume.claim {
        Some(claim) => format!("claim {}/{}", claim.namespace, claim.name),
        None => "its claim".to_owned(),
    };
    let effect = match ReclaimPolicy::parse(policy) {
        Some(ReclaimPolicy::Retain) => format!(
            "When {claim} is deleted, the volume and its data stay (Released) until an administrator reclaims them"
        ),
        Some(ReclaimPolicy::Delete) => format!(
            "When {claim} is deleted, the volume and the storage behind it are deleted with it"
        ),
        None => return Vec::new(),
    };
    vec![effect.into()]
}

/// The intent of setting the policy of `volume`; `None` when the policy or the name is not valid.
pub(crate) fn reclaim_policy_intent(
    scope: &WorkloadScope<'_>,
    volume: &PersistentVolumeSummary,
    policy: &str,
) -> Option<WriteIntent> {
    let (policy, previous) = (
        ReclaimPolicy::parse(policy)?,
        ReclaimPolicy::parse(&volume.reclaim_policy)?,
    );
    let target = ObjectRef::new(ObjectKind::PersistentVolume, None, volume.name.clone())?;
    let request = WriteRequest::new(
        target,
        WriteOperation::SetReclaimPolicy { policy, previous },
    )?;
    let action = ResourceAction::SetReclaimPolicy;
    Some(WriteIntent {
        cluster: scope.cluster.clone(),
        cluster_name: scope.cluster_name.to_owned().into(),
        action,
        label: format!("Set reclaim policy of volume {} to {policy}", volume.name).into(),
        button: "Set policy".into(),
        request,
        risk: action_risk(action),
        warnings: reclaim_warnings(volume, policy.as_str()),
        change_lines: Vec::new(),
        audit_fields: Vec::new(),
    })
}

#[cfg(test)]
#[path = "volume_edits_tests.rs"]
mod volume_edits_tests;
