//! The lazy write permissions of one kind (spec 0031 decision 24, 0033 decision 25): `update
//! {resource}` (when the kind is editable) and `delete {resource}` are asked when a screen of the
//! kind first shows, not at session start, so connecting costs no burst of write checks. Until the
//! answer arrives the gate reads `Checking permissions…`.

use cluster::{AccessCheck, AccessReport, ObjectKind};
use gpui_kit::Task;

/// The permissions one review of `kind` asks: `update` for an editable kind, `patch` for the two
/// kinds whose values are edited (spec 0047 decision 7) and for the pods and workloads whose labels
/// and annotations are edited (spec 0032b), `delete` for all, and `create` for a kind that gets a
/// `New` button (spec 0042).
pub(crate) fn lazy_checks(kind: ObjectKind) -> Vec<AccessCheck> {
    let update = kind.is_editable().then_some(AccessCheck::Update(kind));
    let patch = matches!(
        kind,
        ObjectKind::ConfigMap
            | ObjectKind::Secret
            | ObjectKind::Pod
            | ObjectKind::Deployment
            | ObjectKind::StatefulSet
            | ObjectKind::DaemonSet
            | ObjectKind::ReplicaSet
            | ObjectKind::Job
            | ObjectKind::CronJob
    )
    .then_some(AccessCheck::Patch(kind));
    let create = kind.is_creatable().then_some(AccessCheck::Create(kind));
    update
        .into_iter()
        .chain(patch)
        .chain([AccessCheck::Delete(kind)])
        .chain(create)
        .collect()
}

/// What the session knows about `update` and `delete` on one kind.
pub(crate) enum KindAccess {
    /// Dropping the task cancels the review.
    Checking {
        _task: Task<()>,
    },
    Known(AccessReport),
    /// The review failed; the gate stays closed and says so.
    Unknown,
}

/// The kinds reviewed so far in one scope. A handful at most, so a list is enough, and an empty one
/// is a `const` the gate can borrow when it has no session.
pub(crate) struct KindAccessMap {
    entries: Vec<(ObjectKind, KindAccess)>,
}

impl KindAccessMap {
    pub(crate) const EMPTY: &'static Self = &Self::new();

    pub(crate) const fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    pub(crate) fn get(&self, kind: ObjectKind) -> Option<&KindAccess> {
        self.entries
            .iter()
            .find(|(known, _)| *known == kind)
            .map(|(_, access)| access)
    }

    /// Records `access` for `kind`, replacing an earlier entry.
    pub(crate) fn set(&mut self, kind: ObjectKind, access: KindAccess) {
        match self.entries.iter_mut().find(|(known, _)| *known == kind) {
            Some((_, slot)) => *slot = access,
            None => self.entries.push((kind, access)),
        }
    }

    /// A scope change makes every answer stale.
    pub(crate) fn clear(&mut self) {
        self.entries.clear();
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }
}

#[cfg(test)]
#[path = "kind_access_tests.rs"]
mod kind_access_tests;
