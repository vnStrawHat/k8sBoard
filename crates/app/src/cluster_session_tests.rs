use cluster::{AccessDecision, AccessReview};

use super::*;

fn failure() -> WatchUpdate<u32> {
    WatchUpdate::Failed(ClusterError::TimedOut {
        context: "ctx".to_owned(),
        action: "watching pods",
    })
}

fn ready_items(list: &LiveList<u32>) -> Option<(&[u32], Option<&str>)> {
    match list {
        LiveList::Ready {
            items,
            interruption,
        } => Some((items, interruption.as_deref())),
        LiveList::Loading | LiveList::Failed { .. } => None,
    }
}

#[test]
fn live_list_snapshot_from_loading_becomes_ready() {
    let mut list = LiveList::Loading;
    list.apply(WatchUpdate::Snapshot(vec![1, 2]));
    assert_eq!(ready_items(&list), Some((&[1, 2][..], None)));
}

#[test]
fn live_list_failed_before_data_is_failed() {
    let mut list = LiveList::<u32>::Loading;
    list.apply(failure());
    let LiveList::Failed { message } = &list else {
        panic!("expected a failed list");
    };
    assert!(message.contains("did not answer in time"), "{message}");
    // A second failure keeps it failed.
    list.apply(failure());
    assert!(matches!(list, LiveList::Failed { .. }));
}

#[test]
fn live_list_failed_after_data_keeps_items_and_sets_interruption() {
    let mut list = LiveList::Loading;
    list.apply(WatchUpdate::Snapshot(vec![7]));
    list.apply(failure());
    let (items, interruption) = ready_items(&list).expect("list keeps its data");
    assert_eq!(items, [7]);
    assert!(interruption.is_some());
}

#[test]
fn live_list_snapshot_clears_interruption() {
    let mut list = LiveList::Loading;
    list.apply(WatchUpdate::Snapshot(vec![7]));
    list.apply(failure());
    list.apply(WatchUpdate::Snapshot(vec![8]));
    assert_eq!(ready_items(&list), Some((&[8][..], None)));
}

fn report(list_pods: AccessDecision) -> AccessReport {
    AccessReport {
        reviews: vec![AccessReview {
            check: AccessCheck::ListPods,
            decision: list_pods,
        }],
    }
}

#[test]
fn initial_scope_uses_requested_namespace() {
    let allowed = report(AccessDecision::Allowed);
    let scope = initial_scope(Some("team-a"), Some(&allowed), "default");
    assert_eq!(scope, NamespaceScope::Named("team-a".to_owned()));
}

#[test]
fn initial_scope_all_when_list_pods_allowed_everywhere() {
    let allowed = report(AccessDecision::Allowed);
    assert_eq!(
        initial_scope(None, Some(&allowed), "default"),
        NamespaceScope::All
    );
}

#[test]
fn initial_scope_context_namespace_when_list_pods_denied() {
    let denied = report(AccessDecision::Denied { reason: None });
    assert_eq!(
        initial_scope(None, Some(&denied), "payments"),
        NamespaceScope::Named("payments".to_owned())
    );
}

#[test]
fn initial_scope_context_namespace_when_review_failed() {
    assert_eq!(
        initial_scope(None, None, "payments"),
        NamespaceScope::Named("payments".to_owned())
    );
}

#[test]
fn error_text_adds_first_source_line() {
    let error = ClusterError::Unreachable {
        context: "ctx".to_owned(),
        action: "listing pods",
        source: "connection refused\nsecond line".into(),
    };
    assert_eq!(
        error_text(&error),
        "cannot reach the API server of context 'ctx' while listing pods: connection refused"
    );
}

fn failed_list<T>() -> LiveList<T> {
    LiveList::Failed {
        message: "boom".to_owned(),
    }
}

fn ready_list<T>() -> LiveList<T> {
    LiveList::Ready {
        items: Vec::new(),
        interruption: None,
    }
}

#[test]
fn watch_state_reports_explorer_problem() {
    let failed = failed_list();
    let ready = ready_list();
    let problem =
        |explorer| any_list_has_problem(&ready_list(), &ready_list(), &ready_list(), explorer);
    assert!(problem(Some(&failed)));
    assert!(!problem(Some(&ready)));
    assert!(!problem(None));
}

#[test]
fn watch_state_still_reports_core_list_problems() {
    let nodes_failed = failed_list();
    assert!(any_list_has_problem(
        &ready_list(),
        &ready_list(),
        &nodes_failed,
        None
    ));
}
