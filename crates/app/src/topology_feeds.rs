//! The lazy watches behind the Topology graph: one per kind of an enabled kind chip, started while
//! Topology is shown and dropped when it is left. The session owns them (`LiveCluster::topology`);
//! this module holds what they are and the pure rules for starting, keeping, and counting them.

use std::collections::BTreeSet;

use cluster::BindingSummary;

use crate::cluster_runtime::WatchSubscription;
use crate::cluster_session::{AccessState, CompanionLists, LiveList};
use crate::kind_row::{KindObject, KindRow};
use crate::resource_kind::ResourceKind;
use crate::topology_graph::{FeedRows, KindFilter, TopologyKind};

/// The kinds the graph reads besides pods, which the session lists anyway. ClusterRoles are only
/// there to give the drawer of a ClusterRole node its row.
pub(crate) const TOPOLOGY_FEED_KINDS: [ResourceKind; 15] = [
    ResourceKind::Ingresses,
    ResourceKind::Services,
    ResourceKind::Deployments,
    ResourceKind::StatefulSets,
    ResourceKind::DaemonSets,
    ResourceKind::ReplicaSets,
    ResourceKind::ConfigMaps,
    ResourceKind::Secrets,
    ResourceKind::PersistentVolumeClaims,
    ResourceKind::HorizontalPodAutoscalers,
    ResourceKind::ServiceAccounts,
    ResourceKind::RoleBindings,
    ResourceKind::Roles,
    ResourceKind::ClusterRoleBindings,
    ResourceKind::ClusterRoles,
];

/// The namespace Topology draws and the kind chips that are on. It decides which feeds run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TopologySubject {
    pub(crate) namespace: String,
    pub(crate) kinds: BTreeSet<KindFilter>,
}

impl TopologySubject {
    /// Whether the feed of `kind` runs for this subject: its chip is on.
    fn wants(&self, kind: ResourceKind) -> bool {
        TopologyKind::of_resource_kind(kind).is_some_and(|kind| self.kinds.contains(&kind.filter()))
    }

    /// The feed kinds that run, in `TOPOLOGY_FEED_KINDS` order.
    pub(crate) fn wanted_kinds(&self) -> Vec<ResourceKind> {
        TOPOLOGY_FEED_KINDS
            .into_iter()
            .filter(|kind| self.wants(*kind))
            .collect()
    }
}

/// What to do with the running feeds when the subject changes.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum SubjectChange {
    /// The same subject: nothing to do.
    Keep,
    /// No subject: drop every feed.
    Stop,
    /// A new namespace: drop every feed and start the wanted ones again.
    Restart,
    /// The same namespace with other chips: drop `stop`, start `start`, keep the rest.
    Adjust {
        stop: Vec<ResourceKind>,
        start: Vec<ResourceKind>,
    },
}

pub(crate) fn subject_change(
    running: Option<&TopologySubject>,
    next: Option<&TopologySubject>,
) -> SubjectChange {
    let (Some(running), Some(next)) = (running, next) else {
        return if next.is_some() {
            SubjectChange::Restart
        } else if running.is_some() {
            SubjectChange::Stop
        } else {
            SubjectChange::Keep
        };
    };
    if running == next {
        return SubjectChange::Keep;
    }
    if running.namespace != next.namespace {
        return SubjectChange::Restart;
    }
    let was = running.wanted_kinds();
    let now = next.wanted_kinds();
    SubjectChange::Adjust {
        stop: was
            .iter()
            .copied()
            .filter(|kind| !now.contains(kind))
            .collect(),
        start: now
            .iter()
            .copied()
            .filter(|kind| !was.contains(kind))
            .collect(),
    }
}

/// Whether a feed starts.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum FeedStart {
    Start,
    /// A known access report denies the list, so nothing starts (no retry loop on a 403).
    Off(String),
}

/// `Known` and denied is off; a review that is still running or failed starts the feed: the server
/// answers for itself.
pub(crate) fn feed_plan(kind: ResourceKind, access: &AccessState) -> FeedStart {
    match (access, kind.access_check()) {
        (AccessState::Known(report), Some(check)) if !report.is_allowed(check) => {
            FeedStart::Off("not permitted".to_owned())
        }
        _ => FeedStart::Start,
    }
}

