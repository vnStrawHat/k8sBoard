//! The command palette (Ctrl K, `:`, the title-bar search box): a kit `Dialog` hosting the kit
//! `Command` list. Ranking is done here (`filterable(false)`), the entries come from
//! `palette_search`, and a confirmed entry runs after the dialog has closed. The palette reads
//! memory only: opening it starts no list, watch, or request. A query of two or more characters
//! asks the session for the name index (spec 0056), which lists at most once per 120 s.

use std::ops::Range;
use std::time::{Duration, Instant};

use gpui_kit::assets::IconName;
use gpui_kit::component::command::{Command, CommandGroup, CommandItem, CommandState};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::kbd::Kbd;
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IndexPath, Sizable as _, WindowExt as _, h_flex,
    v_flex,
};
use gpui_kit::{
    Action, App, AppContext as _, Context, Entity, FocusHandle, HighlightStyle,
    InteractiveElement as _, IntoElement, KeyDownEvent, ParentElement as _, Render, SharedString,
    Styled as _, StyledText, Subscription, Task, UnderlineStyle, WeakEntity, Window, div,
    prelude::FluentBuilder as _, px,
};

use crate::app_shell::{AppShell, Screen};
use crate::cluster_health::RowHealth;
use crate::cluster_switcher::{OpenClusterSwitcher, health_color, health_text};
use crate::environment::{Environment, environment_badge};
use crate::fresh_enter::{confirms, is_enter};
use crate::keymap::{
    CloseDockTab, LeavePaletteArgument, NextDockTab, OpenNamespacePicker, PALETTE_LIST,
    PalettePreview, PreviousDockTab, ScaleCursorRow, ShowShortcuts, ToggleDock, ToggleDockZoom,
};
use crate::name_index::IndexSummary;
use crate::navigation::screen_icon;
use crate::palette_search::{
    EntryRanges, EntryState, PaletteEntry, PaletteGroup, PaletteMode, PaletteTarget, empty_text,
    entry_match_ranges, lists_name_index, parse_query, ranked, searching_text,
};
use crate::resource_actions::RowAction;
use crate::resource_kind::{NODE_ICON, POD_ICON, ResourceKind};
use crate::settings_window::OpenSettings;
use crate::shortcut_sheet::row_keys;
use crate::status_tone::{StatusLabel, StatusTone, tone_color, toned_text};
use crate::table_selection::{ClusterObject, ResourceKey};
use crate::workload_actions::parse_replicas;

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
    /// The condition feeds the Resources group searched, for the empty text.
    pub(crate) searched_feeds: Vec<ResourceKind>,
    /// What the name index is searching, searched, or could not list.
    pub(crate) name_index: IndexSummary,
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

/// The least time between two rankings that only a shell change asked for.
const SHELL_REFRESH_INTERVAL: Duration = Duration::from_millis(250);

/// What a shell change does to the ranking.
#[derive(Debug, PartialEq, Eq)]
enum ShellRefresh {
    Now,
    After(Duration),
    /// A ranking is already due, or a timer for one is running.
    AlreadyPending,
}

/// Lets one shell-caused ranking through per `SHELL_REFRESH_INTERVAL`. Pure over the instants it
/// is given, so a test needs no clock.
#[derive(Default)]
struct ShellRefreshThrottle {
    last_refresh: Option<Instant>,
    is_pending: bool,
}

impl ShellRefreshThrottle {
    fn on_shell_changed(&mut self, now: Instant) -> ShellRefresh {
        if self.is_pending {
            return ShellRefresh::AlreadyPending;
        }
        self.is_pending = true;
        let wait = self.last_refresh.map_or(Duration::ZERO, |last| {
            SHELL_REFRESH_INTERVAL.saturating_sub(now.saturating_duration_since(last))
        });
        if wait.is_zero() {
            ShellRefresh::Now
        } else {
            ShellRefresh::After(wait)
        }
    }

    /// Any ranking, a keystroke's included, covers the pending shell change.
    fn on_refreshed(&mut self, now: Instant) {
        self.last_refresh = Some(now);
        self.is_pending = false;
    }
}

