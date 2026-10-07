use cluster::{ControllerRef, WorkloadCondition, WriteOperation};

use super::*;
use crate::app_shell::batch_write::CheckedRow;
use crate::cluster_registry::ClusterRef;
use crate::workload_actions::workload_actions_tests::test_cluster;
use crate::write_guard::ActionRisk;

pub(crate) fn hpa(name: &str, min: u32, max: u32, current: u32) -> HorizontalPodAutoscalerSummary {
    HorizontalPodAutoscalerSummary {
        namespace: "team-a".to_owned(),
        name: name.to_owned(),
        created_at: None,
        labels: Vec::new(),
        target: ControllerRef {
            kind: "Deployment".to_owned(),
            name: "frontend".to_owned(),
        },
        min_replicas: min,
        max_replicas: max,
        current_replicas: current,
        desired_replicas: current,
        metrics: Vec::new(),
        conditions: Vec::new(),
        last_scaled_at: None,
    }
}

fn intent(hpa: &HorizontalPodAutoscalerSummary, min: u32, max: u32) -> Option<WriteIntent> {
    let cluster = test_cluster();
    let scope = WorkloadScope {
        cluster: &cluster,
        cluster_name: "stg-b",
    };
    hpa_range_intent(&scope, hpa, min, max)
}

fn texts(lines: Vec<SharedString>) -> Vec<String> {
    lines.iter().map(ToString::to_string).collect()
}

#[test]
fn range_input_table() {
    let current = Some((3, 20));
    let cases = [
        ("3", "20", RangeInput::Unchanged),
        ("3", "21", RangeInput::Set { min: 3, max: 21 }),
        ("1", "20", RangeInput::Set { min: 1, max: 20 }),
        ("5", "5", RangeInput::Set { min: 5, max: 5 }),
        ("0", "5", RangeInput::Refused(MIN_BELOW_ONE)),
        ("0", "0", RangeInput::Refused(MIN_BELOW_ONE)),
        ("6", "5", RangeInput::Refused(MIN_ABOVE_MAX)),
        ("", "5", RangeInput::Incomplete),
        ("3", "", RangeInput::Incomplete),
        ("-1", "5", RangeInput::Incomplete),
        ("2.5", "5", RangeInput::Incomplete),
        (
            "1",
            "2147483647",
            RangeInput::Set {
                min: 1,
                max: 2_147_483_647,
            },
        ),
        ("1", "2147483648", RangeInput::Incomplete),
    ];
    for (min, max, expected) in cases {
        assert_eq!(range_input(min, max, current), expected, "{min:?} {max:?}");
    }
}

#[test]
fn several_rows_share_no_range_to_compare() {
    assert_eq!(
        range_input("3", "20", None),
        RangeInput::Set { min: 3, max: 20 }
    );
}

#[test]
fn hpa_label_names_the_range() {
    let intent = intent(&hpa("frontend-hpa", 2, 10, 9), 3, 20).expect("an intent");
    assert_eq!(
        intent.label,
        "Set replicas of hpa frontend-hpa to 3\u{2013}20"
    );
    assert_eq!(intent.button, "Set limits");
    assert_eq!(intent.action, ResourceAction::EditHpaRange);
    assert_eq!(intent.risk, ActionRisk::Change);
    assert_eq!(
        intent.request.operation(),
        &WriteOperation::SetHpaReplicaRange {
            min: 3,
            max: 20,
            previous_min: 2,
            previous_max: 10,
        }
    );
    assert_eq!(intent.request.target().namespace(), Some("team-a"));
}

#[test]
fn hpa_intent_refuses_an_invalid_pair() {
    let hpa = hpa("frontend-hpa", 2, 10, 9);
    assert!(intent(&hpa, 0, 5).is_none());
    assert!(intent(&hpa, 6, 5).is_none());
}

#[test]
fn hpa_warns_when_max_below_current() {
    let hpa = hpa("frontend-hpa", 2, 10, 9);
    assert_eq!(
        texts(hpa_range_warnings(&hpa, 3, 5)),
        ["The HPA will scale deployment/frontend down from 9 to 5"]
    );
}