/// One kind's watch, or the reason it does not run. Dropping it stops the watch.
pub(crate) struct TopologyFeed {
    pub(crate) kind: ResourceKind,
    pub(crate) list: LiveList<KindRow>,
    /// Why the feed does not run; `None` while it does.
    off: Option<String>,
    _subscription: Option<WatchSubscription>,
}

impl TopologyFeed {
    pub(crate) fn started(kind: ResourceKind, subscription: WatchSubscription) -> Self {
        Self {
            kind,
            list: LiveList::Loading,
            off: None,
            _subscription: Some(subscription),
        }
    }

    /// A feed that counts as running, without a watch behind it: for tests of the counting rules.
    #[cfg(test)]
    pub(crate) fn watching(kind: ResourceKind, list: LiveList<KindRow>) -> Self {
        Self {
            kind,
            list,
            off: None,
            _subscription: None,
        }
    }

    pub(crate) fn off(kind: ResourceKind, reason: String) -> Self {
        Self {
            kind,
            list: LiveList::Loading,
            off: Some(reason),
            _subscription: None,
        }
    }

    /// Whether the feed runs a watch.
    pub(crate) fn is_open(&self) -> bool {
        self.off.is_none()
    }

    fn rows(&self) -> FeedRows<'_> {
        if self.off.is_some() {
            return FeedRows::Off;
        }
        match (self.list.ready_items(), self.list.failure()) {
            (Some(rows), _) => FeedRows::Ready(rows),
            (None, Some(_)) => FeedRows::Failed,
            (None, None) => FeedRows::Loading,
        }
    }
}

/// The feeds of one subject. A kind whose chip is off has no feed.
pub(crate) struct TopologyFeeds {
    pub(crate) subject: TopologySubject,
    pub(crate) feeds: Vec<TopologyFeed>,
}

impl TopologyFeeds {
    /// The watches that run: Off feeds count 0.
    pub(crate) fn open_count(&self) -> usize {
        self.feeds.iter().filter(|feed| feed.is_open()).count()
    }

    /// The row `name` of `kind`, from a loaded feed.
    pub(crate) fn row(
        &self,
        kind: ResourceKind,
        is_row: impl Fn(&KindRow) -> bool,
    ) -> Option<&KindRow> {
        self.feeds
            .iter()
            .find(|feed| feed.kind == kind)?
            .list
            .items()
            .iter()
            .find(|row| is_row(row))
    }

    /// The feed of `kind`, when it runs a watch.
    pub(crate) fn feed_mut(&mut self, kind: ResourceKind) -> Option<&mut TopologyFeed> {
        self.feeds.iter_mut().find(|feed| feed.kind == kind)
    }

    /// The feeds with their kind, for the graph build and the coverage note: only kinds whose chip
    /// is on. ClusterRoles are left out: their nodes come from the bindings that name them.
    pub(crate) fn feed_rows(&self) -> Vec<(TopologyKind, FeedRows<'_>)> {
        self.feeds
            .iter()
            .filter_map(|feed| Some((TopologyKind::of_resource_kind(feed.kind)?, feed.rows())))
            .filter(|(kind, _)| *kind != TopologyKind::ClusterRole)
            .collect()
    }

    /// The binding lists of the RBAC layer as a Bindings companion holds them, for a drawer open
    /// over the graph, where no explorer runs a companion. The cluster role bindings are cluster
    /// wide, the role bindings those of the Topology namespace. A feed that does not run reads as
    /// failed, with the reason it is off.
    pub(crate) fn bindings_companion(&self, with_cluster_role_bindings: bool) -> CompanionLists {
        CompanionLists::Bindings {
            role_bindings: self.binding_list(ResourceKind::RoleBindings),
            cluster_role_bindings: with_cluster_role_bindings
                .then(|| self.binding_list(ResourceKind::ClusterRoleBindings)),
        }
    }

