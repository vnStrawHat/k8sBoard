use cluster::{ControllerRef, PodSummary};

use super::*;
use crate::log_fixtures::fixture_pod;

fn pod(name: &str, node: Option<&str>, created: i64) -> PodSummary {
    let mut summary = fixture_pod(name, Vec::new());
    summary.node_name = node.map(str::to_owned);
    summary.created_at = jiff::Timestamp::from_second(created).ok();
    summary.controller = Some(ControllerRef {
        kind: "ReplicaSet".to_owned(),
        name: "noisy-5d".to_owned(),
    });
    summary
}

fn known(pods: &[&PodSummary]) -> HashSet<String> {
    pods.iter().map(|pod| pod.name.clone()).collect()
}

fn watched(evicted: PodSummary, replacement: Option<PodSummary>, hint: Option<&str>) -> Watched {
    Watched {
        known: known(&[&evicted]),
        pod: evicted,
        hint: hint.map(str::to_owned),
        replacement,
    }
}

#[test]
fn the_replacement_is_the_newer_pod_of_the_same_controller() {
    let evicted = pod("noisy-a", Some("wk-1"), 100);
    let sibling = pod("noisy-b", Some("wk-2"), 90);
    let replacement = pod("noisy-c", Some("wk-1"), 200);
    let pods = [evicted.clone(), sibling.clone(), replacement.clone()];
    let known = known(&[&evicted, &sibling]);
    assert_eq!(replacement_of(&evicted, &known, &pods), Some(&pods[2]));
    // Before the controller reacts, only older pods exist.
    assert_eq!(replacement_of(&evicted, &known, &pods[..2]), None);
}

#[test]
fn a_statefulset_replacement_keeps_the_name_but_is_newer() {
    let old = pod("db-0", Some("wk-1"), 100);
    let new = pod("db-0", Some("wk-1"), 300);
    let known = known(&[&old]);
    assert_eq!(
        replacement_of(&old, &known, std::slice::from_ref(&old)),
        None
    );
    assert!(replacement_of(&old, &known, &[new]).is_some());
}

#[test]
fn a_pod_of_another_controller_or_namespace_is_no_replacement() {
    let evicted = pod("noisy-a", Some("wk-1"), 100);
    let mut other_owner = pod("web-x", Some("wk-1"), 200);
    other_owner.controller = Some(ControllerRef {
        kind: "ReplicaSet".to_owned(),
        name: "web".to_owned(),
    });
    let mut other_namespace = pod("noisy-y", Some("wk-1"), 200);
    other_namespace.namespace = "billing".to_owned();
    let known = known(&[&evicted]);
    assert_eq!(
        replacement_of(&evicted, &known, &[other_owner, other_namespace]),
        None
    );
}

#[test]
fn a_replacement_on_the_same_node_says_so_and_carries_the_throttling_hint() {
    let item = watched(
        pod("noisy-a", Some("wk-1"), 100),
        Some(pod("noisy-c", Some("wk-1"), 200)),
        Some("300m = limit: throttled"),
    );
    assert_eq!(
        watch_text(&[item]),
        (
            "Evicted noisy-a: replacement noisy-c → wk-1 (same node) · 300m = limit: throttled"
                .to_owned(),
            true
        )
    );
}

#[test]
fn a_replacement_on_another_node_names_it_and_drops_the_hint() {
    let item = watched(
        pod("noisy-a", Some("wk-1"), 100),
        Some(pod("noisy-c", Some("wk-2"), 200)),
        Some("300m = limit: throttled"),
    );
    assert_eq!(
        watch_text(&[item]),
        (
            "Evicted noisy-a: replacement noisy-c → wk-2".to_owned(),
            true
        )
    );
}

#[test]
fn a_replacement_without_a_node_is_pending_and_several_pods_are_counted() {
    let pending = watched(
        pod("db-0", Some("wk-1"), 100),
        Some(pod("db-0", None, 200)),
        None,
    );
    assert_eq!(
        watch_text(std::slice::from_ref(&pending)),
        (
            "Evicted db-0: replacement db-0 is Pending, no node yet".to_owned(),
            false
        )
    );
    let placed = watched(
        pod("noisy-a", Some("wk-1"), 100),
        Some(pod("noisy-c", Some("wk-1"), 200)),
        None,
    );
    assert_eq!(
        watch_text(&[pending, placed]),
        (
            "1 of 2 replacements placed · 1 on the same node".to_owned(),
            false
        )
    );
}