#[test]
fn hpa_warns_when_min_above_current() {
    let hpa = hpa("frontend-hpa", 1, 10, 2);
    assert_eq!(
        texts(hpa_range_warnings(&hpa, 4, 10)),
        ["The HPA will scale deployment/frontend up from 2 to 4"]
    );
}

#[test]
fn hpa_no_warning_inside_range() {
    let hpa = hpa("frontend-hpa", 1, 10, 5);
    assert!(hpa_range_warnings(&hpa, 5, 5).is_empty());
    assert!(hpa_range_warnings(&hpa, 1, 20).is_empty());
}

#[test]
fn hpa_state_line_names_the_current_replicas() {
    assert_eq!(
        hpa_state_text(&hpa("frontend-hpa", 1, 10, 9)),
        "Now 9 replicas"
    );
}

fn objects(specs: &[(&str, u32, u32, u32)]) -> Vec<KindObject> {
    specs
        .iter()
        .map(|(name, min, max, current)| {
            KindObject::HorizontalPodAutoscaler(hpa(name, *min, *max, *current))
        })
        .collect()
}

fn bulk_range(
    objects: &[KindObject],
    cluster: &ClusterRef,
    min: u32,
    max: u32,
) -> Result<BatchIntent, SharedString> {
    let rows: Vec<CheckedRow<'_>> = objects
        .iter()
        .map(|object| CheckedRow { cluster, object })
        .collect();
    let inputs = BulkInputs {
        cluster_name: "stg-b",
        rows: &rows,
        now: jiff::Timestamp::now(),
        hpas: &[],
    };
    bulk_hpa_range_intent(&inputs, min, max)
}

#[test]
fn hpa_edit_limits_skips_rows_already_in_range() {
    let cluster = test_cluster();
    let rows = objects(&[("a", 3, 20, 5), ("b", 1, 4, 2), ("c", 2, 8, 3)]);
    let batch = bulk_range(&rows, &cluster, 3, 20).expect("a batch");
    assert_eq!(batch.label, "Set limits of 2 hpas to 3\u{2013}20");
    let names: Vec<&str> = batch
        .plan
        .items
        .iter()
        .map(|item| item.object.as_ref())
        .collect();
    assert_eq!(names, ["team-a/b", "team-a/c"]);
    assert_eq!(batch.plan.skipped.len(), 1);
    assert_eq!(batch.plan.skipped[0].object, "team-a/a");
    assert_eq!(batch.plan.skipped[0].reason, "already 3\u{2013}20");
    assert_eq!(batch.confirm_label(0), "Set limits 2");
    assert_eq!(batch.risk, ActionRisk::Change);
    assert_eq!(batch.action, ResourceAction::EditHpaRange);
}

#[test]
fn bulk_hpa_range_counts_the_rows_it_moves() {
    let cluster = test_cluster();
    // b has 12 replicas, above the new max; c has 2, below the new min; d sits inside.
    let rows = objects(&[("b", 1, 20, 12), ("c", 1, 20, 2), ("d", 1, 20, 6)]);
    let batch = bulk_range(&rows, &cluster, 4, 10).expect("a batch");
    assert_eq!(
        texts(batch.warnings),
        ["2 of 3 will scale their workload at once (their replicas are outside 4\u{2013}10)"]
    );
}

#[test]
fn bulk_hpa_range_with_everything_in_range_is_refused() {
    let cluster = test_cluster();
    let rows = objects(&[("a", 3, 20, 5)]);
    let Err(reason) = bulk_range(&rows, &cluster, 3, 20) else {
        panic!("nothing to change, so no batch");
    };
    assert_eq!(reason, "already 3\u{2013}20");
}

#[test]
fn bulk_hpa_range_refuses_an_invalid_pair() {
    let cluster = test_cluster();
    let rows = objects(&[("a", 3, 20, 5)]);
    assert!(bulk_range(&rows, &cluster, 0, 5).is_err());
    assert!(bulk_range(&rows, &cluster, 9, 5).is_err());
}