    fn binding_list(&self, kind: ResourceKind) -> LiveList<BindingSummary> {
        let Some(feed) = self.feeds.iter().find(|feed| feed.kind == kind) else {
            return LiveList::Failed {
                message: "the RBAC layer is off".to_owned(),
            };
        };
        if let Some(reason) = &feed.off {
            return LiveList::Failed {
                message: reason.clone(),
            };
        }
        match &feed.list {
            LiveList::Loading => LiveList::Loading,
            LiveList::Failed { message } => LiveList::Failed {
                message: message.clone(),
            },
            LiveList::Ready {
                items,
                interruption,
            } => LiveList::Ready {
                items: items
                    .iter()
                    .filter_map(|row| match &row.object {
                        KindObject::Binding(binding) => Some(binding.clone()),
                        _ => None,
                    })
                    .collect(),
                interruption: interruption.clone(),
            },
        }
    }

    pub(crate) fn remove(&mut self, kinds: &[ResourceKind]) {
        self.feeds.retain(|feed| !kinds.contains(&feed.kind));
    }

    /// Whether a feed that runs has not delivered its first snapshot. A feed that failed is not
    /// pending: its watch keeps retrying, and the graph draws without it.
    pub(crate) fn has_pending(&self) -> bool {
        self.feeds
            .iter()
            .any(|feed| feed.is_open() && feed.list.is_loading())
    }
}

#[cfg(test)]
mod tests {
    use cluster::{AccessCheck, AccessDecision, AccessReport, AccessReview};

    use super::*;

    fn subject(namespace: &str, kinds: &[KindFilter]) -> TopologySubject {
        TopologySubject {
            namespace: namespace.to_owned(),
            kinds: kinds.iter().copied().collect(),
        }
    }

    fn all_chips() -> TopologySubject {
        subject("shop", &KindFilter::ALL)
    }

    fn without_config() -> TopologySubject {
        subject(
            "shop",
            &[
                KindFilter::Ingress,
                KindFilter::Service,
                KindFilter::Workload,
            ],
        )
    }

    fn feed(kind: ResourceKind, list: LiveList<KindRow>) -> TopologyFeed {
        TopologyFeed::watching(kind, list)
    }

    fn feeds_of(feeds: Vec<TopologyFeed>) -> TopologyFeeds {
        TopologyFeeds {
            subject: all_chips(),
            feeds,
        }
    }

    fn report(allowed: &[AccessCheck]) -> AccessState {
        let reviews = AccessCheck::ALL
            .into_iter()
            .map(|check| AccessReview {
                check,
                decision: if allowed.contains(&check) {
                    AccessDecision::Allowed
                } else {
                    AccessDecision::Denied { reason: None }
                },
            })
            .collect();
        AccessState::Known(AccessReport { reviews })
    }

    #[test]
    fn plan_starts_all_kinds_while_checking() {
        assert_eq!(all_chips().wanted_kinds().len(), TOPOLOGY_FEED_KINDS.len());
        for kind in TOPOLOGY_FEED_KINDS {
            assert_eq!(feed_plan(kind, &AccessState::Unknown), FeedStart::Start);
        }
    }

    #[test]
    fn plan_marks_denied_kind_off() {
        let access = report(&[AccessCheck::ListServices]);
        assert_eq!(
            feed_plan(ResourceKind::Secrets, &access),
            FeedStart::Off("not permitted".to_owned())
        );
        assert_eq!(feed_plan(ResourceKind::Services, &access), FeedStart::Start);
    }

    #[test]
    fn chip_off_kinds_get_no_feed() {
        let wanted = without_config().wanted_kinds();
        assert_eq!(wanted.len(), TOPOLOGY_FEED_KINDS.len() - 8);
        for kind in [
            ResourceKind::ConfigMaps,
            ResourceKind::Secrets,
            ResourceKind::PersistentVolumeClaims,
        ] {
            assert!(!wanted.contains(&kind));
        }
    }

    fn default_chips() -> TopologySubject {
        subject("shop", &KindFilter::DEFAULT)
    }

    const RBAC_KINDS: [ResourceKind; 5] = [
        ResourceKind::ServiceAccounts,
        ResourceKind::RoleBindings,
        ResourceKind::Roles,
        ResourceKind::ClusterRoleBindings,
        ResourceKind::ClusterRoles,
    ];

