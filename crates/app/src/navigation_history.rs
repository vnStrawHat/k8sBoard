//! Back and forward through the places a link left (spec 0056 history). Pure: the shell reads its
//! state into a `Place` and writes one back, so the stacks are tested without a window.

use crate::app_shell::Screen;

use crate::drawer::{ContainerTab, DrawerTab};

use crate::table_filter::TableFilter;
use crate::table_selection::{ClusterObject, ResourceKey};

/// The most places `back` (and `forward`) keep; the oldest is dropped.
const HISTORY_CAP: usize = 50;
/// The most characters of the previous name the Back button shows.
const BACK_LABEL_MAX: usize = 20;

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

    /// The place Back would restore; the drawer header's Back button names it.
    pub(crate) fn previous(&self) -> Option<&Place> {
        self.back.last()
    }

    pub(crate) fn clear(&mut self) {
        self.back.clear();
        self.forward.clear();
    }
}

impl Place {
    /// The Back button text: the object's name, cut to 20 characters, or the screen title for a
    /// place without a selection.
    pub(crate) fn back_label(&self) -> String {
        let name = match &self.selection {
            Some(object) => key_name(&object.key),
            None => screen_title(self.screen),
        };
        if name.chars().count() <= BACK_LABEL_MAX {
            return name.to_owned();
        }
        let kept: String = name.chars().take(BACK_LABEL_MAX - 1).collect();
        format!("{kept}…")
    }

    /// The Back button tooltip, such as `Back to Service api (Alt+Left)`.
    pub(crate) fn back_tooltip(&self) -> String {
        match &self.selection {
            Some(object) => format!(
                "Back to {} {} (Alt+Left)",
                key_kind_name(&object.key),
                key_name(&object.key)
            ),
            None => format!("Back to {} (Alt+Left)", screen_title(self.screen)),
        }
    }
}

fn key_name(key: &ResourceKey) -> &str {
    match key {
        ResourceKey::Pod { name, .. }
        | ResourceKey::Node { name }
        | ResourceKey::Kind { name, .. } => name,
    }
}

fn key_kind_name(key: &ResourceKey) -> &'static str {
    match key {
        ResourceKey::Pod { .. } => "Pod",
        ResourceKey::Node { .. } => "Node",
        ResourceKey::Kind { kind, .. } => kind.display_name(),
    }
}

/// The screen as the header names it.
fn screen_title(screen: Screen) -> &'static str {
    match screen {
        Screen::Overview => "Overview",
        Screen::Pods => "Pods",
        Screen::Nodes => "Nodes",
        Screen::Issues => "Issues",
        Screen::Topology => "Topology",
        Screen::PortForwarding => "Port Forwarding",
        Screen::Kind(kind) => kind.label(),
    }
}

/// The 1-based position of the cursor row among the visible rows, for `12 of 40`. `None` when
/// there is no cursor or it lies past the end (the subject is not a visible row).
pub(crate) fn row_position(
    selected_row: Option<usize>,
    visible_rows: usize,
) -> Option<(usize, usize)> {
    let row = selected_row.filter(|row| *row < visible_rows)?;
    Some((row + 1, visible_rows))
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