#[test]
fn bulk_hpa_range_leaves_other_kinds_alone() {
    let cluster = test_cluster();
    let rows = vec![KindObject::Plain];
    assert!(bulk_range(&rows, &cluster, 1, 5).is_err());
}

// ---- PVC Expand ----

pub(crate) fn claim(name: &str, requested: &str, capacity: &str) -> PersistentVolumeClaimSummary {
    PersistentVolumeClaimSummary {
        namespace: "team-a".to_owned(),
        name: name.to_owned(),
        created_at: None,
        labels: Vec::new(),
        phase: "Bound".to_owned(),
        is_terminating: false,
        volume: Some("pv-1".to_owned()),
        capacity: Some(capacity.to_owned()),
        requested: Some(requested.to_owned()),
        access_modes: vec!["ReadWriteOnce".to_owned()],
        storage_class: Some("gp3".to_owned()),
        volume_mode: Some("Filesystem".to_owned()),
        conditions: Vec::new(),
    }
}

pub(crate) fn class(name: &str, is_default: bool, allows_expansion: bool) -> StorageClassSummary {
    StorageClassSummary {
        name: name.to_owned(),
        created_at: None,
        labels: Vec::new(),
        provisioner: "ebs.csi.aws.com".to_owned(),
        reclaim_policy: "Delete".to_owned(),
        binding_mode: "Immediate".to_owned(),
        allows_expansion,
        is_default,
        parameters: Vec::new(),
        mount_options: Vec::new(),
    }
}

fn pending_file_system() -> WorkloadCondition {
    WorkloadCondition {
        name: "FileSystemResizePending".to_owned(),
        is_true: true,
        reason: None,
        message: None,
        last_transition: None,
    }
}

fn expand(claim: &PersistentVolumeClaimSummary, storage: &str) -> Option<WriteIntent> {
    let cluster = test_cluster();
    let scope = WorkloadScope {
        cluster: &cluster,
        cluster_name: "prod-a",
    };
    expand_intent(&scope, claim, storage)
}

#[test]
fn expand_must_grow() {
    let floor = Some("100Gi");
    let refused = StorageInput::Refused("Must be larger than 100Gi".to_owned());
    assert_eq!(storage_input("100Gi", floor), refused);
    assert_eq!(storage_input("99Gi", floor), refused);
    // The same size in another unit is still no growth.
    assert_eq!(storage_input("102400Mi", floor), refused);
    assert_eq!(
        storage_input("101Gi", floor),
        StorageInput::Set("101Gi".to_owned())
    );
    assert_eq!(
        storage_input("1Ti", floor),
        StorageInput::Set("1Ti".to_owned())
    );
}

#[test]
fn expand_text_is_trimmed_and_parsed_safely() {
    assert_eq!(storage_input("", None), StorageInput::Incomplete);
    assert_eq!(storage_input("   ", None), StorageInput::Incomplete);
    assert_eq!(
        storage_input(" 150Gi ", None),
        StorageInput::Set("150Gi".to_owned())
    );
    for bad in [
        "abc",
        "0",
        "0Gi",
        "-5Gi",
        "5 Gi",
        "1..5Gi",
        "99999999999999999999Ei",
    ] {
        assert_eq!(
            storage_input(bad, Some("100Gi")),
            StorageInput::Refused(NO_SIZE.to_owned()),
            "{bad:?}"
        );
    }
}

#[test]
fn expand_compares_with_the_larger_of_request_and_capacity() {
    // A resize to 150Gi is under way on a volume that is 100Gi: 120Gi would shrink the request.
    let resizing = claim("data", "150Gi", "100Gi");
    assert_eq!(claim_floor(&resizing), Some("150Gi"));
    assert_eq!(
        storage_input("120Gi", claim_floor(&resizing)),
        StorageInput::Refused("Must be larger than 150Gi".to_owned())
    );
    // A request below the capacity still floors at the capacity.
    let odd = claim("data", "80Gi", "100Gi");
    assert_eq!(claim_floor(&odd), Some("100Gi"));
    let mut unbound = claim("data", "100Gi", "100Gi");
    unbound.capacity = None;
    assert_eq!(claim_floor(&unbound), Some("100Gi"));
    unbound.requested = None;
    assert_eq!(claim_floor(&unbound), None);
}

