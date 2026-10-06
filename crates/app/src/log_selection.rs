//! The rows a click and a Shift-click picked in a log tab, as indexes into the visible list.

use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RowSelection {
    /// The row of the plain click; Shift-click moves only `end`.
    anchor: usize,
    end: usize,
}

impl RowSelection {
    pub(crate) fn single(index: usize) -> Self {
        Self {
            anchor: index,
            end: index,
        }
    }

    /// The range from the anchor to `index`, whichever side it falls on.
    pub(crate) fn extended_to(self, index: usize) -> Self {
        Self { end: index, ..self }
    }

    pub(crate) fn rows(self) -> Range<usize> {
        self.anchor.min(self.end)..self.anchor.max(self.end) + 1
    }

    pub(crate) fn contains(self, index: usize) -> bool {
        self.rows().contains(&index)
    }

    /// The selection after `removed` rows left the front of the list; `None` once every selected
    /// row is gone. A range that straddles the cut keeps its surviving rows.
    pub(crate) fn after_front_removal(self, removed: usize) -> Option<Self> {
        if self.anchor.max(self.end) < removed {
            return None;
        }
        Some(Self {
            anchor: self.anchor.saturating_sub(removed),
            end: self.end.saturating_sub(removed),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_click_selects_one_row() {
        let selection = RowSelection::single(4);
        assert_eq!(selection.rows(), 4..5);
        assert!(selection.contains(4));
        assert!(!selection.contains(3));
    }

    #[test]
    fn shift_click_extends_from_the_anchor_in_either_direction() {
        let down = RowSelection::single(4).extended_to(7);
        assert_eq!(down.rows(), 4..8);
        let up = RowSelection::single(4).extended_to(1);
        assert_eq!(up.rows(), 1..5);
    }

    #[test]
    fn a_second_shift_click_keeps_the_first_anchor() {
        let selection = RowSelection::single(4).extended_to(9).extended_to(2);
        assert_eq!(selection.rows(), 2..5);
    }

    #[test]
    fn front_removal_shifts_the_selection_up() {
        let selection = RowSelection::single(5).extended_to(7);
        assert_eq!(
            selection.after_front_removal(3).map(RowSelection::rows),
            Some(2..5)
        );
    }

    #[test]
    fn front_removal_clips_a_straddling_range_and_drops_a_gone_one() {
        let straddling = RowSelection::single(1).extended_to(6);
        assert_eq!(
            straddling.after_front_removal(3).map(RowSelection::rows),
            Some(0..4)
        );
        assert_eq!(RowSelection::single(2).after_front_removal(3), None);
    }
}
