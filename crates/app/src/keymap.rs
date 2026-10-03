//! The one key map of the app: every binding, the contexts that scope them, and the rows of the
//! shortcut sheet. The handlers live next to the state they change (`app_shell`,
//! `keyboard_navigation`, `dock`); this module only says which key runs which action.

use gpui_kit::{Action, App, KeyBinding};

use crate::cluster_switcher::{
    OpenClusterSwitcher, SwitchToCluster1, SwitchToCluster2, SwitchToCluster3, SwitchToCluster4,
    SwitchToCluster5, SwitchToCluster6, SwitchToCluster7, SwitchToCluster8, SwitchToCluster9,
};
use crate::fresh_enter::FRESH_ENTER;
use crate::settings_window::{ImportKubeconfig, OpenSettings};

gpui_kit::actions!(
    k8sboard,
    [
        ShowShortcuts,
        OpenPalette,
        OpenKindPalette,
        PalettePreview,
        FocusQuickFilter,
        SelectNextRow,
        SelectPreviousRow,
        SelectFirstRow,
        SelectLastRow,
        SelectNextPage,
        SelectPreviousPage,
        OpenDrawer,
        Dismiss,
        LeaveInput,
        PreviousContainer,
        NextContainer,
        ViewLogs,
        ViewYaml,
        CopyName,
        OpenShell,
        PortForward,
        Cordon,
        Drain,
        EditYaml,
        RestartRollout,
        Scale,
        Delete,
        // Unbound unless the wireframe gives them a key: menus and the palette dispatch them.
        PauseRollout,
        DebugContainer,
        RollBack,
        SuspendCronJob,
        TriggerCronJob,
        RerunJob,
        // The value popover and the palette argument (0032): Escape steps back one level.
        CancelValuePopover,
        ScaleCursorRow,
        LeavePaletteArgument,
        OpenNamespacePicker,
        ToggleDock,
        ToggleDockZoom,
        NextDockTab,
        PreviousDockTab,
        CloseDockTab,
        ToggleReadOnly,
        TerminalCopy,
        TerminalPaste,
        TerminalFind,
        CloseTerminalFind,
        // Edit YAML (0031): applies the edit in the editor.
        ApplyEdit,
    ]
);

/// Chords: they work anywhere in the shell tree, text fields included.
const WINDOW: &str = "AppShell";
/// Single keys: they never act in a text field, menu, popover, dialog, or the Edit YAML view. Dialogs sit
/// outside `AppShell`, so `!Dialog` is redundant; it is kept so the predicate states the rule.
const WORKSPACE: &str =
    "AppShell && !Input && !PopupMenu && !Popover && !Dialog && !Terminal && !YamlEdit";
/// Overrides of the keys the kit table binds itself (`up down home end pageup pagedown escape`).
const TABLE: &str = "AppShell > DataTable";
/// The text fields whose Escape returns the focus to the table.
const FIELDS: [&str; 3] = ["QuickFilter > Input", "Drawer > Input", "Dock > Input"];
/// Only the Settings window has this context, so Ctrl O imports there and nowhere else.
const SETTINGS_WINDOW: &str = "SettingsWindow";
/// The palette's query input. The palette is a dialog outside `AppShell`, so only its own keys
/// apply there.
const PALETTE_INPUT: &str = "Command > Input";
/// The terminal of a shell tab: a program owns every key here that no app chord claims.
const TERMINAL: &str = "Terminal";
/// The Find field of a shell tab.
const TERMINAL_FIND_INPUT: &str = "ShellFind > Input";
/// The content of the confirm dialog, and the text field inside it.
const WRITE_CONFIRM: &str = "WriteConfirm";
const WRITE_CONFIRM_INPUT: &str = "WriteConfirm > Input";
/// The Scale popover, and the text field inside it.
const VALUE_POPOVER: &str = "ValuePopover";
const VALUE_POPOVER_INPUT: &str = "ValuePopover > Input";
/// The palette's inline argument (the replicas of Scale), and the text field inside it.
const PALETTE_ARGUMENT: &str = "PaletteArgument";
const PALETTE_ARGUMENT_INPUT: &str = "PaletteArgument > Input";
/// The New forward and Change local port forms (spec 0035), which handle a fresh Enter themselves.
pub(crate) const FORWARD_FORM: &str = "ForwardForm";
const FORWARD_FORM_INPUT: &str = "ForwardForm > Input";
/// The Edit YAML view (spec 0031): its editor is a text field, and so is the rest of the view for the
/// workspace's single keys.
pub(crate) const YAML_EDIT: &str = "YamlEdit";