#[test]
fn expand_state_line_names_the_size_and_the_class() {
    assert_eq!(
        claim_state_text(&claim("data", "100Gi", "100Gi")),
        "Now 100Gi \u{b7} class gp3"
    );
    let mut none = claim("data", "100Gi", "100Gi");
    none.storage_class = None;
    assert_eq!(claim_state_text(&none), "Now 100Gi \u{b7} class none");
}

#[test]
fn expand_is_change_with_the_irreversible_warning() {
    let claim = claim("data-kafka-0", "100Gi", "100Gi");
    let intent = expand(&claim, "150Gi").expect("an intent");
    assert_eq!(intent.risk, ActionRisk::Change);
    assert_eq!(
        intent.label,
        "Expand claim data-kafka-0 from 100Gi to 150Gi"
    );
    assert_eq!(intent.button, "Expand");
    assert_eq!(intent.action, ResourceAction::ExpandClaim);
    assert_eq!(texts(intent.warnings), [CANNOT_SHRINK]);
    assert_eq!(
        intent.request.operation(),
        &WriteOperation::ExpandClaim {
            storage: "150Gi".to_owned()
        }
    );
}

#[test]
fn expand_sends_the_trimmed_text() {
    let claim = claim("data", "100Gi", "100Gi");
    let intent = expand(&claim, " 150Gi ").expect("an intent");
    assert_eq!(intent.label, "Expand claim data from 100Gi to 150Gi");
    assert_eq!(
        intent.request.operation(),
        &WriteOperation::ExpandClaim {
            storage: "150Gi".to_owned()
        }
    );
}

#[test]
fn expand_intent_refuses_no_growth_and_bad_sizes() {
    let claim = claim("data", "100Gi", "100Gi");
    for storage in ["100Gi", "50Gi", "", "abc", "0"] {
        assert!(expand(&claim, storage).is_none(), "{storage:?}");
    }
}

#[test]
fn expand_warnings_follow_the_claim_state() {
    let mut claim = claim("data", "100Gi", "100Gi");
    assert_eq!(texts(expand_warnings(&claim)), [CANNOT_SHRINK]);
    claim.requested = Some("150Gi".to_owned());
    assert_eq!(
        texts(expand_warnings(&claim)),
        [CANNOT_SHRINK, "A resize to 150Gi is already in progress"]
    );
    claim.conditions = vec![pending_file_system()];
    assert_eq!(
        texts(expand_warnings(&claim)),
        [
            CANNOT_SHRINK,
            "A resize to 150Gi is already in progress",
            "The file system grows when a pod mounts the claim"
        ]
    );
    // A condition that is false says nothing.
    claim.conditions[0].is_true = false;
    assert_eq!(texts(expand_warnings(&claim)).len(), 2);
}

#[test]
fn claim_block_table() {
    let gp3_no = class("gp3", false, false);
    let other = class("io2", false, true);
    let mut pending = claim("data", "100Gi", "100Gi");
    pending.phase = "Pending".to_owned();
    assert_eq!(
        claim_block(&pending, &[]).as_deref(),
        Some("Only a bound claim can be expanded")
    );
    let mut lost = claim("data", "100Gi", "100Gi");
    lost.phase = "Lost".to_owned();
    assert_eq!(
        claim_block(&lost, &[]).as_deref(),
        Some("Only a bound claim can be expanded")
    );
    let mut terminating = claim("data", "100Gi", "100Gi");
    terminating.is_terminating = true;
    assert_eq!(
        claim_block(&terminating, &[]).as_deref(),
        Some("The claim is being deleted")
    );
    let bound = claim("data", "100Gi", "100Gi");
    // The class is checked only when the list is loaded.
    assert_eq!(claim_block(&bound, &[]), None);
    assert_eq!(
        claim_block(&bound, &[&gp3_no]).as_deref(),
        Some("Storage class gp3 does not allow expansion")
    );
    assert_eq!(claim_block(&bound, &[&other]), None);
    assert_eq!(claim_block(&bound, &[&class("gp3", false, true)]), None);
    let mut classless = claim("data", "100Gi", "100Gi");
    classless.storage_class = None;
    assert_eq!(claim_block(&classless, &[&gp3_no]), None);
}

