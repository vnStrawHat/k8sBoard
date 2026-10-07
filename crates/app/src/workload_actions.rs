//! What the workload actions of spec 0032 ask the user to confirm: the intent of each action
//! (label, button, risk, warnings, request) and the reasons a row cannot take an action now.
//! Pure: it reads a row's summary and builds a `WriteIntent`; the shell sends nothing from here.

use std::cell::Cell;

use cluster::{
    CronJobSummary, DeploymentSummary, HELM_MANAGED_WARNING, HorizontalPodAutoscalerSummary,
    ObjectKind, ObjectRef, ReplicaSetSummary, ResourceQuotaSummary, TemplateContainer,
    WriteOperation, WriteRequest, terms_are_helm_managed,
};
use gpui_kit::SharedString;
use jiff::tz::TimeZone;

use crate::age::format_age;
use crate::app_shell::batch_write::{
    BatchExtras, BatchFailure, BatchIntent, BatchItem, BatchPlan, CheckedRow, SkippedItem,
    batch_plan,
};
use crate::app_shell::write_flow::WriteIntent;
use crate::batch_rows::missed_run;
use crate::cluster_registry::ClusterRef;
use crate::kind_row::KindObject;
use crate::live_sections::run_label;
use crate::quota_room::quota_scale_warnings;
use crate::resource_actions::{ResourceAction, action_risk, values_edit_block};
use crate::resource_edits::{claim_block, class_block};
use crate::scale_effects::ScaleEffects;
use crate::volume_edits::{reclaim_block, recreate_block};
use crate::write_guard::ActionRisk;

/// Why a paused Deployment cannot restart or roll back: kubectl refuses both.
pub(crate) const PAUSED_REASON: &str = "Resume the rollout first";
/// The update strategy under which a restart changes no pod by itself.
const ON_DELETE: &str = "OnDelete";

/// The cluster a workload action runs on: the row's own cluster and its display name now.
pub(crate) struct WorkloadScope<'a> {
    pub(crate) cluster: &'a ClusterRef,
    pub(crate) cluster_name: &'a str,
}

/// The object a row stands for.
struct Workload<'a> {
    kind: ObjectKind,
    namespace: &'a str,
    name: &'a str,
}

impl Workload<'_> {
    /// `deployment`, the word the labels use for the kind.
    fn word(&self) -> String {
        self.kind.name().to_ascii_lowercase()
    }
}

fn workload_of(object: &KindObject) -> Option<Workload<'_>> {
    let (kind, namespace, name) = match object {
        KindObject::Deployment(d) => (ObjectKind::Deployment, &d.namespace, &d.name),
        KindObject::StatefulSet(s) => (ObjectKind::StatefulSet, &s.namespace, &s.name),
        KindObject::DaemonSet(d) => (ObjectKind::DaemonSet, &d.namespace, &d.name),
        KindObject::Job(j) => (ObjectKind::Job, &j.namespace, &j.name),
        KindObject::CronJob(c) => (ObjectKind::CronJob, &c.namespace, &c.name),
        _ => return None,
    };
    Some(Workload {
        kind,
        namespace,
        name,
    })
}

/// What the dialog and the audit line say about one change.
struct Described {
    action: ResourceAction,
    label: String,
    button: &'static str,
    risk: ActionRisk,
    operation: WriteOperation,
    warnings: Vec<SharedString>,
}

/// The intent of `described` on `workload`; `None` when the request is not valid for the row
/// (its name or namespace cannot form a path).
fn intent_of(
    scope: &WorkloadScope<'_>,
    workload: &Workload<'_>,
    described: Described,
) -> Option<WriteIntent> {
    let target = ObjectRef::new(
        workload.kind,
        Some(workload.namespace.to_owned()),
        workload.name.to_owned(),
    )?;
    let request = WriteRequest::new(target, described.operation)?;
    Some(WriteIntent {
        cluster: scope.cluster.clone(),
        cluster_name: scope.cluster_name.to_owned().into(),
        action: described.action,
        label: described.label.into(),
        button: described.button.into(),
        request,
        risk: described.risk,
        warnings: described.warnings,
        change_lines: Vec::new(),
        audit_fields: Vec::new(),
    })
}

/// The intent of the actions that need nothing but the row: Restart, Pause or Resume, Suspend or
/// Resume, Trigger now, and Re-run. `None` for any other action, or a row of another kind.
pub(crate) fn workload_intent(
    action: ResourceAction,
    scope: &WorkloadScope<'_>,
    object: &KindObject,
    now: jiff::Timestamp,
) -> Option<WriteIntent> {
    let workload = workload_of(object)?;
    let word = workload.word();
    let name = workload.name;
    let described = match (action, object) {
        (ResourceAction::RestartRollout(_), _) => {
            restart_described(action, &workload, now, restart_warnings(object))?
        }
        (ResourceAction::PauseRollout, KindObject::Deployment(deployment)) => {
            let paused = !deployment.is_paused;
            let (verb, button) = if paused {
                ("Pause", "Pause")
            } else {
                ("Resume", "Resume")
            };
            Described {
                action,
                label: format!("{verb} rollout of {word} {name}"),
                button,
                risk: action_risk(action),
                operation: WriteOperation::SetRolloutPaused { paused },
                warnings: Vec::new(),
            }
        }
        (ResourceAction::SuspendCronJob, KindObject::CronJob(cron_job)) => {
            let suspended = !cron_job.is_suspended;
            let verb = if suspended { "Suspend" } else { "Resume" };
            Described {
                action,
                label: format!("{verb} {word} {name}"),
                button: verb,
                risk: action_risk(action),
                operation: WriteOperation::SetCronJobSuspended { suspended },
                warnings: resume_notes(cron_job, suspended, now),
            }
        }
        (ResourceAction::TriggerCronJob, KindObject::CronJob(cron_job)) => Described {
            action,
            label: format!("Trigger {word} {name} now"),
            button: "Trigger now",
            risk: action_risk(action),
            operation: WriteOperation::TriggerCronJob,
            warnings: trigger_warnings(&cron_job.concurrency_policy, cron_job.active_jobs.len()),
        },
        (ResourceAction::RerunJob, KindObject::Job(_)) => Described {
            action,
            label: format!("Re-run {word} {name}"),
            button: "Re-run",
            risk: action_risk(action),
            operation: WriteOperation::RerunJob,
            warnings: vec![
                format!("Creates a new Job from {name}'s template; the old Job stays").into(),
            ],
        },
        _ => return None,
    };
    intent_of(scope, &workload, described)
}

