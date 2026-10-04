//! Key resolution without a window: the keymap is filled by `gpui_kit::init` and the app's
//! `bind_keys`, then asked what a key does under a context stack, as GPUI asks it when a key is
//! pressed.

use gpui_kit::{AsKeystroke as _, KeyContext, Keystroke, Modifiers, TestAppContext};

use super::*;
use crate::cluster_switcher::{
    CloseClusterSwitcher, SwitcherConfirm, SwitcherNext, SwitcherPrevious,
};

/// Keys that a later spec binds. No binding of this spec may take one; the owner removes the key
/// from this list in the change that binds it.
const RESERVED_KEYS: [&str; 0] = [];

fn bind_all(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        bind_keys(cx);
        crate::cluster_switcher::bind_keys(cx);
    });
}

fn stack(contexts: &[&str]) -> Vec<KeyContext> {
    contexts
        .iter()
        .map(|context| KeyContext::parse(context).expect("a valid context"))
        .collect()
}

/// The name of the action the first matching binding runs, as GPUI would dispatch it.
fn resolve_typed(
    typed: &Keystroke,
    contexts: &[&str],
    cx: &mut TestAppContext,
) -> Option<&'static str> {
    let contexts = stack(contexts);
    cx.update(|cx| {
        let keymap = cx.key_bindings();
        let keymap = keymap.borrow();
        let (bindings, _) = keymap.bindings_for_input(std::slice::from_ref(typed), &contexts);
        bindings.first().map(|binding| binding.action().name())
    })
}

fn resolve(key: &str, contexts: &[&str], cx: &mut TestAppContext) -> Option<&'static str> {
    let typed = Keystroke::parse(key).expect("a valid keystroke");
    resolve_typed(&typed, contexts, cx)
}

fn is_app_action(name: Option<&str>) -> bool {
    name.is_some_and(|name| name.starts_with("k8sboard::"))
}

/// The app's own bindings, not the kit's.
fn app_bindings(cx: &mut TestAppContext) -> Vec<KeyBinding> {
    cx.update(|cx| {
        cx.key_bindings()
            .borrow()
            .bindings()
            .filter(|binding| binding.action().name().starts_with("k8sboard::"))
            .cloned()
            .collect()
    })
}

const SHELL: [&str; 2] = ["Root", "AppShell"];
const TABLE_PATH: [&str; 3] = ["Root", "AppShell", "DataTable"];
const INPUT_PATH: [&str; 3] = ["Root", "AppShell", "Input"];

#[gpui_kit::test]
fn letters_resolve_in_the_workspace(cx: &mut TestAppContext) {
    bind_all(cx);
    assert_eq!(resolve("l", &SHELL, cx), Some("k8sboard::ViewLogs"));
    assert_eq!(resolve("l", &TABLE_PATH, cx), Some("k8sboard::ViewLogs"));
    assert_eq!(resolve("y", &SHELL, cx), Some("k8sboard::ViewYaml"));
    assert_eq!(resolve("shift-s", &SHELL, cx), Some("k8sboard::Scale"));
    assert_eq!(resolve("s", &SHELL, cx), Some("k8sboard::OpenShell"));
    assert_eq!(resolve("delete", &SHELL, cx), Some("k8sboard::Delete"));
}

#[gpui_kit::test]
fn letters_do_nothing_in_text_inputs(cx: &mut TestAppContext) {
    bind_all(cx);
    for key in [
        "?", "/", "j", "k", "[", "]", "enter", "l", "y", "s", "e", "delete",
    ] {
        let name = resolve(key, &INPUT_PATH, cx);
        assert!(!is_app_action(name), "{key} resolved to {name:?}");
    }
}

#[gpui_kit::test]
fn single_keys_do_nothing_in_menus_popovers_and_dialogs(cx: &mut TestAppContext) {
    bind_all(cx);
    let paths: [&[&str]; 4] = [
        &["Root", "AppShell", "PopupMenu"],
        &["Root", "AppShell", "Popover"],
        &["Root", "AppShell", "DataTable", "PopupMenu"],
        &["Root", "Dialog"],
    ];
    for path in paths {
        for key in ["?", "/", "j", "y", "l", "[", "enter", "escape"] {
            let name = resolve(key, path, cx);
            assert!(!is_app_action(name), "{key} under {path:?}: {name:?}");
        }
    }
}