fn claims(specs: &[(&str, &str, &str)]) -> Vec<KindObject> {
    specs
        .iter()
        .map(|(name, requested, capacity)| {
            KindObject::PersistentVolumeClaim(claim(name, requested, capacity))
        })
        .collect()
}

fn bulk_expand_over(
    objects: &[KindObject],
    cluster: &ClusterRef,
    storage: &str,
    classes: &[&StorageClassSummary],
) -> Result<BatchIntent, SharedString> {
    let rows: Vec<CheckedRow<'_>> = objects
        .iter()
        .map(|object| CheckedRow { cluster, object })
        .collect();
    let inputs = BulkInputs {
        cluster_name: "stg-b",
        rows: &rows,
        now: jiff::Timestamp::now(),
        hpas: &[],
    };
    bulk_expand_intent(&inputs, storage, classes)
}

fn bulk_expand(
    objects: &[KindObject],
    cluster: &ClusterRef,
    storage: &str,
) -> Result<BatchIntent, SharedString> {
    bulk_expand_over(objects, cluster, storage, &[])
}

#[test]
fn bulk_expand_skips_claims_already_large_enough() {
    let cluster = test_cluster();
    let rows = claims(&[
        ("a", "100Gi", "100Gi"),
        ("b", "300Gi", "300Gi"),
        ("c", "200Gi", "200Gi"),
    ]);
    let batch = bulk_expand(&rows, &cluster, "200Gi").expect("a batch");
    let names: Vec<&str> = batch
        .plan
        .items
        .iter()
        .map(|item| item.object.as_ref())
        .collect();
    assert_eq!(names, ["team-a/a"]);
    let skips: Vec<(&str, &str)> = batch
        .plan
        .skipped
        .iter()
        .map(|skip| (skip.object.as_ref(), skip.reason.as_ref()))
        .collect();
    assert_eq!(
        skips,
        [
            ("team-a/b", "already 300Gi or more"),
            ("team-a/c", "already 200Gi or more")
        ]
    );
}

#[test]
fn bulk_expand_names_the_count_and_warns_once() {
    let cluster = test_cluster();
    let mut resizing = claim("b", "150Gi", "100Gi");
    resizing.conditions = vec![pending_file_system()];
    let rows = vec![
        KindObject::PersistentVolumeClaim(claim("a", "100Gi", "100Gi")),
        KindObject::PersistentVolumeClaim(resizing),
    ];
    let batch = bulk_expand(&rows, &cluster, "200Gi").expect("a batch");
    assert_eq!(batch.label, "Expand 2 claims to 200Gi");
    assert_eq!(batch.verb, "Expand");
    assert_eq!(batch.confirm_label(0), "Expand 2");
    assert_eq!(batch.risk, ActionRisk::Change);
    assert_eq!(
        texts(batch.warnings),
        [
            CANNOT_SHRINK,
            "1 have a resize in progress",
            "1 wait for a pod to mount them before the file system grows"
        ]
    );
}

#[test]
fn bulk_expand_skips_claims_the_state_refuses() {
    let cluster = test_cluster();
    let mut pending = claim("b", "100Gi", "100Gi");
    pending.phase = "Pending".to_owned();
    let rows = vec![
        KindObject::PersistentVolumeClaim(claim("a", "100Gi", "100Gi")),
        KindObject::PersistentVolumeClaim(pending),
    ];
    let batch = bulk_expand(&rows, &cluster, "200Gi").expect("a batch");
    assert_eq!(batch.plan.items.len(), 1);
    assert_eq!(
        batch.plan.skipped[0].reason,
        "Only a bound claim can be expanded"
    );
}