/// What a Resume tells before it is confirmed: the run that came due while it was suspended and
/// what the controller does with it, then the next run. Nothing for a Suspend.
fn resume_notes(
    cron_job: &CronJobSummary,
    suspended: bool,
    now: jiff::Timestamp,
) -> Vec<SharedString> {
    if suspended {
        return Vec::new();
    }
    missed_run_note(cron_job, now)
        .into_iter()
        .chain(next_run_note(cron_job, now))
        .collect()
}

/// `The 22:22 UTC run missed while suspended starts now (deadline 60 s)`: the controller starts
/// the latest run that came due at once, unless its starting deadline has passed.
fn missed_run_note(cron_job: &CronJobSummary, now: jiff::Timestamp) -> Option<SharedString> {
    let run = missed_run(cron_job, now)?;
    let when = run_label(&run.at.to_zoned(TimeZone::system()), now);
    let deadline = match cron_job.starting_deadline_seconds {
        Some(seconds) => format!("deadline {seconds} s"),
        None => "no starting deadline".to_owned(),
    };
    let text = if run.starts {
        format!("The {when} run missed while suspended starts now ({deadline})")
    } else {
        format!("The {when} run missed while suspended is skipped ({deadline} has passed)")
    };
    Some(text.into())
}

/// `Next run: 10:45 UTC · in 9m`. `None` when the schedule has no run to name (an invalid one,
/// or an `@every` before its first run).
fn next_run_note(cron_job: &CronJobSummary, now: jiff::Timestamp) -> Option<SharedString> {
    let next = cron_job.timetable.as_ref().ok()?.next_after(now)?;
    let away = format_age(Some(now), next.timestamp());
    let when = next_run_label(cron_job, now)?;
    Some(format!("Next run: {when} · in {away}").into())
}

/// `10:45 UTC`: when the schedule runs next after `now`.
pub(crate) fn next_run_label(cron_job: &CronJobSummary, now: jiff::Timestamp) -> Option<String> {
    let next = cron_job.timetable.as_ref().ok()?.next_after(now)?;
    Some(run_label(
        &next.timestamp().to_zoned(TimeZone::system()),
        now,
    ))
}

/// Restart rollout of `workload`, stamped at `now`.
fn restart_described(
    action: ResourceAction,
    workload: &Workload<'_>,
    now: jiff::Timestamp,
    warnings: Vec<SharedString>,
) -> Option<Described> {
    Some(Described {
        action,
        label: format!("Restart rollout of {} {}", workload.word(), workload.name),
        button: "Restart",
        risk: action_risk(action),
        // Whole seconds, so the dry-run and the commit send the same body.
        operation: WriteOperation::RestartRollout {
            restarted_at: jiff::Timestamp::from_second(now.as_second()).ok()?,
        },
        warnings,
    })
}

/// What a restart of a workload known only by name cannot check: the Used by rows of a ConfigMap
/// or Secret name their consumers while no list holds the workload itself.
const NAMED_RESTART_WARNING: &str =
    "Its state is not loaded: a paused rollout or an OnDelete strategy is not checked";

/// Restart rollout of the workload `name` of `namespace`, which no loaded list holds.
fn named_restart_intent(
    scope: &WorkloadScope<'_>,
    kind: ObjectKind,
    namespace: &str,
    name: &str,
    now: jiff::Timestamp,
) -> Option<WriteIntent> {
    let workload = Workload {
        kind,
        namespace,
        name,
    };
    let action = ResourceAction::RestartRollout(kind);
    let warnings = vec![NAMED_RESTART_WARNING.into()];
    let described = restart_described(action, &workload, now, warnings)?;
    intent_of(scope, &workload, described)
}

/// A workload that reads the edited ConfigMap or Secret through env: its name, and its row when a
/// list the session holds has it.
pub(crate) struct Consumer<'a> {
    pub(crate) namespace: &'a str,
    pub(crate) name: &'a str,
    pub(crate) object: Option<&'a KindObject>,
}

