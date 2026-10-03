//! What the resource edits of spec 0032b ask the user to confirm: the HPA min / max, the PVC
//! Expand, and Set as default storage class. Pure: it reads summaries and builds the intents, the
//! warnings, and the input checks of the popover; the shell sends nothing from here.

use std::cell::Cell;

use cluster::{
    ByteAmount, HorizontalPodAutoscalerSummary, ObjectKind, ObjectRef,
    PersistentVolumeClaimSummary, StorageClassSummary, WriteOperation, WriteRequest,
};
use gpui_kit::SharedString;

use crate::app_shell::batch_write::{
    BatchExtras, BatchFailure, BatchIntent, BatchItem, BatchPlan, ItemProgress, SkippedItem,
    batch_plan,
};
use crate::app_shell::write_flow::WriteIntent;
use crate::cluster_registry::ClusterRef;
use crate::kind_row::KindObject;
use crate::resource_actions::{ResourceAction, action_risk};
use crate::resource_kind::ResourceKind;
use crate::table_selection::{ClusterObject, ResourceKey};
use crate::workload_actions::{BulkInputs, WorkloadScope, parse_replicas};

/// Shown under the Min field when it is zero: the API refuses `minReplicas: 0` without an alpha gate.
pub(crate) const MIN_BELOW_ONE: &str = "Min must be at least 1";
pub(crate) const MIN_ABOVE_MAX: &str = "Min must not exceed max";

// ---- HPA min / max ----

/// What the Edit min / max form does with the typed pair.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RangeInput {
    /// A field is empty or not a whole number the API accepts: the button is off, nothing is said.
    Incomplete,
    /// The pair breaks a rule; the text goes under the field.
    Refused(&'static str),
    /// The pair the HPA has already.
    Unchanged,
    Set {
        min: u32,
        max: u32,
    },
}

/// `current` is the `(min, max)` the HPA has now, `None` for several rows, which share no pair.
pub(crate) fn range_input(min: &str, max: &str, current: Option<(u32, u32)>) -> RangeInput {
    let (Some(min), Some(max)) = (parse_replicas(min), parse_replicas(max)) else {
        return RangeInput::Incomplete;
    };
    if min == 0 {
        return RangeInput::Refused(MIN_BELOW_ONE);
    }
    if min > max {
        return RangeInput::Refused(MIN_ABOVE_MAX);
    }
    if current == Some((min, max)) {
        return RangeInput::Unchanged;
    }
    RangeInput::Set { min, max }
}

/// `deployment/frontend`, the workload the HPA scales.
pub(crate) fn hpa_target_text(hpa: &HorizontalPodAutoscalerSummary) -> String {
    format!(
        "{}/{}",
        hpa.target.kind.to_ascii_lowercase(),
        hpa.target.name
    )
}

/// `Now 9 replicas`, the muted line under the fields.
pub(crate) fn hpa_state_text(hpa: &HorizontalPodAutoscalerSummary) -> String {
    format!("Now {} replicas", hpa.current_replicas)
}

/// What the HPA does to its workload at once when the range no longer holds the replicas it has.
pub(crate) fn hpa_range_warnings(
    hpa: &HorizontalPodAutoscalerSummary,
    min: u32,
    max: u32,
) -> Vec<SharedString> {
    let current = hpa.current_replicas;
    let target = hpa_target_text(hpa);
    if max < current {
        vec![format!("The HPA will scale {target} down from {current} to {max}").into()]
    } else if min > current {
        vec![format!("The HPA will scale {target} up from {current} to {min}").into()]
    } else {
        Vec::new()
    }
}

/// The intent of setting the range of `hpa`; `None` when the pair or the name is not valid.
pub(crate) fn hpa_range_intent(
    scope: &WorkloadScope<'_>,
    hpa: &HorizontalPodAutoscalerSummary,
    min: u32,
    max: u32,
) -> Option<WriteIntent> {
    let target = ObjectRef::new(
        ObjectKind::HorizontalPodAutoscaler,
        Some(hpa.namespace.clone()),
        hpa.name.clone(),
    )?;
    let request = WriteRequest::new(target, WriteOperation::SetHpaReplicaRange { min, max })?;
    let action = ResourceAction::EditHpaRange;
    Some(WriteIntent {
        cluster: scope.cluster.clone(),
        cluster_name: scope.cluster_name.to_owned().into(),
        action,
        label: format!("Set replicas of hpa {} to {min}\u{2013}{max}", hpa.name).into(),
        button: "Set limits".into(),
        request,
        risk: action_risk(action),
        expected_name: None,
        warnings: hpa_range_warnings(hpa, min, max),
    })
}

