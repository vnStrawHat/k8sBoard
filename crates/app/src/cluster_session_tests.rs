use cluster::fake_api::FakeApi;
use cluster::{AccessDecision, AccessReview, WritePolicy};
use gpui_kit::Task;

use crate::topology_feeds::TOPOLOGY_FEED_KINDS;
use crate::topology_graph::KindFilter;

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
    let requested = NamespaceScope::Named("team-a".to_owned());
    let scope = initial_scope(Some(&requested), Some(&allowed), "default");
    assert_eq!(scope, requested);
}

#[test]
fn initial_scope_keeps_a_requested_several_scope() {
    let allowed = report(AccessDecision::Allowed);
    let requested = NamespaceScope::of_namespaces(["b".to_owned(), "a".to_owned()]);
    assert_eq!(
        initial_scope(Some(&requested), Some(&allowed), "default"),
        requested
    );
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

#[test]
fn namespaces_label_lists_two_then_counts_the_rest() {
    let names = |count: usize| -> Vec<String> {
        ["a", "b", "c", "d", "e"][..count]
            .iter()
            .map(|name| (*name).to_owned())
            .collect()
    };
    assert_eq!(namespaces_label(&names(2)), "a, b");
    assert_eq!(namespaces_label(&names(5)), "a, b +3");
}

fn ready(items: Vec<u32>) -> LiveList<u32> {
    LiveList::Ready {
        items,
        interruption: None,
    }
}

fn paused() -> StreamFlow<u32> {
    StreamFlow::Paused { held: None }
}

#[test]
fn live_flow_applies_every_update() {
    let mut list = ready(vec![1]);
    StreamFlow::Live.receive(&mut list, WatchUpdate::Snapshot(vec![2]));
    assert_eq!(ready_items(&list), Some((&[2][..], None)));
}

#[test]
fn paused_flow_holds_newest_snapshot() {
    let mut list = ready(vec![1]);
    let mut flow = paused();
    flow.receive(&mut list, WatchUpdate::Snapshot(vec![2]));
    flow.receive(&mut list, WatchUpdate::Snapshot(vec![3]));
    // The rows stand still; only the newest snapshot is kept.
    assert_eq!(ready_items(&list), Some((&[1][..], None)));
    assert!(matches!(&flow, StreamFlow::Paused { held: Some(held) } if *held == [3]));
}

#[test]
fn paused_flow_applies_failures() {
    let mut list = ready(vec![1]);
    let mut flow = paused();
    flow.receive(&mut list, failure());
    let (items, interruption) = ready_items(&list).expect("a ready list");
    assert_eq!(items, [1]);
    assert!(interruption.is_some());
    assert!(matches!(flow, StreamFlow::Paused { held: None }));
}

#[test]
fn resume_applies_held_snapshot() {
    let mut list = ready(vec![1]);
    let mut flow = paused();
    flow.receive(&mut list, WatchUpdate::Snapshot(vec![2]));
    flow.resume(&mut list);
    assert_eq!(ready_items(&list), Some((&[2][..], None)));
    assert!(matches!(flow, StreamFlow::Live));
}

#[test]
fn resume_without_a_held_snapshot_keeps_the_rows() {
    let mut list = ready(vec![1]);
    let mut flow = paused();
    flow.resume(&mut list);
    assert_eq!(ready_items(&list), Some((&[1][..], None)));
    assert!(matches!(flow, StreamFlow::Live));
}

#[test]
fn pause_is_a_no_op_unless_the_list_is_ready() {
    for list in [
        LiveList::<u32>::Loading,
        LiveList::Failed {
            message: "denied".to_owned(),
        },
    ] {
        let mut flow = StreamFlow::Live;
        assert!(!flow.pause(&list));
        assert!(matches!(flow, StreamFlow::Live));
    }
    let mut flow = StreamFlow::Live;
    assert!(flow.pause(&ready(vec![1])));
    assert!(matches!(flow, StreamFlow::Paused { held: None }));
}

#[test]
fn pausing_a_paused_list_keeps_its_held_snapshot() {
    let mut list = ready(vec![1]);
    let mut flow = paused();
    flow.receive(&mut list, WatchUpdate::Snapshot(vec![2]));
    assert!(!flow.pause(&list));
    assert!(matches!(&flow, StreamFlow::Paused { held: Some(held) } if *held == [2]));
}

fn watches(
    namespaces: usize,
    explorer: usize,
    companion: usize,
    object_events: bool,
    related: bool,
) -> OpenWatches {
    OpenWatches {
        namespaces,
        crds: false,
        explorer,
        companion,
        object_events,
        related,
        issue_feeds: 0,
        change_events: 0,
        topology: 0,
    }
}

#[test]
fn open_watch_count_counts_several_related_and_companion() {
    // All with nothing open: namespaces, pods, nodes.
    assert_eq!(open_watch_count(watches(1, 0, 0, false, false)), 3);
    assert_eq!(open_watch_count(watches(1, 1, 0, false, false)), 4);
    assert_eq!(open_watch_count(watches(1, 1, 0, true, false)), 5);
    assert_eq!(open_watch_count(watches(1, 1, 0, true, true)), 6);
    // Three picked namespaces, everything open: 2 + 3 pods + 3 explorer + events + related.
    assert_eq!(
        open_watch_count(watches(3, 3, 0, true, true)),
        2 + 3 + 3 + 2
    );
    // The Services screen adds one slice watch per namespace.
    assert_eq!(
        open_watch_count(watches(3, 3, 3, true, true)),
        2 + 3 + 3 + 3 + 2
    );
    // Five namespaces reach the 3N + 4 bound exactly (before the CRD watch).
    assert_eq!(open_watch_count(watches(5, 5, 5, true, true)), 3 * 5 + 4);
    // StorageClasses is cluster-scoped: one explorer watch and one PV companion watch whatever the
    // scope, so three picked namespaces stay below the bound.
    let companion = CompanionLists::loading_for(CompanionKind::PersistentVolumes);
    let explorer = explorer_watches(ResourceKind::StorageClasses, 3);
    assert_eq!(explorer, 1);
    assert_eq!(
        open_watch_count(watches(3, explorer, companion.watches(3), true, true)),
        2 + 3 + 1 + 1 + 2
    );
}

fn denial_of_volumes(decision: AccessDecision) -> AccessState {
    AccessState::Known(AccessReport {
        reviews: vec![AccessReview {
            check: AccessCheck::ListPersistentVolumes,
            decision,
        }],
    })
}

fn denial_of_slices(decision: AccessDecision) -> AccessState {
    AccessState::Known(AccessReport {
        reviews: vec![AccessReview {
            check: AccessCheck::ListEndpointSlices,
            decision,
        }],
    })
}

#[test]
fn companion_plan_per_kind() {
    let allowed = denial_of_slices(AccessDecision::Allowed);
    assert_eq!(
        companion_plan(
            ResourceKind::StorageClasses,
            &denial_of_volumes(AccessDecision::Allowed)
        ),
        CompanionPlan::Start(CompanionKind::PersistentVolumes)
    );
    assert_eq!(
        companion_plan(
            ResourceKind::StorageClasses,
            &denial_of_volumes(AccessDecision::Denied { reason: None })
        ),
        CompanionPlan::Denied(AccessCheck::ListPersistentVolumes)
    );
    assert_eq!(
        companion_plan(ResourceKind::Services, &allowed),
        CompanionPlan::Start(CompanionKind::EndpointSlices)
    );
    assert_eq!(
        companion_plan(
            ResourceKind::Secrets,
            &denial_of_ingresses(AccessDecision::Allowed)
        ),
        CompanionPlan::Start(CompanionKind::Ingresses)
    );
    // Only Services, StorageClasses, and Secrets join with a second list.
    for kind in [
        ResourceKind::PersistentVolumes,
        ResourceKind::PersistentVolumeClaims,
        ResourceKind::Deployments,
        ResourceKind::ConfigMaps,
        ResourceKind::Namespaces,
        ResourceKind::Events,
    ] {
        assert_eq!(companion_plan(kind, &allowed), CompanionPlan::None);
    }
}

#[test]
fn services_start_endpoint_companion_unless_denied() {
    let denied = denial_of_slices(AccessDecision::Denied { reason: None });
    assert_eq!(
        companion_plan(ResourceKind::Services, &denied),
        CompanionPlan::Denied(AccessCheck::ListEndpointSlices)
    );
    // A review that failed does not block the watch: it shows its own failure.
    assert_eq!(
        companion_plan(ResourceKind::Services, &AccessState::Unknown),
        CompanionPlan::Start(CompanionKind::EndpointSlices)
    );
}

#[test]
fn companion_lists_start_loading_and_apply_snapshots() {
    let mut lists = CompanionLists::loading_for(CompanionKind::EndpointSlices);
    assert!(matches!(&lists, CompanionLists::EndpointSlices(slices) if slices.is_loading()));
    lists.apply(CompanionUpdate::EndpointSlices(WatchUpdate::Snapshot(
        Vec::new(),
    )));
    assert_eq!(
        lists.endpoint_slices().and_then(LiveList::ready_count),
        Some(0)
    );
}

#[test]
fn companion_lists_mark_a_stopped_stream_as_a_problem() {
    let mut lists = CompanionLists::loading_for(CompanionKind::EndpointSlices);
    lists.mark_stopped();
    assert!(lists.endpoint_slices().is_some_and(LiveList::has_problem));
}

#[test]
fn companion_watches_run_once_per_namespace() {
    let lists = CompanionLists::loading_for(CompanionKind::EndpointSlices);
    assert_eq!(lists.watches(3), 3);
}

#[test]
fn cluster_scoped_companion_runs_one_watch() {
    let lists = CompanionLists::loading_for(CompanionKind::PersistentVolumes);
    assert_eq!(lists.watches(3), 1);
}

#[test]
fn companion_list_ignores_other_variant() {
    let mut lists = CompanionLists::loading_for(CompanionKind::PersistentVolumes);
    lists.apply(CompanionUpdate::EndpointSlices(WatchUpdate::Snapshot(
        Vec::new(),
    )));
    assert!(lists.persistent_volumes().is_some_and(LiveList::is_loading));
    assert!(lists.endpoint_slices().is_none());
    lists.apply(CompanionUpdate::PersistentVolumes(WatchUpdate::Snapshot(
        Vec::new(),
    )));
    assert_eq!(
        lists.persistent_volumes().and_then(LiveList::ready_count),
        Some(0)
    );
}

#[test]
fn volume_companion_marks_a_stopped_stream_as_a_problem() {
    let mut lists = CompanionLists::loading_for(CompanionKind::PersistentVolumes);
    lists.mark_stopped();
    assert!(
        lists
            .persistent_volumes()
            .is_some_and(LiveList::has_problem)
    );
}

#[test]
fn items_mut_is_empty_until_the_list_loads() {
    let mut list = LiveList::<u32>::Loading;
    assert!(list.items_mut().is_empty());
}

#[test]
fn items_mut_edits_a_loaded_list_in_place() {
    let mut list = LiveList::<u32>::Loading;
    list.apply(WatchUpdate::Snapshot(vec![1, 2]));
    list.items_mut()[0] = 9;
    assert_eq!(list.items(), [9, 2]);
}

#[test]
fn explorer_watches_follow_the_scope_except_for_namespaces() {
    assert_eq!(explorer_watches(ResourceKind::Deployments, 3), 3);
    assert_eq!(explorer_watches(ResourceKind::Deployments, 1), 1);
    assert_eq!(explorer_watches(ResourceKind::Namespaces, 3), 1);
}

#[test]
fn scope_multiplicity_counts_picked_namespaces() {
    assert_eq!(scope_multiplicity(&NamespaceScope::All), 1);
    assert_eq!(
        scope_multiplicity(&NamespaceScope::Named("a".to_owned())),
        1
    );
    let several = NamespaceScope::Several(vec!["a".to_owned(), "b".to_owned(), "c".to_owned()]);
    assert_eq!(scope_multiplicity(&several), 3);
}

fn replica_set_subject() -> RelatedSubject {
    RelatedSubject::ReplicaSets {
        namespace: "team-a".to_owned(),
        deployment: "api".to_owned(),
        selector: "app=api".to_owned(),
    }
}

#[test]
fn related_list_ignores_other_variant() {
    let mut list = RelatedList::loading_for(&replica_set_subject(), NamespaceListGates::OPEN);
    // A stale Jobs update cannot reach a ReplicaSets list.
    list.apply(RelatedUpdate::Jobs(WatchUpdate::Snapshot(Vec::new())));
    assert!(matches!(&list, RelatedList::ReplicaSets(replica_sets) if replica_sets.is_loading()));
    list.apply(RelatedUpdate::ReplicaSets(
        WatchUpdate::Snapshot(Vec::new()),
    ));
    assert!(
        matches!(&list, RelatedList::ReplicaSets(replica_sets) if replica_sets.ready_count() == Some(0))
    );
    let jobs = RelatedSubject::Jobs {
        namespace: "team-a".to_owned(),
        cron_job: "nightly".to_owned(),
    };
    let mut jobs_list = RelatedList::loading_for(&jobs, NamespaceListGates::OPEN);
    jobs_list.apply(RelatedUpdate::ReplicaSets(
        WatchUpdate::Snapshot(Vec::new()),
    ));
    assert!(matches!(&jobs_list, RelatedList::Jobs(jobs) if jobs.is_loading()));
    jobs_list.mark_stopped();
    assert!(matches!(&jobs_list, RelatedList::Jobs(jobs) if jobs.failure().is_some()));
    // The new lists take their own updates only.
    let rejections = RelatedSubject::QuotaRejections {
        namespace: "team-a".to_owned(),
        quota: "compute".to_owned(),
    };
    let mut events_list = RelatedList::loading_for(&rejections, NamespaceListGates::OPEN);
    events_list.apply(RelatedUpdate::ResourceQuotas(WatchUpdate::Snapshot(
        Vec::new(),
    )));
    assert!(matches!(&events_list, RelatedList::Events(events) if events.is_loading()));
    events_list.apply(RelatedUpdate::Events(WatchUpdate::Snapshot(Vec::new())));
    assert!(matches!(&events_list, RelatedList::Events(events) if events.ready_count() == Some(0)));
    let quotas = RelatedSubject::NamespaceQuotas {
        namespace: "team-a".to_owned(),
    };
    let mut quotas_list = RelatedList::loading_for(&quotas, NamespaceListGates::OPEN);
    quotas_list.apply(RelatedUpdate::Events(WatchUpdate::Snapshot(Vec::new())));
    assert!(matches!(
        &quotas_list,
        RelatedList::NamespaceLimits { quotas, limit_ranges }
            if quotas.is_loading() && limit_ranges.is_loading()
    ));
    assert!(quotas_list.namespace_limits().is_some());
    assert!(quotas_list.events().is_none());
}

fn counted(scope: &NamespaceScope, started_at: Instant) -> KindCounts {
    KindCounts {
        scope: Some(scope.clone()),
        counts: HashMap::from([(ResourceKind::Deployments, 3)]),
        refreshed_at: Some(started_at),
        task: None,
    }
}

#[test]
fn counts_start_once_per_scope_after_review() {
    let now = Instant::now();
    let all = NamespaceScope::All;
    let named = NamespaceScope::Named("team-a".to_owned());
    // The first review of a scope counts.
    assert!(KindCounts::default().wants_run(CountTrigger::Review, &all, now));
    // A retried review for the same scope does not.
    assert!(!counted(&all, now).wants_run(CountTrigger::Review, &all, now));
    // Another scope does.
    assert!(counted(&all, now).wants_run(CountTrigger::Review, &named, now));
}

#[test]
fn navigation_refresh_waits_30_seconds() {
    let started = Instant::now();
    let all = NamespaceScope::All;
    let counts = counted(&all, started);
    let wants = |seconds| {
        counts.wants_run(
            CountTrigger::Navigation,
            &all,
            started + Duration::from_secs(seconds),
        )
    };
    assert!(!wants(0));
    assert!(!wants(29));
    assert!(wants(30));
    // Nothing counted yet: the review starts the first run, not a navigation.
    assert!(!KindCounts::default().wants_run(CountTrigger::Navigation, &all, started));
    // Another scope waits for its review too.
    let named = NamespaceScope::Named("team-a".to_owned());
    assert!(!counts.wants_run(
        CountTrigger::Navigation,
        &named,
        started + Duration::from_secs(60)
    ));
}

#[test]
fn scope_change_clears_counts() {
    let now = Instant::now();
    let all = NamespaceScope::All;
    let counts = counted(&all, now);
    assert_eq!(
        counts.all(EventFilter::All).get(&ResourceKind::Deployments),
        Some(&3)
    );
    // `set_scope` starts over with the default, which holds no numbers and counts for any scope.
    let cleared = KindCounts::default();
    assert!(cleared.all(EventFilter::All).is_empty());
    assert!(cleared.wants_run(CountTrigger::Review, &all, now));
}

#[test]
fn denied_kinds_are_not_counted() {
    let allowed = denial_of_slices(AccessDecision::Allowed);
    // `denial_of_slices` reports only one check, so everything else reads as not allowed.
    assert!(countable_kinds(&allowed).is_empty());
    let report = AccessState::Known(AccessReport {
        reviews: AccessCheck::ALL
            .into_iter()
            .map(|check| AccessReview {
                check,
                decision: if check == AccessCheck::ListDeployments {
                    AccessDecision::Denied { reason: None }
                } else {
                    AccessDecision::Allowed
                },
            })
            .collect(),
    });
    let kinds = countable_kinds(&report);
    assert!(!kinds.contains(&ResourceKind::Deployments));
    assert!(kinds.contains(&ResourceKind::Services));
    // Releases are never counted: their Secrets would count revisions.
    assert!(!kinds.contains(&ResourceKind::HelmReleases));
    assert_eq!(kinds.len(), ResourceKind::ALL.len() - 2);
}

#[test]
fn nothing_is_counted_before_the_report_is_known() {
    assert!(countable_kinds(&AccessState::Unknown).is_empty());
}

#[test]
fn counted_events_are_hidden_while_the_list_shows_warnings_only() {
    let counts = KindCounts {
        counts: HashMap::from([(ResourceKind::Events, 120), (ResourceKind::Jobs, 5)]),
        ..KindCounts::default()
    };
    let all = counts.all(EventFilter::All);
    assert_eq!(all.get(&ResourceKind::Events), Some(&120));
    // The count includes normal events, so it would not match the filtered list.
    let warnings = counts.all(EventFilter::WarningsOnly);
    assert_eq!(warnings.get(&ResourceKind::Events), None);
    assert_eq!(warnings.get(&ResourceKind::Jobs), Some(&5));
}

fn access_with(denied: AccessCheck) -> AccessState {
    AccessState::Known(AccessReport {
        reviews: AccessCheck::ALL
            .into_iter()
            .map(|check| AccessReview {
                check,
                decision: if check == denied {
                    AccessDecision::Denied { reason: None }
                } else {
                    AccessDecision::Allowed
                },
            })
            .collect(),
    })
}

#[test]
fn denied_quota_subject_does_not_start() {
    let rejections = RelatedSubject::QuotaRejections {
        namespace: "team-a".to_owned(),
        quota: "compute".to_owned(),
    };
    let quotas = RelatedSubject::NamespaceQuotas {
        namespace: "team-a".to_owned(),
    };
    assert_eq!(
        denied_related_check(&rejections, &access_with(AccessCheck::ListEvents)),
        Some(AccessCheck::ListEvents)
    );
    // The Namespace drawer's lists are gated each by its own check (`namespace_list_gates`).
    assert_eq!(
        denied_related_check(&quotas, &access_with(AccessCheck::ListResourceQuotas)),
        None
    );
    // Another denial does not stop it, and a report that is not known never does.
    assert_eq!(
        denied_related_check(&quotas, &access_with(AccessCheck::ListEvents)),
        None
    );
    assert_eq!(denied_related_check(&quotas, &AccessState::Unknown), None);
    // Subjects of other kinds are not gated here.
    let jobs = RelatedSubject::Jobs {
        namespace: "team-a".to_owned(),
        cron_job: "nightly".to_owned(),
    };
    assert_eq!(
        denied_related_check(&jobs, &access_with(AccessCheck::ListEvents)),
        None
    );
}

#[test]
fn kubelet_round_rejoins_pvc_rows() {
    for kind in ResourceKind::ALL {
        assert_eq!(
            rejoins_after_kubelet_round(kind),
            kind == ResourceKind::PersistentVolumeClaims,
            "{}",
            kind.label()
        );
    }
}

#[test]
fn scope_change_keeps_cluster_scoped_explorer() {
    assert!(!restarts_on_scope_change(ResourceKind::Namespaces));
    assert!(!restarts_on_scope_change(ResourceKind::PersistentVolumes));
    assert!(restarts_on_scope_change(
        ResourceKind::PersistentVolumeClaims
    ));
    assert!(restarts_on_scope_change(ResourceKind::Deployments));
}

/// A known report that allows every check except those listed.
fn report_denying(denied: &[AccessCheck]) -> AccessState {
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

#[test]
fn bindings_plan_per_kind_and_denial() {
    let allowed = report_denying(&[]);
    // Roles need the role bindings only; ClusterRoles need both lists.
    assert_eq!(
        companion_plan(ResourceKind::Roles, &allowed),
        CompanionPlan::Start(CompanionKind::Bindings {
            with_cluster_role_bindings: false
        })
    );
    assert_eq!(
        companion_plan(ResourceKind::ClusterRoles, &allowed),
        CompanionPlan::Start(CompanionKind::Bindings {
            with_cluster_role_bindings: true
        })
    );
    // A denied list a kind needs means the companion could never be ready, so it does not start.
    let no_cluster_list = report_denying(&[AccessCheck::ListClusterRoleBindings]);
    assert_eq!(
        companion_plan(ResourceKind::ClusterRoles, &no_cluster_list),
        CompanionPlan::Denied(AccessCheck::ListClusterRoleBindings)
    );
    // Roles do not need that list.
    assert_eq!(
        companion_plan(ResourceKind::Roles, &no_cluster_list),
        CompanionPlan::Start(CompanionKind::Bindings {
            with_cluster_role_bindings: false
        })
    );
    let no_role_list = report_denying(&[AccessCheck::ListRoleBindings]);
    assert_eq!(
        companion_plan(ResourceKind::Roles, &no_role_list),
        CompanionPlan::Denied(AccessCheck::ListRoleBindings)
    );
    assert_eq!(
        companion_plan(ResourceKind::ClusterRoles, &no_role_list),
        CompanionPlan::Denied(AccessCheck::ListRoleBindings)
    );
    // A review that failed does not block the watches.
    assert_eq!(
        companion_plan(ResourceKind::ClusterRoles, &AccessState::Unknown),
        CompanionPlan::Start(CompanionKind::Bindings {
            with_cluster_role_bindings: true
        })
    );
}

#[test]
fn denied_binding_checks_name_every_denied_list() {
    let neither = report_denying(&[
        AccessCheck::ListRoleBindings,
        AccessCheck::ListClusterRoleBindings,
    ]);
    assert_eq!(
        denied_binding_checks(ResourceKind::ClusterRoles, &neither),
        [
            AccessCheck::ListRoleBindings,
            AccessCheck::ListClusterRoleBindings
        ]
    );
    assert_eq!(
        denied_binding_checks(ResourceKind::Roles, &neither),
        [AccessCheck::ListRoleBindings]
    );
    assert!(denied_binding_checks(ResourceKind::Roles, &report_denying(&[])).is_empty());
    assert!(denied_binding_checks(ResourceKind::Deployments, &neither).is_empty());
}

#[test]
fn bindings_companion_applies_each_list() {
    let mut lists = CompanionLists::loading_for(CompanionKind::Bindings {
        with_cluster_role_bindings: true,
    });
    lists.apply(CompanionUpdate::ClusterRoleBindings(WatchUpdate::Snapshot(
        Vec::new(),
    )));
    let CompanionLists::Bindings {
        role_bindings,
        cluster_role_bindings: Some(cluster_role_bindings),
    } = &lists
    else {
        panic!("both lists were started");
    };
    assert!(role_bindings.is_loading());
    assert_eq!(cluster_role_bindings.ready_count(), Some(0));
    lists.apply(CompanionUpdate::RoleBindings(WatchUpdate::Snapshot(
        Vec::new(),
    )));
    assert!(matches!(
        &lists,
        CompanionLists::Bindings { role_bindings, cluster_role_bindings: Some(second) }
            if role_bindings.ready_count() == Some(0) && second.ready_count() == Some(0)
    ));
}

#[test]
fn bindings_companion_ignores_a_list_it_does_not_run() {
    let mut roles = CompanionLists::loading_for(CompanionKind::Bindings {
        with_cluster_role_bindings: false,
    });
    roles.apply(CompanionUpdate::ClusterRoleBindings(WatchUpdate::Snapshot(
        Vec::new(),
    )));
    assert!(matches!(
        &roles,
        CompanionLists::Bindings { role_bindings, cluster_role_bindings: None }
            if role_bindings.is_loading()
    ));
    roles.mark_stopped();
    assert!(matches!(
        &roles,
        CompanionLists::Bindings { role_bindings, .. } if role_bindings.has_problem()
    ));
}

#[test]
fn bindings_companion_watch_count() {
    let watches_of = |with_cluster_role_bindings, namespaces| {
        CompanionLists::loading_for(CompanionKind::Bindings {
            with_cluster_role_bindings,
        })
        .watches(namespaces)
    };
    // One role-binding watch per namespace, plus one cluster-wide watch.
    assert_eq!(watches_of(false, 3), 3);
    assert_eq!(watches_of(true, 3), 4);
    // Roles at five namespaces reach the 3N + 4 bound exactly.
    let explorer = explorer_watches(ResourceKind::Roles, 5);
    assert_eq!(
        open_watch_count(watches(5, explorer, watches_of(false, 5), true, true)),
        3 * 5 + 4
    );
    // ServiceAccounts: N explorer watches, N + 1 binding watches, and the drawer events only (no
    // related subject), which stays within the same bound.
    let explorer = explorer_watches(ResourceKind::ServiceAccounts, 5);
    assert_eq!(explorer, 5);
    assert_eq!(
        open_watch_count(watches(5, explorer, watches_of(true, 5), true, false)),
        3 * 5 + 4
    );
    // ClusterRoles are cluster-scoped: one explorer watch and N + 1 binding watches.
    let explorer = explorer_watches(ResourceKind::ClusterRoles, 5);
    assert_eq!(explorer, 1);
    assert_eq!(
        open_watch_count(watches(5, explorer, watches_of(true, 5), true, true)),
        2 + 5 + 1 + 6 + 2
    );
}

fn denial_of_ingresses(decision: AccessDecision) -> AccessState {
    AccessState::Known(AccessReport {
        reviews: vec![AccessReview {
            check: AccessCheck::ListIngresses,
            decision,
        }],
    })
}

#[test]
fn secrets_start_the_ingresses_companion_unless_denied() {
    assert_eq!(
        companion_plan(
            ResourceKind::Secrets,
            &denial_of_ingresses(AccessDecision::Allowed)
        ),
        CompanionPlan::Start(CompanionKind::Ingresses)
    );
    assert_eq!(
        companion_plan(
            ResourceKind::Secrets,
            &denial_of_ingresses(AccessDecision::Denied { reason: None })
        ),
        CompanionPlan::Denied(AccessCheck::ListIngresses)
    );
    assert_eq!(
        companion_plan(ResourceKind::Secrets, &AccessState::Unknown),
        CompanionPlan::Start(CompanionKind::Ingresses)
    );
}

#[test]
fn ingresses_companion_lists_start_loading_and_apply_snapshots() {
    let mut lists = CompanionLists::loading_for(CompanionKind::Ingresses);
    assert!(matches!(&lists, CompanionLists::Ingresses(list) if list.is_loading()));
    lists.apply(CompanionUpdate::Ingresses(
        WatchUpdate::Snapshot(Vec::new()),
    ));
    assert_eq!(lists.ingresses().and_then(LiveList::ready_count), Some(0));
    // A stale update of another companion is ignored.
    lists.apply(CompanionUpdate::EndpointSlices(WatchUpdate::Snapshot(
        Vec::new(),
    )));
    assert!(lists.endpoint_slices().is_none());
}

#[test]
fn open_watch_count_with_secret_companion() {
    // One ingress watch per namespace beside the one secret watch per namespace: five namespaces
    // reach the 3N + 4 bound exactly, and none exceeds it.
    for namespaces in 1..=5 {
        let companion = CompanionLists::loading_for(CompanionKind::Ingresses);
        let explorer = explorer_watches(ResourceKind::Secrets, namespaces);
        assert_eq!(explorer, namespaces);
        let count = open_watch_count(watches(
            namespaces,
            explorer,
            companion.watches(namespaces),
            true,
            true,
        ));
        assert_eq!(count, 2 + namespaces + namespaces + namespaces + 2);
        assert!(count <= 3 * namespaces + 4);
    }
}

fn denial_of_secrets(decision: AccessDecision) -> AccessState {
    AccessState::Known(AccessReport {
        reviews: vec![AccessReview {
            check: AccessCheck::ListSecrets,
            decision,
        }],
    })
}

#[test]
fn ingresses_start_the_tls_secrets_companion_unless_denied() {
    assert_eq!(
        companion_plan(
            ResourceKind::Ingresses,
            &denial_of_secrets(AccessDecision::Allowed)
        ),
        CompanionPlan::Start(CompanionKind::TlsSecrets)
    );
    assert_eq!(
        companion_plan(
            ResourceKind::Ingresses,
            &denial_of_secrets(AccessDecision::Denied { reason: None })
        ),
        CompanionPlan::Denied(AccessCheck::ListSecrets)
    );
}

#[test]
fn tls_secrets_companion_applies_and_counts_watches() {
    let mut lists = CompanionLists::loading_for(CompanionKind::TlsSecrets);
    assert!(matches!(&lists, CompanionLists::TlsSecrets(list) if list.is_loading()));
    lists.apply(CompanionUpdate::TlsSecrets(WatchUpdate::Snapshot(
        Vec::new(),
    )));
    assert_eq!(lists.tls_secrets().and_then(LiveList::ready_count), Some(0));
    // The Ingresses screen: one TLS secret watch per namespace, within 3N + 4.
    for namespaces in 1..=5 {
        let explorer = explorer_watches(ResourceKind::Ingresses, namespaces);
        let count = open_watch_count(watches(
            namespaces,
            explorer,
            lists.watches(namespaces),
            true,
            true,
        ));
        assert!(count <= 3 * namespaces + 4);
    }
}

#[test]
fn releases_watch_count_within_bound() {
    // Releases have no object events and no companion: the explorer (one watch per namespace)
    // plus the history watch.
    for namespaces in 1..=5 {
        let explorer = explorer_watches(ResourceKind::HelmReleases, namespaces);
        assert_eq!(explorer, namespaces);
        let total = open_watch_count(watches(namespaces, explorer, 0, false, true));
        assert_eq!(total, 2 + namespaces + namespaces + 1);
        assert!(total <= 3 * namespaces + 4);
    }
}

#[test]
fn releases_have_a_history_related_list() {
    let subject = RelatedSubject::HelmHistory {
        namespace: "shop".to_owned(),
        release: "api".to_owned(),
    };
    let list = RelatedList::loading_for(&subject, NamespaceListGates::OPEN);
    assert!(list.helm_history().is_some());
    assert!(list.events().is_none());
    assert_eq!(
        denied_related_check(&subject, &report_denying(&[AccessCheck::ListSecrets])),
        None
    );
}

#[test]
fn open_watch_count_stays_within_3n_plus_5() {
    // Without the Issues engine's Warning events watches (`issue_feeds` is 0 here); the total with
    // them is `4N + 5`, checked by `open_watch_count_counts_warning_events`.
    let with_crds = |watches: OpenWatches| OpenWatches {
        crds: true,
        ..watches
    };
    // The CRDs screen adds no watch of its own: the CRD watch feeds it.
    assert!(!crds_explorer().is_watching());
    assert_eq!(
        open_watch_count(with_crds(watches(1, 0, 0, false, false))),
        4
    );
    // Five namespaces, a namespaced companion kind, drawer events and related: the worst case.
    assert_eq!(
        open_watch_count(with_crds(watches(5, 5, 5, true, true))),
        3 * 5 + 5
    );
}

fn crd_named(name: &str) -> CrdSummary {
    CrdSummary {
        name: name.to_owned(),
        group: "example.io".to_owned(),
        kind: "Widget".to_owned(),
        plural: "widgets".to_owned(),
        singular: "widget".to_owned(),
        scope: cluster::ResourceScope::Namespaced,
        versions: Vec::new(),
        state: cluster::CrdState::Established,
        created_at: None,
    }
}

fn crds_explorer() -> KindList {
    KindList {
        kind: ResourceKind::Crds,
        list: LiveList::Loading,
        flow: StreamFlow::Live,
        companion: None,
        subscription: None,
    }
}

fn crd_snapshot(names: &[&str]) -> CrdUpdate {
    let crds: Vec<CrdSummary> = names.iter().map(|name| crd_named(name)).collect();
    let rows = crds.iter().map(crd_row).collect();
    CrdUpdate::Snapshot { crds, rows }
}

fn row_names(list: &LiveList<KindRow>) -> Vec<&str> {
    list.items().iter().map(|row| row.name.as_str()).collect()
}

#[test]
fn crd_watch_waits_for_a_non_denying_review() {
    assert!(!crds_denied(&AccessState::Unknown));
    let report = |decision| {
        AccessState::Known(AccessReport {
            reviews: vec![AccessReview {
                check: AccessCheck::ListCustomResourceDefinitions,
                decision,
            }],
        })
    };
    assert!(!crds_denied(&report(AccessDecision::Allowed)));
    assert!(crds_denied(&report(AccessDecision::Denied {
        reason: None
    })));
}

#[test]
fn crd_snapshot_feeds_the_crds_explorer() {
    let mut crds = LiveList::Loading;
    let mut explorer = crds_explorer();
    feed_crds(
        &mut crds,
        Some(&mut explorer),
        crd_snapshot(&["a.example.io", "b.example.io"]),
    );
    assert_eq!(crds.ready_count(), Some(2));
    assert_eq!(row_names(&explorer.list), ["a.example.io", "b.example.io"]);
}

#[test]
fn crd_snapshot_without_the_crds_screen_only_updates_definitions() {
    let mut crds = LiveList::Loading;
    feed_crds(&mut crds, None, crd_snapshot(&["a.example.io"]));
    assert_eq!(crds.ready_count(), Some(1));
}

#[test]
fn crd_failure_marks_both_lists_and_keeps_stale_rows() {
    let failure = || {
        CrdUpdate::Failed(ClusterError::TimedOut {
            context: "ctx".to_owned(),
            action: "watching custom resource definitions",
        })
    };
    let mut crds = LiveList::Loading;
    let mut explorer = crds_explorer();
    feed_crds(&mut crds, Some(&mut explorer), failure());
    assert!(crds.failure().is_some());
    assert!(explorer.list.failure().is_some());
    feed_crds(
        &mut crds,
        Some(&mut explorer),
        crd_snapshot(&["a.example.io"]),
    );
    feed_crds(&mut crds, Some(&mut explorer), failure());
    assert!(explorer.list.interruption().is_some());
    assert_eq!(row_names(&explorer.list), ["a.example.io"]);
}

#[test]
fn crds_screen_opened_later_is_seeded_from_the_watch() {
    let unknown = AccessState::Unknown;
    assert!(crd_explorer_list(None, &unknown).is_loading());
    let mut crds = LiveList::Loading;
    assert!(crd_explorer_list(Some(&crds), &unknown).is_loading());
    feed_crds(&mut crds, None, crd_snapshot(&["a.example.io"]));
    let seeded = crd_explorer_list(Some(&crds), &unknown);
    assert_eq!(row_names(&seeded), ["a.example.io"]);
}

#[test]
fn paused_crds_screen_holds_the_newest_snapshot() {
    let mut crds = LiveList::Loading;
    let mut explorer = crds_explorer();
    feed_crds(
        &mut crds,
        Some(&mut explorer),
        crd_snapshot(&["a.example.io"]),
    );
    assert!(explorer.flow.pause(&explorer.list));
    feed_crds(
        &mut crds,
        Some(&mut explorer),
        crd_snapshot(&["b.example.io"]),
    );
    assert_eq!(row_names(&explorer.list), ["a.example.io"]);
    explorer.flow.resume(&mut explorer.list);
    assert_eq!(row_names(&explorer.list), ["b.example.io"]);
}

fn widget_kind(is_namespaced: bool) -> CustomKind {
    let crd = CrdSummary {
        name: "widgets.x.io".to_owned(),
        group: "x.io".to_owned(),
        kind: "Widget".to_owned(),
        plural: "widgets".to_owned(),
        singular: "widget".to_owned(),
        scope: if is_namespaced {
            cluster::ResourceScope::Namespaced
        } else {
            cluster::ResourceScope::Cluster
        },
        versions: vec![cluster::CrdVersion {
            name: "v1".to_owned(),
            is_served: true,
            is_storage: true,
            is_deprecated: false,
            deprecation_warning: None,
            printer_columns: Vec::new(),
            schema: cluster::SchemaOutline::default(),
        }],
        state: cluster::CrdState::Established,
        created_at: None,
    };
    custom_kinds(&[crd], &mut CustomKindCache::default())[0]
}

#[test]
fn custom_denied_reason_names_scope() {
    let namespaced = widget_kind(true);
    assert_eq!(
        custom_denied_reason(namespaced, &NamespaceScope::All),
        "Not permitted: list widgets.x.io in all namespaces"
    );
    assert_eq!(
        custom_denied_reason(
            namespaced,
            &NamespaceScope::of_namespaces(["b".to_owned(), "a".to_owned()])
        ),
        "Not permitted: list widgets.x.io in a, b"
    );
    assert_eq!(
        custom_denied_reason(namespaced, &NamespaceScope::Named("a".to_owned())),
        "Not permitted: list widgets.x.io"
    );
    // A cluster-scoped kind never names a scope.
    assert_eq!(
        custom_denied_reason(widget_kind(false), &NamespaceScope::All),
        "Not permitted: list widgets.x.io"
    );
}

#[test]
fn custom_gate_outcome_follows_the_review() {
    let kind = widget_kind(true);
    let scope = NamespaceScope::All;
    assert_eq!(
        gate_outcome(Some(Ok(&AccessDecision::Allowed)), kind, &scope),
        GateOutcome::Allowed
    );
    assert_eq!(
        gate_outcome(
            Some(Ok(&AccessDecision::Denied { reason: None })),
            kind,
            &scope
        ),
        GateOutcome::Denied(custom_denied_reason(kind, &scope))
    );
    // A failed or unfinished review caches nothing and lets the list start.
    let failure = ClusterError::TimedOut {
        context: "ctx".to_owned(),
        action: "reviewing access",
    };
    assert_eq!(
        gate_outcome(Some(Err(&failure)), kind, &scope),
        GateOutcome::Unknown
    );
    assert_eq!(gate_outcome(None, kind, &scope), GateOutcome::Unknown);
}

#[test]
fn custom_kind_cache_moves_to_the_next_session() {
    let mut cache = CustomKindCache::default();
    let first = custom_kinds(&[crd_named_widget()], &mut cache)[0];
    // What `take_custom_kind_cache` hands over keeps every definition.
    let mut moved = std::mem::take(&mut cache);
    let reused = custom_kinds(&[crd_named_widget()], &mut moved)[0];
    assert_eq!(first, reused);
}

fn crd_named_widget() -> CrdSummary {
    CrdSummary {
        versions: vec![cluster::CrdVersion {
            name: "v1".to_owned(),
            is_served: true,
            is_storage: true,
            is_deprecated: false,
            deprecation_warning: None,
            printer_columns: Vec::new(),
            schema: cluster::SchemaOutline::default(),
        }],
        ..crd_named("widgets.x.io")
    }
}

#[test]
fn denied_crd_list_seeds_a_failed_crds_screen() {
    let denied = AccessState::Known(AccessReport {
        reviews: vec![AccessReview {
            check: AccessCheck::ListCustomResourceDefinitions,
            decision: AccessDecision::Denied { reason: None },
        }],
    });
    let list = crd_explorer_list(None, &denied);
    assert_eq!(
        list.failure(),
        Some("Not permitted: list customresourcedefinitions")
    );
    // Not denied (or not known yet): the list waits for the watch.
    assert!(crd_explorer_list(None, &AccessState::Unknown).is_loading());
}

#[test]
fn cluster_scoped_gates_survive_a_scope_change() {
    assert!(keeps_gate_on_scope_change(widget_kind(false)));
    assert!(!keeps_gate_on_scope_change(widget_kind(true)));
}

#[test]
fn explorer_action_follows_the_review_of_the_shown_kind() {
    let shown = ResourceKind::Custom(widget_kind(true));
    let other = ResourceKind::Deployments;
    let denied = || GateOutcome::Denied("no".to_owned());
    assert_eq!(
        explorer_action(Some(shown), shown, denied()),
        ExplorerAction::Fail("no".to_owned())
    );
    assert_eq!(
        explorer_action(Some(shown), shown, GateOutcome::Allowed),
        ExplorerAction::Start
    );
    // A review that failed starts the list so the watch shows its own error.
    assert_eq!(
        explorer_action(Some(shown), shown, GateOutcome::Unknown),
        ExplorerAction::Start
    );
    // The result of another kind, or with nothing shown, only changes its gate.
    assert_eq!(
        explorer_action(Some(other), shown, denied()),
        ExplorerAction::Leave
    );
    assert_eq!(
        explorer_action(None, shown, GateOutcome::Allowed),
        ExplorerAction::Leave
    );
}

#[test]
fn custom_counts_refresh_after_30s() {
    let mut counts = CustomCounts::default();
    let start = Instant::now();
    // Nothing ran yet.
    assert!(counts.wants_run(start));
    counts.refreshed_at = Some(start);
    assert!(!counts.wants_run(start + Duration::from_secs(29)));
    assert!(counts.wants_run(start + Duration::from_secs(30)));
}

#[test]
fn failed_counts_keep_the_previous_number() {
    let kept = widget_kind(true);
    let dropped = widget_kind(false);
    let previous = HashMap::from([(kept, 7), (dropped, 3)]);
    let error = || Err("timed out".to_owned());
    let merged = merge_custom_counts(&previous, vec![(kept, error())]);
    // A failed request keeps its old number; a kind the run did not ask about is dropped.
    assert_eq!(merged, HashMap::from([(kept, 7)]));
    let fresh = merge_custom_counts(&previous, vec![(kept, Ok(Some(9)))]);
    assert_eq!(fresh, HashMap::from([(kept, 9)]));
    // No remaining count from the server leaves the kind without a number.
    let unknown = merge_custom_counts(&previous, vec![(kept, Ok(None))]);
    assert!(unknown.is_empty());
    // A failure with nothing before it stays blank.
    assert!(merge_custom_counts(&HashMap::new(), vec![(kept, error())]).is_empty());
}

#[test]
fn denied_kinds_are_not_counted_for_instances() {
    let allowed = widget_kind(true);
    let denied = widget_kind(false);
    let mut gates = HashMap::new();
    gates.insert(
        denied,
        CustomGate::Denied {
            reason: "no".to_owned(),
        },
    );
    gates.insert(allowed, CustomGate::Allowed);
    assert_eq!(
        countable_custom_kinds(&[allowed, denied], &gates),
        [allowed]
    );
    // A kind without a review yet is counted: the count needs a cluster-wide list anyway.
    assert_eq!(
        countable_custom_kinds(&[allowed, denied], &HashMap::new()),
        [allowed, denied]
    );
}

#[test]
fn open_watch_count_counts_warning_events() {
    // One Warning events watch per namespace on top of the 3N + 5 worst case (CRD watch, drawer
    // events and related objects, a namespaced explorer and companion): 4N + 5.
    for namespaces in [1, 2, 3, 5] {
        let count = open_watch_count(OpenWatches {
            crds: true,
            issue_feeds: namespaces,
            ..watches(namespaces, namespaces, namespaces, true, true)
        });
        assert_eq!(count, 4 * namespaces + 5);
    }
    // A scope change that waits to restart the events watch counts none.
    assert_eq!(open_watch_count(watches(1, 0, 0, false, false)), 3);
}

#[test]
fn time_refresh_notifies_only_when_issues_visible() {
    use IssueChange::{Shape, Unchanged};
    for reason in [RunReason::Dirty, RunReason::TimeRefresh] {
        assert!(refresh_repaints(Shape, reason, false));
    }
    assert!(!refresh_repaints(Unchanged, RunReason::Dirty, true));
    assert!(!refresh_repaints(Unchanged, RunReason::TimeRefresh, false));
    assert!(refresh_repaints(Unchanged, RunReason::TimeRefresh, true));
}

#[test]
fn hidden_board_does_not_notify_for_an_age_change() {
    for reason in [RunReason::Dirty, RunReason::TimeRefresh] {
        assert!(!refresh_repaints(IssueChange::TextOnly, reason, false));
        assert!(refresh_repaints(IssueChange::TextOnly, reason, true));
    }
}

fn empty_snapshot() -> RbacSnapshot {
    RbacSnapshot {
        roles: Vec::new(),
        cluster_roles: Vec::new(),
        role_bindings: Vec::new(),
        cluster_role_bindings: Vec::new(),
        coverage: cluster::RbacCoverage {
            cluster_roles: true,
            cluster_bindings: true,
            roles: cluster::NamespaceCoverage::AllNamespaces,
            role_bindings: cluster::NamespaceCoverage::AllNamespaces,
        },
    }
}

fn rbac_ready() -> RbacState {
    RbacState::Ready {
        snapshot: Rc::new(empty_snapshot()),
        listed_at: jiff::Timestamp::UNIX_EPOCH,
    }
}

#[test]
fn condition_plan_opens_the_expected_number_of_watches() {
    use crate::issue_feeds::{FeedPlan, condition_plan};
    let access = AccessState::Unknown;
    let issue_watches = |names: &[&str]| {
        let scope = NamespaceScope::of_namespaces(names.iter().map(|name| (*name).to_owned()));
        let events = scope_multiplicity(&scope);
        let conditions: usize = condition_plan(&scope, &access)
            .into_iter()
            .map(|(_, plan)| match plan {
                FeedPlan::Start { watch_scope } => scope_multiplicity(&watch_scope),
                FeedPlan::Wait | FeedPlan::Off(_) => 0,
            })
            .sum();
        events + conditions
    };
    // N + 8N up to two namespaces: 9 and 18.
    assert_eq!(issue_watches(&["a"]), 9);
    assert_eq!(issue_watches(&["a", "b"]), 18);
    // Above two, one cluster-wide watch per kind: N events + 8.
    assert_eq!(issue_watches(&["a", "b", "c"]), 3 + 8);
    assert_eq!(issue_watches(&["a", "b", "c", "d", "e"]), 5 + 8);
}

#[test]
fn rbac_trigger_table() {
    let loading = || RbacState::Loading {
        _task: Task::ready(()),
    };
    let failed = || RbacState::Failed("no".to_owned());
    let cases = [
        (RbacState::Idle, RbacTrigger::Request, true),
        (RbacState::Idle, RbacTrigger::Refresh, false),
        (loading(), RbacTrigger::Request, false),
        (loading(), RbacTrigger::Refresh, false),
        (rbac_ready(), RbacTrigger::Request, false),
        (rbac_ready(), RbacTrigger::Refresh, true),
        (failed(), RbacTrigger::Request, true),
        (failed(), RbacTrigger::Refresh, true),
    ];
    for (state, trigger, expected) in cases {
        assert_eq!(starts_fetch(&state, trigger), expected, "{trigger:?}");
    }
}

#[test]
fn rbac_resets_on_scope_change() {
    let mut state = rbac_ready();
    state.reset_for_scope_change();
    assert!(matches!(state, RbacState::Idle));
}

#[test]
fn request_then_refresh_starts_a_listing_from_every_state_but_loading() {
    let loading = RbacState::Loading {
        _task: Task::ready(()),
    };
    assert!(!starts_fetch(&loading, RbacTrigger::Request));
    assert!(!starts_fetch(&loading, RbacTrigger::Refresh));
    for state in [
        RbacState::Idle,
        rbac_ready(),
        RbacState::Failed("no".to_owned()),
    ] {
        let starts = starts_fetch(&state, RbacTrigger::Request)
            || starts_fetch(&state, RbacTrigger::Refresh);
        assert!(starts);
    }
}

#[test]
fn open_watch_count_includes_change_events() {
    let with_changes = |namespaces: usize, change_events: usize| OpenWatches {
        change_events,
        ..watches(namespaces, 0, 0, false, false)
    };
    // Hidden Overview adds nothing; All adds the two watches; two namespaces add four.
    assert_eq!(open_watch_count(with_changes(1, 0)), 3);
    assert_eq!(open_watch_count(with_changes(1, 2)), 3 + 2);
    assert_eq!(open_watch_count(with_changes(2, 4)), 4 + 4);
}

#[test]
fn change_feed_denied_by_known_review() {
    assert!(is_change_feed_denied(&access_with(AccessCheck::ListEvents)));
    assert!(!is_change_feed_denied(&access_with(
        AccessCheck::ListPodMetrics
    )));
}

#[test]
fn change_feed_starts_while_checking_or_unknown() {
    assert!(!is_change_feed_denied(&AccessState::Unknown));
    let checking = AccessState::Checking {
        _task: Task::ready(()),
    };
    assert!(!is_change_feed_denied(&checking));
}

impl ClusterSession {
    /// A seam for the shell tests: the session goes live over `connection` without the round
    /// trip, so a headless window can show a live session offline. The pending connect is
    /// replaced (and so aborted) with the phase.
    pub(crate) fn go_live_for_test(
        &mut self,
        connection: ClusterConnection,
        scope: NamespaceScope,
        cx: &mut Context<Self>,
    ) {
        let connected = Connected {
            connection,
            server_version: ServerVersion {
                git_version: "v1.29.5".to_owned(),
                platform: "linux/amd64".to_owned(),
            },
            api_latency: Duration::from_millis(7),
            scope,
            access: Err("no cluster in a test".to_owned()),
        };
        self.finish_connect(Ok(Ok(connected)), cx);
    }

    /// A seam for the shell tests: the permission report the gate reads, as if the review had
    /// answered.
    pub(crate) fn set_access_for_test(&mut self, access: AccessState, cx: &mut Context<Self>) {
        if let Some(live) = self.live_mut() {
            live.access = access;
        }
        cx.notify();
    }

    /// A seam for the shell tests: the node list a live session shows.
    pub(crate) fn set_nodes_for_test(&mut self, nodes: Vec<NodeSummary>, cx: &mut Context<Self>) {
        if let Some(live) = self.live_mut() {
            live.nodes.apply(WatchUpdate::Snapshot(nodes));
        }
        cx.notify();
    }

    /// A seam for the shell tests: the pod list a live session shows, as if its watch had sent
    /// this snapshot.
    pub(crate) fn set_pods_for_test(&mut self, pods: Vec<PodSummary>, cx: &mut Context<Self>) {
        if let Some(live) = self.live_mut() {
            live.pods.apply(WatchUpdate::Snapshot(pods));
        }
        cx.notify();
    }

    /// A seam for the shell tests: the ReplicaSets the open drawer's related watch holds, as if it
    /// had sent this snapshot. Nothing happens while the drawer watches another kind of subject.
    pub(crate) fn set_replica_sets_for_test(
        &mut self,
        replica_sets: Vec<ReplicaSetSummary>,
        cx: &mut Context<Self>,
    ) {
        let related = self.live_mut().and_then(|live| live.related.as_mut());
        if let Some(related) = related {
            related.list = RelatedList::ReplicaSets(LiveList::Ready {
                items: replica_sets,
                interruption: None,
            });
        }
        cx.notify();
    }

    /// The explorer list of `kind` becomes this loaded snapshot, as if its watch had sent it; the
    /// fake server of a test lists nothing itself. Nothing happens while another kind is shown.
    pub(crate) fn set_kind_rows_for_test(
        &mut self,
        kind: ResourceKind,
        rows: Vec<KindRow>,
        cx: &mut Context<Self>,
    ) {
        let explorer = self
            .live_mut()
            .and_then(|live| live.explorer.as_mut())
            .filter(|explorer| explorer.kind == kind);
        if let Some(explorer) = explorer {
            explorer.list.apply(WatchUpdate::Snapshot(rows));
        }
        cx.notify();
    }
}

fn subject_with(namespace: &str, kinds: &[KindFilter]) -> TopologySubject {
    TopologySubject {
        namespace: namespace.to_owned(),
        kinds: kinds.iter().copied().collect(),
    }
}

/// Topology feeds that run for `kinds`, and an Off feed for each of `denied`.
fn topology_feeds_of(kinds: &[ResourceKind], denied: &[ResourceKind]) -> TopologyFeeds {
    let running = kinds
        .iter()
        .map(|kind| TopologyFeed::watching(*kind, LiveList::Loading));
    let off = denied
        .iter()
        .map(|kind| TopologyFeed::off(*kind, "not permitted".to_owned()));
    TopologyFeeds {
        subject: subject_with("shop", &KindFilter::ALL),
        feeds: running.chain(off).collect(),
    }
}

#[test]
fn open_watch_count_includes_topology() {
    let with = |feeds: Option<&TopologyFeeds>| {
        open_watch_count(OpenWatches {
            topology: feeds.map_or(0, TopologyFeeds::open_count),
            ..watches(1, 0, 0, false, false)
        })
    };
    let base = open_watch_count(watches(1, 0, 0, false, false));
    // Hidden: no feeds, nothing added.
    assert_eq!(with(None), base);
    // Visible with the default chips: the ten kinds run; RBAC is off.
    let default = subject_with("shop", &KindFilter::DEFAULT).wanted_kinds();
    assert_eq!(default.len(), 10);
    assert_eq!(with(Some(&topology_feeds_of(&default, &[]))), base + 10);
    // RBAC on: the four RBAC feeds run too.
    let all = topology_feeds_of(&TOPOLOGY_FEED_KINDS, &[]);
    assert_eq!(with(Some(&all)), base + 14);
    // One kind denied: its Off feed counts 0.
    let denied = topology_feeds_of(&default[..9], &[ResourceKind::Secrets]);
    assert_eq!(with(Some(&denied)), base + 9);
    // One RBAC kind denied: three of the four run.
    let rbac_denied = topology_feeds_of(
        &TOPOLOGY_FEED_KINDS[..13],
        &[ResourceKind::ClusterRoleBindings],
    );
    assert_eq!(with(Some(&rbac_denied)), base + 13);
    // Config chip off: ConfigMaps, Secrets, and PVCs have no feed: seven run.
    let without_config = subject_with(
        "shop",
        &[
            KindFilter::Ingress,
            KindFilter::Service,
            KindFilter::Workload,
        ],
    );
    let kinds = without_config.wanted_kinds();
    assert_eq!(kinds.len(), 7);
    assert_eq!(with(Some(&topology_feeds_of(&kinds, &[]))), base + 7);
}

#[test]
fn row_of_prefers_explorer_then_topology() {
    use crate::network_rows::service_row;
    use crate::topology_fixtures::service;

    fn named(name: &str, selector: &str) -> KindRow {
        let mut row = service_row(&service(name, &[selector]));
        // The selector tells the two copies of one row apart.
        row.labels = vec![selector.to_owned().into()];
        row
    }
    let ready = |rows: Vec<KindRow>| LiveList::Ready {
        items: rows,
        interruption: None,
    };
    let explorer = KindList::unsubscribed(
        ResourceKind::Services,
        ready(vec![named("web", "from=explorer")]),
    );
    let mut feeds = topology_feeds_of(&[], &[]);
    feeds.feeds.push(TopologyFeed::watching(
        ResourceKind::Services,
        ready(vec![
            named("web", "from=topology"),
            named("api", "from=topology"),
        ]),
    ));
    let key = |name: &str| ResourceKey::Kind {
        kind: ResourceKind::Services,
        namespace: Some("shop".to_owned()),
        name: name.to_owned(),
    };
    let source = |row: Option<&KindRow>| row.map(|row| row.labels[0].to_string());
    let found = |name: &str, explorer: Option<&KindList>, topology: Option<&TopologyFeeds>| {
        source(row_in(
            &key(name),
            ResourceKind::Services,
            explorer,
            topology,
        ))
    };
    // The explorer wins where it has the row; the feeds fill in the rest.
    assert_eq!(
        found("web", Some(&explorer), Some(&feeds)).as_deref(),
        Some("from=explorer")
    );
    assert_eq!(
        found("api", Some(&explorer), Some(&feeds)).as_deref(),
        Some("from=topology")
    );
    // Over Topology there is no explorer.
    assert_eq!(
        found("web", None, Some(&feeds)).as_deref(),
        Some("from=topology")
    );
    assert_eq!(found("gone", Some(&explorer), Some(&feeds)), None);
    assert_eq!(found("web", None, None), None);
}

#[test]
fn same_topology_subject_is_noop() {
    let running = subject_with("shop", &KindFilter::ALL);
    assert_eq!(
        subject_change(Some(&running), Some(&running.clone())),
        SubjectChange::Keep
    );
    assert_eq!(subject_change(None, None), SubjectChange::Keep);
}

#[test]
fn subject_change_keeps_unchanged_feeds() {
    let all = subject_with("shop", &KindFilter::DEFAULT);
    let without_config = subject_with(
        "shop",
        &[
            KindFilter::Ingress,
            KindFilter::Service,
            KindFilter::Workload,
        ],
    );
    let SubjectChange::Adjust { stop, start } = subject_change(Some(&all), Some(&without_config))
    else {
        panic!("a chip change adjusts the feeds");
    };
    assert_eq!(stop.len(), 3);
    assert!(start.is_empty());
    let back = subject_change(Some(&without_config), Some(&all));
    assert!(matches!(
        back,
        SubjectChange::Adjust { stop, start } if stop.is_empty() && start.len() == 3
    ));
}

#[test]
fn topology_scope_includes_every_namespace_of_all() {
    let several = NamespaceScope::Several(vec!["a".to_owned(), "b".to_owned()]);
    assert!(scope_includes(&several, "a"));
    assert!(!scope_includes(&several, "c"));
    assert!(scope_includes(&NamespaceScope::All, "anything"));
}

#[test]
fn generations_only_grow() {
    let first = next_generation();
    let second = next_generation();
    assert!(second > first);
}

// ---- Spec 0039 step 4: the Namespace drawer's quotas and LimitRanges ----

fn namespace_subject() -> RelatedSubject {
    RelatedSubject::NamespaceQuotas {
        namespace: "team-a".to_owned(),
    }
}

fn limit_range(name: &str) -> LimitRangeSummary {
    LimitRangeSummary {
        namespace: "team-a".to_owned(),
        name: name.to_owned(),
        limits: Vec::new(),
    }
}

#[test]
fn namespace_related_lists_quotas_and_limit_ranges() {
    let mut list = RelatedList::loading_for(&namespace_subject(), NamespaceListGates::OPEN);
    list.apply(RelatedUpdate::ResourceQuotas(WatchUpdate::Snapshot(
        Vec::new(),
    )));
    let Some((quotas, limit_ranges)) = list.namespace_limits() else {
        panic!("a Namespace drawer holds both lists");
    };
    // Each update reaches only its own half.
    assert_eq!(quotas.ready_count(), Some(0));
    assert!(limit_ranges.is_loading());
}

#[test]
fn limit_range_update_reaches_its_list() {
    let mut list = RelatedList::loading_for(&namespace_subject(), NamespaceListGates::OPEN);
    list.apply(RelatedUpdate::LimitRanges(WatchUpdate::Snapshot(vec![
        limit_range("defaults"),
    ])));
    let Some((quotas, limit_ranges)) = list.namespace_limits() else {
        panic!("a Namespace drawer holds both lists");
    };
    assert!(quotas.is_loading());
    assert_eq!(limit_ranges.items().len(), 1);
    assert_eq!(limit_ranges.items()[0].name, "defaults");
    // A stale update of another subject's kind changes nothing.
    list.apply(RelatedUpdate::Jobs(WatchUpdate::Snapshot(Vec::new())));
    assert_eq!(
        list.namespace_limits().map(|(_, l)| l.items().len()),
        Some(1)
    );
}

#[test]
fn namespace_list_gates_each_follow_their_check() {
    let gates = |access: &AccessState| namespace_list_gates(access, NamespaceListGates::OPEN);
    let open = NamespaceListGates::OPEN;
    assert_eq!(gates(&access_with(AccessCheck::ListEvents)), open);
    assert_eq!(
        gates(&access_with(AccessCheck::ListResourceQuotas)),
        NamespaceListGates {
            quotas: false,
            limit_ranges: true
        }
    );
    assert_eq!(
        gates(&access_with(AccessCheck::ListLimitRanges)),
        NamespaceListGates {
            quotas: true,
            limit_ranges: false
        }
    );
    // A report that is not known never closes a gate: the server answers.
    assert_eq!(gates(&AccessState::Unknown), open);
}

#[test]
fn a_review_in_progress_keeps_the_running_gates() {
    let checking = AccessState::Checking {
        _task: Task::ready(()),
    };
    let limited = NamespaceListGates {
        quotas: true,
        limit_ranges: false,
    };
    // A re-review must not reopen a stream that was left out, or it would start twice.
    assert_eq!(namespace_list_gates(&checking, limited), limited);
    assert_eq!(
        namespace_list_gates(&checking, NamespaceListGates::OPEN),
        NamespaceListGates::OPEN
    );
    // Once the answer is known it decides again.
    assert_eq!(
        namespace_list_gates(&access_with(AccessCheck::ListEvents), limited),
        NamespaceListGates::OPEN
    );
}

#[test]
fn both_denied_drop_the_subject() {
    let subject = namespace_subject();
    let both = report_denying(&[
        AccessCheck::ListResourceQuotas,
        AccessCheck::ListLimitRanges,
    ]);
    assert!(is_related_denied(&subject, &both));
    // One denied list leaves the other running, so the subject stays.
    assert!(!is_related_denied(
        &subject,
        &report_denying(&[AccessCheck::ListResourceQuotas])
    ));
    assert!(!is_related_denied(
        &subject,
        &report_denying(&[AccessCheck::ListLimitRanges])
    ));
    assert!(!is_related_denied(&subject, &AccessState::Unknown));
    // Other subjects keep their single check.
    let rejections = RelatedSubject::QuotaRejections {
        namespace: "team-a".to_owned(),
        quota: "compute".to_owned(),
    };
    assert!(is_related_denied(
        &rejections,
        &access_with(AccessCheck::ListEvents)
    ));
    assert!(!is_related_denied(&rejections, &both));
}

#[test]
fn a_closed_half_starts_failed_not_loading() {
    let gates = NamespaceListGates {
        quotas: true,
        limit_ranges: false,
    };
    let list = RelatedList::loading_for(&namespace_subject(), gates);
    let Some((quotas, limit_ranges)) = list.namespace_limits() else {
        panic!("a Namespace drawer holds both lists");
    };
    assert!(quotas.is_loading());
    // Nothing waits on a list that never starts.
    assert!(limit_ranges.failure().is_some() && !limit_ranges.is_loading());
}

const EMPTY_LIST: &str =
    r#"{"apiVersion":"v1","kind":"List","metadata":{"resourceVersion":"1"},"items":[]}"#;

/// The collection paths the streams of `gates` ask for first, in a namespace `shop`.
async fn first_requests(gates: NamespaceListGates, polls: usize) -> Vec<String> {
    let (connection, api) =
        FakeApi::connection(WritePolicy::Blocked, |_| (200, EMPTY_LIST.to_owned()));
    let mut updates = namespace_updates(&connection, "shop", gates);
    for _ in 0..polls {
        let _ = tokio::time::timeout(Duration::from_millis(300), updates.next()).await;
    }
    let mut paths: Vec<String> = api
        .requests()
        .into_iter()
        .filter(|request| request.method == "GET")
        .map(|request| request.path)
        .collect();
    paths.sort();
    paths.dedup();
    paths
}

#[tokio::test]
async fn denied_limit_ranges_leave_the_quota_stream() {
    let paths = first_requests(
        NamespaceListGates {
            quotas: true,
            limit_ranges: false,
        },
        1,
    )
    .await;
    assert_eq!(paths, ["/api/v1/namespaces/shop/resourcequotas"]);
}

#[tokio::test]
async fn denied_quotas_leave_the_limit_range_stream() {
    let paths = first_requests(
        NamespaceListGates {
            quotas: false,
            limit_ranges: true,
        },
        1,
    )
    .await;
    assert_eq!(paths, ["/api/v1/namespaces/shop/limitranges"]);
}

#[tokio::test]
async fn open_gates_start_both_streams() {
    // Two snapshots, one per stream.
    let paths = first_requests(NamespaceListGates::OPEN, 2).await;
    assert_eq!(
        paths,
        [
            "/api/v1/namespaces/shop/limitranges",
            "/api/v1/namespaces/shop/resourcequotas"
        ]
    );
}

#[test]
fn related_watch_is_current_only_for_the_same_subject_and_gates() {
    let subject = namespace_subject();
    let open = NamespaceListGates::OPEN;
    let limited = NamespaceListGates {
        quotas: true,
        limit_ranges: false,
    };
    // Nothing running and nothing wanted needs no restart and no repaint: this runs on every render.
    assert!(related_watch_is_current(None, None, open));
    assert!(related_watch_is_current(
        Some((&subject, open)),
        Some(&subject),
        open
    ));
    // A permission that closes a list restarts the Namespace watch with the streams that remain.
    assert!(!related_watch_is_current(
        Some((&subject, open)),
        Some(&subject),
        limited
    ));
    assert!(!related_watch_is_current(None, Some(&subject), open));
    assert!(!related_watch_is_current(
        Some((&subject, open)),
        None,
        open
    ));
    assert!(!related_watch_is_current(
        Some((&subject, open)),
        Some(&replica_set_subject()),
        open
    ));
}