/// The batch of Restart rollout over the consumers of `source` (the edited ConfigMap or Secret)
/// that are workloads of one `kind`. A consumer whose row is loaded is checked like any Restart (a
/// paused rollout is skipped, OnDelete and Helm are warned about); one known only by name is
/// restarted unchecked and the dialog says so. `Err` is why nothing starts.
pub(crate) fn consumer_restart_batch(
    scope: &WorkloadScope<'_>,
    kind: ObjectKind,
    consumers: &[Consumer<'_>],
    source: &str,
    now: jiff::Timestamp,
) -> Result<BatchIntent, SharedString> {
    let action = ResourceAction::RestartRollout(kind);
    let mut items: Vec<BatchItem> = Vec::new();
    let mut skipped: Vec<SkippedItem> = Vec::new();
    let mut checked: Vec<&KindObject> = Vec::new();
    let mut unchecked = 0_usize;
    for consumer in consumers {
        let (namespace, name) = (consumer.namespace, consumer.name);
        let text: SharedString = format!("{namespace}/{name}").into();
        let intent = match consumer.object {
            Some(object) => {
                if let Some(reason) = row_block(action, object, None) {
                    skipped.push(SkippedItem {
                        object: text,
                        reason,
                    });
                    continue;
                }
                workload_intent(action, scope, object, now)
            }
            None => named_restart_intent(scope, kind, namespace, name, now),
        };
        let Some(intent) = intent else {
            continue;
        };
        match consumer.object {
            Some(object) => checked.push(object),
            None => unchecked += 1,
        }
        items.push(BatchItem {
            object: text,
            label: intent.label,
            request: intent.request,
        });
    }
    if items.is_empty() {
        let reason = skipped.first().map(|item| item.reason.clone());
        return Err(reason.unwrap_or_else(|| "the object name is not valid".into()));
    }
    let count = items.len();
    let mut warnings: Vec<SharedString> = Vec::new();
    if unchecked == count {
        warnings.push(NAMED_RESTART_WARNING.into());
    } else if unchecked > 0 {
        warnings.push(
            format!(
                "The state of {unchecked} of them is not loaded: a paused rollout or an {ON_DELETE} strategy is not checked"
            )
            .into(),
        );
    }
    warnings.extend(on_delete_warning(checked.iter().copied()));
    let helm = checked
        .iter()
        .filter(|object| is_helm_managed(object))
        .count();
    if helm == 1 && count == 1 {
        warnings.push(HELM_MANAGED_WARNING.into());
    } else if helm > 0 {
        warnings.push(
            format!("{helm} of them are managed by Helm: the next upgrade replaces this change")
                .into(),
        );
    }
    let noun = kind.name().to_ascii_lowercase();
    let (plural, verb) = if count == 1 {
        ("", "reads")
    } else {
        ("s", "read")
    };
    Ok(BatchIntent {
        cluster: scope.cluster.clone(),
        cluster_name: scope.cluster_name.to_owned().into(),
        action,
        label: format!("Restart {count} {noun}{plural} that {verb} {source}").into(),
        verb: "Restart".into(),
        button: "Restart".into(),
        risk: action_risk(action),
        warnings,
        plan: BatchPlan {
            cluster: scope.cluster.clone(),
            items,
            skipped,
            extras: BatchExtras::None,
            on_failure: BatchFailure::Continue,
        },
    })
}

/// Whether Helm renders the workload, by its `managed-by` label.
fn is_helm_managed(object: &KindObject) -> bool {
    let labels = match object {
        KindObject::Deployment(deployment) => &deployment.labels,
        KindObject::StatefulSet(set) => &set.labels,
        KindObject::DaemonSet(set) => &set.labels,
        _ => return false,
    };
    terms_are_helm_managed(labels)
}

/// A restart under `OnDelete` changes the template but no pod: the pods follow when deleted.
fn restart_warnings(object: &KindObject) -> Vec<SharedString> {
    let strategy = match object {
        KindObject::StatefulSet(set) => &set.update_strategy,
        KindObject::DaemonSet(set) => &set.update_strategy,
        _ => return Vec::new(),
    };
    if strategy == ON_DELETE {
        vec![format!("Strategy {ON_DELETE}: pods restart only when deleted").into()]
    } else {
        Vec::new()
    }
}

/// What a manual run does beside the scheduled ones, from the CronJob's concurrency policy and
/// the jobs it runs now.
fn trigger_warnings(policy: &str, active: usize) -> Vec<SharedString> {
    let mut warnings: Vec<SharedString> = Vec::new();
    if active > 0 {
        warnings.push(
            format!("{active} job(s) of this CronJob are running; this run starts anyway").into(),
        );
    }
    match policy {
        "Forbid" => warnings.push(
            "While this run is active, scheduled runs are skipped (concurrency Forbid)".into(),
        ),
        "Replace" => warnings.push(
            "A scheduled run replaces this job if it is still running (concurrency Replace)".into(),
        ),
        _ => {}
    }
    warnings
}

/// Why `action` cannot run on this row now, after the gate has said yes. `None` when it can.
/// `replica_sets` is what the row's drawer has loaded of its ReplicaSets, `None` while it has not
/// (Roll back reads it).
pub(crate) fn row_block(
    action: ResourceAction,
    object: &KindObject,
    replica_sets: Option<&[ReplicaSetSummary]>,
) -> Option<SharedString> {
    match (action, object) {
        (ResourceAction::RestartRollout(_), KindObject::Deployment(deployment))
            if deployment.is_paused =>
        {
            Some(PAUSED_REASON.into())
        }
        (ResourceAction::EditValues(_), object) => values_edit_block(object),
        (ResourceAction::ExpandClaim, KindObject::PersistentVolumeClaim(claim)) => {
            claim_block(claim)
        }
        (ResourceAction::SetDefaultStorageClass, KindObject::StorageClass(class)) => {
            class_block(class)
        }
        (ResourceAction::RecreateClaim, KindObject::PersistentVolumeClaim(claim)) => {
            recreate_block(claim)
        }
        (ResourceAction::SetReclaimPolicy, KindObject::PersistentVolume(volume)) => {
            reclaim_block(volume)
        }
        (ResourceAction::RollBack, KindObject::Deployment(deployment)) => {
            match roll_back_choice(deployment, replica_sets) {
                // Revisions that are not loaded yet are no reason to refuse: Roll back… opens the
                // drawer on its Revisions, which loads them and shows `Loading revisions…`.
                RollBackChoice::To(_) | RollBackChoice::NotLoaded => None,
                RollBackChoice::Paused => Some(PAUSED_REASON.into()),
                RollBackChoice::NoEarlier => Some("No earlier revision".into()),
            }
        }
        _ => None,
    }
}

/// The tag of an image reference: the text after the last `:` that follows the last `/`, else
/// the whole reference (a registry port such as `registry:5000/api` is not a tag).
pub(crate) fn image_tag(image: &str) -> &str {
    let name_start = image.rfind('/').map_or(0, |slash| slash + 1);
    match image[name_start..].rfind(':') {
        Some(colon) => &image[name_start + colon + 1..],
        None => image,
    }
}

/// The revision a Roll back goes to: one ReplicaSet of the Deployment, by its revision number.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RevisionTarget {
    pub(crate) replica_set: String,
    pub(crate) revision: u64,
    /// The tag of its first container image, when it has one: `2.13.4`.
    pub(crate) tag: Option<String>,
}