/// `team-a/frontend-hpa`, as the list of the batch dialog shows an object.
fn namespaced_text(namespace: &str, name: &str) -> SharedString {
    format!("{namespace}/{name}").into()
}

fn skipped(object: SharedString, reason: impl Into<SharedString>) -> SkippedItem {
    SkippedItem {
        object,
        reason: reason.into(),
    }
}

/// The batch of one edit over `plan`: `verb` is the confirm button and the notice, `audit` the
/// action of each audit line, `title` the dialog title after the verb (`of 4 hpas to 3–20`).
fn edit_batch(
    action: ResourceAction,
    cluster_name: &str,
    plan: BatchPlan,
    verb: &'static str,
    title: String,
    warnings: Vec<SharedString>,
) -> BatchIntent {
    BatchIntent {
        cluster: plan.cluster.clone(),
        cluster_name: cluster_name.to_owned().into(),
        action,
        label: format!("{verb} {title}").into(),
        verb: verb.into(),
        button: verb.into(),
        risk: action_risk(action),
        warnings,
        expected_name: None,
        plan,
    }
}

/// The batch of one range over the ticked HPAs. A row that has the range already is skipped.
/// `Err` is the reason the popover's button or the dialog cannot open.
pub(crate) fn bulk_hpa_range_intent(
    inputs: &BulkInputs<'_>,
    min: u32,
    max: u32,
) -> Result<BatchIntent, SharedString> {
    let moved = Cell::new(0_usize);
    let plan = batch_plan(inputs.rows, |row| {
        let KindObject::HorizontalPodAutoscaler(hpa) = row.object else {
            return Err(skipped("object".into(), "is not an HPA"));
        };
        let object = namespaced_text(&hpa.namespace, &hpa.name);
        if (hpa.min_replicas, hpa.max_replicas) == (min, max) {
            return Err(skipped(object, format!("already {min}\u{2013}{max}")));
        }
        let scope = WorkloadScope {
            cluster: row.cluster,
            cluster_name: inputs.cluster_name,
        };
        let intent = hpa_range_intent(&scope, hpa, min, max)
            .ok_or_else(|| skipped(object.clone(), "the object name is not valid"))?;
        if !hpa_range_warnings(hpa, min, max).is_empty() {
            moved.set(moved.get() + 1);
        }
        Ok(BatchItem {
            object,
            label: intent.label,
            request: intent.request,
        })
    })?;
    let moved = moved.get();
    let warnings = if moved == 0 {
        Vec::new()
    } else {
        vec![
            format!(
                "{moved} of {} will scale their workload at once (their replicas are outside {min}\u{2013}{max})",
                plan.items.len()
            )
            .into(),
        ]
    };
    let title = format!("of {} hpas to {min}\u{2013}{max}", plan.items.len());
    Ok(edit_batch(
        ResourceAction::EditHpaRange,
        inputs.cluster_name,
        plan,
        "Set limits",
        title,
        warnings,
    ))
}

// ---- PVC Expand ----

/// Shown under the field when the text is no size the API accepts.
pub(crate) const NO_SIZE: &str = "Enter a size such as 150Gi";
/// The irreversibility warning of every Expand: the API forbids a smaller request afterwards.
pub(crate) const CANNOT_SHRINK: &str = "A volume cannot shrink; this cannot be undone";
const ONLY_BOUND: &str = "Only a bound claim can be expanded";
const BEING_DELETED: &str = "The claim is being deleted";
const FILE_SYSTEM_RESIZE_PENDING: &str = "FileSystemResizePending";

/// What the Expand form does with the typed size.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum StorageInput {
    /// The field is empty: the button is off, nothing is said.
    Incomplete,
    /// The text breaks a rule; the reason goes under the field.
    Refused(String),
    /// The trimmed size the intent sends.
    Set(String),
}

