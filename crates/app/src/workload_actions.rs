//! What the workload actions of spec 0032 ask the user to confirm: the intent of each action
//! (label, button, risk, warnings, request) and the reasons a row cannot take an action now.
//! Pure: it reads a row's summary and builds a `WriteIntent`; the shell sends nothing from here.

use std::cell::Cell;

use cluster::{
    DeploymentSummary, HELM_MANAGED_WARNING, ObjectKind, ObjectRef, ReplicaSetSummary,
    WriteOperation, WriteRequest, terms_are_helm_managed,
};
use gpui_kit::SharedString;

use crate::app_shell::batch_write::{
    BatchExtras, BatchFailure, BatchIntent, BatchItem, BatchPlan, CheckedRow, SkippedItem,
    batch_plan,
};
use crate::app_shell::write_flow::WriteIntent;
use crate::cluster_registry::ClusterRef;
use crate::kind_row::KindObject;
use crate::resource_actions::{ResourceAction, action_risk, values_edit_block};
use crate::resource_edits::{claim_block, class_block};
use crate::write_guard::ActionRisk;

/// Why a paused Deployment cannot restart or roll back: kubectl refuses both.
pub(crate) const PAUSED_REASON: &str = "Resume the rollout first";
/// Why Roll back is off for a row whose drawer has not loaded its ReplicaSets.
pub(crate) const NOT_LOADED_REASON: &str = "Open the deployment to load its revisions";
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
                warnings: Vec::new(),
            }
        }
        (ResourceAction::TriggerCronJob, KindObject::CronJob(cron_job)) => Described {
            action,
            label: format!("Run {word} {name} now"),
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
            warnings: Vec::new(),
        },
        _ => return None,
    };
    intent_of(scope, &workload, described)
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
pub(crate) fn named_restart_intent(
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

/// The batch of Restart rollout over workloads of one `kind`, each `(namespace, name)`, which no
/// loaded list holds: the consumers of an edited ConfigMap or Secret. `Err` is why nothing starts.
pub(crate) fn named_restart_batch(
    scope: &WorkloadScope<'_>,
    kind: ObjectKind,
    workloads: &[(&str, &str)],
    now: jiff::Timestamp,
) -> Result<BatchIntent, SharedString> {
    let items: Vec<BatchItem> = workloads
        .iter()
        .filter_map(|(namespace, name)| {
            let intent = named_restart_intent(scope, kind, namespace, name, now)?;
            Some(BatchItem {
                object: format!("{namespace}/{name}").into(),
                label: intent.label,
                request: intent.request,
            })
        })
        .collect();
    if items.is_empty() {
        return Err("the object name is not valid".into());
    }
    let action = ResourceAction::RestartRollout(kind);
    let noun = kind.name().to_ascii_lowercase();
    Ok(BatchIntent {
        cluster: scope.cluster.clone(),
        cluster_name: scope.cluster_name.to_owned().into(),
        action,
        label: format!("Restart {} {noun}s", items.len()).into(),
        verb: "Restart".into(),
        button: "Restart".into(),
        risk: action_risk(action),
        warnings: vec![NAMED_RESTART_WARNING.into()],
        plan: BatchPlan {
            cluster: scope.cluster.clone(),
            items,
            skipped: Vec::new(),
            extras: BatchExtras::None,
            on_failure: BatchFailure::Continue,
        },
    })
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
            claim_block(claim, &[])
        }
        (ResourceAction::SetDefaultStorageClass, KindObject::StorageClass(class)) => {
            class_block(class)
        }
        (ResourceAction::RollBack, KindObject::Deployment(deployment)) => {
            match roll_back_choice(deployment, replica_sets) {
                RollBackChoice::To(_) => None,
                RollBackChoice::Paused => Some(PAUSED_REASON.into()),
                RollBackChoice::NotLoaded => Some(NOT_LOADED_REASON.into()),
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
}

impl ScaleTarget {
    /// `None` for a row that does not scale (anything but a Deployment or a StatefulSet).
    /// `hpas` is the HPA list when it is loaded, else empty: no list starts for a hint.
    pub(crate) fn of(object: &KindObject, hpas: &[KindObject]) -> Option<Self> {
        let (kind, namespace, name, desired, ready, labels) = match object {
            KindObject::Deployment(d) => (
                ObjectKind::Deployment,
                &d.namespace,
                &d.name,
                d.desired,
                d.ready,
                &d.labels,
            ),
            KindObject::StatefulSet(s) => (
                ObjectKind::StatefulSet,
                &s.namespace,
                &s.name,
                s.desired,
                s.ready,
                &s.labels,
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
        })
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

fn managing_hpa(
    hpas: &[KindObject],
    kind: ObjectKind,
    namespace: &str,
    name: &str,
) -> Option<ManagingHpa> {
    hpas.iter().find_map(|object| {
        let KindObject::HorizontalPodAutoscaler(hpa) = object else {
            return None;
        };
        let targets_it =
            hpa.namespace == namespace && hpa.target.kind == kind.name() && hpa.target.name == name;
        targets_it.then(|| ManagingHpa {
            name: hpa.name.clone(),
            min: hpa.min_replicas,
            max: hpa.max_replicas,
        })
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
        operation: WriteOperation::ScaleWorkload { replicas },
        warnings: scale_warnings(target, replicas),
    };
    intent_of(scope, &workload, described)
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
    let on_delete = rows
        .iter()
        .filter(|row| !restart_warnings(row.object).is_empty())
        .count();
    if on_delete == 0 {
        return Vec::new();
    }
    vec![
        format!(
            "{on_delete} use update strategy {ON_DELETE}: their pods restart only when deleted"
        )
        .into(),
    ]
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
