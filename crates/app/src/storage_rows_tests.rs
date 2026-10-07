use cluster::ClaimRef;

use super::*;
use crate::resource_kind::ResourceKind;

fn claim() -> PersistentVolumeClaimSummary {
    PersistentVolumeClaimSummary {
        namespace: "shop".to_owned(),
        name: "data".to_owned(),
        created_at: None,
        labels: vec!["app=db".to_owned()],
        phase: "Bound".to_owned(),
        is_terminating: false,
        volume: Some("pv-1".to_owned()),
        capacity: Some("10Gi".to_owned()),
        requested: Some("8Gi".to_owned()),
        access_modes: vec!["ReadWriteOnce".to_owned()],
        storage_class: Some("gp3".to_owned()),
        volume_mode: Some("Filesystem".to_owned()),
        conditions: Vec::new(),
        class_allows_expansion: None,
    }
}

fn volume() -> PersistentVolumeSummary {
    PersistentVolumeSummary {
        name: "pv-1".to_owned(),
        created_at: None,
        labels: Vec::new(),
        capacity: Some("10Gi".to_owned()),
        access_modes: vec!["ReadWriteOnce".to_owned(), "ReadOnlyMany".to_owned()],
        reclaim_policy: "Delete".to_owned(),
        phase: "Bound".to_owned(),
        is_terminating: false,
        claim: Some(ClaimRef {
            namespace: "shop".to_owned(),
            name: "data".to_owned(),
        }),
        storage_class: Some("gp3".to_owned()),
        volume_mode: Some("Filesystem".to_owned()),
        backend: VolumeBackend::Csi {
            driver: "ebs.csi.aws.com".to_owned(),
            volume_handle: "vol-0abc".to_owned(),
            fs_type: Some("ext4".to_owned()),
        },
        node_affinity: vec!["topology.kubernetes.io/zone in (eu-central-1a)".to_owned()],
        mount_options: Vec::new(),
        reason: None,
        message: None,
    }
}

fn condition(name: &str) -> WorkloadCondition {
    WorkloadCondition {
        name: name.to_owned(),
        is_true: true,
        reason: None,
        message: None,
        last_transition: None,
    }
}

fn labeled(text: &str, tone: StatusTone) -> StatusLabel {
    StatusLabel {
        text: text.to_owned().into(),
        tone,
    }
}

#[test]
fn pvc_row_cells_match_column_count() {
    let row = persistent_volume_claim_row(&claim());
    assert_eq!(
        row.cells.len(),
        ResourceKind::PersistentVolumeClaims.columns().len()
    );
    assert_eq!(row.namespace.as_deref(), Some("shop"));
    assert_eq!(
        row.cells[0],
        KindCell::Toned(labeled("Bound", StatusTone::Ok))
    );
    // The kubelet join fills Used.
    assert_eq!(row.cells[2], KindCell::Absent);
    assert_eq!(row.cells[3], KindCell::Mono("RWO".into()));
    assert_eq!(row.cells[4], KindCell::Text("gp3".into()));
}

#[test]
fn pv_row_cells_match_column_count() {
    let row = persistent_volume_row(&volume());
    assert_eq!(
        row.cells.len(),
        ResourceKind::PersistentVolumes.columns().len()
    );
    assert_eq!(row.namespace, None);
    assert_eq!(row.cells[5], KindCell::Text("Delete".into()));
    assert_eq!(
        row.cells[0],
        KindCell::Toned(labeled("Bound", StatusTone::Ok))
    );
}

#[test]
fn phase_label_tones() {
    let tone = |phase: &str| phase_label(phase, false).tone;
    assert_eq!(tone("Bound"), StatusTone::Ok);
    assert_eq!(tone("Available"), StatusTone::Ok);
    assert_eq!(tone("Pending"), StatusTone::Warn);
    assert_eq!(tone("Released"), StatusTone::Warn);
    assert_eq!(tone("Lost"), StatusTone::Bad);
    assert_eq!(tone("Failed"), StatusTone::Bad);
    assert_eq!(tone("Something"), StatusTone::Warn);
    assert_eq!(
        phase_label("Bound", true),
        labeled("Terminating", StatusTone::Info)
    );
}

#[test]
fn capacity_falls_back_to_requested() {
    let mut pending = claim();
    pending.capacity = None;
    let row = persistent_volume_claim_row(&pending);
    // An unbound claim shows its request muted.
    assert_eq!(
        row.cells[1],
        KindCell::Quantity {
            text: "8Gi".into(),
            value: 8 * 1024 * 1024 * 1024,
            tone: Some(StatusTone::Done),
        }
    );
}