#[gpui_kit::test]
fn table_arrows_outrank_the_kit_table(cx: &mut TestAppContext) {
    bind_all(cx);
    for (key, action) in [
        ("down", "SelectNextRow"),
        ("up", "SelectPreviousRow"),
        ("home", "SelectFirstRow"),
        ("end", "SelectLastRow"),
        ("pageup", "SelectPreviousPage"),
        ("pagedown", "SelectNextPage"),
    ] {
        assert_eq!(
            resolve(key, &TABLE_PATH, cx),
            Some(format!("k8sboard::{action}").as_str()),
            "{key}"
        );
    }
}

#[gpui_kit::test]
fn table_escape_outranks_the_kit_table(cx: &mut TestAppContext) {
    bind_all(cx);
    assert_eq!(
        resolve("escape", &TABLE_PATH, cx),
        Some("k8sboard::Dismiss")
    );
}

#[gpui_kit::test]
fn escape_leaves_the_quick_filter(cx: &mut TestAppContext) {
    bind_all(cx);
    for field in ["QuickFilter", "Drawer", "Dock"] {
        assert_eq!(
            resolve("escape", &["Root", "AppShell", field, "Input"], cx),
            Some("k8sboard::LeaveInput"),
            "{field}"
        );
    }
}

#[gpui_kit::test]
fn escape_stays_with_other_inputs(cx: &mut TestAppContext) {
    bind_all(cx);
    // The namespace picker search: a popover without the cluster switcher in its path.
    let path = ["Root", "AppShell", "Popover", "Input"];
    for key in ["escape", "up", "down", "enter"] {
        let name = resolve(key, &path, cx);
        assert!(!is_app_action(name), "{key}: {name:?}");
    }
}

#[gpui_kit::test]
fn chords_work_inside_text_inputs(cx: &mut TestAppContext) {
    bind_all(cx);
    for (key, action) in [
        ("secondary-n", "OpenNamespacePicker"),
        ("secondary-w", "CloseDockTab"),
        ("ctrl-`", "ToggleDock"),
        ("ctrl-tab", "NextDockTab"),
        ("ctrl-shift-tab", "PreviousDockTab"),
        ("secondary-shift-m", "ToggleDockZoom"),
    ] {
        assert_eq!(
            resolve(key, &INPUT_PATH, cx),
            Some(format!("k8sboard::{action}").as_str()),
            "{key}"
        );
    }
}

#[gpui_kit::test]
fn copy_name_is_not_bound_inside_inputs(cx: &mut TestAppContext) {
    bind_all(cx);
    assert_eq!(
        resolve("secondary-c", &SHELL, cx),
        Some("k8sboard::CopyName")
    );
    let name = resolve("secondary-c", &INPUT_PATH, cx);
    assert!(!is_app_action(name), "{name:?}");
    assert!(name.is_some(), "the kit copies inside an input");
}

#[gpui_kit::test]
fn question_mark_matches_a_shifted_slash(cx: &mut TestAppContext) {
    bind_all(cx);
    // A US keyboard sends the key `/` with shift and the character `?`.
    let shifted = Keystroke {
        modifiers: Modifiers::shift(),
        key: "/".to_owned(),
        key_char: Some("?".to_owned()),
    };
    assert_eq!(
        resolve_typed(&shifted, &SHELL, cx),
        Some("k8sboard::ShowShortcuts")
    );
    assert_eq!(resolve("/", &SHELL, cx), Some("k8sboard::FocusQuickFilter"));
}

#[test]
fn secondary_is_the_platform_modifier() {
    let keystroke = Keystroke::parse("secondary-n").expect("a valid keystroke");
    assert_eq!(keystroke.modifiers.platform, cfg!(target_os = "macos"));
    assert_eq!(keystroke.modifiers.control, !cfg!(target_os = "macos"));
}

#[gpui_kit::test]
fn bindings_never_share_a_keystroke_in_one_context(cx: &mut TestAppContext) {
    bind_all(cx);
    let bindings = app_bindings(cx);
    for (index, binding) in bindings.iter().enumerate() {
        for other in &bindings[index + 1..] {
            let same_keys = binding
                .keystrokes()
                .iter()
                .map(|key| key.as_keystroke())
                .eq(other.keystrokes().iter().map(|key| key.as_keystroke()));
            let same_context = binding.predicate() == other.predicate();
            assert!(
                !(same_keys && same_context),
                "{} and {} share {:?}",
                binding.action().name(),
                other.action().name(),
                binding.keystrokes()
            );
        }
    }
}