impl RevisionTarget {
    /// `rev 37 (2.13.4)`, or `rev 37` without a tag.
    pub(crate) fn text(&self) -> String {
        match &self.tag {
            Some(tag) => format!("rev {} ({tag})", self.revision),
            None => format!("rev {}", self.revision),
        }
    }
}

/// What Roll back can do for a Deployment now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RollBackChoice {
    /// kubectl refuses to roll back a paused rollout.
    Paused,
    /// The ReplicaSets are known only while the Deployment's drawer is open.
    NotLoaded,
    /// The Deployment has no older revision.
    NoEarlier,
    /// The previous revision.
    To(RevisionTarget),
}

/// The highest revision the Deployment owns below the one it runs now. Ownership comes from the
/// controller reference, not the name; a ReplicaSet without a numeric revision does not count.
pub(crate) fn previous_revision(
    deployment: &DeploymentSummary,
    replica_sets: &[ReplicaSetSummary],
) -> Option<RevisionTarget> {
    let current: u64 = deployment.revision.as_deref()?.parse().ok()?;
    replica_sets
        .iter()
        .filter(|set| {
            set.namespace == deployment.namespace
                && set.owner.as_ref().is_some_and(|owner| {
                    owner.kind == "Deployment" && owner.name == deployment.name
                })
        })
        .filter_map(|set| {
            let revision: u64 = set.revision.as_deref()?.parse().ok()?;
            (revision < current).then_some((revision, set))
        })
        .max_by_key(|(revision, _)| *revision)
        .map(|(revision, set)| RevisionTarget {
            replica_set: set.name.clone(),
            revision,
            tag: set
                .containers
                .first()
                .map(|container| image_tag(&container.image))
                .filter(|tag| !tag.is_empty())
                .map(str::to_owned),
        })
}

pub(crate) fn roll_back_choice(
    deployment: &DeploymentSummary,
    replica_sets: Option<&[ReplicaSetSummary]>,
) -> RollBackChoice {
    if deployment.is_paused {
        return RollBackChoice::Paused;
    }
    let Some(replica_sets) = replica_sets else {
        return RollBackChoice::NotLoaded;
    };
    match previous_revision(deployment, replica_sets) {
        Some(target) => RollBackChoice::To(target),
        None => RollBackChoice::NoEarlier,
    }
}

/// The intent of rolling `deployment` back to `target`.
pub(crate) fn roll_back_intent(
    scope: &WorkloadScope<'_>,
    deployment: &DeploymentSummary,
    target: &RevisionTarget,
) -> Option<WriteIntent> {
    let action = ResourceAction::RollBack;
    let workload = Workload {
        kind: ObjectKind::Deployment,
        namespace: &deployment.namespace,
        name: &deployment.name,
    };
    let described = Described {
        action,
        label: format!(
            "Roll back {} {} to {}",
            workload.word(),
            deployment.name,
            target.text()
        ),
        button: "Roll back",
        risk: action_risk(action),
        operation: WriteOperation::RollBackDeployment {
            replica_set: target.replica_set.clone(),
            revision: target.revision,
        },
        warnings: if terms_are_helm_managed(&deployment.labels) {
            vec![HELM_MANAGED_WARNING.into()]
        } else {
            Vec::new()
        },
    };
    intent_of(scope, &workload, described)
}

/// The menu text of `action` for this row: `label` (the kind table's), except where the state
/// flips the verb.
pub(crate) fn state_label(
    action: ResourceAction,
    label: &'static str,
    object: &KindObject,
) -> &'static str {
    match (action, object) {
        (ResourceAction::PauseRollout, KindObject::Deployment(deployment))
            if deployment.is_paused =>
        {
            "Resume rollout"
        }
        (ResourceAction::SuspendCronJob, KindObject::CronJob(cron_job))
            if cron_job.is_suspended =>
        {
            "Resume"
        }
        _ => label,
    }
}

/// What a Resume tells before it is confirmed: the image changes made while the rollout was paused,
/// read from the ReplicaSet that runs now. Summaries keep no env or resources (they can hold
/// secrets), so those changes cannot be listed; the line says so.
pub(crate) fn pending_changes_note(
    deployment: &DeploymentSummary,
    replica_sets: Option<&[ReplicaSetSummary]>,
) -> SharedString {
    let running = replica_sets.and_then(|sets| running_replica_set(deployment, sets));
    let Some(running) = running else {
        return "Rolls out the pod template changes made while paused".into();
    };
    let changes = image_changes(&running.containers, &deployment.containers);
    if changes.is_empty() {
        return "Rolls out the pod template changes made while paused; no image changed".into();
    }
    let count = changes.len();
    let plural = if count == 1 { "" } else { "s" };
    let shown: Vec<_> = changes.iter().take(MAX_PENDING_SHOWN).cloned().collect();
    let more = count - shown.len();
    let more = if more > 0 {
        format!(", +{more} more")
    } else {
        String::new()
    };
    format!(
        "Rolls out {count} pending image change{plural}: {}{more}",
        shown.join(", ")
    )
    .into()
}