#[test]
fn bulk_expand_checks_the_class_when_the_list_is_loaded() {
    let cluster = test_cluster();
    let rows = claims(&[("a", "100Gi", "100Gi")]);
    let gp3 = class("gp3", false, false);
    let Err(reason) = bulk_expand_over(&rows, &cluster, "200Gi", &[&gp3]) else {
        panic!("the only claim is refused, so there is no batch");
    };
    assert_eq!(reason, "Storage class gp3 does not allow expansion");
}

#[test]
fn bulk_expand_refuses_a_bad_size() {
    let cluster = test_cluster();
    let rows = claims(&[("a", "100Gi", "100Gi")]);
    for storage in ["", "abc", "0", "-1Gi"] {
        assert!(
            bulk_expand(&rows, &cluster, storage).is_err(),
            "{storage:?}"
        );
    }
}

// ---- Set as default storage class ----

fn default_plan(
    target: &StorageClassSummary,
    classes: &[&StorageClassSummary],
) -> Option<BatchIntent> {
    let cluster = test_cluster();
    let scope = WorkloadScope {
        cluster: &cluster,
        cluster_name: "stg-b",
    };
    default_class_intent(&scope, target, classes)
}

fn operations(batch: &BatchIntent) -> Vec<(String, bool)> {
    batch
        .plan
        .items
        .iter()
        .map(|item| {
            let WriteOperation::SetDefaultStorageClass { is_default } = item.request.operation()
            else {
                panic!("a default-class item");
            };
            (item.request.target().name().to_owned(), *is_default)
        })
        .collect()
}

#[test]
fn set_default_plan_sets_new_then_unsets_old() {
    let (gp3, io2) = (class("gp3", false, true), class("io2", true, true));
    let batch = default_plan(&gp3, &[&gp3, &io2]).expect("a plan");
    assert_eq!(
        operations(&batch),
        [("gp3".to_owned(), true), ("io2".to_owned(), false)]
    );
    assert_eq!(batch.plan.on_failure, BatchFailure::Stop);
    assert!(batch.plan.skipped.is_empty());
    assert_eq!(batch.label, "Make gp3 the default storage class");
    assert_eq!(batch.verb, "Set default");
    assert_eq!(batch.risk, ActionRisk::Change);
    assert_eq!(batch.confirm_label(0), "Set default 2");
    assert_eq!(
        texts(batch.warnings),
        [
            "New claims without a class will use gp3; existing claims keep their class",
            "io2 stops being the default"
        ]
    );
    let lines: Vec<&str> = batch
        .plan
        .items
        .iter()
        .map(|item| item.object.as_ref())
        .collect();
    assert_eq!(
        lines,
        [
            "gp3 \u{b7} becomes the default",
            "io2 \u{b7} stops being the default"
        ]
    );
}

#[test]
fn set_default_unsets_every_other_default_in_name_order() {
    let target = class("gp3", false, true);
    let (zeta, alpha, mid) = (
        class("zeta", true, true),
        class("alpha", true, true),
        class("mid", false, true),
    );
    let batch = default_plan(&target, &[&target, &zeta, &mid, &alpha]).expect("a plan");
    assert_eq!(
        operations(&batch),
        [
            ("gp3".to_owned(), true),
            ("alpha".to_owned(), false),
            ("zeta".to_owned(), false)
        ]
    );
}

#[test]
fn set_default_without_previous_default_is_one_item() {
    let (gp3, io2) = (class("gp3", false, true), class("io2", false, true));
    let batch = default_plan(&gp3, &[&gp3, &io2]).expect("a plan");
    assert_eq!(operations(&batch), [("gp3".to_owned(), true)]);
    assert_eq!(
        texts(batch.warnings),
        ["New claims without a class will use gp3; existing claims keep their class"]
    );
}

#[test]
fn set_default_disabled_on_the_default() {
    assert_eq!(
        class_block(&class("io2", true, true)).as_deref(),
        Some("Already the default")
    );
    assert_eq!(class_block(&class("gp3", false, true)), None);
    // Nothing to do at all: the only default is the target.
    let io2 = class("io2", true, true);
    assert!(default_plan(&io2, &[&io2]).is_none());
}

