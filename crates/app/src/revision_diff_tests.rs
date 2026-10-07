use cluster::{ReplicaSetSummary, TemplateContainer};

use super::*;
use crate::resource_kind::ResourceKind;

fn deployment_key() -> ResourceKey {
    ResourceKey::Kind {
        kind: ResourceKind::Deployments,
        namespace: Some("shop".to_owned()),
        name: "api".to_owned(),
    }
}

fn side(replica_set: &str, revision: Option<u64>, is_current: bool) -> RevisionSide {
    RevisionSide {
        replica_set: replica_set.to_owned(),
        revision,
        tag: Some("v1".to_owned()),
        is_current,
        created_at: None,
        change_cause: None,
    }
}

fn replica_set(revision: Option<&str>, image: &str) -> ReplicaSetSummary {
    ReplicaSetSummary {
        namespace: "shop".to_owned(),
        name: "api-7d9f8c".to_owned(),
        created_at: None,
        labels: Vec::new(),
        desired: 1,
        current: 1,
        ready: 1,
        owner: None,
        revision: revision.map(str::to_owned),
        change_cause: None,
        selector: Vec::new(),
        containers: vec![TemplateContainer {
            resources: Vec::new(),
            name: "api".to_owned(),
            image: image.to_owned(),
            ports: Vec::new(),
        }],
    }
}

#[test]
fn diff_request_puts_older_left() {
    let request = diff_request(
        deployment_key(),
        side("api-old", Some(37), false),
        side("api-new", Some(38), true),
    );
    assert_eq!(request.older.replica_set, "api-old");
    assert_eq!(request.newer.replica_set, "api-new");
    assert_eq!(request.deployment, deployment_key());
}

#[test]
fn diff_request_when_current_is_older() {
    // After a roll back the Deployment runs a lower revision than a row that is still listed.
    let request = diff_request(
        deployment_key(),
        side("api-newer", Some(40), false),
        side("api-current", Some(38), true),
    );
    assert_eq!(request.older.replica_set, "api-current");
    assert_eq!(request.newer.replica_set, "api-newer");
}

#[test]
fn revision_parses_from_the_annotation_text() {
    let parsed =
        |text: Option<&str>| RevisionSide::of(&replica_set(text, "api:v2"), false).revision;
    assert_eq!(parsed(Some("38")), Some(38));
    assert_eq!(parsed(Some("x")), None);
    assert_eq!(parsed(None), None);
    // The tag comes from the first container image, a registry port is not a tag.
    let side = RevisionSide::of(&replica_set(Some("1"), "registry:5000/api:v2"), true);
    assert_eq!(side.tag.as_deref(), Some("v2"));
    assert!(side.is_current);
}

#[test]
fn subtitle_names_revisions_and_tags() {
    let request = diff_request(
        deployment_key(),
        side("api-old", Some(37), false),
        side("api-new", Some(38), true),
    );
    assert_eq!(request.subtitle(), "rev 37 · v1 → rev 38 · v1 (current)");
    assert_eq!(request.title(), "Revision diff · deployment/api");
    // A side with no revision number or tag still reads.
    let bare = RevisionSide {
        replica_set: "api-x".to_owned(),
        revision: None,
        tag: None,
        is_current: false,
        created_at: None,
        change_cause: None,
    };
    assert_eq!(bare.label(), "rev —");
}

fn rows_of(older: &str, newer: &str) -> Vec<DiffRow> {
    diff_rows(older, newer)
}

#[test]
fn same_templates_say_so() {
    let rows = rows_of("a: 1\nb: 2\n", "a: 1\nb: 2\n");
    assert_eq!(same_note(&rows, 0), Some(SAME_NOTE));
}

#[test]
fn equal_texts_with_hidden_env_say_no_visible_difference() {
    let rows = rows_of("env: <hidden>\n", "env: <hidden>\n");
    assert_eq!(
        same_note(&rows, 2),
        Some("No visible difference; env values are hidden")
    );
}

#[test]
fn a_changed_line_is_a_diff_not_a_note() {
    let rows = rows_of("image: api:1\n", "image: api:2\n");
    assert_eq!(same_note(&rows, 0), None);
    assert_eq!(same_note(&rows, 3), None);
}

#[test]
fn env_toggle_shown_only_when_hidden() {
    assert!(shows_env_toggle(EnvValues::Hidden, 1));
    assert!(shows_env_toggle(EnvValues::Shown, 0));
    assert!(!shows_env_toggle(EnvValues::Hidden, 0));
}

fn numbered_set(name: &str, revision: Option<&str>) -> ReplicaSetSummary {
    ReplicaSetSummary {
        name: name.to_owned(),
        ..replica_set(revision, "api:v1")
    }
}

fn names(sides: &[RevisionSide]) -> Vec<&str> {
    sides.iter().map(|side| side.replica_set.as_str()).collect()
}

#[test]
fn revision_list_is_newest_first_with_current_marked() {
    let sets = [
        numbered_set("a36", Some("36")),
        numbered_set("a38", Some("38")),
        numbered_set("a37", Some("37")),
        numbered_set("a-none", None),
    ];
    let sides = revision_list(&sets);
    assert_eq!(names(&sides), ["a38", "a37", "a36", "a-none"]);
    let current: Vec<bool> = sides.iter().map(|side| side.is_current).collect();
    assert_eq!(current, [true, false, false, false]);
}