/// The ReplicaSet whose pods run now: the newest one of the Deployment that still has replicas. The
/// Deployment's own revision is not it: while paused, the controller stamps it on the ReplicaSet of
/// the pending template when one already exists.
fn running_replica_set<'a>(
    deployment: &DeploymentSummary,
    replica_sets: &'a [ReplicaSetSummary],
) -> Option<&'a ReplicaSetSummary> {
    replica_sets
        .iter()
        .filter(|set| {
            set.namespace == deployment.namespace
                && set.desired > 0
                && set.owner.as_ref().is_some_and(|owner| {
                    owner.kind == "Deployment" && owner.name == deployment.name
                })
        })
        .max_by_key(|set| {
            set.revision
                .as_deref()
                .and_then(|text| text.parse::<u64>().ok())
        })
}

/// How many image changes the Resume confirm names before it counts the rest.
const MAX_PENDING_SHOWN: usize = 3;

/// `image web 1.27-alpine → 1.26-alpine` per container whose image differs; a container only one
/// side has reads `container db added` or `container db removed`.
fn image_changes(running: &[TemplateContainer], pending: &[TemplateContainer]) -> Vec<String> {
    let mut changes = Vec::new();
    for container in pending {
        match running.iter().find(|old| old.name == container.name) {
            Some(old) if old.image == container.image => {}
            Some(old) => changes.push(format!(
                "image {} {}",
                container.name,
                image_change(&old.image, &container.image)
            )),
            None => changes.push(format!("container {} added", container.name)),
        }
    }
    for old in running {
        if !pending.iter().any(|container| container.name == old.name) {
            changes.push(format!("container {} removed", old.name));
        }
    }
    changes
}

/// `1.27 → 1.26` when only the tag changed, else the whole references.
fn image_change(old: &str, new: &str) -> String {
    let repository = |image: &str| {
        let tag = image_tag(image);
        image
            .strip_suffix(tag)
            .map(|rest| rest.trim_end_matches(':').to_owned())
    };
    match (repository(old), repository(new)) {
        (Some(old_repo), Some(new_repo)) if old_repo == new_repo => {
            format!("{} → {}", image_tag(old), image_tag(new))
        }
        _ => format!("{old} → {new}"),
    }
}

/// The most replicas the API accepts (`spec.replicas` is an `int32`).
const MAX_REPLICAS: u32 = i32::MAX as u32;

/// The HPA that manages the replicas of a workload, as the Scale warning names it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ManagingHpa {
    pub(crate) name: String,
    pub(crate) min: u32,
    pub(crate) max: u32,
}

/// A row that can scale, with what the popover and the palette say about it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ScaleTarget {
    pub(crate) kind: ObjectKind,
    pub(crate) namespace: String,
    pub(crate) name: String,
    pub(crate) desired: u32,
    pub(crate) ready: u32,
    /// Set only when the HPA list is already loaded and an HPA targets this workload.
    pub(crate) hpa: Option<ManagingHpa>,
    /// Helm renders the workload, so its next upgrade sets the replicas again.
    pub(crate) is_helm_managed: bool,
    /// The pod template, for the quota check.
    pub(crate) template: Vec<TemplateContainer>,
    /// The quotas of the namespace, set by `with_quotas` only when that list is already loaded.
    pub(crate) quotas: Vec<ResourceQuotaSummary>,
    /// What a scale-down does besides removing pods: claims left behind, budgets that block.
    pub(crate) effects: ScaleEffects,
}

impl ScaleTarget {
    /// `None` for a row that does not scale (anything but a Deployment or a StatefulSet).
    /// `hpas` is the HPA list when it is loaded, else empty: no list starts for a hint.
    pub(crate) fn of(object: &KindObject, hpas: &[KindObject]) -> Option<Self> {
        let (kind, namespace, name, desired, ready, labels, template) = match object {
            KindObject::Deployment(d) => (
                ObjectKind::Deployment,
                &d.namespace,
                &d.name,
                d.desired,
                d.ready,
                &d.labels,
                &d.containers,
            ),
            KindObject::StatefulSet(s) => (
                ObjectKind::StatefulSet,
                &s.namespace,
                &s.name,
                s.desired,
                s.ready,
                &s.labels,
                &s.containers,
            ),
            _ => return None,
        };
        Some(Self {
            kind,
            namespace: namespace.clone(),
            name: name.clone(),
            desired,
            ready,
            hpa: managing_hpa(hpas, kind, namespace, name),
            is_helm_managed: terms_are_helm_managed(labels),
            template: template.clone(),
            quotas: Vec::new(),
            effects: ScaleEffects::of(object),
        })
    }

    /// Adds the namespace quotas of `quotas` (the ResourceQuota list when it is loaded, else empty:
    /// no list starts for a hint), so a Scale can say which new pods the quota refuses.
    pub(crate) fn with_quotas(mut self, quotas: &[KindObject]) -> Self {
        self.quotas = quotas
            .iter()
            .filter_map(|object| match object {
                KindObject::ResourceQuota(quota) if quota.namespace == self.namespace => {
                    Some(quota.clone())
                }
                _ => None,
            })
            .collect();
        self
    }

    /// The same target with the budgets among `budgets` that cover its pods (the PDBs the session
    /// already holds, empty while that list is not loaded).
    pub(crate) fn with_budgets(self, budgets: &[KindObject]) -> Self {
        let effects = self.effects.clone().with_budgets(&self.namespace, budgets);
        Self { effects, ..self }
    }

    /// `deployment/api`.
    pub(crate) fn subject_text(&self) -> String {
        format!("{}/{}", self.kind.name().to_ascii_lowercase(), self.name)
    }

    /// `Now 3 desired · 2 ready`.
    pub(crate) fn state_text(&self) -> String {
        format!("Now {} desired · {} ready", self.desired, self.ready)
    }
}

