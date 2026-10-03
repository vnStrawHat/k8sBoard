//! The `?` shortcut sheet: the key map as a grid of label and keys. The keys are read from the
//! live keymap, so the sheet cannot drift from the bindings, and the kit `Kbd` formats them for
//! the OS.

use gpui_kit::component::kbd::Kbd;
use gpui_kit::component::{ActiveTheme as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::{
    Action, App, AsKeystroke as _, IntoElement, Keystroke, ParentElement as _, Styled as _, Window,
    div, prelude::FluentBuilder as _, px,
};

use crate::cluster_switcher::SwitchToCluster1;
use crate::keymap::{ShortcutGroup, ShortcutRow, shortcut_rows};

const SHEET_WIDTH: f32 = 720.;

/// The wireframe line under its key grid.
const SHEET_NOTE: &str = "Single-letter keys work only while a resource is selected and no text field has focus. Destructive actions always open a confirmation.";

/// Opens the sheet in a kit dialog, which traps focus, closes on Esc, and restores focus.
pub(crate) fn open_shortcut_sheet(window: &mut Window, cx: &mut App) {
    window.open_dialog(cx, |dialog, _, cx| {
        dialog
            .title("Keyboard shortcuts")
            .w(px(SHEET_WIDTH))
            .child(shortcut_sheet(cx))
    });
}

/// The grid alone, shared by the dialog and Settings › Keyboard Shortcuts. It is rebuilt on every
/// render: about 30 rows are too few to cache.
pub(crate) fn shortcut_sheet(cx: &App) -> impl IntoElement + use<> {
    let theme = cx.theme();
    let rows = shortcut_rows();
    v_flex()
        .gap_4()
        .children(ShortcutGroup::ALL.into_iter().map(|group| {
            v_flex()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(group.title()),
                )
                .child(
                    h_flex().flex_wrap().children(
                        rows.iter()
                            .filter(|row| row.group == group)
                            .map(|row| sheet_row(row, cx)),
                    ),
                )
        }))
        .child(
            div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(SHEET_NOTE),
        )
}

fn sheet_row(row: &ShortcutRow, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    // One row stands for the keys 1 to 9; the binding that is looked up is the first.
    let has_range = row.action.as_any().is::<SwitchToCluster1>();
    h_flex()
        .w_1_2()
        .px_2()
        .py_1()
        .gap_2()
        .items_center()
        .justify_between()
        .child(div().text_sm().child(row.label))
        .child(
            h_flex()
                .gap_1()
                .items_center()
                .children(row_keys(&*row.action, cx).into_iter().map(Kbd::new))
                .when(has_range, |keys| {
                    keys.child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child("… 9"),
                    )
                }),
        )
}

/// The first keystroke of each binding of `action`, without repeats, in registration order. The
/// same key bound in two contexts shows once.
pub(crate) fn row_keys(action: &dyn Action, cx: &App) -> Vec<Keystroke> {
    let keymap = cx.key_bindings();
    let keymap = keymap.borrow();
    let mut keys: Vec<Keystroke> = Vec::new();
    for binding in keymap.bindings_for_action(action) {
        let Some(first) = binding.keystrokes().first() else {
            continue;
        };
        let key = first.as_keystroke();
        if !keys.contains(key) {
            keys.push(key.clone());
        }
    }
    keys
}

#[cfg(test)]
#[path = "shortcut_sheet_tests.rs"]
mod shortcut_sheet_tests;
