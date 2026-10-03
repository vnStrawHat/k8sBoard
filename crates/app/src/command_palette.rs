//! The command palette (Ctrl K, `:`, the title-bar search box): a kit `Dialog` hosting the kit
//! `Command` list. Ranking is done here (`filterable(false)`), the entries come from
//! `palette_search`, and a confirmed entry runs after the dialog has closed. The palette reads
//! memory only: no list, watch, or request starts while it opens or while the user types.

use std::time::Instant;

use gpui_kit::assets::IconName;
use gpui_kit::component::command::{Command, CommandGroup, CommandItem, CommandState};
use gpui_kit::component::kbd::Kbd;
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IndexPath, Sizable as _, WindowExt as _, h_flex,
    v_flex,
};
use gpui_kit::{
    Action, App, AppContext as _, Context, Entity, FocusHandle, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, SharedString, Styled as _, Subscription, WeakEntity,
    Window, div, prelude::FluentBuilder as _, px,
};

use crate::app_shell::{AppShell, Screen};
use crate::cluster_health::RowHealth;
use crate::cluster_switcher::{OpenClusterSwitcher, health_color, health_text};
use crate::environment::{Environment, environment_badge};
use crate::keymap::{
    CloseDockTab, NextDockTab, OpenNamespacePicker, PalettePreview, PreviousDockTab, ShowShortcuts,
    ToggleDock, ToggleDockZoom,
};
use crate::palette_search::{
    EntryState, PaletteEntry, PaletteGroup, PaletteTarget, empty_text, lists_resources,
    parse_query, ranked,
};
use crate::resource_actions::ResourceAction;
use crate::settings_window::OpenSettings;
use crate::shortcut_sheet::row_keys;
use crate::status_tone::{StatusLabel, StatusTone, tone_color, toned_text};
use crate::table_selection::ResourceKey;

const PALETTE_WIDTH: f32 = 640.;
const PALETTE_TOP_MARGIN: f32 = 56.;
const LIST_MAX_HEIGHT: f32 = 400.;
const PLACEHOLDER: &str = "Search resources or run a command…";
/// The syntax line of the footer (W9 note 5).
const SYNTAX_HINTS: [&str; 4] = [":po resource kind", "@ cluster", "# namespace", "> action"];

/// The active cluster as the header chip shows it.
#[derive(Clone)]
pub(crate) struct ActiveCluster {
    pub(crate) environment: Environment,
    pub(crate) name: SharedString,
}

/// What the header and the empty text need besides the entries.
#[derive(Clone)]
pub(crate) struct PaletteContext {
    pub(crate) screen: Screen,
    pub(crate) has_session: bool,
    /// `None` without a profile.
    pub(crate) cluster: Option<ActiveCluster>,
    /// The `ns: …` chip text; `None` without a session.
    pub(crate) scope_label: Option<SharedString>,
}

/// What the shell hands the palette: the unranked entries and their context. It is built from
/// memory by `AppShell::palette_snapshot`.
pub(crate) struct PaletteSnapshot {
    pub(crate) entries: Vec<PaletteEntry>,
    pub(crate) context: PaletteContext,
}

/// The dialog content. It is created once per open, together with its `CommandState`.
pub(crate) struct CommandPalette {
    state: Entity<CommandState>,
    shell: WeakEntity<AppShell>,
    /// The `AppShell` root: commands are dispatched there, where their handlers live.
    shell_focus: FocusHandle,
    /// The entries the last render handed to the kit. The kit's index paths (highlight, confirm)
    /// refer to this list, so confirming and previewing resolve against it, never against a list
    /// that was ranked after that render.
    shown: Vec<PaletteEntry>,
    more: usize,
    context: PaletteContext,
    query: String,
    /// The shell or the query changed since `shown` was ranked. Ranking happens once per frame, in
    /// `render`, however many notifications arrived.
    is_stale: bool,
    /// The resource to highlight once the next ranking has placed it (after a Tab preview moved
    /// the cursor, the row actions of the new cursor row join the list above it).
    reselect: Option<ResourceKey>,
    /// Opened by `:` and still showing just that: the first Esc (or Backspace) closes at once.
    is_seed_untouched: bool,
    _shell_observer: Subscription,
}

