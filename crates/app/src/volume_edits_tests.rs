use cluster::{ClaimRef, VolumeBackend};

use super::*;
use crate::resource_edits::resource_edits_tests::{claim, class};
use crate::workload_actions::workload_actions_tests::test_cluster;

fn pending_claim() -> PersistentVolumeClaimSummary {
    let mut pending = claim("pvc-wrong-class", "100Mi", "100Mi");
    pending.phase = "Pending".to_owned();
    pending.volume = None;
    pending.capacity = None;
    pending.storage_class = Some("missing".to_owned());
    pending
}

fn volume(policy: &str) -> PersistentVolumeSummary {
    PersistentVolumeSummary {
        name: "pvc-e7e63176".to_owned(),
        created_at: None,
        labels: Vec::new(),
        capacity: Some("100Mi".to_owned()),
        access_modes: vec!["ReadWriteOnce".to_owned()],
        reclaim_policy: policy.to_owned(),
        phase: "Bound".to_owned(),
        is_terminating: false,
        claim: Some(ClaimRef {
            namespace: "lab-house".to_owned(),
            name: "data-house-db-0".to_owned(),
        }),
        storage_class: Some("standard".to_owned()),
        volume_mode: Some("Filesystem".to_owned()),
        backend: VolumeBackend::HostPath {
            path: "/var/lib".to_owned(),
        },
        node_affinity: Vec::new(),
        mount_options: Vec::new(),
        reason: None,
        message: None,
    }
}

fn texts(lines: Vec<SharedString>) -> Vec<String> {
    lines.into_iter().map(|line| line.to_string()).collect()
}

#[test]
fn only_a_pending_unbound_claim_can_be_recreated() {
    assert_eq!(recreate_block(&pending_claim()), None);
    let mut bound = pending_claim();
    bound.phase = "Bound".to_owned();
    bound.volume = Some("pv-1".to_owned());
    assert_eq!(
        recreate_block(&bound).as_deref(),
        Some("Only a Pending claim that no volume is bound to can be recreated")
    );
    let mut going = pending_claim();
    going.is_terminating = true;
    assert_eq!(
        recreate_block(&going).as_deref(),
        Some("The claim is being deleted")
    );
}

#[test]
fn the_default_class_is_picked_first_and_the_claims_own_class_is_marked() {
    let (standard, gp3, fast) = (
        class("standard", true, false),
        class("gp3", false, true),
        class("fast", false, true),
    );
    let (options, selected) =
        class_choices(&[&fast, &gp3, &standard], Some("missing")).expect("classes are listed");
    assert_eq!(selected, 2);
    assert_eq!(
        options,
        [
            ChoiceOption {
                value: "fast".to_owned(),
                hint: None
            },
            ChoiceOption {
                value: "gp3".to_owned(),
                hint: None
            },
            ChoiceOption {
                value: "standard".to_owned(),
                hint: Some("default".to_owned())
            },
        ]
    );
    // The claim's own class is not offered as the pick, even when it is the default.
    let (options, selected) =
        class_choices(&[&gp3, &standard], Some("standard")).expect("classes are listed");
    assert_eq!(selected, 0);
    assert_eq!(options[1].hint.as_deref(), Some("current"));
    assert!(class_choices(&[], None).is_none());
}

#[test]
fn the_recreate_confirm_says_what_goes_and_what_stays() {
    let cluster = test_cluster();
    let scope = WorkloadScope {
        cluster: &cluster,
        cluster_name: "stg-b",
    };
    let intent =
        recreate_intent(&scope, &pending_claim(), "claim-uid", "standard").expect("an intent");
    assert_eq!(
        intent.label,
        "Recreate claim pvc-wrong-class with class standard"
    );
    assert_eq!(intent.button, "Recreate");
    assert_eq!(intent.risk, crate::write_guard::ActionRisk::Destructive);
    assert_eq!(
        texts(intent.warnings),
        [
            "Deletes claim pvc-wrong-class and creates it again with class standard; 100Mi and ReadWriteOnce stay",
            "It is Pending and no pod mounts it, so no data is lost",
        ]
    );
    let fields = intent.request.changed_fields();
    assert_eq!(fields[0].path, "spec.storageClassName");
    assert_eq!(fields[0].value.as_deref(), Some("standard"));
    assert_eq!(fields[0].from.as_deref(), Some("missing"));
    let audited: Vec<(&str, Option<&str>)> = intent
        .audit_fields
        .iter()
        .map(|field| (field.path.as_str(), field.value.as_deref()))
        .collect();
    assert_eq!(
        audited,
        [
            ("spec.resources.requests.storage", Some("100Mi")),
            ("spec.accessModes", Some("ReadWriteOnce")),
        ]
    );
    assert!(recreate_intent(&scope, &pending_claim(), "claim-uid", "Not A Class").is_none());
}

#[test]
fn the_policy_popover_picks_the_other_policy() {
    let (options, selected) = policy_choices(&volume("Delete"));
    assert_eq!(selected, 0);
    assert_eq!(options[0].value, "Retain");
    assert_eq!(options[1].hint.as_deref(), Some("current"));
    let (options, selected) = policy_choices(&volume("Retain"));
    assert_eq!(selected, 1);
    assert_eq!(options[1].value, "Delete");
}

#[test]
fn the_policy_confirm_says_what_happens_to_the_volume_when_the_claim_goes() {
    let cluster = test_cluster();
    let scope = WorkloadScope {
        cluster: &cluster,
        cluster_name: "stg-b",
    };
    let retain = reclaim_policy_intent(&scope, &volume("Delete"), "Retain").expect("an intent");
    assert_eq!(
        retain.label,
        "Set reclaim policy of volume pvc-e7e63176 to Retain"
    );
    assert_eq!(
        texts(retain.warnings),
        [
            "When claim lab-house/data-house-db-0 is deleted, the volume and its data stay (Released) until an administrator reclaims them"
        ]
    );
    let delete = reclaim_policy_intent(&scope, &volume("Retain"), "Delete").expect("an intent");
    assert_eq!(
        texts(delete.warnings),
        [
            "When claim lab-house/data-house-db-0 is deleted, the volume and the storage behind it are deleted with it"
        ]
    );
    assert!(reclaim_policy_intent(&scope, &volume("Retain"), "Recycle").is_none());
    let mut loose = volume("Delete");
    loose.claim = None;
    assert!(texts(reclaim_warnings(&loose, "Retain"))[0].starts_with("When its claim is deleted"));
}

#[test]
fn a_volume_going_away_or_with_a_policy_the_app_does_not_set_is_refused() {
    assert_eq!(reclaim_block(&volume("Retain")), None);
    let mut going = volume("Retain");
    going.is_terminating = true;
    assert_eq!(
        reclaim_block(&going).as_deref(),
        Some("The volume is being deleted")
    );
    assert_eq!(
        reclaim_block(&volume("Recycle")).as_deref(),
        Some("Reclaim policy Recycle cannot be changed here")
    );
}

#[test]
fn the_popover_lines_name_the_state_now() {
    assert_eq!(
        class_state_text(&pending_claim()),
        "Now class missing \u{b7} Pending \u{b7} 100Mi"
    );
    assert_eq!(
        policy_state_text(&volume("Delete")),
        "Now Delete \u{b7} claim lab-house/data-house-db-0"
    );
}
