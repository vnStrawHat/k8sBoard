//! Row builders for the storage kinds: PersistentVolumeClaims, PersistentVolumes, and
//! StorageClasses.

use cluster::{
    ByteAmount, PersistentVolumeClaimSummary, PersistentVolumeSummary, StorageClassSummary,
    StorageParameter, VolumeBackend, WorkloadCondition,
};

use crate::kind_row::{
    DetailRow, DetailSection, KindCell, KindObject, KindRow, LiveContent, access_mode_short, chips,
};
use crate::status_tone::{StatusLabel, StatusTone};
use crate::table_selection::ResourceKey;
use crate::workload_rows::condition_row_toned;

const BOUND: &str = "Bound";
const RESIZING: &str = "Resizing";
const FILE_SYSTEM_RESIZE_PENDING: &str = "FileSystemResizePending";
/// The drawer adds what to do; the table and the subtitle stay short.
const RESIZE_PENDING: &str = "Resize pending";
const RESIZE_PENDING_DETAIL: &str = "Resize pending; restart the pod";

/// The phase as the status the tables and drawers show. The tones are the app's own: idle
/// states are Ok or Done, because Info and Warn count as unhealthy in the table chips.
pub(crate) fn phase_label(phase: &str, is_terminating: bool) -> StatusLabel {
    let tone = if is_terminating {
        StatusTone::Info
    } else {
        match phase {
            "Bound" | "Available" => StatusTone::Ok,
            "Released" => StatusTone::Done,
            "Lost" | "Failed" => StatusTone::Bad,
            _ => StatusTone::Warn,
        }
    };
    let text = if is_terminating { "Terminating" } else { phase };
    StatusLabel {
        text: text.to_owned().into(),
        tone,
    }
}

/// A quantity as written, sorted by its bytes; text the parser cannot read stays text. `tone`
/// colours a parsed quantity.
fn size_cell(text: Option<&str>, tone: Option<StatusTone>) -> KindCell {
    let Some(text) = text else {
        return KindCell::Absent;
    };
    match ByteAmount::parse(text) {
        Some(bytes) => KindCell::Quantity {
            text: text.to_owned().into(),
            value: bytes.bytes(),
            tone,
        },
        None => KindCell::Text(text.to_owned().into()),
    }
}

/// Short names joined with `,`, such as `RWO,ROX`.
fn modes_text(modes: &[String]) -> String {
    let short: Vec<&str> = modes.iter().map(|mode| access_mode_short(mode)).collect();
    short.join(",")
}

/// `ReadWriteOnce (RWO), ReadOnlyMany (ROX)` for the drawer.
fn modes_detail(modes: &[String]) -> String {
    let detailed: Vec<String> = modes
        .iter()
        .map(|mode| match access_mode_short(mode) {
            short if short == mode => mode.clone(),
            short => format!("{mode} ({short})"),
        })
        .collect();
    detailed.join(", ")
}

fn modes_cell(modes: &[String]) -> KindCell {
    KindCell::mono_or_absent(&modes_text(modes))
}

fn class_cell(class: Option<&str>) -> KindCell {
    KindCell::text_or_absent(class)
}

/// A link to the object when k8sBoard has a screen for its kind, else its name as text.
fn object_row(
    label: &'static str,
    kind: &str,
    namespace: Option<&str>,
    name: Option<&str>,
) -> DetailRow {
    let Some(name) = name else {
        return DetailRow::field(label, KindCell::Absent);
    };
    match ResourceKey::of_object(kind, namespace, name) {
        Some(target) => DetailRow::Link {
            label: label.into(),
            text: name.to_owned().into(),
            target,
        },
        None => DetailRow::field(label, KindCell::Text(name.to_owned().into())),
    }
}

/// The drawer's Status text: a pending resize says what to do about it.
fn detail_status(status: &StatusLabel) -> StatusLabel {
    if status.text.as_ref() != RESIZE_PENDING {
        return status.clone();
    }
    StatusLabel {
        text: RESIZE_PENDING_DETAIL.into(),
        tone: status.tone,
    }
}

/// A resize in progress is transitional, not a healthy state, so a true `Resizing` or
/// `FileSystemResizePending` reads Info instead of Ok.
fn claim_condition_row(condition: &WorkloadCondition) -> DetailRow {
    let is_resize = matches!(
        condition.name.as_str(),
        RESIZING | FILE_SYSTEM_RESIZE_PENDING
    );
    let true_tone = if is_resize {
        StatusTone::Info
    } else {
        StatusTone::Ok
    };
    condition_row_toned(condition, true_tone)
}

fn condition_is_true(conditions: &[WorkloadCondition], name: &str) -> bool {
    conditions
        .iter()
        .any(|condition| condition.name == name && condition.is_true)
}

// ---- PersistentVolumeClaims ----