/// Opens the dialog with `initial` typed (`""` or `":"`) and focuses the query input. `snapshot`
/// is built by the caller because the shell is mid-update when its key handler gets here.
pub(crate) fn open_palette(
    initial: &str,
    shell: &Entity<AppShell>,
    snapshot: PaletteSnapshot,
    shell_focus: FocusHandle,
    window: &mut Window,
    cx: &mut App,
) {
    let state = cx.new(|cx| CommandState::new(window, cx));
    let palette =
        cx.new(|cx| CommandPalette::new(state.clone(), shell, snapshot, shell_focus, initial, cx));
    // The builder runs on every render: it only clones the handle.
    window.open_dialog(cx, move |dialog, _, _| {
        dialog
            .close_button(false)
            .width(px(PALETTE_WIDTH))
            .margin_top(px(PALETTE_TOP_MARGIN))
            .child(palette.clone())
    });
    state.update(cx, |state, cx| {
        if !initial.is_empty() {
            state.set_query(initial.to_owned(), window, cx);
        }
        state.focus(window, cx);
    });
}

impl CommandPalette {
    fn new(
        state: Entity<CommandState>,
        shell: &Entity<AppShell>,
        snapshot: PaletteSnapshot,
        shell_focus: FocusHandle,
        initial: &str,
        cx: &mut Context<Self>,
    ) -> Self {
        // Live statuses (W9 note 4): a shell change marks the ranking stale; the next render ranks.
        let shell_observer = cx.observe(shell, |palette, _, cx| {
            palette.is_stale = true;
            cx.notify();
        });
        let mut palette = Self {
            state,
            shell: shell.downgrade(),
            shell_focus,
            shown: Vec::new(),
            more: 0,
            context: snapshot.context.clone(),
            query: initial.to_owned(),
            is_stale: false,
            reselect: None,
            is_seed_untouched: initial == ":",
            _shell_observer: shell_observer,
        };
        palette.rank(snapshot);
        palette
    }

    fn rank(&mut self, snapshot: PaletteSnapshot) {
        let ranked = ranked(snapshot.entries, &parse_query(&self.query));
        self.shown = ranked.entries;
        self.more = ranked.more;
        self.context = snapshot.context;
    }

    /// Builds the entries again from the shell and ranks them for the current query.
    fn refresh(&mut self, cx: &App) {
        self.is_stale = false;
        let Some(shell) = self.shell.upgrade() else {
            return;
        };
        let started = Instant::now();
        let wants_resources = lists_resources(&parse_query(&self.query));
        let snapshot = shell.read(cx).palette_snapshot(wants_resources, cx);
        let candidates = snapshot.entries.len();
        self.rank(snapshot);
        // Counts and a duration only: the query is never traced.
        tracing::trace!(
            candidates,
            shown = self.shown.len(),
            more = self.more,
            micros = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX),
            "palette ranked"
        );
    }

    fn on_query(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_seed_untouched {
            if text.is_empty() {
                window.close_dialog(cx);
                return;
            }
            if text != ":" {
                self.is_seed_untouched = false;
            }
        }
        self.query = text.to_owned();
        self.is_stale = true;
        cx.notify();
    }

    /// The entries the kit draws, one list per non-empty group in `PaletteGroup::ALL` order: the
    /// position in this list is the kit's section.
    fn sections(&self) -> Vec<Vec<&PaletteEntry>> {
        PaletteGroup::ALL
            .into_iter()
            .map(|group| {
                self.shown
                    .iter()
                    .filter(|entry| entry.group == group)
                    .collect::<Vec<_>>()
            })
            .filter(|members| !members.is_empty())
            .collect()
    }

    fn entry_at(&self, path: IndexPath) -> Option<&PaletteEntry> {
        self.sections().get(path.section)?.get(path.row).copied()
    }

    /// The kit path of the first shown entry `is_wanted` picks.
    fn path_of(&self, is_wanted: impl Fn(&PaletteEntry) -> bool) -> Option<IndexPath> {
        self.sections()
            .iter()
            .enumerate()
            .find_map(|(section, members)| {
                let row = members.iter().position(|entry| is_wanted(entry))?;
                Some(IndexPath::new(row).section(section))
            })
    }

    /// The resource under the kit's highlight.
    fn highlighted_resource(&self, cx: &App) -> Option<&ResourceKey> {
        let path = self.state.read(cx).selected_index()?;
        match &self.entry_at(path)?.target {
            PaletteTarget::Resource(key) => Some(key),
            _ => None,
        }
    }

    /// Whether Tab would move the table cursor: the highlighted entry is a row of the screen that
    /// is open, the filter shows it, and no drawer is open. `selected` is the kit's highlight.
    fn can_preview(&self, selected: Option<IndexPath>, cx: &App) -> bool {
        let Some(PaletteTarget::Resource(key)) = selected
            .and_then(|path| self.entry_at(path))
            .map(|entry| &entry.target)
        else {
            return false;
        };
        self.shell
            .upgrade()
            .is_some_and(|shell| shell.read(cx).can_preview_row(key, cx))
    }

    /// Tab: moves the table cursor behind the scrim to the highlighted resource, without opening
    /// its drawer, switching screen, or starting a watch. Anywhere else it does nothing.
    fn preview(&mut self, cx: &mut Context<Self>) {
        let Some(key) = self.highlighted_resource(cx).cloned() else {
            return;
        };
        let Some(shell) = self.shell.upgrade() else {
            return;
        };
        shell.update(cx, |shell, cx| shell.preview_resource(&key, cx));
        self.reselect = Some(key);
        self.is_stale = true;
        cx.notify();
    }

    /// Closes the dialog, then runs the entry with the window the kit gave `on_confirm`. A
    /// disabled entry never runs: the kit skips it too, and this is the second gate.
    fn confirm(&mut self, path: IndexPath, window: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self.entry_at(path).filter(|entry| entry.is_enabled()) else {
            return;
        };
        let target = entry.target.clone();
        window.close_dialog(cx);
        let shell_focus = self.shell_focus.clone();
        let dispatch = |action: &dyn Action, window: &mut Window, cx: &mut App| {
            shell_focus.dispatch_action(action, window, cx);
        };
        match target {
            PaletteTarget::Command(action) => dispatch(&*action, window, cx),
            PaletteTarget::RowAction(action) => dispatch(&*action.key_action(), window, cx),
            PaletteTarget::Screen(screen) => self.update_shell(cx, |shell, cx| {
                shell.show_screen(screen, cx);
            }),
            PaletteTarget::Resource(key) => {
                self.update_shell(cx, |shell, cx| shell.reveal(key, cx))
            }
            PaletteTarget::Namespace(scope) => {
                self.update_shell(cx, |shell, cx| shell.set_namespace(scope, cx));
            }
            PaletteTarget::Cluster(row) => {
                self.update_shell(cx, |shell, cx| shell.switch_cluster(&row.cluster, cx));
            }
        }
    }

    fn update_shell(&self, cx: &mut App, run: impl FnOnce(&mut AppShell, &mut Context<AppShell>)) {
        let _ = self.shell.update(cx, run);
    }
}