/// `floor` is the size the claim has now, `None` for several rows, which share none: then any
/// positive size passes the form and each row is checked when the batch is built.
pub(crate) fn storage_input(text: &str, floor: Option<&str>) -> StorageInput {
    let text = text.trim();
    if text.is_empty() {
        return StorageInput::Incomplete;
    }
    let Some(size) = ByteAmount::parse(text).filter(|size| size.bytes() > 0) else {
        return StorageInput::Refused(NO_SIZE.to_owned());
    };
    if let Some(floor) = floor
        && ByteAmount::parse(floor).is_some_and(|current| size <= current)
    {
        return StorageInput::Refused(format!("Must be larger than {floor}"));
    }
    StorageInput::Set(text.to_owned())
}

/// The size a new request must exceed: the larger of what the claim requests and what it has been
/// given, as written. The API refuses less than the previous request, and a request above the
/// capacity is a resize still under way.
pub(crate) fn claim_floor(claim: &PersistentVolumeClaimSummary) -> Option<&str> {
    [claim.requested.as_deref(), claim.capacity.as_deref()]
        .into_iter()
        .flatten()
        .filter_map(|text| ByteAmount::parse(text).map(|size| (size, text)))
        .max_by_key(|(size, _)| *size)
        .map(|(_, text)| text)
}

/// `Now 100Gi · class gp3`, the muted line under the field.
pub(crate) fn claim_state_text(claim: &PersistentVolumeClaimSummary) -> String {
    let size = claim_floor(claim).unwrap_or("unknown size");
    let class = claim.storage_class.as_deref().unwrap_or("none");
    format!("Now {size} \u{b7} class {class}")
}

/// Why the claim cannot be expanded now, `None` when it can. `classes` is the StorageClasses list
/// when it is loaded, else empty: then the class is not checked and the dry-run is the backstop
/// (the admission plugin refuses a class without expansion).
pub(crate) fn claim_block(
    claim: &PersistentVolumeClaimSummary,
    classes: &[&StorageClassSummary],
) -> Option<SharedString> {
    if claim.phase != "Bound" {
        return Some(ONLY_BOUND.into());
    }
    if claim.is_terminating {
        return Some(BEING_DELETED.into());
    }
    let class = claim.storage_class.as_deref()?;
    let known = classes.iter().find(|known| known.name == class)?;
    (!known.allows_expansion)
        .then(|| format!("Storage class {class} does not allow expansion").into())
}

/// A resize to a size above the capacity that is already requested.
fn is_resize_in_progress(claim: &PersistentVolumeClaimSummary) -> bool {
    let sizes = claim
        .requested
        .as_deref()
        .zip(claim.capacity.as_deref())
        .and_then(|(requested, capacity)| {
            Some((ByteAmount::parse(requested)?, ByteAmount::parse(capacity)?))
        });
    sizes.is_some_and(|(requested, capacity)| requested > capacity)
}

fn is_file_system_resize_pending(claim: &PersistentVolumeClaimSummary) -> bool {
    claim
        .conditions
        .iter()
        .any(|condition| condition.name == FILE_SYSTEM_RESIZE_PENDING && condition.is_true)
}

/// The context lines of an Expand: it cannot be undone, and what is still under way.
pub(crate) fn expand_warnings(claim: &PersistentVolumeClaimSummary) -> Vec<SharedString> {
    let mut warnings: Vec<SharedString> = vec![CANNOT_SHRINK.into()];
    if is_resize_in_progress(claim)
        && let Some(requested) = &claim.requested
    {
        warnings.push(format!("A resize to {requested} is already in progress").into());
    }
    if is_file_system_resize_pending(claim) {
        warnings.push("The file system grows when a pod mounts the claim".into());
    }
    warnings
}

/// The intent of growing `claim` to `storage`; `None` when the name is not valid or the size is not
/// larger than the claim has now.
pub(crate) fn expand_intent(
    scope: &WorkloadScope<'_>,
    claim: &PersistentVolumeClaimSummary,
    storage: &str,
) -> Option<WriteIntent> {
    let StorageInput::Set(storage) = storage_input(storage, claim_floor(claim)) else {
        return None;
    };
    let target = ObjectRef::new(
        ObjectKind::PersistentVolumeClaim,
        Some(claim.namespace.clone()),
        claim.name.clone(),
    )?;
    let request = WriteRequest::new(
        target,
        WriteOperation::ExpandClaim {
            storage: storage.clone(),
        },
    )?;
    let action = ResourceAction::ExpandClaim;
    let label = match claim_floor(claim) {
        Some(from) => format!("Expand claim {} from {from} to {storage}", claim.name),
        None => format!("Expand claim {} to {storage}", claim.name),
    };
    Some(WriteIntent {
        cluster: scope.cluster.clone(),
        cluster_name: scope.cluster_name.to_owned().into(),
        action,
        label: label.into(),
        button: "Expand".into(),
        request,
        risk: action_risk(action),
        expected_name: None,
        warnings: expand_warnings(claim),
    })
}