#[gpui_kit::test]
fn bindings_avoid_reserved_keys(cx: &mut TestAppContext) {
    bind_all(cx);
    let reserved: Vec<Keystroke> = RESERVED_KEYS
        .iter()
        .map(|key| Keystroke::parse(key).expect("a valid reserved key"))
        .collect();
    for binding in app_bindings(cx) {
        for key in binding.keystrokes() {
            assert!(
                !reserved.contains(key.as_keystroke()),
                "{} takes a reserved key",
                binding.action().name()
            );
        }
    }
}

#[gpui_kit::test]
fn every_sheet_row_has_a_binding(cx: &mut TestAppContext) {
    bind_all(cx);
    let bound = app_bindings(cx);
    for row in shortcut_rows() {
        assert!(
            bound
                .iter()
                .any(|binding| binding.action().partial_eq(&*row.action)),
            "{} has no binding",
            row.label
        );
    }
}

#[gpui_kit::test]
fn every_bound_action_is_on_the_sheet(cx: &mut TestAppContext) {
    bind_all(cx);
    let rows = shortcut_rows();
    let without_row: [&dyn Action; 18] = [
        &LeaveInput,
        &CloseTerminalFind,
        &PalettePreview,
        &ScaleCursorRow,
        &CancelValuePopover,
        &LeavePaletteArgument,
        &SwitchToCluster2,
        &SwitchToCluster3,
        &SwitchToCluster4,
        &SwitchToCluster5,
        &SwitchToCluster6,
        &SwitchToCluster7,
        &SwitchToCluster8,
        &SwitchToCluster9,
        &SwitcherNext,
        &SwitcherPrevious,
        &SwitcherConfirm,
        &CloseClusterSwitcher,
    ];
    for binding in app_bindings(cx) {
        let action = binding.action();
        let is_listed = rows.iter().any(|row| action.partial_eq(&*row.action));
        let is_exempt = without_row.iter().any(|exempt| action.partial_eq(*exempt));
        assert!(
            is_listed || is_exempt,
            "{} is not on the sheet",
            action.name()
        );
    }
}

#[gpui_kit::test]
fn space_is_ignored_in_both_switcher_contexts(cx: &mut TestAppContext) {
    bind_all(cx);
    // The kit Popover binds space to Confirm, which would close the popover.
    assert!(
        resolve("space", &["Root", "AppShell", "Popover"], cx).is_some(),
        "the kit binds space in a popover"
    );
    // The switcher is deeper and binds it to NoAction, which resolves to no action at all.
    for path in [
        &["Root", "AppShell", "Popover", "ClusterSwitcher"][..],
        &["Root", "AppShell", "Popover", "ClusterSwitcher", "Input"][..],
    ] {
        assert_eq!(resolve("space", path, cx), None, "{path:?}");
    }
}

#[gpui_kit::test]
fn cluster_switcher_chords_resolve_everywhere(cx: &mut TestAppContext) {
    bind_all(cx);
    let paths: [&[&str]; 3] = [
        &["Root", "AppShell"],
        &["Root", "AppShell", "Input"],
        &["Root", "AppShell", "Popover", "ClusterSwitcher", "Input"],
    ];
    for path in paths {
        assert_eq!(
            resolve("secondary-shift-c", path, cx),
            Some("k8sboard::OpenClusterSwitcher"),
            "{path:?}"
        );
        assert_eq!(
            resolve("secondary-1", path, cx),
            Some("k8sboard::SwitchToCluster1"),
            "{path:?}"
        );
    }
}

#[gpui_kit::test]
fn settings_window_gets_no_shell_keys(cx: &mut TestAppContext) {
    bind_all(cx);
    let paths: [&[&str]; 2] = [
        &["Root", "SettingsWindow"],
        &["Root", "SettingsWindow", "Input"],
    ];
    for path in paths {
        for key in ["j", "?", "/", "enter", "l", "secondary-n", "secondary-w"] {
            let name = resolve(key, path, cx);
            assert!(!is_app_action(name), "{key} under {path:?}: {name:?}");
        }
    }
    for path in [
        &["Root", "SettingsWindow", "Input"][..],
        &["Root", "Dialog", "Input"][..],
    ] {
        assert_ne!(
            resolve("escape", path, cx),
            Some("k8sboard::LeaveInput"),
            "{path:?}"
        );
    }
}