impl Render for CommandPalette {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.is_stale {
            self.refresh(cx);
        }
        self.schedule_reselect(window, cx);
        let palette = cx.weak_entity();
        let mode = parse_query(&self.query).mode;
        let empty = empty_text(mode, self.context.has_session, self.context.screen);
        let header = HeaderChips {
            cluster: self.context.cluster.clone(),
            scope: self.context.scope_label.clone(),
        };
        let more = self.more;
        let mut command = Command::new(&self.state)
            .filterable(false)
            .bordered(false)
            .placeholder(PLACEHOLDER)
            .max_h(px(LIST_MAX_HEIGHT))
            .header(move |_, _, cx| header.render(cx))
            .footer({
                let palette = palette.clone();
                move |state, _, cx| {
                    // Read here, not at render: the highlight moves without the palette rendering.
                    let selected = state.selected_index();
                    let can_preview = palette
                        .read_with(&*cx, |palette, cx| palette.can_preview(selected, cx))
                        .unwrap_or(false);
                    footer(can_preview, more, cx)
                }
            })
            .empty(move |_, _, cx| {
                div()
                    .p_4()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(empty.clone())
            })
            .on_query({
                let palette = palette.clone();
                move |text, window, cx| {
                    let _ = palette.update(cx, |palette, cx| palette.on_query(text, window, cx));
                }
            })
            .on_confirm(move |path, window, cx| {
                let _ = palette.update(cx, |palette, cx| palette.confirm(path, window, cx));
            });
        for members in self.sections() {
            let heading = members.first().map_or("", |entry| entry.group.heading());
            let items = members.into_iter().map(command_item);
            command = command.group(CommandGroup::new().label(heading).items(items));
        }
        v_flex()
            .on_action(cx.listener(|palette, _: &PalettePreview, _, cx| palette.preview(cx)))
            .child(command)
    }
}

impl CommandPalette {
    /// Highlights the entry a Tab preview asked for once the ranking has placed it. The kit has not
    /// installed this render's list yet, so the highlight is set after the frame.
    fn schedule_reselect(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(key) = self.reselect.take() else {
            return;
        };
        let wanted = |entry: &PaletteEntry| matches!(&entry.target, PaletteTarget::Resource(other) if *other == key);
        let Some(path) = self.path_of(wanted) else {
            return;
        };
        let state = self.state.clone();
        window.defer(cx, move |window, cx| {
            state.update(cx, |state, cx| {
                state.set_selected_index(Some(path), window, cx);
            });
        });
    }
}