/// A ranked entry with the matched characters of its label and detail.
struct ShownEntry {
    entry: PaletteEntry,
    ranges: EntryRanges,
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
    shown: Vec<ShownEntry>,
    more: usize,
    context: PaletteContext,
    query: String,
    /// The query changed, or the shell changed and the throttle let it through, since `shown` was
    /// ranked. Ranking happens once per frame, in `render`, however many notifications arrived.
    is_stale: bool,
    /// Keeps a burst of shell changes (each batched watch update) from ranking again and again.
    shell_refresh: ShellRefreshThrottle,
    /// Marks the ranking stale once the throttle's wait is over; dropped by the next ranking.
    shell_refresh_timer: Option<Task<()>>,
    /// The resource to highlight once the next ranking has placed it (after a Tab preview moved
    /// the cursor, the row actions of the new cursor row join the list above it).
    reselect: Option<ClusterObject>,
    /// Opened by `:` and still showing just that: the first Esc (or Backspace) closes at once.
    is_seed_untouched: bool,
    /// The replicas field that `Ctrl Enter` on Scale turns the query into.
    argument: Option<ReplicasArgument>,
    _shell_observer: Subscription,
}

/// The inline field of an entry that takes a number: the replicas of Scale (W9 note, `Ctrl Enter`).
struct ReplicasArgument {
    /// `Replicas for deployment/payments-api (now 3)`.
    prompt: SharedString,
    input: Entity<InputState>,
    /// The last Enter found no whole number; typing clears it.
    has_error: bool,
    _subscription: Subscription,
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
        let shell_observer = cx.observe(shell, |palette, _, cx| palette.on_shell_changed(cx));
        let mut palette = Self {
            state,
            shell: shell.downgrade(),
            shell_focus,
            shown: Vec::new(),
            more: 0,
            context: snapshot.context.clone(),
            query: initial.to_owned(),
            is_stale: false,
            shell_refresh: ShellRefreshThrottle::default(),
            shell_refresh_timer: None,
            reselect: None,
            is_seed_untouched: initial == ":",
            argument: None,
            _shell_observer: shell_observer,
        };
        palette.rank(snapshot);
        // The shell is mid-update while it opens the palette, so the request waits for it.
        if lists_name_index(&parse_query(initial)) {
            let this = cx.weak_entity();
            cx.defer(move |cx| {
                let _ = this.update(cx, |palette, cx| palette.request_name_index(cx));
            });
        }
        palette
    }

    fn rank(&mut self, snapshot: PaletteSnapshot) {
        let query = parse_query(&self.query);
        let ranked = ranked(snapshot.entries, &query);
        self.more = ranked.more;
        // The underlines are computed here, once per ranking, for the shown rows only (at most
        // 100): a render only draws them.
        self.shown = ranked
            .entries
            .into_iter()
            .map(|entry| ShownEntry {
                ranges: entry_match_ranges(&entry, query.text),
                entry,
            })
            .collect();
        self.context = snapshot.context;
    }

    /// Builds the entries again from the shell and ranks them for the current query.
    fn refresh(&mut self, cx: &App) {
        self.is_stale = false;
        self.shell_refresh.on_refreshed(Instant::now());
        // A refresh already has what the waiting one would fetch.
        self.shell_refresh_timer = None;
        let Some(shell) = self.shell.upgrade() else {
            return;
        };
        let started = Instant::now();
        let snapshot = shell
            .read(cx)
            .palette_snapshot(&parse_query(&self.query), cx);
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

    /// A shell change (live statuses, W9 note 4) ranks again at once when the last ranking is
    /// old enough, else once the interval is over. A keystroke never waits (`on_query`).
    fn on_shell_changed(&mut self, cx: &mut Context<Self>) {
        match self.shell_refresh.on_shell_changed(Instant::now()) {
            ShellRefresh::Now => {
                self.is_stale = true;
                cx.notify();
            }
            ShellRefresh::After(wait) => {
                self.shell_refresh_timer = Some(cx.spawn(async move |palette, cx| {
                    cx.background_executor().timer(wait).await;
                    let _ = palette.update(cx, |palette, cx| {
                        palette.is_stale = true;
                        cx.notify();
                    });
                }));
            }
            ShellRefresh::AlreadyPending => {}
        }
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
        self.request_name_index(cx);
        cx.notify();
    }

    /// A query of two or more characters asks the session to list the names of the kinds no feed
    /// holds; the session starts at most one run per 120 s.
    fn request_name_index(&self, cx: &mut Context<Self>) {
        if lists_name_index(&parse_query(&self.query)) {
            self.update_shell(cx, |shell, cx| shell.request_name_index(cx));
        }
    }

    /// The entries the kit draws, one list per non-empty group in `PaletteGroup::ALL` order: the
    /// position in this list is the kit's section.
    fn sections(&self) -> Vec<Vec<&ShownEntry>> {
        PaletteGroup::ALL
            .into_iter()
            .map(|group| {
                self.shown
                    .iter()
                    .filter(|shown| shown.entry.group == group)
                    .collect::<Vec<_>>()
            })
            .filter(|members| !members.is_empty())
            .collect()
    }

    fn entry_at(&self, path: IndexPath) -> Option<&PaletteEntry> {
        let shown = self.sections().get(path.section)?.get(path.row).copied()?;
        Some(&shown.entry)
    }

    /// The kit path of the first shown entry `is_wanted` picks.
    fn path_of(&self, is_wanted: impl Fn(&PaletteEntry) -> bool) -> Option<IndexPath> {
        self.sections()
            .iter()
            .enumerate()
            .find_map(|(section, members)| {
                let row = members.iter().position(|shown| is_wanted(&shown.entry))?;
                Some(IndexPath::new(row).section(section))
            })
    }

    /// The resource under the kit's highlight.
    fn highlighted_resource(&self, cx: &App) -> Option<&ClusterObject> {
        let path = self.state.read(cx).selected_index()?;
        match &self.entry_at(path)?.target {
            PaletteTarget::Resource(object) => Some(object),
            _ => None,
        }
    }

    /// Whether Tab would move the table cursor: the highlighted entry is a row of the screen that
    /// is open, the filter shows it, and no drawer is open. `selected` is the kit's highlight.
    fn can_preview(&self, selected: Option<IndexPath>, cx: &App) -> bool {
        let Some(PaletteTarget::Resource(object)) = selected
            .and_then(|path| self.entry_at(path))
            .map(|entry| &entry.target)
        else {
            return false;
        };
        self.shell
            .upgrade()
            .is_some_and(|shell| shell.read(cx).can_preview_row(object, cx))
    }

    /// Tab: moves the table cursor behind the scrim to the highlighted resource, without opening
    /// its drawer, switching screen, or starting a watch. Anywhere else it does nothing.
    fn preview(&mut self, cx: &mut Context<Self>) {
        let Some(object) = self.highlighted_resource(cx).cloned() else {
            return;
        };
        let Some(shell) = self.shell.upgrade() else {
            return;
        };
        shell.update(cx, |shell, cx| shell.preview_resource(&object, cx));
        self.reselect = Some(object);
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
            PaletteTarget::RollBack(object, revision) => self.update_shell(cx, |shell, cx| {
                shell.start_roll_back(&object, &revision, window, cx);
            }),
            PaletteTarget::ObjectAction(object, action) => self.update_shell(cx, |shell, cx| {
                shell.run_row_action_on(object, action, window, cx);
            }),
            PaletteTarget::Screen(screen) => self.update_shell(cx, |shell, cx| {
                shell.show_screen(screen, cx);
            }),
            PaletteTarget::Resource(object) => {
                self.update_shell(cx, |shell, cx| shell.reveal_object(object, cx))
            }
            PaletteTarget::Namespace(scope) => {
                self.update_shell(cx, |shell, cx| shell.set_namespace(scope, cx));
            }
            PaletteTarget::Cluster(row, scope) => self.update_shell(cx, |shell, cx| {
                shell.switch_cluster_in_scope(&row.cluster, scope, cx);
            }),
        }
    }

    /// `Ctrl Enter` on an enabled Scale entry: the query becomes a replicas field for the cursor row.
    /// On any other entry, or a disabled one, nothing happens: the list stays as it is.
    fn ask_replicas(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.argument.is_some() {
            return;
        }
        let Some(path) = self.state.read(cx).selected_index() else {
            return;
        };
        let Some(entry) = self.entry_at(path) else {
            return;
        };
        let is_scale = matches!(entry.target, PaletteTarget::RowAction(RowAction::Scale));
        if !(is_scale && entry.is_enabled()) {
            return;
        }
        let Some(prompt) = self
            .shell
            .upgrade()
            .and_then(|shell| shell.read(cx).scale_prompt(cx))
        else {
            return;
        };
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Replicas")
                .validate(|text, _| text.bytes().all(|byte| byte.is_ascii_digit()))
        });
        // Enter is not read here: the key-down handler takes the fresh press (`on_argument_key`).
        let subscription = cx.subscribe_in(&input, window, |palette, _, event, _, cx| {
            if let InputEvent::Change = event {
                palette.clear_argument_error(cx);
            }
        });
        input.update(cx, |input, cx| input.focus(window, cx));
        self.argument = Some(ReplicasArgument {
            prompt: format!("Replicas for {prompt}").into(),
            input,
            has_error: false,
            _subscription: subscription,
        });
        cx.notify();
    }

    fn clear_argument_error(&mut self, cx: &mut Context<Self>) {
        if let Some(argument) = self.argument.as_mut().filter(|argument| argument.has_error) {
            argument.has_error = false;
            cx.notify();
        }
    }

    /// Enter in the replicas field: a whole number closes the palette and starts the scale (the
    /// confirm dialog follows); anything else says what is expected.
    fn submit_replicas(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Taking the argument makes a second Enter a no-op: the first one already closed the palette.
        let Some(mut argument) = self.argument.take() else {
            return;
        };
        let Some(replicas) = parse_replicas(&argument.input.read(cx).value()) else {
            argument.has_error = true;
            self.argument = Some(argument);
            cx.notify();
            return;
        };
        window.close_dialog(cx);
        self.update_shell(cx, |shell, cx| shell.scale_cursor_row(replicas, window, cx));
    }

    /// Esc in the replicas field: back to the list, with the query as it was.
    fn leave_argument(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.argument.take().is_none() {
            return;
        }
        self.state.update(cx, |state, cx| state.focus(window, cx));
        cx.notify();
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
        let empty = empty_text(
            mode,
            self.context.has_session,
            self.context.screen,
            &self.context.searched_feeds,
            &self.context.name_index,
        );
        let searching = (mode == PaletteMode::All && !self.context.name_index.searching.is_empty())
            .then(|| searching_text(&self.context.name_index));
        let header = HeaderChips::of(&self.context);
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
                    footer(can_preview, more, searching.as_deref(), cx)
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
            let heading = members
                .first()
                .map_or("", |shown| shown.entry.group.heading());
            let items = members.into_iter().map(command_item);
            command = command.group(CommandGroup::new().label(heading).items(items));
        }
        let argument = self.render_argument(cx);
        v_flex()
            // The list takes Enter itself (the kit's bindings are off in this context): a held
            // Enter repeats, and a repeat must never confirm an entry that opens a write dialog.
            .when(self.argument.is_none(), |root| {
                root.key_context(PALETTE_LIST)
                    .on_key_down(cx.listener(Self::on_list_key))
            })
            .on_action(cx.listener(|palette, _: &PalettePreview, _, cx| palette.preview(cx)))
            .on_action(cx.listener(|palette, _: &ScaleCursorRow, window, cx| {
                palette.ask_replicas(window, cx);
            }))
            .on_action(
                cx.listener(|palette, _: &LeavePaletteArgument, window, cx| {
                    palette.leave_argument(window, cx);
                }),
            )
            .child(argument.unwrap_or_else(|| command.into_any_element()))
    }
}