#[test]
fn bound_capacity_is_not_muted() {
    let row = persistent_volume_claim_row(&claim());
    assert_eq!(
        row.cells[1],
        KindCell::Quantity {
            text: "10Gi".into(),
            value: 10 * 1024 * 1024 * 1024,
            tone: None,
        }
    );
}

#[test]
fn capacity_is_absent_without_capacity_and_request() {
    let mut empty = claim();
    empty.capacity = None;
    empty.requested = None;
    assert_eq!(
        persistent_volume_claim_row(&empty).cells[1],
        KindCell::Absent
    );
}

#[test]
fn an_unmounted_bound_claim_is_an_orphan_and_nothing_else_is() {
    let bound = claim();
    let orphan = mounted_by_cell(&bound, Some(&[]));
    assert!(is_orphan_cell(&orphan));
    assert_eq!(
        orphan,
        KindCell::Toned(StatusLabel {
            text: "Orphan".into(),
            tone: StatusTone::Warn
        })
    );
    assert_eq!(ORPHAN_NOTE, "Orphan \u{b7} not mounted by any pod");
    let mut pending = claim();
    pending.phase = "Pending".to_owned();
    assert_eq!(mounted_by_cell(&pending, Some(&[])), KindCell::Absent);
    let mut going = claim();
    going.is_terminating = true;
    assert_eq!(mounted_by_cell(&going, Some(&[])), KindCell::Absent);
    assert_eq!(mounted_by_cell(&bound, None), KindCell::Absent);
    assert_eq!(
        mounted_by_cell(&bound, Some(&["web-0"])),
        KindCell::Text("web-0".into())
    );
    assert!(!is_orphan_cell(&KindCell::Text("Orphan".into())));
}

#[test]
fn unreadable_size_stays_text() {
    assert_eq!(size_cell(Some("lots"), None), KindCell::Text("lots".into()));
}

fn modes(names: &[&str]) -> Vec<String> {
    names.iter().map(|&name| name.to_owned()).collect()
}

#[test]
fn access_modes_abbreviate() {
    assert_eq!(
        modes_text(&modes(&[
            "ReadWriteOnce",
            "ReadOnlyMany",
            "ReadWriteMany",
            "ReadWriteOncePod",
            "Custom",
        ])),
        "RWO,ROX,RWX,RWOP,Custom"
    );
}

#[test]
fn access_modes_detail_spells_out_known_modes() {
    assert_eq!(
        modes_detail(&modes(&["ReadWriteOnce", "Custom"])),
        "ReadWriteOnce (RWO), Custom"
    );
}

#[test]
fn no_access_modes_read_absent() {
    let mut no_modes = claim();
    no_modes.access_modes.clear();
    assert_eq!(
        persistent_volume_claim_row(&no_modes).cells[3],
        KindCell::Absent
    );
}

#[test]
fn resizing_wins_over_resize_pending() {
    let mut resizing = claim();
    resizing.conditions = vec![condition("Resizing"), condition("FileSystemResizePending")];
    assert_eq!(
        persistent_volume_claim_row(&resizing).status,
        labeled("Resizing", StatusTone::Info)
    );
}

#[test]
fn resize_pending_status_is_short_and_the_drawer_adds_the_action() {
    let mut pending = claim();
    pending.conditions = vec![condition("FileSystemResizePending")];
    let row = persistent_volume_claim_row(&pending);
    assert_eq!(row.status, labeled("Resize pending", StatusTone::Info));
    assert_eq!(
        row.cells[0],
        KindCell::Toned(labeled("Resize pending", StatusTone::Info))
    );
    assert_eq!(
        claim_section_row(&row, "Status"),
        &DetailRow::field(
            "Status",
            KindCell::Toned(labeled("Resize pending; restart the pod", StatusTone::Info))
        )
    );
}

#[test]
fn resize_conditions_only_apply_to_bound_claims() {
    let mut resizing = claim();
    resizing.conditions = vec![condition("Resizing")];
    resizing.phase = "Pending".to_owned();
    assert_eq!(
        persistent_volume_claim_row(&resizing).status,
        labeled("Pending", StatusTone::Warn)
    );
}

#[test]
fn true_resize_conditions_read_info() {
    let mut resizing = claim();
    resizing.conditions = vec![condition("Resizing"), condition("FileSystemResizePending")];
    let row = persistent_volume_claim_row(&resizing);
    let conditions = row.section("Conditions").expect("section");
    for detail in &conditions.rows {
        let DetailRow::Condition { status, .. } = detail else {
            panic!("a condition row");
        };
        assert_eq!(status, &labeled("True", StatusTone::Info));
    }
}