/// The HPA that targets the workload `kind` `namespace`/`name`, among the loaded HPAs.
pub(crate) fn hpa_targeting<'a>(
    hpas: &'a [KindObject],
    kind: ObjectKind,
    namespace: &str,
    name: &str,
) -> Option<&'a HorizontalPodAutoscalerSummary> {
    hpas.iter().find_map(|object| {
        let KindObject::HorizontalPodAutoscaler(hpa) = object else {
            return None;
        };
        let targets_it =
            hpa.namespace == namespace && hpa.target.kind == kind.name() && hpa.target.name == name;
        targets_it.then_some(hpa)
    })
}

fn managing_hpa(
    hpas: &[KindObject],
    kind: ObjectKind,
    namespace: &str,
    name: &str,
) -> Option<ManagingHpa> {
    hpa_targeting(hpas, kind, namespace, name).map(|hpa| ManagingHpa {
        name: hpa.name.clone(),
        min: hpa.min_replicas,
        max: hpa.max_replicas,
    })
}

/// What the Scale button does with the typed text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReplicasInput {
    /// Empty, not a whole number, or more than the API accepts.
    Invalid,
    /// The number the workload already has.
    Unchanged,
    Set(u32),
}

/// A whole number of replicas the API accepts: digits only, so a sign or a fraction is refused.
pub(crate) fn parse_replicas(text: &str) -> Option<u32> {
    let text = text.trim();
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    text.parse()
        .ok()
        .filter(|replicas| *replicas <= MAX_REPLICAS)
}

pub(crate) fn replicas_input(text: &str, desired: u32) -> ReplicasInput {
    match parse_replicas(text) {
        None => ReplicasInput::Invalid,
        Some(replicas) if replicas == desired => ReplicasInput::Unchanged,
        Some(replicas) => ReplicasInput::Set(replicas),
    }
}

/// The warnings of scaling `target` to `replicas`, in the order the dialog and the popover show them.
pub(crate) fn scale_warnings(target: &ScaleTarget, replicas: u32) -> Vec<SharedString> {
    let mut warnings: Vec<SharedString> = Vec::new();
    if replicas < target.desired {
        warnings.push(format!("Scaling down from {} to {replicas}", target.desired).into());
    }
    warnings.extend(target.effects.lines(target.desired, replicas));
    if let Some(hpa) = &target.hpa {
        // A value inside the range may still move with the load; one outside is always reverted.
        let text = if (hpa.min..=hpa.max).contains(&replicas) {
            format!(
                "HPA {} manages replicas ({}–{}); it will override this",
                hpa.name, hpa.min, hpa.max
            )
        } else {
            format!(
                "HPA {} keeps {}–{}; a value outside is reverted",
                hpa.name, hpa.min, hpa.max
            )
        };
        warnings.push(text.into());
    }
    warnings.extend(
        quota_scale_warnings(&target.quotas, &target.template, target.desired, replicas)
            .into_iter()
            .map(SharedString::from),
    );
    if target.is_helm_managed {
        warnings.push(HELM_MANAGED_WARNING.into());
    }
    warnings
}

/// The intent of scaling `target` to `replicas`. Zero takes the workload down, so it is
/// `Destructive`; every other count is a `Change`.
pub(crate) fn scale_intent(
    scope: &WorkloadScope<'_>,
    target: &ScaleTarget,
    replicas: u32,
) -> Option<WriteIntent> {
    let action = ResourceAction::Scale(target.kind);
    let workload = Workload {
        kind: target.kind,
        namespace: &target.namespace,
        name: &target.name,
    };
    let risk = if replicas == 0 {
        ActionRisk::Destructive
    } else {
        ActionRisk::Change
    };
    let described = Described {
        action,
        label: format!(
            "Scale {} {} from {} to {replicas}",
            workload.word(),
            target.name,
            target.desired
        ),
        button: "Scale",
        risk,
        operation: WriteOperation::ScaleWorkload {
            replicas,
            previous: target.desired,
        },
        warnings: scale_warnings(target, replicas),
    };
    intent_of(scope, &workload, described)
}

// ---- Set image: one container of one workload ----

/// A row that can take Set image, with what the popover says about it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ImageTarget {
    pub(crate) kind: ObjectKind,
    pub(crate) namespace: String,
    pub(crate) name: String,
    /// The containers of the pod template, in spec order; init containers are not listed.
    pub(crate) containers: Vec<TemplateContainer>,
    /// Helm renders the workload, so its next upgrade sets the image again.
    pub(crate) is_helm_managed: bool,
    /// A paused Deployment keeps its pods until it resumes.
    pub(crate) is_paused: bool,
}

impl ImageTarget {
    /// `None` for a row with no pod template (anything but a Deployment, StatefulSet, or
    /// DaemonSet) or one with no container.
    pub(crate) fn of(object: &KindObject) -> Option<Self> {
        let (kind, namespace, name, containers, labels, is_paused) = match object {
            KindObject::Deployment(d) => (
                ObjectKind::Deployment,
                &d.namespace,
                &d.name,
                &d.containers,
                &d.labels,
                d.is_paused,
            ),
            KindObject::StatefulSet(s) => (
                ObjectKind::StatefulSet,
                &s.namespace,
                &s.name,
                &s.containers,
                &s.labels,
                false,
            ),
            KindObject::DaemonSet(d) => (
                ObjectKind::DaemonSet,
                &d.namespace,
                &d.name,
                &d.containers,
                &d.labels,
                false,
            ),
            _ => return None,
        };
        (!containers.is_empty()).then(|| Self {
            kind,
            namespace: namespace.clone(),
            name: name.clone(),
            containers: containers.clone(),
            is_helm_managed: terms_are_helm_managed(labels),
            is_paused,
        })
    }

    /// `deployment/api`.
    pub(crate) fn subject_text(&self) -> String {
        format!("{}/{}", self.kind.name().to_ascii_lowercase(), self.name)
    }

    /// The image `container` has now.
    pub(crate) fn image_of(&self, container: &str) -> Option<&str> {
        self.containers
            .iter()
            .find(|candidate| candidate.name == container)
            .map(|candidate| candidate.image.as_str())
    }
}