/// Registers every binding of the app except the switcher popover's own keys
/// (`cluster_switcher::bind_keys`). It runs after `gpui_kit::init`, so at equal depth these win
/// over the kit's bindings.
pub(crate) fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        // Chords.
        KeyBinding::new("secondary-k", OpenPalette, Some(WINDOW)),
        KeyBinding::new("secondary-n", OpenNamespacePicker, Some(WINDOW)),
        KeyBinding::new("ctrl-`", ToggleDock, Some(WINDOW)),
        KeyBinding::new("secondary-shift-m", ToggleDockZoom, Some(WINDOW)),
        KeyBinding::new("ctrl-tab", NextDockTab, Some(WINDOW)),
        KeyBinding::new("ctrl-shift-tab", PreviousDockTab, Some(WINDOW)),
        KeyBinding::new("secondary-w", CloseDockTab, Some(WINDOW)),
        KeyBinding::new("secondary-shift-c", OpenClusterSwitcher, Some(WINDOW)),
        KeyBinding::new("secondary-shift-r", ToggleReadOnly, Some(WINDOW)),
        KeyBinding::new("secondary-1", SwitchToCluster1, Some(WINDOW)),
        KeyBinding::new("secondary-2", SwitchToCluster2, Some(WINDOW)),
        KeyBinding::new("secondary-3", SwitchToCluster3, Some(WINDOW)),
        KeyBinding::new("secondary-4", SwitchToCluster4, Some(WINDOW)),
        KeyBinding::new("secondary-5", SwitchToCluster5, Some(WINDOW)),
        KeyBinding::new("secondary-6", SwitchToCluster6, Some(WINDOW)),
        KeyBinding::new("secondary-7", SwitchToCluster7, Some(WINDOW)),
        KeyBinding::new("secondary-8", SwitchToCluster8, Some(WINDOW)),
        KeyBinding::new("secondary-9", SwitchToCluster9, Some(WINDOW)),
        // Settings: Ctrl , has no context, so it also works in dialogs and in the Settings window.
        KeyBinding::new("secondary-,", OpenSettings, None),
        KeyBinding::new("secondary-o", ImportKubeconfig, Some(SETTINGS_WINDOW)),
        // Single keys.
        KeyBinding::new("?", ShowShortcuts, Some(WORKSPACE)),
        KeyBinding::new(":", OpenKindPalette, Some(WORKSPACE)),
        KeyBinding::new("/", FocusQuickFilter, Some(WORKSPACE)),
        KeyBinding::new("j", SelectNextRow, Some(WORKSPACE)),
        KeyBinding::new("k", SelectPreviousRow, Some(WORKSPACE)),
        KeyBinding::new("down", SelectNextRow, Some(WORKSPACE)),
        KeyBinding::new("up", SelectPreviousRow, Some(WORKSPACE)),
        KeyBinding::new("home", SelectFirstRow, Some(WORKSPACE)),
        KeyBinding::new("end", SelectLastRow, Some(WORKSPACE)),
        KeyBinding::new("pageup", SelectPreviousPage, Some(WORKSPACE)),
        KeyBinding::new("pagedown", SelectNextPage, Some(WORKSPACE)),
        KeyBinding::new("enter", OpenDrawer, Some(WORKSPACE)),
        KeyBinding::new("escape", Dismiss, Some(WORKSPACE)),
        KeyBinding::new("[", PreviousContainer, Some(WORKSPACE)),
        KeyBinding::new("]", NextContainer, Some(WORKSPACE)),
        KeyBinding::new("l", ViewLogs, Some(WORKSPACE)),
        KeyBinding::new("y", ViewYaml, Some(WORKSPACE)),
        KeyBinding::new("secondary-c", CopyName, Some(WORKSPACE)),
        KeyBinding::new("s", OpenShell, Some(WORKSPACE)),
        KeyBinding::new("f", PortForward, Some(WORKSPACE)),
        KeyBinding::new("c", Cordon, Some(WORKSPACE)),
        KeyBinding::new("d", Drain, Some(WORKSPACE)),
        KeyBinding::new("e", EditYaml, Some(WORKSPACE)),
        KeyBinding::new("r", RestartRollout, Some(WORKSPACE)),
        KeyBinding::new("shift-s", Scale, Some(WORKSPACE)),
        KeyBinding::new("delete", Delete, Some(WORKSPACE)),
        // The table: the kit table binds these keys deeper, so they are taken over here.
        KeyBinding::new("down", SelectNextRow, Some(TABLE)),
        KeyBinding::new("up", SelectPreviousRow, Some(TABLE)),
        KeyBinding::new("home", SelectFirstRow, Some(TABLE)),
        KeyBinding::new("end", SelectLastRow, Some(TABLE)),
        KeyBinding::new("pageup", SelectPreviousPage, Some(TABLE)),
        KeyBinding::new("pagedown", SelectNextPage, Some(TABLE)),
        KeyBinding::new("escape", Dismiss, Some(TABLE)),
        // The palette: Tab previews the highlighted resource, which the kit would otherwise use to
        // move focus out of the query.
        KeyBinding::new("tab", PalettePreview, Some(PALETTE_INPUT)),
        // Ctrl Enter on a Scale entry turns the query into a replicas field (0032).
        KeyBinding::new("secondary-enter", ScaleCursorRow, Some(PALETTE_INPUT)),
        // Ctrl S in the Edit YAML view checks the edit with the server, then applies it.
        KeyBinding::new("secondary-s", ApplyEdit, Some(YAML_EDIT)),
    ]);
    // The confirm dialog handles Enter itself (a held Enter must never confirm), so the kit's Enter
    // bindings of the dialog and of its text field are switched off inside it.
    cx.bind_keys([
        KeyBinding::new("enter", gpui_kit::NoAction, Some(FRESH_ENTER)),
        KeyBinding::new("enter", gpui_kit::NoAction, Some(WRITE_CONFIRM)),
        KeyBinding::new("enter", gpui_kit::NoAction, Some(WRITE_CONFIRM_INPUT)),
        KeyBinding::new("enter", gpui_kit::NoAction, Some(FORWARD_FORM)),
        KeyBinding::new("enter", gpui_kit::NoAction, Some(FORWARD_FORM_INPUT)),
    ]);
    // Escape in the popover and in the palette argument steps back one level, text field included:
    // the kit binds Escape inside its own `Input`, which would otherwise win.
    cx.bind_keys([
        KeyBinding::new("escape", CancelValuePopover, Some(VALUE_POPOVER)),
        KeyBinding::new("escape", CancelValuePopover, Some(VALUE_POPOVER_INPUT)),
        KeyBinding::new("escape", LeavePaletteArgument, Some(PALETTE_ARGUMENT)),
        KeyBinding::new("escape", LeavePaletteArgument, Some(PALETTE_ARGUMENT_INPUT)),
        // The palette argument handles Enter itself, once: the kit Dialog's Enter would close the
        // palette in the same key press, before the number is read.
        KeyBinding::new("enter", gpui_kit::NoAction, Some(PALETTE_ARGUMENT)),
        KeyBinding::new("enter", gpui_kit::NoAction, Some(PALETTE_ARGUMENT_INPUT)),
    ]);
    cx.bind_keys(
        FIELDS
            .into_iter()
            .map(|context| KeyBinding::new("escape", LeaveInput, Some(context))),
    );
    // The terminal (spec 0036). These come last: Esc in the Find field has the depth of `Dock >
    // Input`, so it wins only by being registered after it.
    cx.bind_keys([
        KeyBinding::new("ctrl-shift-c", TerminalCopy, Some(TERMINAL)),
        KeyBinding::new("ctrl-shift-v", TerminalPaste, Some(TERMINAL)),
        KeyBinding::new("ctrl-shift-f", TerminalFind, Some(TERMINAL)),
        KeyBinding::new("escape", CloseTerminalFind, Some(TERMINAL_FIND_INPUT)),
    ]);
    // On macOS the platform key is Cmd, which a shell never receives.
    #[cfg(target_os = "macos")]
    cx.bind_keys([
        KeyBinding::new("cmd-c", TerminalCopy, Some(TERMINAL)),
        KeyBinding::new("cmd-v", TerminalPaste, Some(TERMINAL)),
        KeyBinding::new("cmd-f", TerminalFind, Some(TERMINAL)),
        // A Mac keyboard has no forward Delete key (spec 0033 decision 18).
        KeyBinding::new("cmd-backspace", Delete, Some(WORKSPACE)),
    ]);
    // Keys a shell uses: Tab and Shift Tab (the kit moves focus with them), Ctrl C (the kit copies
    // with it), and the chords Ctrl K, N, W, Shift R, and 1 to 9, which are app chords on Windows and
    // Linux and which shells use (Ctrl Shift R is a history search in some).
    // `NoAction` lets the key reach the terminal's key handler. macOS app chords use Cmd, which a
    // shell never receives, so they keep their meaning.
    cx.bind_keys(
        [
            "tab",
            "shift-tab",
            "ctrl-c",
            "ctrl-k",
            "ctrl-n",
            "ctrl-w",
            "ctrl-shift-r",
            "ctrl-1",
            "ctrl-2",
            "ctrl-3",
            "ctrl-4",
            "ctrl-5",
            "ctrl-6",
            "ctrl-7",
            "ctrl-8",
            "ctrl-9",
        ]
        .map(|key| KeyBinding::new(key, gpui_kit::NoAction, Some(TERMINAL))),
    );
}

