//! Back and forward through the places a link left (spec 0056 history). Pure: the shell reads its
//! state into a `Place` and writes one back, so the stacks are tested without a window.

use crate::app_shell::Screen;
use crate::drawer::{ContainerTab, DrawerTab};
use crate::table_filter::TableFilter;
use crate::table_selection::ClusterObject;

/// The most places `back` (and `forward`) keep; the oldest is dropped.
const HISTORY_CAP: usize = 50;

/// What the shell showed when a link was followed. Scroll offset, sort, hidden columns and the
/// Monitor range are not stored: the first is restored by revealing the row, the next two live in
/// the per-screen table view, and the Monitor resets on any screen change.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Place {
    pub(crate) screen: Screen,
    pub(crate) selection: Option<ClusterObject>,
    pub(crate) is_drawer_open: bool,
    pub(crate) tab: DrawerTab,
    /// The Pod drawer's selected container.
    pub(crate) container: Option<String>,
    pub(crate) container_tab: ContainerTab,
    /// The origin table's filter, which a same-screen link may clear to show its target; `None`
    /// for a screen without a table.
    pub(crate) filter: Option<TableFilter>,
}

#[derive(Default)]
pub(crate) struct NavigationHistory {
    back: Vec<Place>,
    forward: Vec<Place>,
}

impl NavigationHistory {
    /// A link was followed away from `from`: it becomes the Back target and the forward stack is
    /// forgotten.
    pub(crate) fn record(&mut self, from: Place) {
        push_capped(&mut self.back, from);
        self.forward.clear();
    }

    /// Steps back from `current`, which goes onto the forward stack. Places `is_served` rejects
    /// (a custom kind that is gone) are consumed on the way. `None` leaves both stacks as they are
    /// when no place is left.
    pub(crate) fn back(
        &mut self,
        current: Place,
        is_served: impl Fn(&Place) -> bool,
    ) -> Option<Place> {
        let place = pop_served(&mut self.back, is_served)?;
        push_capped(&mut self.forward, current);
        Some(place)
    }

    /// The mirror of `back`.
    pub(crate) fn forward(
        &mut self,
        current: Place,
        is_served: impl Fn(&Place) -> bool,
    ) -> Option<Place> {
        let place = pop_served(&mut self.forward, is_served)?;
        push_capped(&mut self.back, current);
        Some(place)
    }

    /// The place Back would restore. The header's Back button label reads it (spec 0056 A2), so
    /// until that lands only the tests do.
    #[cfg(test)]
    pub(crate) fn previous(&self) -> Option<&Place> {
        self.back.last()
    }

    pub(crate) fn clear(&mut self) {
        self.back.clear();
        self.forward.clear();
    }
}

fn push_capped(stack: &mut Vec<Place>, place: Place) {
    if stack.len() == HISTORY_CAP {
        stack.remove(0);
    }
    stack.push(place);
}

fn pop_served(stack: &mut Vec<Place>, is_served: impl Fn(&Place) -> bool) -> Option<Place> {
    while let Some(place) = stack.pop() {
        if is_served(&place) {
            return Some(place);
        }
    }
    None
}

#[cfg(test)]
#[path = "navigation_history_tests.rs"]
mod navigation_history_tests;