#[test]
fn retry_plan_omits_the_set_when_target_is_default() {
    let mut gp3 = class("gp3", true, true);
    gp3.created_at = "2025-06-01T00:00:00Z".parse().ok();
    let mut io2 = class("io2", true, true);
    io2.created_at = "2024-01-01T00:00:00Z".parse().ok();
    let batch = default_plan(&gp3, &[&gp3, &io2]).expect("a plan");
    assert_eq!(operations(&batch), [("io2".to_owned(), false)]);
    assert_eq!(
        texts(batch.warnings),
        [
            "io2 stops being the default",
            "Both gp3 and io2 are marked default; the cluster uses the newer one (gp3) until io2 is unset"
        ]
    );
}

#[test]
fn two_defaults_text_names_the_newer_class() {
    let mut gp3 = class("gp3", true, true);
    let mut io2 = class("io2", true, true);
    gp3.created_at = "2025-06-01T00:00:00Z".parse().ok();
    io2.created_at = "2024-01-01T00:00:00Z".parse().ok();
    assert_eq!(
        two_defaults_text(&gp3, &io2),
        "Both gp3 and io2 are marked default; the cluster uses the newer one (gp3) until io2 is unset"
    );
    // The old default is the newer object.
    gp3.created_at = "2023-01-01T00:00:00Z".parse().ok();
    assert_eq!(
        two_defaults_text(&gp3, &io2),
        "Both gp3 and io2 are marked default; the cluster uses the newer one (io2) until io2 is unset"
    );
}

#[test]
fn the_state_left_by_a_stopped_run_depends_on_whether_the_set_went_through() {
    let (gp3, io2) = (class("gp3", false, true), class("io2", true, true));
    let batch = default_plan(&gp3, &[&gp3, &io2]).expect("a plan");
    let BatchExtras::DefaultClass(extras) = &batch.plan.extras else {
        panic!("a default-class plan");
    };
    // The set failed: nothing changed.
    let failed = [
        ItemProgress::Failed("x".into()),
        ItemProgress::NotSent("an earlier step failed".into()),
    ];
    assert_eq!(extras.state_left(&failed), None);
    // The set went through and the unset did not: two defaults.
    let partial = [ItemProgress::Done, ItemProgress::Failed("x".into())];
    assert!(
        extras
            .state_left(&partial)
            .is_some_and(|text| text.starts_with("Both gp3 and io2"))
    );
}

fn bulk_default(
    objects: &[KindObject],
    classes: &[&StorageClassSummary],
) -> Result<BatchIntent, SharedString> {
    let cluster = test_cluster();
    let rows: Vec<CheckedRow<'_>> = objects
        .iter()
        .map(|object| CheckedRow {
            cluster: &cluster,
            object,
        })
        .collect();
    let inputs = BulkInputs {
        cluster_name: "stg-b",
        rows: &rows,
        now: jiff::Timestamp::now(),
        hpas: &[],
    };
    bulk_default_class_intent(&inputs, classes)
}

#[test]
fn bulk_set_default_needs_exactly_one_class() {
    let (gp3, io2) = (class("gp3", false, true), class("io2", true, true));
    let both = [
        KindObject::StorageClass(gp3.clone()),
        KindObject::StorageClass(io2.clone()),
    ];
    let Err(reason) = bulk_default(&both, &[&gp3, &io2]) else {
        panic!("two classes are ticked, so no batch");
    };
    assert_eq!(reason, "Tick one storage class");
    let Err(reason) = bulk_default(&[], &[&gp3, &io2]) else {
        panic!("no class is ticked, so no batch");
    };
    assert_eq!(reason, "Tick one storage class");
    let Err(reason) = bulk_default(&[KindObject::StorageClass(io2.clone())], &[&gp3, &io2]) else {
        panic!("the default is ticked, so no batch");
    };
    assert_eq!(reason, "Already the default");
    let one = bulk_default(&[KindObject::StorageClass(gp3.clone())], &[&gp3, &io2]);
    assert!(one.is_ok_and(|batch| batch.plan.items.len() == 2));
}

// ---- Review fixes ----

fn extras_of(batch: &BatchIntent) -> &DefaultClassExtras {
    let BatchExtras::DefaultClass(extras) = &batch.plan.extras else {
        panic!("a default-class plan");
    };
    extras
}