#[gpui_kit::test]
fn settings_keys_keep_their_0025_contexts(cx: &mut TestAppContext) {
    bind_all(cx);
    for path in [
        &["Root", "SettingsWindow"][..],
        &["Root", "AppShell"][..],
        &["Root", "Dialog"][..],
    ] {
        assert_eq!(
            resolve("secondary-,", path, cx),
            Some("k8sboard::OpenSettings"),
            "{path:?}"
        );
    }
    assert_eq!(
        resolve("secondary-o", &["Root", "SettingsWindow"], cx),
        Some("k8sboard::ImportKubeconfig")
    );
    assert_ne!(
        resolve("secondary-o", &SHELL, cx),
        Some("k8sboard::ImportKubeconfig")
    );
}

#[gpui_kit::test]
fn no_binding_uses_the_windows_or_super_key(cx: &mut TestAppContext) {
    bind_all(cx);
    // `secondary` is Cmd on macOS, so only there may a binding carry the platform modifier.
    if cfg!(target_os = "macos") {
        return;
    }
    for binding in app_bindings(cx) {
        for key in binding.keystrokes() {
            assert!(
                !key.as_keystroke().modifiers.platform,
                "{}",
                binding.action().name()
            );
        }
    }
}

#[gpui_kit::test]
fn ctrl_k_opens_the_palette_inside_text_fields(cx: &mut TestAppContext) {
    bind_all(cx);
    for path in [&SHELL[..], &INPUT_PATH[..], &TABLE_PATH[..]] {
        assert_eq!(
            resolve("secondary-k", path, cx),
            Some("k8sboard::OpenPalette"),
            "{path:?}"
        );
    }
}

#[gpui_kit::test]
fn colon_opens_kind_mode_outside_text_fields_only(cx: &mut TestAppContext) {
    bind_all(cx);
    // A US keyboard sends the key `;` with shift and the character `:`.
    let colon = Keystroke {
        modifiers: Modifiers::shift(),
        key: ";".to_owned(),
        key_char: Some(":".to_owned()),
    };
    for path in [&SHELL[..], &TABLE_PATH[..]] {
        assert_eq!(
            resolve_typed(&colon, path, cx),
            Some("k8sboard::OpenKindPalette"),
            "{path:?}"
        );
    }
    let name = resolve_typed(&colon, &INPUT_PATH, cx);
    assert!(!is_app_action(name), "{name:?}");
}

#[gpui_kit::test]
fn tab_previews_only_inside_the_palette_query(cx: &mut TestAppContext) {
    bind_all(cx);
    let palette = ["Root", "Dialog", "Command", "Input"];
    assert_eq!(
        resolve("tab", &palette, cx),
        Some("k8sboard::PalettePreview")
    );
    assert!(!is_app_action(resolve("tab", &SHELL, cx)));
    assert!(!is_app_action(resolve("tab", &INPUT_PATH, cx)));
}

#[gpui_kit::test]
fn palette_keys_leave_the_shell_keys_alone_inside_the_dialog(cx: &mut TestAppContext) {
    bind_all(cx);
    // The palette is a dialog outside `AppShell`: no single key reaches the table behind it.
    let palette = ["Root", "Dialog", "Command", "Input"];
    for key in ["j", "k", "l", "y", "enter", "?", "/"] {
        let name = resolve(key, &palette, cx);
        assert!(!is_app_action(name), "{key}: {name:?}");
    }
}

#[gpui_kit::test]
fn toggle_read_only_is_bound_in_window(cx: &mut TestAppContext) {
    bind_all(cx);
    let paths: [&[&str]; 3] = [
        &["Root", "AppShell"],
        &["Root", "AppShell", "DataTable"],
        &["Root", "AppShell", "Input"],
    ];
    for path in paths {
        assert_eq!(
            resolve("secondary-shift-r", path, cx),
            Some("k8sboard::ToggleReadOnly"),
            "{path:?}"
        );
    }
    // A dialog sits outside the shell, so the chord does not reach through one.
    assert_ne!(
        resolve("secondary-shift-r", &["Root", "Dialog"], cx),
        Some("k8sboard::ToggleReadOnly")
    );
}

