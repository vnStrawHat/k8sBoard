//! The lazy write permission of one kind (spec 0031 decision 24): `update {resource}` is asked when
//! a screen of the kind first shows, not at session start, so connecting costs no burst of write
//! checks. Until the answer arrives the gate reads `Checking permissions…`.

use cluster::{AccessReport, ObjectKind};
use gpui_kit::Task;

/// What the session knows about `update` on one kind.
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