/// The scope header (W9 note 2): the active cluster with its environment badge, and `ns: …`.
struct HeaderChips {
    cluster: Option<ActiveCluster>,
    scope: Option<SharedString>,
}

impl HeaderChips {
    fn render(&self, cx: &App) -> impl IntoElement + use<> {
        let cluster = match &self.cluster {
            Some(cluster) => h_flex()
                .gap_1()
                .items_center()
                .child(environment_badge(cluster.environment, cx))
                .child(cluster.name.clone())
                .into_any_element(),
            None => div().child("No cluster").into_any_element(),
        };
        h_flex()
            .w_full()
            .px_3()
            .py_2()
            .gap_2()
            .items_center()
            .justify_end()
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .child(Tag::secondary().small().child(cluster))
            .children(
                self.scope
                    .clone()
                    .map(|scope| Tag::secondary().small().child(scope)),
            )
    }
}

/// The syntax footer (W9 note 5), with `Tab preview` only while it applies and "+N more" when a
/// cap cut a group.
fn footer(can_preview: bool, more: usize, cx: &App) -> impl IntoElement + use<> {
    let keys = if can_preview {
        "↑↓ select · Tab preview · Esc close"
    } else {
        "↑↓ select · Esc close"
    };
    h_flex()
        .w_full()
        .px_3()
        .py_2()
        .gap_3()
        .items_center()
        .border_t_1()
        .border_color(cx.theme().border)
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .children(SYNTAX_HINTS.map(|hint| div().child(hint)))
        .when(more > 0, |footer| {
            footer.child(div().child(format!("+{more} more")))
        })
        .child(div().ml_auto().child(keys))
}

/// The kit item of an entry. Its content is custom, so the kit adds no hint of its own, and it
/// carries no action: the palette runs the target itself, on the shell (decision 11).
fn command_item(entry: &PaletteEntry) -> CommandItem {
    let row = RowContent::of(entry);
    let is_disabled = !entry.is_enabled();
    CommandItem::new()
        .label(entry.label.clone())
        .disabled(is_disabled)
        .child(move |_, cx| row.render(cx))
}

/// What leads a row: a kind badge, `@`, or `#` as text, or an icon for an action.
enum RowIcon {
    Text(SharedString),
    Glyph(IconName),
}

/// A cluster row's environment and health, which stand in for the status pill.
#[derive(Clone, Copy)]
struct ClusterLine {
    environment: Environment,
    health: RowHealth,
}

/// Everything one row draws, owned so the kit may build the row again for measuring.
struct RowContent {
    icon: RowIcon,
    label: SharedString,
    detail: Option<SharedString>,
    status: Option<StatusLabel>,
    reason: Option<SharedString>,
    is_current: bool,
    cluster: Option<ClusterLine>,
    /// The 0028 action whose first key is the hint. Cluster rows show none: their `Ctrl n`
    /// numbers belong to the switcher.
    key_action: Option<Box<dyn Action>>,
}

impl RowContent {
    fn of(entry: &PaletteEntry) -> Self {
        let key_action = match &entry.target {
            PaletteTarget::Command(action) => Some(action.boxed_clone()),
            PaletteTarget::RowAction(action) => Some(action.key_action()),
            PaletteTarget::Screen(_)
            | PaletteTarget::Resource(_)
            | PaletteTarget::Namespace(_)
            | PaletteTarget::Cluster(_) => None,
        };
        let cluster = match &entry.target {
            PaletteTarget::Cluster(row) => Some(ClusterLine {
                environment: row.environment,
                health: row.health,
            }),
            _ => None,
        };
        Self {
            icon: row_icon(&entry.target),
            label: entry.label.clone(),
            detail: entry.detail.clone(),
            status: entry.status.clone(),
            reason: match &entry.state {
                EntryState::Enabled => None,
                EntryState::Disabled { reason } => Some(reason.clone()),
            },
            is_current: entry.is_current,
            cluster,
            key_action,
        }
    }