#[gpui_kit::test]
fn enter_is_suppressed_in_write_confirm(cx: &mut TestAppContext) {
    bind_all(cx);
    let paths: [&[&str]; 2] = [
        &["Root", "Dialog", "WriteConfirm"],
        &["Root", "Dialog", "WriteConfirm", "Input"],
    ];
    // A `NoAction` binding unbinds the key: nothing resolves inside the confirm dialog.
    for path in paths {
        assert_eq!(resolve("enter", path, cx), None, "{path:?}");
    }
    // Other dialogs keep the kit's Enter.
    assert!(resolve("enter", &["Root", "Dialog"], cx).is_some());
}

const TERMINAL_PATH: [&str; 4] = ["Root", "AppShell", "Dock", "Terminal"];

#[gpui_kit::test]
fn single_keys_do_nothing_in_the_terminal(cx: &mut TestAppContext) {
    bind_all(cx);
    for key in [
        "j", "k", "y", "l", "s", "?", "/", ":", "enter", "escape", "delete", "[", "shift-s",
    ] {
        let name = resolve(key, &TERMINAL_PATH, cx);
        assert!(!is_app_action(name), "{key} resolved to {name:?}");
    }
    // The same keys still work in the workspace around the dock.
    assert_eq!(
        resolve("j", &["Root", "AppShell", "Dock"], cx),
        Some("k8sboard::SelectNextRow")
    );
}

#[gpui_kit::test]
fn shell_keys_reach_the_program_on_windows_and_linux(cx: &mut TestAppContext) {
    bind_all(cx);
    if cfg!(target_os = "macos") {
        // Every app chord there uses Cmd, which a shell never receives.
        return;
    }
    // `NoAction` unbinds the key under the terminal: it resolves to nothing, so the key reaches
    // the terminal's key handler and the program, and not the chord of the same key.
    for key in [
        "ctrl-k",
        "ctrl-n",
        "ctrl-w",
        "ctrl-c",
        "tab",
        "shift-tab",
        "ctrl-1",
        "ctrl-5",
        "ctrl-9",
    ] {
        assert_eq!(resolve(key, &TERMINAL_PATH, cx), None, "{key}");
    }
    // Outside the terminal they keep their meaning.
    assert_eq!(
        resolve("ctrl-k", &["Root", "AppShell", "Dock"], cx),
        Some("k8sboard::OpenPalette")
    );
    assert_eq!(
        resolve("ctrl-w", &["Root", "AppShell", "Dock"], cx),
        Some("k8sboard::CloseDockTab")
    );
    assert_eq!(
        resolve("ctrl-2", &["Root", "AppShell", "Dock"], cx),
        Some("k8sboard::SwitchToCluster2")
    );
}

#[gpui_kit::test]
fn terminal_copy_and_paste_chords_resolve(cx: &mut TestAppContext) {
    bind_all(cx);
    assert_eq!(
        resolve("ctrl-shift-c", &TERMINAL_PATH, cx),
        Some("k8sboard::TerminalCopy")
    );
    assert_eq!(
        resolve("ctrl-shift-v", &TERMINAL_PATH, cx),
        Some("k8sboard::TerminalPaste")
    );
    assert_eq!(
        resolve("ctrl-shift-f", &TERMINAL_PATH, cx),
        Some("k8sboard::TerminalFind")
    );
    if cfg!(target_os = "macos") {
        assert_eq!(
            resolve("cmd-v", &TERMINAL_PATH, cx),
            Some("k8sboard::TerminalPaste")
        );
        assert_eq!(
            resolve("cmd-c", &TERMINAL_PATH, cx),
            Some("k8sboard::TerminalCopy")
        );
    }
}

#[gpui_kit::test]
fn ctrl_enter_is_bound_in_the_palette_query_only(cx: &mut TestAppContext) {
    bind_all(cx);
    let palette = ["Root", "Dialog", "Command", "Input"];
    assert_eq!(
        resolve("secondary-enter", &palette, cx),
        Some("k8sboard::ScaleCursorRow")
    );
    // The shell, its text fields, and the other dialogs keep the key to themselves.
    for path in [
        &SHELL[..],
        &TABLE_PATH[..],
        &INPUT_PATH[..],
        &["Root", "Dialog"],
    ] {
        assert!(
            !is_app_action(resolve("secondary-enter", path, cx)),
            "{path:?}"
        );
    }
}

