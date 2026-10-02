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
        explorer,
        companion,
        object_events,
        related,
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
    // Five namespaces reach the 3N + 4 bound exactly.
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
    // Only Services and StorageClasses join with a second list.
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
    let mut list = RelatedList::loading_for(&replica_set_subject());
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
    let mut jobs_list = RelatedList::loading_for(&jobs);
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
    let mut events_list = RelatedList::loading_for(&rejections);
    events_list.apply(RelatedUpdate::ResourceQuotas(WatchUpdate::Snapshot(
        Vec::new(),
    )));
    assert!(matches!(&events_list, RelatedList::Events(events) if events.is_loading()));
    events_list.apply(RelatedUpdate::Events(WatchUpdate::Snapshot(Vec::new())));
    assert!(matches!(&events_list, RelatedList::Events(events) if events.ready_count() == Some(0)));
    let quotas = RelatedSubject::NamespaceQuotas {
        namespace: "team-a".to_owned(),
    };
    let mut quotas_list = RelatedList::loading_for(&quotas);
    quotas_list.apply(RelatedUpdate::Events(WatchUpdate::Snapshot(Vec::new())));
    assert!(matches!(&quotas_list, RelatedList::ResourceQuotas(list) if list.is_loading()));
    assert!(quotas_list.resource_quotas().is_some());
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
    assert_eq!(kinds.len(), ResourceKind::ALL.len() - 1);
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
    assert_eq!(
        denied_related_check(&quotas, &access_with(AccessCheck::ListResourceQuotas)),
        Some(AccessCheck::ListResourceQuotas)
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