/// What the Set image button does with the typed image.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ImageInput {
    /// Empty, or with a space in it: the server would refuse it.
    Invalid,
    /// The image the container has already.
    Unchanged,
    Set(String),
}

pub(crate) fn image_input(text: &str, current: &str) -> ImageInput {
    let image = text.trim();
    if image.is_empty()
        || image
            .chars()
            .any(|ch| ch.is_whitespace() || ch.is_control())
    {
        return ImageInput::Invalid;
    }
    if image == current {
        return ImageInput::Unchanged;
    }
    ImageInput::Set(image.to_owned())
}

/// The bytes of `image` the popover selects for editing: the digest after `@`, else the tag after
/// the last `:` of the name, else nothing (an empty range at the end, where a tag is typed).
pub(crate) fn tag_range(image: &str) -> std::ops::Range<usize> {
    if let Some(at) = image.find('@') {
        return at + 1..image.len();
    }
    let name_start = image.rfind('/').map_or(0, |slash| slash + 1);
    match image[name_start..].rfind(':') {
        Some(colon) => name_start + colon + 1..image.len(),
        None => image.len()..image.len(),
    }
}

/// What a Set image of `target` warns about, in the order the popover and the dialog show it.
pub(crate) fn image_warnings(target: &ImageTarget) -> Vec<SharedString> {
    let mut warnings: Vec<SharedString> = Vec::new();
    if target.is_paused {
        warnings.push("The rollout is paused: the pods change after Resume".into());
    }
    if target.is_helm_managed {
        warnings.push(HELM_MANAGED_WARNING.into());
    }
    warnings
}

/// The intent of changing the image of `container` of `target` to `image`, with `change_cause`
/// (empty for none). `None` when the container is no longer in the template or the request is not
/// valid.
pub(crate) fn set_image_intent(
    scope: &WorkloadScope<'_>,
    target: &ImageTarget,
    container: &str,
    image: &str,
    change_cause: &str,
) -> Option<WriteIntent> {
    let previous = target.image_of(container)?;
    let action = ResourceAction::SetImage(target.kind);
    let workload = Workload {
        kind: target.kind,
        namespace: &target.namespace,
        name: &target.name,
    };
    let described = Described {
        action,
        label: format!("Set image of {} {}", workload.word(), target.name),
        button: "Set image",
        risk: action_risk(action),
        operation: WriteOperation::SetContainerImage {
            container: container.to_owned(),
            image: image.to_owned(),
            previous_image: previous.to_owned(),
            change_cause: Some(change_cause.to_owned()),
        },
        warnings: image_warnings(target),
    };
    let mut intent = intent_of(scope, &workload, described)?;
    intent.change_lines = image_change_lines(target, container, previous, &intent.request);
    Some(intent)
}

/// `image: a → b` (named after the container when the template has several), then the cause as it
/// is stored.
fn image_change_lines(
    target: &ImageTarget,
    container: &str,
    previous: &str,
    request: &WriteRequest,
) -> Vec<SharedString> {
    let WriteOperation::SetContainerImage {
        image,
        change_cause,
        ..
    } = request.operation()
    else {
        return Vec::new();
    };
    let label = if target.containers.len() > 1 {
        format!("image ({container})")
    } else {
        "image".to_owned()
    };
    let cause = match change_cause {
        Some(cause) => format!("change cause: {cause}"),
        None => "change cause: none (an earlier one is cleared)".to_owned(),
    };
    vec![
        format!("{label}: {previous} → {image}").into(),
        cause.into(),
    ]
}

// ---- Bulk: the same actions on the ticked rows of one cluster ----

/// What a bulk action reads: the ticked rows, the cluster's display name, the time a restart is
/// stamped with, and the HPAs the session already holds (empty while that list is not loaded).
pub(crate) struct BulkInputs<'a> {
    pub(crate) cluster_name: &'a str,
    pub(crate) rows: &'a [CheckedRow<'a>],
    pub(crate) now: jiff::Timestamp,
    pub(crate) hpas: &'a [KindObject],
}

/// The words a bulk action goes by.
struct BulkWords {
    /// The confirm button and the notice: `Restart`.
    verb: &'static str,
    /// The audit line of each item: `Trigger now`.
    audit: &'static str,
}

fn bulk_words(action: ResourceAction, resume: bool) -> Option<BulkWords> {
    let (verb, audit) = match action {
        ResourceAction::RestartRollout(_) => ("Restart", "Restart"),
        ResourceAction::Scale(_) => ("Scale", "Scale"),
        ResourceAction::RerunJob => ("Re-run", "Re-run"),
        ResourceAction::TriggerCronJob => ("Run", "Trigger now"),
        ResourceAction::SuspendCronJob if resume => ("Resume", "Resume"),
        ResourceAction::SuspendCronJob => ("Suspend", "Suspend"),
        _ => return None,
    };
    Some(BulkWords { verb, audit })
}

/// `team-a/api`, as the list of the batch dialog shows an object.
fn object_text(object: &KindObject) -> SharedString {
    match workload_of(object) {
        Some(workload) => format!("{}/{}", workload.namespace, workload.name).into(),
        None => "object".into(),
    }
}

fn skipped(object: &KindObject, reason: impl Into<SharedString>) -> SkippedItem {
    SkippedItem {
        object: object_text(object),
        reason: reason.into(),
    }
}

fn batch_item(intent: WriteIntent, object: &KindObject) -> BatchItem {
    BatchItem {
        object: object_text(object),
        label: intent.label,
        request: intent.request,
    }
}