#[gpui_kit::test]
fn terminal_copy_outranks_the_cluster_switcher_only_inside_the_terminal(cx: &mut TestAppContext) {
    bind_all(cx);
    if cfg!(target_os = "macos") {
        return;
    }
    assert_eq!(
        resolve("ctrl-shift-c", &["Root", "AppShell", "Dock"], cx),
        Some("k8sboard::OpenClusterSwitcher")
    );
    assert_eq!(
        resolve("ctrl-shift-c", &TERMINAL_PATH, cx),
        Some("k8sboard::TerminalCopy")
    );
}

#[gpui_kit::test]
fn dock_chords_still_work_in_the_terminal(cx: &mut TestAppContext) {
    bind_all(cx);
    for (key, action) in [
        ("ctrl-`", "ToggleDock"),
        ("ctrl-tab", "NextDockTab"),
        ("ctrl-shift-tab", "PreviousDockTab"),
        ("secondary-shift-m", "ToggleDockZoom"),
    ] {
        assert_eq!(
            resolve(key, &TERMINAL_PATH, cx),
            Some(format!("k8sboard::{action}").as_str()),
            "{key}"
        );
    }
    assert_eq!(
        resolve("secondary-,", &TERMINAL_PATH, cx),
        Some("k8sboard::OpenSettings")
    );
}

#[gpui_kit::test]
fn escape_in_find_closes_find_not_leave_input(cx: &mut TestAppContext) {
    bind_all(cx);
    assert_eq!(
        resolve(
            "escape",
            &["Root", "AppShell", "Dock", "ShellFind", "Input"],
            cx
        ),
        Some("k8sboard::CloseTerminalFind")
    );
    // Any other field of the dock keeps the 0028 leave-input Escape.
    assert_eq!(
        resolve("escape", &["Root", "AppShell", "Dock", "Input"], cx),
        Some("k8sboard::LeaveInput")
    );
}

#[gpui_kit::test]
fn the_terminal_context_is_the_only_one_with_shell_chords(cx: &mut TestAppContext) {
    bind_all(cx);
    for key in ["ctrl-shift-c", "ctrl-shift-v", "ctrl-shift-f"] {
        for path in [&SHELL[..], &INPUT_PATH[..], &TABLE_PATH[..]] {
            let name = resolve(key, path, cx);
            assert!(
                !matches!(
                    name,
                    Some(
                        "k8sboard::TerminalCopy"
                            | "k8sboard::TerminalPaste"
                            | "k8sboard::TerminalFind"
                    )
                ),
                "{key} under {path:?}: {name:?}"
            );
        }
    }
}

#[gpui_kit::test]
fn escape_steps_back_inside_the_popover_and_the_palette_argument(cx: &mut TestAppContext) {
    bind_all(cx);
    let cases: [(&[&str], &str); 4] = [
        (
            &["Root", "AppShell", "ValuePopover"],
            "k8sboard::CancelValuePopover",
        ),
        (
            &["Root", "AppShell", "ValuePopover", "Input"],
            "k8sboard::CancelValuePopover",
        ),
        (
            &["Root", "Dialog", "PaletteArgument"],
            "k8sboard::LeavePaletteArgument",
        ),
        (
            &["Root", "Dialog", "PaletteArgument", "Input"],
            "k8sboard::LeavePaletteArgument",
        ),
    ];
    for (path, expected) in cases {
        assert_eq!(resolve("escape", path, cx), Some(expected), "{path:?}");
    }
}

#[gpui_kit::test]
fn the_read_only_toggle_is_not_an_app_chord_inside_the_terminal(cx: &mut TestAppContext) {
    bind_all(cx);
    if cfg!(target_os = "macos") {
        // There the chord is Cmd, which a shell never receives.
        return;
    }
    // Ctrl Shift R is a history search in some shells; outside the terminal it still toggles.
    assert_eq!(resolve("ctrl-shift-r", &TERMINAL_PATH, cx), None);
    assert_eq!(
        resolve("ctrl-shift-r", &["Root", "AppShell", "Dock"], cx),
        Some("k8sboard::ToggleReadOnly")
    );
}