#[test]
fn other_true_conditions_read_ok() {
    let mut other = claim();
    other.conditions = vec![condition("Other")];
    let row = persistent_volume_claim_row(&other);
    let conditions = row.section("Conditions").expect("section");
    assert_eq!(
        conditions.rows,
        [DetailRow::Condition {
            name: "Other".into(),
            status: labeled("True", StatusTone::Ok),
            since: None,
            message: None,
        }]
    );
}

#[test]
fn claim_without_conditions_has_no_conditions_section() {
    assert!(
        persistent_volume_claim_row(&claim())
            .section("Conditions")
            .is_none()
    );
}

fn claim_section_row<'a>(row: &'a KindRow, label: &str) -> &'a DetailRow {
    row.section("Claim")
        .and_then(|section| {
            section.rows.iter().find(|detail| match detail {
                DetailRow::Field { label: found, .. } | DetailRow::Link { label: found, .. } => {
                    found.as_ref() == label
                }
                _ => false,
            })
        })
        .expect("row exists")
}

#[test]
fn pvc_volume_is_a_link() {
    let row = persistent_volume_claim_row(&claim());
    assert_eq!(
        claim_section_row(&row, "Volume"),
        &DetailRow::Link {
            label: "Volume".into(),
            text: "pv-1".into(),
            target: ResourceKey::Kind {
                kind: ResourceKind::PersistentVolumes,
                namespace: None,
                name: "pv-1".to_owned(),
            },
        }
    );
    let mut unbound = claim();
    unbound.volume = None;
    let row = persistent_volume_claim_row(&unbound);
    assert_eq!(
        claim_section_row(&row, "Volume"),
        &DetailRow::field("Volume", KindCell::Absent)
    );
}

#[test]
fn pvc_and_pv_class_link_to_storage_class() {
    let link = DetailRow::Link {
        label: "Class".into(),
        text: "gp3".into(),
        target: ResourceKey::Kind {
            kind: ResourceKind::StorageClasses,
            namespace: None,
            name: "gp3".to_owned(),
        },
    };
    let row = persistent_volume_claim_row(&claim());
    assert_eq!(claim_section_row(&row, "Class"), &link);
    let row = persistent_volume_row(&volume());
    let class = row
        .section("Volume")
        .and_then(|section| {
            section.rows.iter().find(|detail| {
                matches!(detail, DetailRow::Link { label, .. } if label.as_ref() == "Class")
            })
        })
        .expect("class row");
    assert_eq!(class, &link);
}

#[test]
fn pv_claim_cell_is_qualified() {
    let row = persistent_volume_row(&volume());
    assert_eq!(
        row.cells[1],
        KindCell::Qualified {
            prefix: Some("shop".into()),
            text: "data".into(),
        }
    );
    let mut available = volume();
    available.claim = None;
    assert_eq!(persistent_volume_row(&available).cells[1], KindCell::Absent);
}

#[test]
fn pv_source_rows_per_backend() {
    let rows = |backend: VolumeBackend| {
        let mut source = volume();
        source.backend = backend;
        persistent_volume_row(&source)
            .section("Source")
            .expect("source section")
            .rows
            .clone()
    };
    let field = |label: &str, value: KindCell| DetailRow::field(label.to_owned(), value);
    assert_eq!(
        rows(VolumeBackend::Csi {
            driver: "d".to_owned(),
            volume_handle: "h".to_owned(),
            fs_type: None,
        }),
        [
            field("Driver", KindCell::Mono("d".into())),
            field("Volume handle", KindCell::Mono("h".into())),
        ]
    );
    assert_eq!(
        rows(VolumeBackend::Nfs {
            server: "10.0.0.5".to_owned(),
            path: "/exports".to_owned(),
        }),
        [
            field("Server", KindCell::Mono("10.0.0.5".into())),
            field("Path", KindCell::Mono("/exports".into())),
        ]
    );
    assert_eq!(
        rows(VolumeBackend::HostPath {
            path: "/mnt".to_owned()
        }),
        [
            field("Type", KindCell::Text("HostPath".into())),
            field("Path", KindCell::Mono("/mnt".into())),
        ]
    );
    assert_eq!(
        rows(VolumeBackend::Other { kind: "rbd" }),
        [field("Type", KindCell::Text("rbd".into()))]
    );
}

