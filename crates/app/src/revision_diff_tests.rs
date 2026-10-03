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
        selector: Vec::new(),
        containers: vec![TemplateContainer {
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