/// The status before the Used join: the phase, or a transitional resize of a bound claim.
pub(crate) fn claim_status(claim: &PersistentVolumeClaimSummary) -> StatusLabel {
    let status = phase_label(&claim.phase, claim.is_terminating);
    if claim.phase != BOUND || claim.is_terminating {
        return status;
    }
    let text = if condition_is_true(&claim.conditions, RESIZING) {
        "Resizing"
    } else if condition_is_true(&claim.conditions, FILE_SYSTEM_RESIZE_PENDING) {
        RESIZE_PENDING
    } else {
        return status;
    };
    StatusLabel {
        text: text.into(),
        tone: StatusTone::Info,
    }
}

pub(crate) fn persistent_volume_claim_row(claim: &PersistentVolumeClaimSummary) -> KindRow {
    let status = claim_status(claim);
    // An unbound claim has no capacity yet; its request is shown muted.
    let size_cell = match (&claim.capacity, &claim.requested) {
        (Some(capacity), _) => size_cell(Some(capacity), None),
        (None, requested) => size_cell(requested.as_deref(), Some(StatusTone::Done)),
    };
    let mut claim_rows = vec![
        DetailRow::field("Status", KindCell::Toned(detail_status(&status))),
        object_row("Volume", "PersistentVolume", None, claim.volume.as_deref()),
        object_row(
            "Class",
            "StorageClass",
            None,
            claim.storage_class.as_deref(),
        ),
        DetailRow::field(
            "Requested",
            KindCell::text_or_absent(claim.requested.as_deref()),
        ),
        DetailRow::field(
            "Capacity",
            KindCell::text_or_absent(claim.capacity.as_deref()),
        ),
    ];
    claim_rows.push(DetailRow::field(
        "Access modes",
        if claim.access_modes.is_empty() {
            KindCell::Absent
        } else {
            KindCell::Text(modes_detail(&claim.access_modes).into())
        },
    ));
    claim_rows.push(DetailRow::field(
        "Volume mode",
        KindCell::text_or_absent(claim.volume_mode.as_deref()),
    ));
    let mut sections = vec![
        DetailSection {
            title: "Claim",
            rows: claim_rows,
        },
        DetailSection {
            title: "Usage",
            rows: vec![DetailRow::Live(LiveContent::ClaimUsage)],
        },
        DetailSection {
            title: "Mounted by",
            rows: vec![DetailRow::Live(LiveContent::MountedBy)],
        },
    ];
    if !claim.conditions.is_empty() {
        sections.push(DetailSection {
            title: "Conditions",
            rows: claim.conditions.iter().map(claim_condition_row).collect(),
        });
    }
    KindRow {
        namespace: Some(claim.namespace.clone()),
        name: claim.name.clone(),
        created_at: claim.created_at,
        status: status.clone(),
        cells: vec![
            KindCell::Toned(status),
            size_cell,
            // The kubelet join fills Used.
            KindCell::Absent,
            modes_cell(&claim.access_modes),
            class_cell(claim.storage_class.as_deref()),
            KindCell::age(claim.created_at),
        ],
        sections,
        event: None,
        related_pods: None,
        labels: chips(&claim.labels),
        object: KindObject::PersistentVolumeClaim(claim.clone()),
    }
}

// ---- PersistentVolumes ----

pub(crate) fn persistent_volume_row(volume: &PersistentVolumeSummary) -> KindRow {
    let status = phase_label(&volume.phase, volume.is_terminating);
    let claim_cell = match &volume.claim {
        Some(claim) => KindCell::Qualified {
            prefix: Some(claim.namespace.clone().into()),
            text: claim.name.clone().into(),
        },
        None => KindCell::Absent,
    };
    let mut volume_rows = vec![
        DetailRow::field("Status", KindCell::Toned(status.clone())),
        match &volume.claim {
            Some(claim) => object_row(
                "Claim",
                "PersistentVolumeClaim",
                Some(&claim.namespace),
                Some(&claim.name),
            ),
            None => DetailRow::field("Claim", KindCell::Absent),
        },
        DetailRow::field(
            "Reclaim policy",
            KindCell::Text(volume.reclaim_policy.clone().into()),
        ),
        object_row(
            "Class",
            "StorageClass",
            None,
            volume.storage_class.as_deref(),
        ),
        DetailRow::field(
            "Capacity",
            KindCell::text_or_absent(volume.capacity.as_deref()),
        ),
        DetailRow::field(
            "Access modes",
            if volume.access_modes.is_empty() {
                KindCell::Absent
            } else {
                KindCell::Text(modes_detail(&volume.access_modes).into())
            },
        ),
        DetailRow::field(
            "Volume mode",
            KindCell::text_or_absent(volume.volume_mode.as_deref()),
        ),
    ];
    if !volume.mount_options.is_empty() {
        volume_rows.push(DetailRow::field(
            "Mount options",
            KindCell::Mono(volume.mount_options.join(", ").into()),
        ));
    }
    let mut sections = vec![
        DetailSection {
            title: "Volume",
            rows: volume_rows,
        },
        DetailSection {
            title: "Source",
            rows: source_rows(&volume.backend),
        },
    ];
    if !volume.node_affinity.is_empty() {
        sections.push(DetailSection {
            title: "Node affinity",
            rows: vec![DetailRow::Chips(chips(&volume.node_affinity))],
        });
    }
    KindRow {
        namespace: None,
        name: volume.name.clone(),
        created_at: volume.created_at,
        status: status.clone(),
        cells: vec![
            KindCell::Toned(status),
            claim_cell,
            size_cell(volume.capacity.as_deref(), None),
            class_cell(volume.storage_class.as_deref()),
            modes_cell(&volume.access_modes),
            KindCell::Text(volume.reclaim_policy.clone().into()),
            KindCell::age(volume.created_at),
        ],
        sections,
        event: None,
        related_pods: None,
        labels: chips(&volume.labels),
        object: KindObject::PersistentVolume(volume.clone()),
    }
}