/// The batch of one size over the ticked claims. A claim the state refuses, or that has that size
/// or more already, becomes a skipped line. `Err` is the reason the batch cannot open.
pub(crate) fn bulk_expand_intent(
    inputs: &BulkInputs<'_>,
    storage: &str,
    classes: &[&StorageClassSummary],
) -> Result<BatchIntent, SharedString> {
    let storage = match storage_input(storage, None) {
        StorageInput::Set(storage) => storage,
        StorageInput::Refused(reason) => return Err(reason.into()),
        StorageInput::Incomplete => return Err(NO_SIZE.into()),
    };
    let (in_progress, file_system) = (Cell::new(0_usize), Cell::new(0_usize));
    let plan = batch_plan(inputs.rows, |row| {
        let KindObject::PersistentVolumeClaim(claim) = row.object else {
            return Err(skipped("object".into(), "is not a claim"));
        };
        let object = namespaced_text(&claim.namespace, &claim.name);
        if let Some(reason) = claim_block(claim, classes) {
            return Err(skipped(object, reason));
        }
        let scope = WorkloadScope {
            cluster: row.cluster,
            cluster_name: inputs.cluster_name,
        };
        let Some(intent) = expand_intent(&scope, claim, &storage) else {
            let reason = match claim_floor(claim) {
                Some(floor) => format!("already {floor} or more"),
                None => "the object name is not valid".to_owned(),
            };
            return Err(skipped(object, reason));
        };
        if is_resize_in_progress(claim) {
            in_progress.set(in_progress.get() + 1);
        }
        if is_file_system_resize_pending(claim) {
            file_system.set(file_system.get() + 1);
        }
        Ok(BatchItem {
            object,
            label: intent.label,
            request: intent.request,
        })
    })?;
    let mut warnings: Vec<SharedString> = vec![CANNOT_SHRINK.into()];
    if in_progress.get() > 0 {
        warnings.push(format!("{} have a resize in progress", in_progress.get()).into());
    }
    if file_system.get() > 0 {
        warnings.push(
            format!(
                "{} wait for a pod to mount them before the file system grows",
                file_system.get()
            )
            .into(),
        );
    }
    let title = format!("{} claims to {storage}", plan.items.len());
    Ok(edit_batch(
        ResourceAction::ExpandClaim,
        inputs.cluster_name,
        plan,
        "Expand",
        title,
        warnings,
    ))
}

// ---- Set as default storage class ----

const ALREADY_DEFAULT: &str = "Already the default";
const TICK_ONE_CLASS: &str = "Tick one storage class";

/// What a Set default batch adds: the class it makes the default, and what a run that stops halfway
/// leaves behind.
#[derive(Clone, Debug)]
pub(crate) struct DefaultClassExtras {
    pub(crate) target: String,
    /// Whether the plan sets the target. A Retry does not: the target is the default already.
    sets_target: bool,
    /// The text of two classes marked default, for the first class the plan unsets.
    two_defaults: Option<String>,
}

impl DefaultClassExtras {
    /// The class the action started from, as the Retry names it.
    pub(crate) fn subject(&self, cluster: &ClusterRef) -> ClusterObject {
        let key = ResourceKey::Kind {
            kind: ResourceKind::StorageClasses,
            namespace: None,
            name: self.target.clone(),
        };
        ClusterObject::new(cluster.clone(), key)
    }

    /// The state a stopped run left behind, when it is two classes marked default: the target is
    /// the default by then (the set went through, or the plan had none), and an unset did not.
    pub(crate) fn state_left(&self, results: &[ItemProgress]) -> Option<String> {
        let is_target_default =
            !self.sets_target || results.first().is_some_and(ItemProgress::is_settled);
        if is_target_default {
            self.two_defaults.clone()
        } else {
            None
        }
    }
}

/// Why `class` cannot be made the default, `None` when it can.
pub(crate) fn class_block(class: &StorageClassSummary) -> Option<SharedString> {
    class.is_default.then(|| ALREADY_DEFAULT.into())
}