    #[test]
    fn rbac_chip_starts_five_feeds() {
        let wanted = all_chips().wanted_kinds();
        for kind in RBAC_KINDS {
            assert!(wanted.contains(&kind), "{kind:?}");
        }
        let default = default_chips().wanted_kinds();
        assert_eq!(wanted.len(), default.len() + 5);
    }

    #[test]
    fn default_chips_leave_rbac_off() {
        let wanted = default_chips().wanted_kinds();
        assert_eq!(wanted.len(), 10);
        assert!(RBAC_KINDS.iter().all(|kind| !wanted.contains(kind)));
        // Turning the chip on adds exactly the five feeds and stops none.
        let mut with_rbac = default_chips();
        with_rbac.kinds.insert(KindFilter::Rbac);
        assert_eq!(
            subject_change(Some(&default_chips()), Some(&with_rbac)),
            SubjectChange::Adjust {
                stop: Vec::new(),
                start: RBAC_KINDS.to_vec(),
            }
        );
        assert_eq!(
            subject_change(Some(&with_rbac), Some(&default_chips())),
            SubjectChange::Adjust {
                stop: RBAC_KINDS.to_vec(),
                start: Vec::new(),
            }
        );
    }

    #[test]
    fn denied_rbac_feed_is_off() {
        let access = report(&[AccessCheck::ListServiceAccounts, AccessCheck::ListRoles]);
        for kind in [
            ResourceKind::RoleBindings,
            ResourceKind::ClusterRoleBindings,
        ] {
            assert_eq!(
                feed_plan(kind, &access),
                FeedStart::Off("not permitted".to_owned()),
                "{kind:?}"
            );
        }
        for kind in [ResourceKind::ServiceAccounts, ResourceKind::Roles] {
            assert_eq!(feed_plan(kind, &access), FeedStart::Start, "{kind:?}");
        }
    }

    #[test]
    fn open_count_is_at_most_fifteen() {
        let feeds: Vec<TopologyFeed> = all_chips()
            .wanted_kinds()
            .into_iter()
            .map(|kind| feed(kind, LiveList::Loading))
            .collect();
        assert_eq!(feeds_of(feeds).open_count(), 15);
        assert_eq!(TOPOLOGY_FEED_KINDS.len(), 15);
    }

    #[test]
    fn open_count_skips_off_feeds() {
        let feeds = feeds_of(vec![
            TopologyFeed::off(ResourceKind::Secrets, "not permitted".to_owned()),
            feed(ResourceKind::Services, LiveList::Loading),
        ]);
        assert_eq!(feeds.open_count(), 1);
        assert!(feeds.feeds[0].list.is_loading() && !feeds.feeds[0].is_open());
    }

    #[test]
    fn rows_reports_loading_ready_off() {
        let feeds = feeds_of(vec![
            TopologyFeed::off(ResourceKind::Secrets, "not permitted".to_owned()),
            feed(ResourceKind::Services, LiveList::Loading),
            feed(
                ResourceKind::Deployments,
                LiveList::Ready {
                    items: Vec::new(),
                    interruption: None,
                },
            ),
        ]);
        let rows = feeds.feed_rows();
        let find = |kind| {
            &rows
                .iter()
                .find(|(k, _)| *k == kind)
                .expect("feed listed")
                .1
        };
        assert!(matches!(find(TopologyKind::Secret), FeedRows::Off));
        assert!(matches!(find(TopologyKind::Service), FeedRows::Loading));
        assert!(matches!(find(TopologyKind::Deployment), FeedRows::Ready(_)));
        // A kind whose chip is off has no feed, so it is not listed at all.
        assert!(
            rows.iter()
                .all(|(kind, _)| *kind != TopologyKind::ConfigMap)
        );
    }

    #[test]
    fn a_feed_that_failed_reads_as_failed_not_loading() {
        let failed = feed(
            ResourceKind::Services,
            LiveList::Failed {
                message: "timed out".to_owned(),
            },
        );
        let feeds = feeds_of(vec![failed]);
        assert!(matches!(feeds.feed_rows()[0].1, FeedRows::Failed));
    }