#[gpui_kit::test]
fn enter_is_suppressed_in_the_fresh_enter_content(cx: &mut TestAppContext) {
    bind_all(cx);
    // `NoAction` unbinds the kit Confirm on Enter there, so a held Enter cannot confirm.
    assert_eq!(
        resolve("enter", &["Root", "Dialog", "FreshEnter"], cx),
        None
    );
}

#[gpui_kit::test]
fn enter_is_suppressed_in_the_palette_argument(cx: &mut TestAppContext) {
    bind_all(cx);
    // The kit Dialog confirms on Enter and would close the palette before the number is read.
    let paths: [&[&str]; 2] = [
        &["Root", "Dialog", "PaletteArgument"],
        &["Root", "Dialog", "PaletteArgument", "Input"],
    ];
    for path in paths {
        assert_eq!(resolve("enter", path, cx), None, "{path:?}");
    }
    // The palette's own query keeps the kit's Enter.
    assert!(resolve("enter", &["Root", "Dialog", "Command", "Input"], cx).is_some());
}

const YAML_EDIT_PATH: [&str; 3] = ["Root", "AppShell", "YamlEdit"];
const YAML_EDIT_EDITOR_PATH: [&str; 4] = ["Root", "AppShell", "YamlEdit", "Input"];

#[gpui_kit::test]
fn ctrl_s_is_bound_in_yaml_edit(cx: &mut TestAppContext) {
    bind_all(cx);
    assert_eq!(
        resolve("secondary-s", &YAML_EDIT_PATH, cx),
        Some("k8sboard::ApplyEdit")
    );
    // The code editor is a text field inside the view: the chord works there too.
    assert_eq!(
        resolve("secondary-s", &YAML_EDIT_EDITOR_PATH, cx),
        Some("k8sboard::ApplyEdit")
    );
    // Outside the view the chord is free.
    assert_eq!(resolve("secondary-s", &SHELL, cx), None);
}

#[gpui_kit::test]
fn single_keys_stay_silent_in_the_edit_yaml_view(cx: &mut TestAppContext) {
    bind_all(cx);
    for key in ["e", "r", "j", "k", "l"] {
        assert_eq!(resolve(key, &YAML_EDIT_PATH, cx), None, "{key}");
        assert_eq!(resolve(key, &YAML_EDIT_EDITOR_PATH, cx), None, "{key}");
    }
    // Enter and Escape are the kit's own there, never the workspace's OpenDrawer and Dismiss.
    for key in ["enter", "escape"] {
        assert!(!is_app_action(resolve(key, &YAML_EDIT_PATH, cx)), "{key}");
    }
    // The same keys keep their meaning in the workspace.
    assert_eq!(resolve("e", &SHELL, cx), Some("k8sboard::EditYaml"));
}

#[gpui_kit::test]
fn the_app_chords_work_inside_the_edit_yaml_view(cx: &mut TestAppContext) {
    bind_all(cx);
    assert_eq!(
        resolve("secondary-k", &YAML_EDIT_EDITOR_PATH, cx),
        Some("k8sboard::OpenPalette")
    );
}

#[gpui_kit::test]
fn del_runs_the_delete_action_on_every_platform(cx: &mut TestAppContext) {
    bind_all(cx);
    assert_eq!(resolve("delete", &SHELL, cx), Some("k8sboard::Delete"));
    assert_eq!(resolve("delete", &TABLE_PATH, cx), Some("k8sboard::Delete"));
}

#[cfg(target_os = "macos")]
#[gpui_kit::test]
fn cmd_backspace_is_bound_on_macos(cx: &mut TestAppContext) {
    bind_all(cx);
    assert_eq!(
        resolve("cmd-backspace", &SHELL, cx),
        Some("k8sboard::Delete")
    );
    assert!(!is_app_action(resolve("cmd-backspace", &INPUT_PATH, cx)));
}

#[cfg(not(target_os = "macos"))]
#[gpui_kit::test]
fn cmd_backspace_is_not_bound_off_macos(cx: &mut TestAppContext) {
    bind_all(cx);
    assert!(!is_app_action(resolve("cmd-backspace", &SHELL, cx)));
}