/// Whether every ticked row is a suspended CronJob: then the Suspend button reads Resume.
pub(crate) fn all_suspended(rows: &[CheckedRow<'_>]) -> bool {
    !rows.is_empty()
        && rows
            .iter()
            .all(|row| matches!(row.object, KindObject::CronJob(job) if job.is_suspended))
}

/// The batch of Restart, Re-run, Trigger now, or Suspend/Resume over the ticked rows. A row the
/// state refuses (a paused Deployment on Restart, a CronJob that already is where the batch goes)
/// becomes a skipped line. `Err` is the reason the button is off.
pub(crate) fn bulk_intent(
    action: ResourceAction,
    inputs: &BulkInputs<'_>,
) -> Result<BatchIntent, SharedString> {
    let resume = all_suspended(inputs.rows);
    let words =
        bulk_words(action, resume).ok_or_else(|| SharedString::from("Not a bulk action"))?;
    let plan = batch_plan(inputs.rows, |row| {
        let scope = WorkloadScope {
            cluster: row.cluster,
            cluster_name: inputs.cluster_name,
        };
        if let Some(reason) = row_block(action, row.object, None) {
            return Err(skipped(row.object, reason));
        }
        if let (ResourceAction::SuspendCronJob, KindObject::CronJob(cron_job)) =
            (action, row.object)
        {
            // Every row goes the same way: to suspended, or back when all are suspended already.
            if cron_job.is_suspended != resume {
                let reason = if resume {
                    "already running"
                } else {
                    "already suspended"
                };
                return Err(skipped(row.object, reason));
            }
        }
        let intent = workload_intent(action, &scope, row.object, inputs.now)
            .ok_or_else(|| skipped(row.object, "the object name is not valid"))?;
        Ok(batch_item(intent, row.object))
    })?;
    Ok(build_batch(
        action,
        inputs,
        plan,
        &words,
        BatchShape {
            risk: action_risk(action),
            suffix: if action == ResourceAction::TriggerCronJob {
                " now".to_owned()
            } else {
                String::new()
            },
            warnings: bulk_restart_warnings(inputs.rows),
        },
    ))
}

/// The batch of Scale over the ticked rows, to the same count. A row that has it already is
/// skipped.
pub(crate) fn bulk_scale_intent(
    inputs: &BulkInputs<'_>,
    replicas: u32,
    kind: ObjectKind,
) -> Result<BatchIntent, SharedString> {
    let action = ResourceAction::Scale(kind);
    let words = bulk_words(action, false).ok_or_else(|| SharedString::from("Not a bulk action"))?;
    // Counted for the rows that became items, so the warnings agree with the dialog's list.
    let (shrinking, managed) = (Cell::new(0_usize), Cell::new(0_usize));
    let plan = batch_plan(inputs.rows, |row| {
        let scope = WorkloadScope {
            cluster: row.cluster,
            cluster_name: inputs.cluster_name,
        };
        let target = ScaleTarget::of(row.object, inputs.hpas)
            .ok_or_else(|| skipped(row.object, "does not scale"))?;
        if target.desired == replicas {
            return Err(skipped(row.object, format!("already {replicas}")));
        }
        let intent = scale_intent(&scope, &target, replicas)
            .ok_or_else(|| skipped(row.object, "the object name is not valid"))?;
        if replicas < target.desired {
            shrinking.set(shrinking.get() + 1);
        }
        if target.hpa.is_some() {
            managed.set(managed.get() + 1);
        }
        Ok(batch_item(intent, row.object))
    })?;
    let mut warnings: Vec<SharedString> = Vec::new();
    if shrinking.get() > 0 {
        warnings.push(format!("Scaling down {} of {}", shrinking.get(), plan.items.len()).into());
    }
    if managed.get() > 0 {
        let text = format!(
            "{} of them are managed by an HPA, which will override this",
            managed.get()
        );
        warnings.push(text.into());
    }
    Ok(build_batch(
        action,
        inputs,
        plan,
        &words,
        BatchShape {
            risk: if replicas == 0 {
                ActionRisk::Destructive
            } else {
                ActionRisk::Change
            },
            suffix: format!(" to {replicas}"),
            warnings,
        },
    ))
}

/// The OnDelete line once, however many rows have that update strategy.
fn bulk_restart_warnings(rows: &[CheckedRow<'_>]) -> Vec<SharedString> {
    on_delete_warning(rows.iter().map(|row| row.object))
        .into_iter()
        .collect()
}

/// The OnDelete line for the `objects` that have that update strategy, or `None` when none do.
fn on_delete_warning<'a>(objects: impl Iterator<Item = &'a KindObject>) -> Option<SharedString> {
    let on_delete = objects
        .filter(|object| !restart_warnings(object).is_empty())
        .count();
    (on_delete > 0).then(|| {
        format!("{on_delete} use update strategy {ON_DELETE}: their pods restart only when deleted")
            .into()
    })
}

/// What a batch adds to the words of its action.
struct BatchShape {
    risk: ActionRisk,
    /// After the count and the noun in the title: ` to 5`, ` now`.
    suffix: String,
    warnings: Vec<SharedString>,
}

fn build_batch(
    action: ResourceAction,
    inputs: &BulkInputs<'_>,
    plan: BatchPlan,
    words: &BulkWords,
    shape: BatchShape,
) -> BatchIntent {
    let noun = plan.items.first().map_or_else(String::new, |item| {
        format!(
            "{}s",
            item.request.target().kind_name().to_ascii_lowercase()
        )
    });
    let label = format!("{} {} {noun}{}", words.verb, plan.items.len(), shape.suffix);
    BatchIntent {
        cluster: plan.cluster.clone(),
        cluster_name: inputs.cluster_name.to_owned().into(),
        action,
        label: label.into(),
        verb: words.verb.into(),
        button: words.audit.into(),
        risk: shape.risk,
        warnings: shape.warnings,
        plan,
    }
}

#[cfg(test)]
#[path = "workload_actions_tests.rs"]
pub(crate) mod workload_actions_tests;