    #[test]
    fn a_failed_feed_is_not_pending() {
        let failed = feed(
            ResourceKind::Services,
            LiveList::Failed {
                message: "timed out".to_owned(),
            },
        );
        assert!(!feeds_of(vec![failed]).has_pending());
        let loading = feed(ResourceKind::Services, LiveList::Loading);
        assert!(feeds_of(vec![loading]).has_pending());
        let off = TopologyFeed::off(ResourceKind::Secrets, "not permitted".to_owned());
        assert!(!feeds_of(vec![off]).has_pending());
    }

    #[test]
    fn no_subject_stops_and_a_first_one_starts() {
        assert_eq!(
            subject_change(Some(&all_chips()), None),
            SubjectChange::Stop
        );
        assert_eq!(
            subject_change(None, Some(&all_chips())),
            SubjectChange::Restart
        );
    }

    #[test]
    fn new_namespace_restarts_every_feed() {
        let other = subject("blog", &KindFilter::ALL);
        assert_eq!(
            subject_change(Some(&all_chips()), Some(&other)),
            SubjectChange::Restart
        );
    }

    fn binding_feed(kind: ResourceKind, rows: Vec<KindRow>) -> TopologyFeed {
        feed(
            kind,
            LiveList::Ready {
                items: rows,
                interruption: None,
            },
        )
    }

    fn reader_binding() -> KindRow {
        use crate::access_rows::role_binding_row;
        use crate::topology_fixtures::{NAMESPACE, account_subject, binding};
        role_binding_row(&binding(
            Some(NAMESPACE),
            "reader-binding",
            (cluster::RoleKind::Role, "reader"),
            vec![account_subject("api")],
        ))
    }

    #[test]
    fn rbac_feeds_load_a_drawer_over_topology() {
        let feeds = feeds_of(vec![
            binding_feed(ResourceKind::RoleBindings, vec![reader_binding()]),
            binding_feed(ResourceKind::ClusterRoleBindings, Vec::new()),
        ]);
        let companion = feeds.bindings_companion(true);
        let crate::access_bindings::BindingsStatus::Ready(lists) =
            crate::access_bindings::bindings_status(
                ResourceKind::ClusterRoles,
                &AccessState::Unknown,
                Some(&companion),
            )
        else {
            panic!("both feeds are loaded");
        };
        assert_eq!(lists.role_bindings.len(), 1);
        assert_eq!(lists.role_bindings[0].name, "reader-binding");
        assert!(lists.cluster_role_bindings.is_empty());
    }

    #[test]
    fn empty_rbac_feeds_are_ready_not_loading() {
        let feeds = feeds_of(vec![binding_feed(ResourceKind::RoleBindings, Vec::new())]);
        // Roles do not read the cluster role bindings, so their missing feed does not matter.
        let companion = feeds.bindings_companion(false);
        assert!(crate::access_bindings::ready_binding_lists(Some(&companion)).is_some());
    }

    #[test]
    fn a_loading_rbac_feed_keeps_the_drawer_loading() {
        let feeds = feeds_of(vec![
            binding_feed(ResourceKind::RoleBindings, Vec::new()),
            feed(ResourceKind::ClusterRoleBindings, LiveList::Loading),
        ]);
        let companion = feeds.bindings_companion(true);
        assert!(crate::access_bindings::ready_binding_lists(Some(&companion)).is_none());
        assert!(matches!(
            crate::access_bindings::bindings_status(
                ResourceKind::ClusterRoles,
                &AccessState::Unknown,
                Some(&companion),
            ),
            crate::access_bindings::BindingsStatus::Loading
        ));
    }

    #[test]
    fn an_off_or_missing_rbac_feed_fails_with_its_reason() {
        let feeds = feeds_of(vec![TopologyFeed::off(
            ResourceKind::RoleBindings,
            "not permitted".to_owned(),
        )]);
        let CompanionLists::Bindings {
            role_bindings,
            cluster_role_bindings: Some(cluster_role_bindings),
        } = feeds.bindings_companion(true)
        else {
            panic!("both lists were asked for");
        };
        assert_eq!(role_bindings.failure(), Some("not permitted"));
        assert_eq!(
            cluster_role_bindings.failure(),
            Some("the RBAC layer is off")
        );
    }
}