#[test]
fn pv_node_affinity_and_mount_options_only_when_present() {
    let row = persistent_volume_row(&volume());
    assert_eq!(
        row.section("Node affinity")
            .map(|section| section.rows.clone()),
        Some(vec![DetailRow::Chips(vec![
            "topology.kubernetes.io/zone in (eu-central-1a)".into()
        ])])
    );
    let mut bare = volume();
    bare.node_affinity.clear();
    let row = persistent_volume_row(&bare);
    assert!(row.section("Node affinity").is_none());
    let volume_section = row.section("Volume").expect("section");
    assert!(
        !volume_section
            .rows
            .iter()
            .any(|detail| matches!(detail, DetailRow::Field { label, .. } if label.as_ref() == "Mount options"))
    );
    bare.mount_options = vec!["hard".to_owned(), "nfsvers=4.1".to_owned()];
    let row = persistent_volume_row(&bare);
    let volume_section = row.section("Volume").expect("section");
    assert!(volume_section.rows.contains(&DetailRow::field(
        "Mount options",
        KindCell::Mono("hard, nfsvers=4.1".into())
    )));
}

// ---- StorageClasses ----

fn storage_class() -> StorageClassSummary {
    StorageClassSummary {
        name: "gp3".to_owned(),
        created_at: None,
        labels: vec!["tier=fast".to_owned()],
        provisioner: "ebs.csi.aws.com".to_owned(),
        reclaim_policy: "Delete".to_owned(),
        binding_mode: "WaitForFirstConsumer".to_owned(),
        allows_expansion: true,
        is_default: true,
        parameters: vec![
            StorageParameter {
                key: "type".to_owned(),
                value: Some("gp3".to_owned()),
            },
            StorageParameter {
                key: "adminPassword".to_owned(),
                value: None,
            },
        ],
        mount_options: Vec::new(),
    }
}

#[test]
fn storage_class_row_cells_match_column_count() {
    let row = storage_class_row(&storage_class());
    assert_eq!(
        row.cells.len(),
        ResourceKind::StorageClasses.columns().len()
    );
}

#[test]
fn storage_class_cells_show_provisioner_policies_and_expansion() {
    let row = storage_class_row(&storage_class());
    assert_eq!(row.namespace, None);
    assert_eq!(row.cells[0], KindCell::Mono("ebs.csi.aws.com".into()));
    assert_eq!(row.cells[2], KindCell::Text("WaitForFirstConsumer".into()));
    assert_eq!(row.cells[3], KindCell::Text("Yes".into()));
}

#[test]
fn storage_class_pvs_cell_waits_for_the_join() {
    let row = storage_class_row(&storage_class());
    assert_eq!(row.cells[5], KindCell::Absent);
}

#[test]
fn default_class_shows_star() {
    let default = storage_class_row(&storage_class());
    assert_eq!(default.cells[4], KindCell::Text("★".into()));
    let mut other = storage_class();
    other.is_default = false;
    assert_eq!(storage_class_row(&other).cells[4], KindCell::Absent);
}

#[test]
fn default_class_status() {
    let default = storage_class_row(&storage_class());
    assert_eq!(default.status, labeled("Default", StatusTone::Ok));
    let mut other = storage_class();
    other.is_default = false;
    assert_eq!(
        storage_class_row(&other).status,
        labeled("Not default", StatusTone::Done)
    );
}

#[test]
fn hidden_parameter_reads_hidden() {
    let row = storage_class_row(&storage_class());
    let parameters = row.section("Parameters").expect("section");
    assert_eq!(
        parameters.rows,
        [
            DetailRow::field("type", KindCell::Mono("gp3".into())),
            DetailRow::field(
                "adminPassword",
                KindCell::Toned(labeled("hidden", StatusTone::Done))
            ),
        ]
    );
}

#[test]
fn class_without_parameters_has_a_note() {
    let mut bare = storage_class();
    bare.parameters.clear();
    let row = storage_class_row(&bare);
    assert_eq!(
        row.section("Parameters")
            .map(|section| section.rows.clone()),
        Some(vec![DetailRow::Note("No parameters".into())])
    );
}

#[test]
fn class_mount_options_only_when_present() {
    let row = storage_class_row(&storage_class());
    let class = row.section("Class").expect("section");
    assert!(!class.rows.iter().any(
        |detail| matches!(detail, DetailRow::Field { label, .. } if label.as_ref() == "Mount options")
    ));
    let mut with_options = storage_class();
    with_options.mount_options = vec!["debug".to_owned(), "noatime".to_owned()];
    let row = storage_class_row(&with_options);
    assert!(
        row.section("Class")
            .expect("section")
            .rows
            .contains(&DetailRow::field(
                "Mount options",
                KindCell::Mono("debug, noatime".into())
            ))
    );
}

#[test]
fn class_drawer_lists_volumes_live() {
    let row = storage_class_row(&storage_class());
    assert_eq!(
        row.section("Volumes").map(|section| section.rows.clone()),
        Some(vec![DetailRow::Live(LiveContent::ClassVolumes)])
    );
}
