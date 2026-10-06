//! The palette's name index (spec 0056): the names of the kinds the live feeds and the visible
//! screen do not hold (Services, Ingresses, StatefulSets, CronJobs, NetworkPolicies), listed once
//! while the user types and kept by the session. Only namespace and name are stored; nothing of an
//! object or a query is kept, logged, or traced.

use std::time::{Duration, Instant};

use cluster::{AccessCheck, NameList, NamespaceScope, ObjectName};
use gpui_kit::Task;

use crate::cluster_session::AccessState;
use crate::resource_kind::ResourceKind;

/// The kinds the index lists, in the order the palette names them.
pub(crate) const NAME_INDEX_KINDS: [ResourceKind; 5] = [
    ResourceKind::Services,
    ResourceKind::Ingresses,
    ResourceKind::StatefulSets,
    ResourceKind::CronJobs,
    ResourceKind::NetworkPolicies,
];

/// The shortest query text that starts a run: one character would list five kinds for nothing.
pub(crate) const NAME_INDEX_MIN_CHARS: usize = 2;

/// How long a finished run, failed or not, keeps the next one from starting.
const NAME_INDEX_MAX_AGE: Duration = Duration::from_secs(120);

/// What one kind's list is now.
pub(crate) enum NameListState {
    Loading,
    Ready(NameList),
    /// The known access report denies listing this kind, so it was not asked for.
    Denied,
    /// The server refused or failed; the error is logged when the run finishes.
    Failed,
}

/// One kind's outcome of a run, as the runtime hands it back.
pub(crate) type NameListResult = (ResourceKind, Result<NameList, String>);

/// The names of the index kinds in one scope. A scope change replaces the whole value, which drops
/// (and so aborts) a running list.
#[derive(Default)]
pub(crate) struct NameIndex {
    /// The scope the lists, or the run that is filling them, belong to.
    scope: Option<NamespaceScope>,
    /// When the last run finished, failed or not.
    fetched_at: Option<Instant>,
    lists: Vec<(ResourceKind, NameListState)>,
    run: Option<Task<()>>,
}

/// What the palette says about the index, in kinds.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct IndexSummary {
    pub(crate) searching: Vec<ResourceKind>,
    pub(crate) searched: Vec<ResourceKind>,
    pub(crate) not_permitted: Vec<ResourceKind>,
    pub(crate) unavailable: Vec<ResourceKind>,
    /// Searched, but cut at the name limit.
    pub(crate) truncated: Vec<ResourceKind>,
}

/// The index kinds to list and the ones the known report denies, in `NAME_INDEX_KINDS` order.
pub(crate) fn name_index_plan(access: &AccessState) -> Vec<(ResourceKind, Option<AccessCheck>)> {
    NAME_INDEX_KINDS
        .into_iter()
        .map(|kind| {
            let denied = match (access, kind.access_check()) {
                (AccessState::Known(report), Some(check)) if !report.is_allowed(check) => {
                    Some(check)
                }
                _ => None,
            };
            (kind, denied)
        })
        .collect()
}

impl NameIndex {
    /// Whether a run should start now: none is running, and the lists were never fetched, belong
    /// to another scope, or were fetched 120 s ago. A failed run counts as fetched, so a failure
    /// is retried after the age, not on the next keystroke.
    pub(crate) fn wants_run(&self, scope: &NamespaceScope, now: Instant) -> bool {
        if self.run.is_some() {
            return false;
        }
        if self.scope.as_ref() != Some(scope) {
            return true;
        }
        self.fetched_at
            .is_none_or(|at| now.saturating_duration_since(at) >= NAME_INDEX_MAX_AGE)
    }

    /// Records a run that starts now. Lists that are ready for this scope stay searchable while
    /// the new ones load; a denied kind is not asked for.
    pub(crate) fn begin(
        &mut self,
        scope: NamespaceScope,
        plan: &[(ResourceKind, Option<AccessCheck>)],
        run: Task<()>,
    ) {
        let is_same_scope = self.scope.as_ref() == Some(&scope);
        let mut previous = std::mem::take(&mut self.lists);
        self.lists = plan
            .iter()
            .map(|(kind, denied)| {
                let state = match denied {
                    Some(_) => NameListState::Denied,
                    None => previous
                        .iter()
                        .position(|(other, _)| other == kind)
                        .map(|index| previous.swap_remove(index).1)
                        .filter(|state| is_same_scope && matches!(state, NameListState::Ready(_)))
                        .unwrap_or(NameListState::Loading),
                };
                (*kind, state)
            })
            .collect();
        self.scope = Some(scope);
        self.run = Some(run);
    }

    /// Applies a finished run and stamps `fetched_at`. A result of another scope is discarded:
    /// `false`. A list the results do not answer (a run that stopped) fails.
    pub(crate) fn finish(
        &mut self,
        scope: &NamespaceScope,
        results: Vec<NameListResult>,
        now: Instant,
    ) -> bool {
        if self.scope.as_ref() != Some(scope) {
            return false;
        }
        self.run = None;
        self.fetched_at = Some(now);
        for (kind, result) in results {
            let state = match result {
                Ok(list) => NameListState::Ready(list),
                Err(_) => NameListState::Failed,
            };
            if let Some(slot) = self.lists.iter_mut().find(|(other, _)| *other == kind) {
                slot.1 = state;
            }
        }
        for (_, state) in &mut self.lists {
            if matches!(state, NameListState::Loading) {
                *state = NameListState::Failed;
            }
        }
        true
    }

    /// Every name of every ready list, with its kind.
    pub(crate) fn ready_names(&self) -> impl Iterator<Item = (ResourceKind, &ObjectName)> {
        self.lists.iter().flat_map(|(kind, state)| {
            match state {
                NameListState::Ready(list) => list.names.as_slice(),
                _ => &[],
            }
            .iter()
            .map(|name| (*kind, name))
        })
    }

    /// Whether a run has not delivered its names. Only the screenshot hook waits on it.
    #[cfg(feature = "screenshot")]
    pub(crate) fn is_running(&self) -> bool {
        self.run.is_some()
    }

    pub(crate) fn summary(&self) -> IndexSummary {
        let mut summary = IndexSummary::default();
        for (kind, state) in &self.lists {
            match state {
                NameListState::Loading => summary.searching.push(*kind),
                NameListState::Ready(list) => {
                    summary.searched.push(*kind);
                    if list.is_truncated {
                        summary.truncated.push(*kind);
                    }
                }
                NameListState::Denied => summary.not_permitted.push(*kind),
                NameListState::Failed => summary.unavailable.push(*kind),
            }
        }
        summary
    }
}

#[cfg(test)]
#[path = "name_index_tests.rs"]
mod name_index_tests;
