use gpui_kit::TestAppContext;

use super::*;
use crate::keymap::{SelectNextRow, ShowShortcuts, bind_keys};

fn keys_of(action: &dyn Action, cx: &mut TestAppContext) -> Vec<String> {
    cx.update(|cx| {
        gpui_kit::init(cx);
        bind_keys(cx);
        row_keys(action, cx)
            .into_iter()
            .map(|keystroke| keystroke.key)
            .collect()
    })
}

#[gpui_kit::test]
fn a_row_lists_each_key_once_in_registration_order(cx: &mut TestAppContext) {
    // `down` is bound in the workspace and in the table, and shows once.
    assert_eq!(keys_of(&SelectNextRow, cx), ["j", "down"]);
}

#[gpui_kit::test]
fn the_question_mark_row_shows_the_character(cx: &mut TestAppContext) {
    assert_eq!(keys_of(&ShowShortcuts, cx), ["?"]);
}

#[gpui_kit::test]
fn shortcut_sheet_lists_attach(cx: &mut TestAppContext) {
    assert_eq!(keys_of(&crate::keymap::Attach, cx), ["a"]);
    assert!(
        crate::keymap::shortcut_rows()
            .iter()
            .any(|row| row.label.starts_with("Attach"))
    );
}

#[test]
fn every_label_fits_beside_its_keys_in_a_half_width_column() {
    // A half column of the 720 px sheet is about 50 characters wide at the body size, less the keys.
    for row in shortcut_rows() {
        assert!(row.label.chars().count() <= 48, "{}", row.label);
    }
}