#[test]
fn revision_list_never_marks_an_unnumbered_set_current() {
    let sides = revision_list(&[numbered_set("only", None)]);
    assert!(!sides[0].is_current);
}

#[test]
fn latest_pair_needs_two_numbered_revisions() {
    let one = revision_list(&[numbered_set("a1", Some("1")), numbered_set("x", None)]);
    assert_eq!(latest_pair(&one), None);
    let two = revision_list(&[numbered_set("a1", Some("1")), numbered_set("a2", Some("2"))]);
    let (newest, previous) = latest_pair(&two).expect("two numbered revisions");
    assert_eq!(
        (newest.replica_set.as_str(), previous.replica_set.as_str()),
        ("a2", "a1")
    );
}
fn three() -> Vec<RevisionSide> {
    revision_list(&[
        numbered_set("a", Some("1")),
        numbered_set("b", Some("2")),
        numbered_set("c", Some("3")),
    ])
}

#[test]
fn change_pair_diffs_the_named_set_against_its_predecessor() {
    let (newer, older) = change_pair(&three(), Some("b")).expect("a pair");
    assert_eq!(
        (newer.replica_set.as_str(), older.replica_set.as_str()),
        ("b", "a")
    );
    assert!(!newer.is_current);
}

#[test]
fn change_pair_falls_back_without_predecessor() {
    // The lowest number has no predecessor, and an unlisted name has no side: both use the latest
    // pair.
    for named in [Some("a"), Some("gone"), None] {
        let (newer, older) = change_pair(&three(), named).expect("a pair");
        assert_eq!(
            (newer.replica_set.as_str(), older.replica_set.as_str()),
            ("c", "b")
        );
    }
    assert_eq!(
        change_pair(&revision_list(&[numbered_set("a", Some("1"))]), Some("a")),
        None
    );
}

#[test]
fn roll_back_goes_to_the_side_that_is_not_current() {
    let request = diff_request(
        deployment_key(),
        side("api-old", Some(37), false),
        side("api-new", Some(38), true),
    );
    let target = request.roll_back_target().expect("a target");
    assert_eq!(
        (target.replica_set.as_str(), target.revision),
        ("api-old", 37)
    );
}

#[test]
fn roll_back_has_no_target_without_exactly_one_current_side() {
    let neither = diff_request(
        deployment_key(),
        side("api-a", Some(1), false),
        side("api-b", Some(2), false),
    );
    assert_eq!(neither.roll_back_target(), None);
    // A ReplicaSet without a revision number cannot be named in a roll back.
    let unnumbered = diff_request(
        deployment_key(),
        side("api-old", None, false),
        side("api-new", Some(38), true),
    );
    assert_eq!(unnumbered.roll_back_target(), None);
}

#[test]
fn the_current_revision_has_no_roll_back_target() {
    assert_eq!(side("api-new", Some(38), true).roll_back_target(), None);
}

fn with_cause(mut set: ReplicaSetSummary, cause: Option<&str>) -> ReplicaSetSummary {
    set.change_cause = cause.map(str::to_owned);
    set
}

#[test]
fn a_side_keeps_the_change_cause_of_its_replica_set() {
    let set = with_cause(replica_set(Some("7"), "api:v1"), Some("release test"));
    assert_eq!(
        RevisionSide::of(&set, false).change_cause.as_deref(),
        Some("release test")
    );
    assert_eq!(
        RevisionSide::of(&replica_set(Some("7"), "api:v1"), false).change_cause,
        None
    );
}

#[test]
fn the_tooltip_has_the_whole_cause_then_the_absolute_creation_time() {
    let created: jiff::Timestamp = "2026-10-06T10:26:00Z".parse().expect("a timestamp");
    let zone = jiff::tz::TimeZone::fixed(jiff::tz::offset(7));
    assert_eq!(
        revision_tooltip(Some(created), Some("release test"), &zone).as_deref(),
        Some("release test\nCreated 2026-10-06 17:26 +07")
    );
    assert_eq!(
        revision_tooltip(Some(created), None, &zone).as_deref(),
        Some("Created 2026-10-06 17:26 +07")
    );
    assert_eq!(
        revision_tooltip(None, Some("release test"), &zone).as_deref(),
        Some("release test")
    );
    assert_eq!(revision_tooltip(None, None, &zone), None);
}

#[test]
fn the_diff_header_names_the_cause_of_each_side_that_has_one() {
    let mut older = side("api-old", Some(37), false);
    let mut newer = side("api-new", Some(38), true);
    older.change_cause = Some("hotfix".to_owned());
    let request = diff_request(deployment_key(), older.clone(), newer.clone());
    assert_eq!(request.cause_lines(), ["rev 37: hotfix"]);
    newer.change_cause = Some("release test".to_owned());
    let request = diff_request(deployment_key(), older, newer);
    assert_eq!(
        request.cause_lines(),
        ["rev 37: hotfix", "rev 38: release test"]
    );
    let plain = diff_request(
        deployment_key(),
        side("api-a", Some(1), false),
        side("api-b", Some(2), true),
    );
    assert!(plain.cause_lines().is_empty());
}
