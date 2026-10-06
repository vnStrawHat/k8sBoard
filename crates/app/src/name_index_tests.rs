use cluster::{AccessDecision, AccessReport, AccessReview};

use super::*;

fn named(namespace: &str, name: &str) -> ObjectName {
    ObjectName {
        namespace: Some(namespace.to_owned()),
        name: name.to_owned(),
    }
}

fn list_of(names: &[(&str, &str)]) -> NameList {
    NameList {
        names: names
            .iter()
            .map(|(namespace, name)| named(namespace, name))
            .collect(),
        is_truncated: false,
    }
}

fn access_denying(denied: &[AccessCheck]) -> AccessState {
    let reviews = AccessCheck::ALL
        .into_iter()
        .map(|check| AccessReview {
            check,
            decision: if denied.contains(&check) {
                AccessDecision::Denied { reason: None }
            } else {
                AccessDecision::Allowed
            },
        })
        .collect();
    AccessState::Known(AccessReport { reviews })
}

fn open_plan() -> Vec<(ResourceKind, Option<AccessCheck>)> {
    name_index_plan(&access_denying(&[]))
}

/// An index whose run is in flight for `scope`.
fn running(scope: &NamespaceScope) -> NameIndex {
    let mut index = NameIndex::default();
    index.begin(scope.clone(), &open_plan(), Task::ready(()));
    index
}

fn all_ready(names: &[(&str, &str)]) -> Vec<NameListResult> {
    NAME_INDEX_KINDS
        .into_iter()
        .map(|kind| (kind, Ok(list_of(names))))
        .collect()
}

#[test]
fn a_new_index_wants_a_run() {
    let index = NameIndex::default();
    assert!(index.wants_run(&NamespaceScope::All, Instant::now()));
}

#[test]
fn a_fresh_index_waits_for_the_age_and_then_runs() {
    let scope = NamespaceScope::All;
    let start = Instant::now();
    let mut index = running(&scope);
    assert!(index.finish(&scope, all_ready(&[]), start));
    assert!(!index.wants_run(&scope, start + Duration::from_secs(119)));
    assert!(index.wants_run(&scope, start + Duration::from_secs(120)));
}

#[test]
fn a_run_in_flight_is_never_joined_by_another() {
    let scope = NamespaceScope::All;
    let index = running(&scope);
    assert!(!index.wants_run(&scope, Instant::now() + Duration::from_secs(600)));
}

#[test]
fn another_scope_wants_a_run() {
    let scope = NamespaceScope::All;
    let start = Instant::now();
    let mut index = running(&scope);
    index.finish(&scope, all_ready(&[]), start);
    let other = NamespaceScope::Named("shop".to_owned());
    assert!(index.wants_run(&other, start));
}

#[test]
fn a_failed_run_is_not_retried_within_the_age() {
    let scope = NamespaceScope::All;
    let start = Instant::now();
    let mut index = running(&scope);
    let failed = NAME_INDEX_KINDS
        .into_iter()
        .map(|kind| (kind, Err("connection refused".to_owned())))
        .collect();
    assert!(index.finish(&scope, failed, start));
    // Typing again right after the failure starts nothing...
    assert!(!index.wants_run(&scope, start + Duration::from_secs(1)));
    assert!(!index.wants_run(&scope, start + Duration::from_secs(119)));
    // ...and the retry comes once the age is over.
    assert!(index.wants_run(&scope, start + Duration::from_secs(120)));
    assert_eq!(index.summary().unavailable, NAME_INDEX_KINDS);
}

#[test]
fn a_run_that_stopped_fails_its_lists_and_stamps_the_time() {
    let scope = NamespaceScope::All;
    let start = Instant::now();
    let mut index = running(&scope);
    assert!(index.finish(&scope, Vec::new(), start));
    assert_eq!(index.summary().unavailable, NAME_INDEX_KINDS);
    assert!(!index.wants_run(&scope, start + Duration::from_secs(60)));
}