/// What a partial run leaves behind: `target` is marked default, and `old` still is. The API server
/// uses the newer of the two by `creationTimestamp` until `old` is unset (Kubernetes 1.26+).
pub(crate) fn two_defaults_text(target: &StorageClassSummary, old: &StorageClassSummary) -> String {
    let newer = if old.created_at > target.created_at {
        old
    } else {
        target
    };
    format!(
        "Both {} and {} are marked default; the cluster uses the newer one ({}) until {} is unset",
        target.name, old.name, newer.name, old.name
    )
}

/// One class set or unset as the default: its line in the list of the dialog and its audit label.
fn default_item(class: &StorageClassSummary, is_default: bool) -> Option<BatchItem> {
    let target = ObjectRef::new(ObjectKind::StorageClass, None, class.name.clone())?;
    let request = WriteRequest::new(
        target,
        WriteOperation::SetDefaultStorageClass { is_default },
    )?;
    let (object, label) = if is_default {
        (
            format!("{} \u{b7} becomes the default", class.name),
            format!("Set {} as the default storage class", class.name),
        )
    } else {
        (
            format!("{} \u{b7} stops being the default", class.name),
            format!("Unset {} as the default storage class", class.name),
        )
    };
    Some(BatchItem {
        object: object.into(),
        label: label.into(),
        request,
    })
}

/// The two-object write of Set default: one `Batch` that sets `target` first and then unsets every
/// other default, so there is never a moment without a default. `classes` is the list on screen. A
/// target that is the default already (a Retry after a partial run) is not set again. `None` when
/// there is nothing to change or a name is not valid.
pub(crate) fn default_class_intent(
    scope: &WorkloadScope<'_>,
    target: &StorageClassSummary,
    classes: &[&StorageClassSummary],
) -> Option<BatchIntent> {
    let mut others: Vec<&StorageClassSummary> = classes
        .iter()
        .copied()
        .filter(|class| class.is_default && class.name != target.name)
        .collect();
    others.sort_by(|a, b| a.name.cmp(&b.name));
    let mut items = Vec::new();
    if !target.is_default {
        items.push(default_item(target, true)?);
    }
    for old in &others {
        items.push(default_item(old, false)?);
    }
    if items.is_empty() {
        return None;
    }
    let mut warnings: Vec<SharedString> = Vec::new();
    if !target.is_default {
        warnings.push(
            format!(
                "New claims without a class will use {}; existing claims keep their class",
                target.name
            )
            .into(),
        );
    }
    for old in &others {
        warnings.push(format!("{} stops being the default", old.name).into());
    }
    // A target that is the default already has a partial run behind it: say which one is in use.
    let two_defaults = others.first().map(|old| two_defaults_text(target, old));
    if target.is_default {
        warnings.extend(
            others
                .iter()
                .map(|old| SharedString::from(two_defaults_text(target, old))),
        );
    }
    let action = ResourceAction::SetDefaultStorageClass;
    Some(BatchIntent {
        cluster: scope.cluster.clone(),
        cluster_name: scope.cluster_name.to_owned().into(),
        action,
        label: format!("Make {} the default storage class", target.name).into(),
        verb: "Set default".into(),
        button: "Set default".into(),
        risk: action_risk(action),
        warnings,
        expected_name: None,
        plan: BatchPlan {
            cluster: scope.cluster.clone(),
            items,
            skipped: Vec::new(),
            extras: BatchExtras::DefaultClass(DefaultClassExtras {
                target: target.name.clone(),
                sets_target: !target.is_default,
                two_defaults,
            }),
            on_failure: BatchFailure::Stop,
        },
    })
}

/// The Set default of the selection bar: exactly one ticked class, planned like the menu item.
pub(crate) fn bulk_default_class_intent(
    inputs: &BulkInputs<'_>,
    classes: &[&StorageClassSummary],
) -> Result<BatchIntent, SharedString> {
    let [row] = inputs.rows else {
        return Err(TICK_ONE_CLASS.into());
    };
    let KindObject::StorageClass(target) = row.object else {
        return Err("Not a storage class".into());
    };
    if let Some(reason) = class_block(target) {
        return Err(reason);
    }
    let scope = WorkloadScope {
        cluster: row.cluster,
        cluster_name: inputs.cluster_name,
    };
    default_class_intent(&scope, target, classes)
        .ok_or_else(|| SharedString::from("The object name is not valid"))
}

#[cfg(test)]
#[path = "resource_edits_tests.rs"]
pub(crate) mod resource_edits_tests;