fn mono_field(label: &'static str, text: &str) -> DetailRow {
    DetailRow::field(label, KindCell::mono_or_absent(text))
}

fn source_rows(backend: &VolumeBackend) -> Vec<DetailRow> {
    match backend {
        VolumeBackend::Csi {
            driver,
            volume_handle,
            fs_type,
        } => {
            let mut rows = vec![
                mono_field("Driver", driver),
                mono_field("Volume handle", volume_handle),
            ];
            if let Some(fs_type) = fs_type {
                rows.push(DetailRow::field(
                    "FS type",
                    KindCell::Text(fs_type.clone().into()),
                ));
            }
            rows
        }
        VolumeBackend::Nfs { server, path } => {
            vec![mono_field("Server", server), mono_field("Path", path)]
        }
        VolumeBackend::HostPath { path } => vec![
            DetailRow::field("Type", KindCell::Text("HostPath".into())),
            mono_field("Path", path),
        ],
        VolumeBackend::Local { path } => vec![
            DetailRow::field("Type", KindCell::Text("Local".into())),
            mono_field("Path", path),
        ],
        VolumeBackend::Other { kind } => {
            vec![DetailRow::field("Type", KindCell::Text((*kind).into()))]
        }
    }
}

// ---- StorageClasses ----

fn yes_no(is_yes: bool) -> KindCell {
    KindCell::Text(if is_yes { "Yes" } else { "No" }.into())
}

pub(crate) fn storage_class_row(class: &StorageClassSummary) -> KindRow {
    let status = if class.is_default {
        labeled("Default", StatusTone::Ok)
    } else {
        labeled("Not default", StatusTone::Done)
    };
    let mut class_rows = vec![
        mono_field("Provisioner", &class.provisioner),
        DetailRow::field("Default", yes_no(class.is_default)),
        DetailRow::field(
            "Reclaim policy",
            KindCell::Text(class.reclaim_policy.clone().into()),
        ),
        DetailRow::field(
            "Binding mode",
            KindCell::Text(class.binding_mode.clone().into()),
        ),
        DetailRow::field("Volume expansion", yes_no(class.allows_expansion)),
    ];
    if !class.mount_options.is_empty() {
        class_rows.push(DetailRow::field(
            "Mount options",
            KindCell::Mono(class.mount_options.join(", ").into()),
        ));
    }
    let parameter_rows = if class.parameters.is_empty() {
        vec![DetailRow::Note("No parameters".into())]
    } else {
        class.parameters.iter().map(parameter_row).collect()
    };
    KindRow {
        namespace: None,
        name: class.name.clone(),
        created_at: class.created_at,
        status,
        cells: vec![
            KindCell::mono_or_absent(&class.provisioner),
            KindCell::Text(class.reclaim_policy.clone().into()),
            KindCell::Text(class.binding_mode.clone().into()),
            yes_no(class.allows_expansion),
            if class.is_default {
                KindCell::Text("★".into())
            } else {
                KindCell::Absent
            },
            // The PV companion join fills PVs.
            KindCell::Absent,
            KindCell::age(class.created_at),
        ],
        sections: vec![
            DetailSection {
                title: "Class",
                rows: class_rows,
            },
            DetailSection {
                title: "Parameters",
                rows: parameter_rows,
            },
            DetailSection {
                title: "Volumes",
                rows: vec![DetailRow::Live(LiveContent::ClassVolumes)],
            },
        ],
        event: None,
        related_pods: None,
        labels: chips(&class.labels),
        object: KindObject::StorageClass(class.clone()),
    }
}

fn labeled(text: &str, tone: StatusTone) -> StatusLabel {
    StatusLabel {
        text: text.to_owned().into(),
        tone,
    }
}

/// A hidden value (see `is_secret_key`) reads as muted `hidden`, never as an empty string.
fn parameter_row(parameter: &StorageParameter) -> DetailRow {
    let value = match &parameter.value {
        Some(value) => KindCell::Mono(value.clone().into()),
        None => KindCell::Toned(labeled("hidden", StatusTone::Done)),
    };
    DetailRow::field(parameter.key.clone(), value)
}

#[cfg(test)]
#[path = "storage_rows_tests.rs"]
mod storage_rows_tests;