#[test]
fn a_result_of_another_scope_is_discarded() {
    let scope = NamespaceScope::All;
    let mut index = running(&scope);
    let stale = NamespaceScope::Named("shop".to_owned());
    assert!(!index.finish(&stale, all_ready(&[("shop", "cart")]), Instant::now()));
    assert_eq!(index.summary().searching, NAME_INDEX_KINDS);
    assert_eq!(index.ready_names().count(), 0);
    // The run is still the current one.
    assert!(!index.wants_run(&scope, Instant::now() + Duration::from_secs(600)));
}

#[test]
fn denied_kinds_are_planned_apart_and_not_listed() {
    let plan = name_index_plan(&access_denying(&[AccessCheck::ListIngresses]));
    let denied: Vec<_> = plan
        .iter()
        .filter(|(_, check)| check.is_some())
        .map(|(kind, _)| *kind)
        .collect();
    assert_eq!(denied, [ResourceKind::Ingresses]);

    let mut index = NameIndex::default();
    index.begin(NamespaceScope::All, &plan, Task::ready(()));
    let summary = index.summary();
    assert_eq!(summary.not_permitted, [ResourceKind::Ingresses]);
    assert_eq!(summary.searching.len(), NAME_INDEX_KINDS.len() - 1);
}

#[test]
fn an_unknown_report_denies_nothing() {
    let plan = name_index_plan(&AccessState::Unknown);
    assert!(plan.iter().all(|(_, check)| check.is_none()));
}

#[test]
fn ready_names_carry_their_kind() {
    let scope = NamespaceScope::All;
    let mut index = running(&scope);
    let results = vec![
        (
            ResourceKind::Services,
            Ok(list_of(&[("shop", "kong-proxy")])),
        ),
        (ResourceKind::Ingresses, Ok(list_of(&[("shop", "web")]))),
    ];
    index.finish(&scope, results, Instant::now());
    let names: Vec<_> = index
        .ready_names()
        .map(|(kind, name)| (kind, name.name.as_str()))
        .collect();
    assert_eq!(
        names,
        [
            (ResourceKind::Services, "kong-proxy"),
            (ResourceKind::Ingresses, "web")
        ]
    );
}

#[test]
fn the_summary_names_each_state_and_the_cut_lists() {
    let scope = NamespaceScope::All;
    let plan = name_index_plan(&access_denying(&[AccessCheck::ListIngresses]));
    let mut index = NameIndex::default();
    index.begin(scope.clone(), &plan, Task::ready(()));
    let cut = NameList {
        names: Vec::new(),
        is_truncated: true,
    };
    let results = vec![
        (ResourceKind::Services, Ok(cut)),
        (ResourceKind::StatefulSets, Ok(NameList::default())),
        (ResourceKind::CronJobs, Err("forbidden".to_owned())),
    ];
    index.finish(&scope, results, Instant::now());
    let summary = index.summary();
    assert_eq!(
        summary.searched,
        [ResourceKind::Services, ResourceKind::StatefulSets]
    );
    assert_eq!(summary.truncated, [ResourceKind::Services]);
    assert_eq!(summary.not_permitted, [ResourceKind::Ingresses]);
    // Not answered (a 403 without a report, or a lost result) is unavailable too.
    assert_eq!(
        summary.unavailable,
        [ResourceKind::CronJobs, ResourceKind::NetworkPolicies]
    );
    assert!(summary.searching.is_empty());
}

#[test]
fn a_rerun_of_the_same_scope_keeps_the_ready_names_searchable() {
    let scope = NamespaceScope::All;
    let start = Instant::now();
    let mut index = running(&scope);
    index.finish(&scope, all_ready(&[("shop", "cart")]), start);
    index.begin(scope.clone(), &open_plan(), Task::ready(()));
    assert_eq!(index.ready_names().count(), NAME_INDEX_KINDS.len());
    assert!(index.summary().searching.is_empty());
}

#[test]
fn a_run_for_another_scope_starts_from_loading_lists() {
    let scope = NamespaceScope::All;
    let mut index = running(&scope);
    index.finish(&scope, all_ready(&[("shop", "cart")]), Instant::now());
    index.begin(
        NamespaceScope::Named("shop".to_owned()),
        &open_plan(),
        Task::ready(()),
    );
    assert_eq!(index.ready_names().count(), 0);
    assert_eq!(index.summary().searching, NAME_INDEX_KINDS);
}