impl CommandPalette {
    /// Highlights the entry a Tab preview asked for once the ranking has placed it. The kit has not
    /// installed this render's list yet, so the highlight is set after the frame.
    fn schedule_reselect(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(object) = self.reselect.take() else {
            return;
        };
        let wanted = |entry: &PaletteEntry| matches!(&entry.target, PaletteTarget::Resource(other) if *other == object);
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
    fn of(context: &PaletteContext) -> Self {
        Self {
            cluster: context.cluster.clone(),
            scope: context.scope_label.clone(),
        }
    }

    fn render(&self, cx: &App) -> impl IntoElement + use<> {
        let cluster = match &self.cluster {
            Some(cluster) => h_flex()
                .gap_1()
                .items_center()
                .child(environment_badge(&cluster.environment, cx))
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

impl CommandPalette {
    /// Enter in the replicas field, as a fresh key press: the kit Dialog confirms on the same key
    /// and would close the palette before the number is read, so the key is bound to nothing here
    /// (`keymap.rs`) and handled once, like the confirm dialog does. A held Enter does nothing.
    fn on_argument_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !is_enter(event) {
            return;
        }
        window.prevent_default();
        cx.stop_propagation();
        if confirms(event) {
            self.submit_replicas(window, cx);
        }
    }

    /// Enter on the list, as a fresh key press: it confirms the highlighted entry. A held Enter
    /// does nothing, and neither does a modified one (Ctrl Enter has its own binding).
    fn on_list_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !is_enter(event) {
            return;
        }
        window.prevent_default();
        cx.stop_propagation();
        if !confirms(event) {
            return;
        }
        if let Some(path) = self.state.read(cx).selected_index() {
            self.confirm(path, window, cx);
        }
    }

    /// The palette body while the field is open: the header, the prompt, the field, and a hint.
    /// `None` while the list shows.
    fn render_argument(&self, cx: &mut Context<Self>) -> Option<gpui_kit::AnyElement> {
        let argument = self.argument.as_ref()?;
        let header = HeaderChips::of(&self.context);
        let theme = cx.theme();
        let error = argument.has_error.then(|| {
            div()
                .text_sm()
                .text_color(theme.danger)
                .child("Enter a whole number")
        });
        let body = v_flex()
            .key_context("PaletteArgument")
            .on_key_down(cx.listener(Self::on_argument_key))
            .w_full()
            .child(header.render(cx))
            .child(
                v_flex()
                    .gap_2()
                    .px_3()
                    .pb_3()
                    .child(div().text_sm().child(argument.prompt.clone()))
                    .child(Input::new(&argument.input))
                    .children(error),
            )
            .child(
                h_flex()
                    .w_full()
                    .px_3()
                    .py_2()
                    .border_t_1()
                    .border_color(theme.border)
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(div().ml_auto().child("⏎ scale · Esc back to the list")),
            );
        Some(body.into_any_element())
    }
}

/// The syntax footer (W9 note 5), with `Tab preview` only while it applies and "+N more" when a
/// cap cut a group. While the name index loads, `searching` says what is still being listed in
/// place of the syntax line.
fn footer(
    can_preview: bool,
    more: usize,
    searching: Option<&str>,
    cx: &App,
) -> impl IntoElement + use<> {
    let searching = searching.map(str::to_owned);
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
        .when(searching.is_none(), |footer| {
            footer.children(SYNTAX_HINTS.map(|hint| div().child(hint)))
        })
        .children(searching.map(|text| div().child(text)))
        .when(more > 0, |footer| {
            footer.child(div().child(format!("+{more} more")))
        })
        .child(div().ml_auto().child(keys))
}

/// The kit item of an entry. Its content is custom, so the kit adds no hint of its own, and it
/// carries no action: the palette runs the target itself, on the shell (decision 11).
fn command_item(shown: &ShownEntry) -> CommandItem {
    let entry = &shown.entry;
    let row = RowContent::of(entry, shown.ranges.clone());
    let is_disabled = !entry.is_enabled();
    CommandItem::new()
        .label(entry.label.clone())
        .disabled(is_disabled)
        .child(move |_, cx| row.render(cx))
}

/// A cluster row's environment and health, which stand in for the status pill.
#[derive(Clone)]
struct ClusterLine {
    environment: Environment,
    health: RowHealth,
}

/// Everything one row draws, owned so the kit may build the row again for measuring.
struct RowContent {
    icon: IconName,
    label: SharedString,
    detail: Option<SharedString>,
    /// The characters the query matched in the label and the detail, drawn underlined.
    ranges: EntryRanges,
    status: Option<StatusLabel>,
    reason: Option<SharedString>,
    /// An enabled entry whose action reaches a 0030 confirm.
    needs_confirm: bool,
    is_current: bool,
    cluster: Option<ClusterLine>,
    /// The 0028 action whose first key is the hint. Cluster rows show none: their `Ctrl n`
    /// numbers belong to the switcher.
    key_action: Option<Box<dyn Action>>,
}

impl RowContent {
    fn of(entry: &PaletteEntry, ranges: EntryRanges) -> Self {
        let key_action = match &entry.target {
            PaletteTarget::Command(action) => Some(action.boxed_clone()),
            PaletteTarget::RowAction(action) => Some(action.key_action()),
            // The revision is in the label, and the hint of Roll back… is its menu item, not this.
            PaletteTarget::RollBack(..) => None,
            PaletteTarget::ObjectAction(_, action) => Some(action.key_action()),
            PaletteTarget::Screen(_)
            | PaletteTarget::Resource(_)
            | PaletteTarget::Namespace(_)
            | PaletteTarget::Cluster(..) => None,
        };
        let cluster = match &entry.target {
            PaletteTarget::Cluster(row, _) => Some(ClusterLine {
                environment: row.environment.clone(),
                health: row.health,
            }),
            _ => None,
        };
        Self {
            icon: row_icon(&entry.target),
            label: entry.label.clone(),
            detail: entry.detail.clone().or_else(|| entry.note.clone()),
            ranges,
            status: entry.status.clone(),
            reason: match &entry.state {
                EntryState::Enabled => None,
                EntryState::Disabled { reason } => Some(reason.clone()),
            },
            needs_confirm: entry.needs_confirm,
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
            .child(Icon::new(self.icon).size_4());
        h_flex()
            .w_full()
            .gap_2()
            .items_center()
            .child(leading)
            .when_some(self.cluster.as_ref(), |row, cluster| {
                row.child(environment_badge(&cluster.environment, cx))
            })
            .child(
                div()
                    .flex_none()
                    .child(underlined(self.label.clone(), &self.ranges.label)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_xs()
                    .text_color(muted)
                    .font_family(theme.mono_font_family.clone())
                    .children(
                        self.detail
                            .clone()
                            .map(|detail| underlined(detail, &self.ranges.detail)),
                    ),
            )
            .when(self.is_current, |row| {
                row.child(Icon::new(IconName::Check).size_4())
            })
            .children(
                self.status
                    .clone()
                    .map(|status| div().text_xs().child(toned_text(status, cx))),
            )
            .when_some(self.cluster.as_ref(), |row, cluster| {
                row.child(
                    div()
                        .text_xs()
                        .text_color(health_color(cluster.health, cx))
                        .child(health_text(cluster.health)),
                )
            })
            .children(self.reason.clone().map(|reason| reason_pill(reason, cx)))
            .when(self.needs_confirm, |row| {
                row.child(reason_pill(NEEDS_CONFIRM.into(), cx))
            })
            .children(keys.into_iter().take(1).map(Kbd::new))
    }
}

/// `text` with the matched byte ranges underlined. The underline takes the text color (muted for
/// the detail, and again muted on a disabled row), so it adds no color of its own.
fn underlined(text: SharedString, ranges: &[Range<usize>]) -> StyledText {
    let highlight = HighlightStyle {
        underline: Some(UnderlineStyle {
            thickness: px(1.),
            color: None,
            wavy: false,
        }),
        ..Default::default()
    };
    let highlights: Vec<_> = ranges
        .iter()
        .map(|range| (range.clone(), highlight))
        .collect();
    StyledText::new(text).with_highlights(highlights)
}

/// The icon before a label: an icon for an action, else the icon of the screen, resource kind,
/// namespace, or cluster.
fn row_icon(target: &PaletteTarget) -> IconName {
    match target {
        PaletteTarget::Command(action) => command_icon(&**action),
        PaletteTarget::RowAction(action) => action.icon(),
        PaletteTarget::RollBack(..) => RowAction::RollBack.icon(),
        PaletteTarget::ObjectAction(_, action) => action.icon(),
        PaletteTarget::Screen(screen) => screen_icon(*screen),
        PaletteTarget::Resource(ClusterObject {
            key: ResourceKey::Pod { .. },
            ..
        }) => POD_ICON,
        PaletteTarget::Resource(ClusterObject {
            key: ResourceKey::Node { .. },
            ..
        }) => NODE_ICON,
        PaletteTarget::Resource(ClusterObject {
            key: ResourceKey::Kind { kind, .. },
            ..
        }) => kind.icon(),
        PaletteTarget::Namespace(_) => IconName::Folder,
        PaletteTarget::Cluster(..) => IconName::Building2,
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

/// What an enabled entry says when its action reaches a 0030 confirm (W9 note 3).
const NEEDS_CONFIRM: &str = "needs confirm";

/// The pill of a disabled entry: its reason, outlined in the theme's warning tone (the kit tag's
/// own colors wash out on a muted, disabled row). `needs confirm` uses it too.
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

#[cfg(test)]
mod tests {
    use super::*;
    use cluster::NamespaceScope;

    use crate::cluster_registry::ClusterRef;
    use crate::cluster_switcher_rows::SwitcherRow;
    use crate::resource_kind::ResourceKind;

    fn cluster_ref() -> ClusterRef {
        ClusterRef {
            kubeconfig: "config.yaml".into(),
            context: "dev".to_owned(),
        }
    }

    fn cluster_row() -> SwitcherRow {
        SwitcherRow {
            cluster: cluster_ref(),
            label: "dev".to_owned(),
            environment: Environment::DEVELOPMENT,
            health: RowHealth::NotChecked,
            failure: None,
            note: None,
            shortcut: None,
            is_active: false,
            search_text: String::new(),
        }
    }

    #[test]
    fn rows_lead_with_the_icon_of_their_target() {
        let pod = ClusterObject::new(
            cluster_ref(),
            ResourceKey::Pod {
                namespace: "default".to_owned(),
                name: "web".to_owned(),
            },
        );
        let screen = Screen::Kind(ResourceKind::Services);
        assert_eq!(
            row_icon(&PaletteTarget::Screen(screen)),
            screen_icon(screen)
        );
        assert_eq!(row_icon(&PaletteTarget::Resource(pod)), POD_ICON);
        assert_eq!(
            row_icon(&PaletteTarget::Namespace(NamespaceScope::All)),
            IconName::Folder
        );
        assert_eq!(
            row_icon(&PaletteTarget::Cluster(cluster_row(), None)),
            IconName::Building2
        );
        assert_eq!(
            row_icon(&PaletteTarget::RowAction(RowAction::Delete)),
            IconName::Trash
        );
    }

    #[test]
    fn the_first_shell_change_ranks_at_once() {
        let mut throttle = ShellRefreshThrottle::default();
        assert_eq!(throttle.on_shell_changed(Instant::now()), ShellRefresh::Now);
    }

    #[test]
    fn a_burst_of_shell_changes_ranks_once_per_interval() {
        let start = Instant::now();
        let mut throttle = ShellRefreshThrottle::default();
        throttle.on_refreshed(start);
        let at = |millis: u64| start + Duration::from_millis(millis);
        // The first change after a ranking waits out the rest of the interval...
        assert_eq!(
            throttle.on_shell_changed(at(50)),
            ShellRefresh::After(Duration::from_millis(200))
        );
        // ...and the rest of the burst adds nothing.
        for millis in [60, 120, 240] {
            assert_eq!(
                throttle.on_shell_changed(at(millis)),
                ShellRefresh::AlreadyPending
            );
        }
        // Once a ranking ran, the next change after the interval is let through at once.
        throttle.on_refreshed(at(250));
        assert_eq!(throttle.on_shell_changed(at(600)), ShellRefresh::Now);
    }

    #[test]
    fn a_keystroke_ranking_covers_the_pending_shell_change() {
        let start = Instant::now();
        let mut throttle = ShellRefreshThrottle::default();
        throttle.on_refreshed(start);
        let after = |wait: ShellRefresh| matches!(wait, ShellRefresh::After(_));
        assert!(after(throttle.on_shell_changed(start)));
        // A keystroke ranked the fresh shell state itself, so the next change starts a new wait.
        throttle.on_refreshed(start + Duration::from_millis(10));
        assert!(after(
            throttle.on_shell_changed(start + Duration::from_millis(20))
        ));
    }
}