/// The sections of the shortcut sheet, in the order they are drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ShortcutGroup {
    General,
    Tables,
    Drawer,
    SelectedResource,
    Dock,
    Terminal,
}

impl ShortcutGroup {
    pub(crate) const ALL: [Self; 6] = [
        Self::General,
        Self::Tables,
        Self::Drawer,
        Self::SelectedResource,
        Self::Dock,
        Self::Terminal,
    ];

    pub(crate) fn title(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Tables => "Tables",
            Self::Drawer => "Drawer",
            Self::SelectedResource => "Selected resource",
            Self::Dock => "Dock",
            Self::Terminal => "Terminal",
        }
    }
}

/// One line of the sheet. Its keys are read from the live keymap, never written here.
pub(crate) struct ShortcutRow {
    pub(crate) group: ShortcutGroup,
    pub(crate) label: &'static str,
    pub(crate) action: Box<dyn Action>,
}

fn row(group: ShortcutGroup, label: &'static str, action: impl Action) -> ShortcutRow {
    ShortcutRow {
        group,
        label,
        action: Box::new(action),
    }
}

/// Every row of the sheet in wireframe order. `LeaveInput`, `SwitchToCluster2`…`9` (covered by the
/// 1–9 row) and the switcher popover's own keys have no row.
pub(crate) fn shortcut_rows() -> Vec<ShortcutRow> {
    use ShortcutGroup::{Dock, Drawer, General, SelectedResource, Tables, Terminal};
    vec![
        row(General, "Show all shortcuts", ShowShortcuts),
        row(General, "Command palette", OpenPalette),
        row(General, "Jump to a resource kind", OpenKindPalette),
        row(General, "Choose namespace", OpenNamespacePicker),
        row(General, "Open cluster switcher", OpenClusterSwitcher),
        row(General, "Toggle read-only", ToggleReadOnly),
        row(General, "Switch to cluster 1–9", SwitchToCluster1),
        row(General, "Open Settings", OpenSettings),
        row(
            General,
            "Import kubeconfig file (Settings window)",
            ImportKubeconfig,
        ),
        row(Tables, "Filter the table", FocusQuickFilter),
        row(Tables, "Next row", SelectNextRow),
        row(Tables, "Previous row", SelectPreviousRow),
        row(Tables, "First row", SelectFirstRow),
        row(Tables, "Last row", SelectLastRow),
        row(Tables, "Next page", SelectNextPage),
        row(Tables, "Previous page", SelectPreviousPage),
        row(Tables, "Open the drawer", OpenDrawer),
        row(
            Tables,
            "Close the drawer, then clear the selection",
            Dismiss,
        ),
        row(Drawer, "Previous container", PreviousContainer),
        row(Drawer, "Next container", NextContainer),
        row(SelectedResource, "View logs", ViewLogs),
        row(SelectedResource, "View YAML", ViewYaml),
        row(SelectedResource, "Copy name", CopyName),
        row(SelectedResource, "Open shell (pod or node)", OpenShell),
        row(SelectedResource, "Port-forward", PortForward),
        row(SelectedResource, "Cordon or uncordon node", Cordon),
        row(SelectedResource, "Drain node (opens a dialog)", Drain),
        row(SelectedResource, "Edit YAML", EditYaml),
        row(
            SelectedResource,
            "Apply the edit (Edit YAML view)",
            ApplyEdit,
        ),
        row(SelectedResource, "Restart rollout", RestartRollout),
        row(SelectedResource, "Scale", Scale),
        row(SelectedResource, "Delete", Delete),
        row(Dock, "Toggle the dock", ToggleDock),
        row(Dock, "Zoom the dock in or out", ToggleDockZoom),
        row(Dock, "Next dock tab", NextDockTab),
        row(Dock, "Previous dock tab", PreviousDockTab),
        row(Dock, "Close the dock tab", CloseDockTab),
        row(Terminal, "Copy the selection (shell tab)", TerminalCopy),
        row(Terminal, "Paste (shell tab)", TerminalPaste),
        row(Terminal, "Find in the terminal (shell tab)", TerminalFind),
    ]
}

#[cfg(test)]
#[path = "keymap_tests.rs"]
mod keymap_tests;