// ---- Edit values (spec 0047) ----

const VALUES_SCREEN: [&str; 2] = ["Root", "AppShell ValuesScreen"];
const VALUES_SCREEN_TABLE: [&str; 3] = ["Root", "AppShell ValuesScreen", "DataTable"];
const VALUES_EDIT_PATH: [&str; 4] = ["Root", "AppShell ValuesScreen", "ValuesEdit", "Input"];

#[gpui_kit::test]
fn e_binds_edit_values_only_in_values_screen(cx: &mut TestAppContext) {
    bind_all(cx);
    assert_eq!(
        resolve("e", &VALUES_SCREEN, cx),
        Some("k8sboard::EditValues")
    );
    assert_eq!(
        resolve("e", &VALUES_SCREEN_TABLE, cx),
        Some("k8sboard::EditValues")
    );
    // Every other screen keeps E = Edit YAML.
    assert_eq!(resolve("e", &SHELL, cx), Some("k8sboard::EditYaml"));
    assert_eq!(resolve("e", &TABLE_PATH, cx), Some("k8sboard::EditYaml"));
}

#[gpui_kit::test]
fn e_stays_silent_in_text_fields_and_the_values_view(cx: &mut TestAppContext) {
    bind_all(cx);
    assert_eq!(resolve("e", &VALUES_EDIT_PATH, cx), None);
    assert_eq!(
        resolve("e", &["Root", "AppShell ValuesScreen", "ValuesEdit"], cx),
        None
    );
    assert_eq!(
        resolve("e", &["Root", "AppShell ValuesScreen", "Input"], cx),
        None
    );
}

#[gpui_kit::test]
fn ctrl_s_is_bound_in_values_edit(cx: &mut TestAppContext) {
    bind_all(cx);
    assert_eq!(
        resolve("secondary-s", &VALUES_EDIT_PATH, cx),
        Some("k8sboard::ApplyEdit")
    );
    // Outside the view the chord is free.
    assert_eq!(resolve("secondary-s", &VALUES_SCREEN, cx), None);
}

#[gpui_kit::test]
fn the_single_keys_are_silent_in_the_values_view(cx: &mut TestAppContext) {
    bind_all(cx);
    for key in ["r", "j", "k", "l", "d"] {
        assert_eq!(
            resolve(key, &["Root", "AppShell ValuesScreen", "ValuesEdit"], cx),
            None,
            "{key}"
        );
    }
}

#[test]
fn shortcut_rows_name_both_edit_keys() {
    let labels: Vec<&str> = shortcut_rows().iter().map(|row| row.label).collect();
    assert!(
        labels.contains(&"Edit values (ConfigMaps, Secrets)"),
        "{labels:?}"
    );
    assert!(labels.contains(&"Edit YAML (other kinds)"), "{labels:?}");
    assert!(!labels.contains(&"Edit YAML"));
}

#[gpui_kit::test]
fn a_runs_attach_on_pods_only(cx: &mut TestAppContext) {
    use crate::resource_actions::{ResourceAction, RowAction, subject_action};
    use crate::table_selection::ResourceKey;
    bind_all(cx);
    assert_eq!(resolve("a", &SHELL, cx), Some("k8sboard::Attach"));
    assert_eq!(resolve("a", &TABLE_PATH, cx), Some("k8sboard::Attach"));
    // A single key never acts in a text field or in the terminal.
    for path in [&INPUT_PATH[..], &TERMINAL_PATH[..]] {
        let name = resolve("a", path, cx);
        assert!(!is_app_action(name), "a under {path:?}: {name:?}");
    }
    let pod = ResourceKey::Pod {
        namespace: "shop".to_owned(),
        name: "api-0".to_owned(),
    };
    let node = ResourceKey::Node {
        name: "wk-01".to_owned(),
    };
    let deployment = ResourceKey::Kind {
        kind: crate::resource_kind::ResourceKind::Deployments,
        namespace: Some("shop".to_owned()),
        name: "api".to_owned(),
    };
    assert_eq!(
        subject_action(RowAction::Attach, &pod),
        Some(ResourceAction::Attach)
    );
    assert_eq!(subject_action(RowAction::Attach, &node), None);
    assert_eq!(subject_action(RowAction::Attach, &deployment), None);
}