    fn render(&self, cx: &App) -> impl IntoElement + use<> {
        let theme = cx.theme();
        let muted = theme.muted_foreground;
        let keys = self
            .key_action
            .as_ref()
            .map(|action| row_keys(&**action, cx))
            .unwrap_or_default();
        let leading = div()
            .w(px(24.))
            .flex_none()
            .text_xs()
            .text_color(muted)
            .child(match &self.icon {
                RowIcon::Text(text) => div().child(text.clone()).into_any_element(),
                RowIcon::Glyph(name) => Icon::new(*name).size_4().into_any_element(),
            });
        h_flex()
            .w_full()
            .gap_2()
            .items_center()
            .child(leading)
            .when_some(self.cluster, |row, cluster| {
                row.child(environment_badge(cluster.environment, cx))
            })
            .child(div().flex_none().child(self.label.clone()))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_xs()
                    .text_color(muted)
                    .font_family(theme.mono_font_family.clone())
                    .children(self.detail.clone()),
            )
            .when(self.is_current, |row| {
                row.child(Icon::new(IconName::Check).size_4())
            })
            .children(
                self.status
                    .clone()
                    .map(|status| div().text_xs().child(toned_text(status, cx))),
            )
            .when_some(self.cluster, |row, cluster| {
                row.child(
                    div()
                        .text_xs()
                        .text_color(health_color(cluster.health, cx))
                        .child(health_text(cluster.health)),
                )
            })
            .children(self.reason.clone().map(|reason| reason_pill(reason, cx)))
            .children(keys.into_iter().take(1).map(Kbd::new))
    }
}

/// The icon before a label: an icon for an action, else the kind badge of a resource or screen,
/// `@`, or `#`.
fn row_icon(target: &PaletteTarget) -> RowIcon {
    let text = |text: &'static str| RowIcon::Text(text.into());
    match target {
        PaletteTarget::Command(action) => RowIcon::Glyph(command_icon(&**action)),
        PaletteTarget::RowAction(action) => RowIcon::Glyph(row_action_icon(*action)),
        PaletteTarget::Screen(screen) => text(screen_badge(*screen)),
        PaletteTarget::Resource(ResourceKey::Pod { .. }) => text("Po"),
        PaletteTarget::Resource(ResourceKey::Node { .. }) => text("No"),
        PaletteTarget::Resource(ResourceKey::Kind { kind, .. }) => text(kind.badge()),
        PaletteTarget::Namespace(_) => text("#"),
        PaletteTarget::Cluster(_) => text("@"),
    }
}

fn row_action_icon(action: ResourceAction) -> IconName {
    match action {
        ResourceAction::ViewLogs => IconName::FileText,
        ResourceAction::ViewYaml => IconName::Eye,
        ResourceAction::CopyName => IconName::Copy,
        ResourceAction::OpenShell | ResourceAction::OpenNodeShell => IconName::SquareTerminal,
        ResourceAction::PortForward => IconName::Network,
        ResourceAction::Cordon => IconName::Ban,
        ResourceAction::Drain => IconName::ArrowDown,
        ResourceAction::EditYaml => IconName::Replace,
        ResourceAction::RestartRollout => IconName::RotateCw,
        ResourceAction::Scale => IconName::ChevronsUpDown,
        ResourceAction::Delete => IconName::Delete,
    }
}

fn command_icon(action: &dyn Action) -> IconName {
    let action = action.as_any();
    if action.is::<ShowShortcuts>() {
        IconName::Info
    } else if action.is::<OpenSettings>() {
        IconName::Settings
    } else if action.is::<OpenClusterSwitcher>() {
        IconName::Building2
    } else if action.is::<OpenNamespacePicker>() {
        IconName::Folder
    } else if action.is::<ToggleDock>()
        || action.is::<ToggleDockZoom>()
        || action.is::<NextDockTab>()
        || action.is::<PreviousDockTab>()
        || action.is::<CloseDockTab>()
    {
        IconName::PanelBottom
    } else {
        IconName::ChevronRight
    }
}

fn screen_badge(screen: Screen) -> &'static str {
    match screen {
        Screen::Pods => "Po",
        Screen::Nodes => "No",
        Screen::Kind(kind) => kind.badge(),
        Screen::Overview | Screen::Issues | Screen::Topology => "·",
    }
}

/// The pill of a disabled entry: its reason, outlined in the theme's warning tone (the kit tag's
/// own colors wash out on a muted, disabled row).
fn reason_pill(reason: SharedString, cx: &App) -> impl IntoElement {
    let color = tone_color(StatusTone::Warn, cx);
    div()
        .flex_none()
        .px_1()
        .rounded(cx.theme().radius)
        .border_1()
        .border_color(color)
        .text_xs()
        .text_color(color)
        .child(reason)
}