#[test]
fn the_state_left_names_the_unset_that_did_not_go_through() {
    // a and b are both default: the plan is [set gp3, unset a, unset b].
    let gp3 = class("gp3", false, true);
    let (a, b) = (class("a", true, true), class("b", true, true));
    let batch = default_plan(&gp3, &[&gp3, &a, &b]).expect("a plan");
    let extras = extras_of(&batch);
    let failed = || ItemProgress::Failed("boom".into());
    let not_sent = || ItemProgress::NotSent("an earlier step failed".into());
    // a's unset went through and b's did not: the state names b, not a.
    let results = [ItemProgress::Done, ItemProgress::Done, failed()];
    let text = extras.state_left(&results).expect("two defaults");
    assert!(text.starts_with("Both gp3 and b "), "{text}");
    assert!(text.ends_with("until b is unset"), "{text}");
    // a's unset failed and b's was never sent: a.
    let results = [ItemProgress::Done, failed(), not_sent()];
    let text = extras.state_left(&results).expect("two defaults");
    assert!(text.starts_with("Both gp3 and a "), "{text}");
    // Everything went through: nothing is left behind.
    assert_eq!(extras.state_left(&vec![ItemProgress::Done; 3]), None);
}

#[test]
fn the_state_left_of_a_retry_counts_from_its_first_unset() {
    let gp3 = class("gp3", true, true);
    let (a, b) = (class("a", true, true), class("b", true, true));
    let batch = default_plan(&gp3, &[&gp3, &a, &b]).expect("a plan");
    // The set is omitted: the items are [unset a, unset b].
    let results = [ItemProgress::Done, ItemProgress::Failed("boom".into())];
    let text = extras_of(&batch)
        .state_left(&results)
        .expect("two defaults");
    assert!(text.starts_with("Both gp3 and b "), "{text}");
}

fn done_plan(
    gp3: &StorageClassSummary,
    olds: &[&StorageClassSummary],
) -> (BatchIntent, Vec<ItemProgress>) {
    let mut classes = vec![gp3];
    classes.extend(olds);
    let batch = default_plan(gp3, &classes).expect("a plan");
    let results = vec![ItemProgress::Done; batch.plan.items.len()];
    (batch, results)
}

#[test]
fn a_default_made_meanwhile_is_still_a_default_after_the_run() {
    let (gp3, io2) = (class("gp3", false, true), class("io2", true, true));
    let (batch, results) = done_plan(&gp3, &[&io2]);
    // The list as the watch shows it after the commits: the plan's own changes are laid over it,
    // so a list that has not caught up still reads one default.
    let stale = [&gp3, &io2];
    assert_eq!(defaults_after(&batch, &results, &stale), ["gp3"]);
    assert_eq!(many_defaults_warning(&["gp3".to_owned()]), None);
    // Someone made st1 the default after the plan was drawn.
    let st1 = class("st1", true, true);
    let with_st1 = [&gp3, &io2, &st1];
    let defaults = defaults_after(&batch, &results, &with_st1);
    assert_eq!(defaults, ["gp3", "st1"]);
    assert_eq!(
        many_defaults_warning(&defaults).as_deref(),
        Some(
            "More than one storage class is marked default now (gp3, st1); the cluster uses the newest"
        )
    );
}

#[test]
fn a_step_that_did_not_go_through_leaves_the_list_as_it_is() {
    let (gp3, io2) = (class("gp3", false, true), class("io2", true, true));
    let (batch, _) = done_plan(&gp3, &[&io2]);
    let results = [ItemProgress::Done, ItemProgress::Failed("boom".into())];
    assert_eq!(
        defaults_after(&batch, &results, &[&gp3, &io2]),
        ["gp3", "io2"]
    );
}

#[test]
fn no_loaded_list_means_no_defaults_to_report() {
    let (gp3, io2) = (class("gp3", false, true), class("io2", true, true));
    let (batch, results) = done_plan(&gp3, &[&io2]);
    assert!(defaults_after(&batch, &results, &[]).is_empty());
}
