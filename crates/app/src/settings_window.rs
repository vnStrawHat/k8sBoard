//! The Settings window: a second OS window, one instance, over the shared `AppSettings` global
//! and the `ClusterCatalog`. The pages are the kit's `Settings` component.

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::Input;
use gpui_kit::component::label::Label;
use gpui_kit::component::setting::{
    SelectIndex, SettingField, SettingGroup, SettingItem, SettingPage, Settings,
};
use gpui_kit::component::{
    ActiveTheme as _, Icon, Sizable as _, StyledExt as _, TitleBar, h_flex, v_flex,
};
use gpui_kit::{
    AnyElement, AnyWindowHandle, App, AppContext as _, Bounds, Context, Entity, FocusHandle,
    Focusable, Global, InteractiveElement as _, IntoElement, ParentElement as _, PathPromptOptions,
    Render, SharedString, Styled as _, Subscription, WeakEntity, Window, WindowBounds, WindowId,
    WindowOptions, div, prelude::FluentBuilder as _, px, size,
};

use crate::active_session::ActiveConnection;
use crate::audit_log::audit_path;
use crate::cluster_catalog::CatalogHandle;
use crate::cluster_form::MoveStep;
use crate::cluster_registry::ClusterRef;
use crate::clusters_page::{ClustersPage, add_cluster_button};
use crate::environment::{CustomEnvironment, EnvironmentTier, usable_environments};
use crate::environments_page::EnvironmentsPage;
use crate::settings::{
    AppSettings, COLOR_THEME_OPTIONS, DENSITY_OPTIONS, FONT_SIZE_OPTIONS, OptionTable,
    SCROLLBACK_OPTIONS, SHELL_OPTIONS, Settings as SettingsData, TAIL_OPTIONS, theme_choices,
    theme_from_label, theme_label,
};
use crate::shortcut_sheet::shortcut_sheet;
use crate::usage_format::group_digits;
use crate::write_guard::{ActionRisk, ConfirmMode, DialogConfirm, confirm_step};

gpui_kit::actions!(
    k8sboard,
    [
        OpenSettings,
        ManageClusters,
        ImportKubeconfig,
        MoveClusterUp,
        MoveClusterDown
    ]
);

const WINDOW_WIDTH: f32 = 1000.;
const WINDOW_HEIGHT: f32 = 620.;
/// For a screenshot of the whole Clusters form, which is taller than the standard window.
const TALL_WINDOW_HEIGHT: f32 = 900.;
const SIDEBAR_WIDTH: f32 = 250.;
/// The search box of the Clusters header (W2).
const SEARCH_WIDTH: f32 = 200.;
/// The kit group of the Clusters page that holds the Metrics section. The group has no title, so
/// the kit sidebar gets no sub-item; the kit scrolls to a group by its index.
const CLUSTER_METRICS_GROUP: usize = 1;

/// The pages in W2 nav order, keeping only those with content. A later spec inserts its page
/// at its W2 position.
const PAGES: [SettingsPage; 9] = [
    SettingsPage::General,
    SettingsPage::Clusters,
    SettingsPage::Environments,
    SettingsPage::Appearance,
    SettingsPage::KeyboardShortcuts,
    SettingsPage::Safety,
    SettingsPage::TerminalAndShell,
    SettingsPage::Logs,
    SettingsPage::About,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SettingsPage {
    General,
    Clusters,
    Environments,
    Appearance,
    KeyboardShortcuts,
    Safety,
    TerminalAndShell,
    Logs,
    About,
}

impl SettingsPage {
    fn title(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Clusters => "Clusters",
            Self::Environments => "Environments",
            Self::Appearance => "Appearance",
            Self::KeyboardShortcuts => "Keyboard Shortcuts",
            Self::Safety => "Safety",
            Self::TerminalAndShell => "Terminal & Shell",
            Self::Logs => "Logs",
            Self::About => "About",
        }
    }

    fn icon(self) -> IconName {
        match self {
            Self::General => IconName::Settings,
            Self::Clusters => IconName::Building2,
            Self::Environments => IconName::Tag,
            Self::Appearance => IconName::Palette,
            Self::KeyboardShortcuts => IconName::Keyboard,
            Self::Safety => IconName::ShieldCheck,
            Self::TerminalAndShell => IconName::SquareTerminal,
            Self::Logs => IconName::FileText,
            Self::About => IconName::Info,
        }
    }

    fn index(self) -> usize {
        PAGES
            .iter()
            .position(|page| *page == self)
            .unwrap_or_default()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SettingsSize {
    Standard,
    Tall,
}

impl SettingsSize {
    fn height(self) -> f32 {
        match self {
            Self::Standard => WINDOW_HEIGHT,
            Self::Tall => TALL_WINDOW_HEIGHT,
        }
    }
}

/// The open Settings window, so a second request activates it instead of opening another.
#[derive(Default)]
pub(crate) struct SettingsWindowHandle(Option<OpenWindow>);

#[derive(Clone)]
struct OpenWindow {
    window: AnyWindowHandle,
    view: WeakEntity<SettingsWindow>,
}

impl Global for SettingsWindowHandle {}

/// Activates the Settings window (its page stays), or opens it on `page`. `None` when it could
/// not open.
pub(crate) fn open_settings_window(
    page: SettingsPage,
    window_size: SettingsSize,
    cx: &mut App,
) -> Option<AnyWindowHandle> {
    let open = open_window_of(cx);
    if let Some(open) = open {
        // An error means the window is gone, which `forget_closed_window` normally prevents.
        if open
            .window
            .update(cx, |_, window, _| window.activate_window())
            .is_ok()
        {
            return Some(open.window);
        }
    }
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(WINDOW_WIDTH), px(window_size.height())),
            cx,
        ))),
        ..TitleBar::window_options()
    };
    let opened = gpui_kit::open_window(options, cx, |window, cx| {
        cx.new(|cx| SettingsWindow::new(page, window, cx))
    });
    match opened {
        Ok((window, view)) => {
            let view = view.downgrade();
            cx.set_global(SettingsWindowHandle(Some(OpenWindow { window, view })));
            Some(window)
        }
        Err(error) => {
            tracing::error!(%error, "failed to open the Settings window");
            None
        }
    }
}

/// "Manage clusters…": the Settings window on the Clusters page, opened or brought forward. Unlike
/// `Ctrl ,`, it leaves another page of an open window.
pub(crate) fn manage_clusters(cx: &mut App) {
    show_clusters_page(cx);
}

fn show_clusters_page(cx: &mut App) -> Option<AnyWindowHandle> {
    if let Some(open) = open_window_of(cx) {
        let _ = open
            .view
            .update(cx, |view, cx| view.show_page(SettingsPage::Clusters, cx));
    }
    open_settings_window(SettingsPage::Clusters, SettingsSize::Standard, cx)
}

/// The two ways the first-run screen adds a cluster; both flows belong to the Clusters page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ClusterAddition {
    ImportFile,
    PasteYaml,
}

/// Opens the Clusters page and starts `how` there, so the first-run screen needs no dialogs of its
/// own.
pub(crate) fn add_cluster(how: ClusterAddition, cx: &mut App) {
    let Some(window) = show_clusters_page(cx) else {
        return;
    };
    let Some(view) = open_window_of(cx).and_then(|open| open.view.upgrade()) else {
        return;
    };
    let _ = window.update(cx, |_, window, cx| {
        let page = view.read(cx).clusters.clone();
        page.update(cx, |page, cx| match how {
            ClusterAddition::ImportFile => page.import_file(window, cx),
            ClusterAddition::PasteYaml => page.start_paste(window, cx),
        });
    });
}

/// Settings › Clusters with the open cluster selected and its Metrics section scrolled into view;
/// opens the window or brings it forward. `None` when the window could not open.
pub(crate) fn show_cluster_metrics(cx: &mut App) -> Option<AnyWindowHandle> {
    let cluster = cx
        .try_global::<ActiveConnection>()
        .map(|active| active.cluster.clone());
    let window = open_settings_window(SettingsPage::Clusters, SettingsSize::Standard, cx)?;
    let view = open_window_of(cx).and_then(|open| open.view.upgrade())?;
    let _ = window.update(cx, |_, _, cx| {
        view.update(cx, |view, cx| view.select_cluster_metrics(cluster, cx));
    });
    Some(window)
}

fn open_window_of(cx: &App) -> Option<OpenWindow> {
    cx.try_global::<SettingsWindowHandle>()
        .and_then(|handle| handle.0.clone())
}

/// Whether the open Settings window is scrolled to the Metrics section and that section still
/// waits for its detection; a screenshot of it waits too.
#[cfg(feature = "screenshot")]
pub(crate) fn is_cluster_metrics_pending(cx: &App) -> bool {
    let Some(view) = open_window_of(cx).and_then(|open| open.view.upgrade()) else {
        return false;
    };
    let window = view.read(cx);
    window.first_group == Some(CLUSTER_METRICS_GROUP)
        && !window
            .clusters
            .read(cx)
            .metrics_section()
            .read(cx)
            .is_settled()
}

/// Clears the handle when the Settings window closes.
pub(crate) fn forget_closed_window(id: WindowId, cx: &mut App) {
    let Some(handle) = cx.try_global::<SettingsWindowHandle>() else {
        return;
    };
    if handle
        .0
        .as_ref()
        .is_some_and(|open| open.window.window_id() == id)
    {
        cx.set_global(SettingsWindowHandle(None));
    }
}

/// Registers the close hook: closing the main window quits the app (Settings closes with it),
/// closing Settings does not. `quit` is `|cx| cx.quit()` in production; tests pass a flag setter.
pub(crate) fn quit_when_main_window_closes(
    main: WindowId,
    quit: impl Fn(&mut App) + 'static,
    cx: &mut App,
) {
    cx.on_window_closed(move |cx, id| {
        forget_closed_window(id, cx);
        // A second quit while the app shuts down is harmless.
        if id == main || cx.windows().is_empty() {
            quit(cx);
        }
    })
    .detach();
}

/// Holds no clipboard text or credential: the pages read the shared globals when they render.
pub(crate) struct SettingsWindow {
    first_page: SettingsPage,
    /// The kit group of `first_page` to scroll to; `None` is the top of the page.
    first_group: Option<usize>,
    /// Changes with each `show_page`: a new key gives the kit `Settings` a fresh selection.
    page_generation: usize,
    clusters: Entity<ClustersPage>,
    environments: Entity<EnvironmentsPage>,
    focus_handle: FocusHandle,
    _observers: Vec<Subscription>,
}

impl SettingsWindow {
    fn new(first_page: SettingsPage, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let catalog = CatalogHandle::of(cx);
        // A status left over from a closed window must not show here.
        catalog.update(cx, |catalog, cx| catalog.reset_paste_status(cx));
        let clusters = cx.new(|cx| ClustersPage::new(catalog.clone(), window, cx));
        let environments = cx.new(|cx| EnvironmentsPage::new(window, cx));
        Self {
            first_page,
            first_group: None,
            page_generation: 0,
            clusters,
            environments,
            focus_handle: cx.focus_handle(),
            _observers: vec![
                cx.observe_global::<AppSettings>(|_, cx| cx.notify()),
                cx.observe(&catalog, |_, _, cx| cx.notify()),
            ],
        }
    }

    fn show_page(&mut self, page: SettingsPage, cx: &mut Context<Self>) {
        self.first_page = page;
        self.first_group = None;
        self.page_generation += 1;
        cx.notify();
    }

    /// Clusters page scrolled to the Metrics section, with `cluster` selected when there is one.
    fn select_cluster_metrics(&mut self, cluster: Option<ClusterRef>, cx: &mut Context<Self>) {
        self.first_page = SettingsPage::Clusters;
        self.first_group = Some(CLUSTER_METRICS_GROUP);
        self.page_generation += 1;
        if let Some(cluster) = cluster {
            self.clusters
                .update(cx, |clusters, cx| clusters.select(cluster, cx));
        }
        cx.notify();
    }

    fn pages(&self, cx: &App) -> Vec<SettingPage> {
        PAGES
            .iter()
            .map(|page| {
                let built = match page {
                    SettingsPage::General => general_page(),
                    SettingsPage::Clusters => clusters_page(&self.clusters, cx),
                    SettingsPage::Environments => environments_page(&self.environments),
                    SettingsPage::Appearance => appearance_page(),
                    SettingsPage::KeyboardShortcuts => keyboard_shortcuts_page(),
                    SettingsPage::Safety => safety_page(),
                    SettingsPage::TerminalAndShell => terminal_page(),
                    SettingsPage::Logs => logs_page(),
                    SettingsPage::About => about_page(cx),
                };
                built.icon(page.icon())
            })
            .collect()
    }
}

impl Focusable for SettingsWindow {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for SettingsWindow {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        v_flex()
            .size_full()
            .key_context("SettingsWindow")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(|this, _: &ImportKubeconfig, window, cx| {
                this.clusters
                    .update(cx, |page, cx| page.import_file(window, cx));
            }))
            .on_action(cx.listener(|this, _: &MoveClusterUp, _, cx| {
                this.clusters
                    .update(cx, |page, cx| page.step_selected(MoveStep::Up, cx));
            }))
            .on_action(cx.listener(|this, _: &MoveClusterDown, _, cx| {
                this.clusters
                    .update(cx, |page, cx| page.step_selected(MoveStep::Down, cx));
            }))
            .bg(theme.background)
            .text_color(theme.foreground)
            .child(TitleBar::new().child(div().font_semibold().child("Settings")))
            .child(
                div().flex_1().min_h_0().child(
                    Settings::new(("settings", self.page_generation))
                        .sidebar_width(px(SIDEBAR_WIDTH))
                        .default_selected_index(SelectIndex {
                            page_ix: self.first_page.index(),
                            group_ix: self.first_group,
                        })
                        .pages(self.pages(cx)),
                ),
            )
    }
}

/// One element item: the list and the form are a view of their own, because the item closure
/// only gets the app context.
fn clusters_page(page: &Entity<ClustersPage>, cx: &App) -> SettingPage {
    let (body, add) = (page.clone(), page.clone());
    let metrics = page.read(cx).metrics_section();
    let search = page.read(cx).search_input();
    let blocked = ClustersPage::paste_blocked_reason(cx);
    SettingPage::new(SettingsPage::Clusters.title())
        .resettable(false)
        .description(page.read(cx).description(cx))
        .title_suffix(move |_, _| {
            h_flex()
                .gap_2()
                .items_center()
                .child(Input::new(&search).w(px(SEARCH_WIDTH)))
                .child(add_cluster_button(add.clone(), blocked))
        })
        .group(SettingGroup::new().item(SettingItem::render(move |_, _, _| body.clone())))
        // Always present, so group 1 exists on the first frame (the kit drops a missing index).
        .group(SettingGroup::new().item(SettingItem::render(move |_, _, _| metrics.clone())))
}

/// The Environments page: one element item, because the rows and their inputs are a view of their
/// own (the item closure only gets the app context).
fn environments_page(page: &Entity<EnvironmentsPage>) -> SettingPage {
    let body = page.clone();
    SettingPage::new(SettingsPage::Environments.title())
        .resettable(false)
        .description("Group clusters and choose how changes to them are confirmed.")
        .group(SettingGroup::new().item(SettingItem::render(move |_, _, _| body.clone())))
}

/// The General page: where exports start, and what the Issues engine watches.
fn general_page() -> SettingPage {
    let folder = SettingField::render(|_, _, cx| export_folder_field(cx)).on_reset(
        |cx| AppSettings::get(cx).general.export_dir.is_some(),
        |_, cx| AppSettings::update(cx, |settings| settings.general.export_dir = None),
    );
    let tls = SettingField::switch(
        |cx| AppSettings::get(cx).general.watch_tls_secrets,
        |value, cx| AppSettings::update(cx, |settings| settings.general.watch_tls_secrets = value),
    )
    .default_value(SettingsData::default().general.watch_tls_secrets);
    SettingPage::new(SettingsPage::General.title())
        .group(
            SettingGroup::new().title("Files").item(
                SettingItem::new("Export folder", folder)
                    .description("Where Export dialogs start. Updated after each export."),
            ),
        )
        .group(
            SettingGroup::new().title("Issues").item(
                SettingItem::new("Watch TLS Secrets for expiry", tls).description(
                    "Lists and watches Secrets of type kubernetes.io/tls; this shows in API audit logs. Applies when the cluster is opened again.",
                ),
            ),
        )
}

/// The saved export folder in monospace (or the home folder, muted), a picker, and the way back
/// to the home folder.
fn export_folder_field(cx: &App) -> AnyElement {
    let stored = AppSettings::get(cx).general.export_dir.clone();
    let shown = match &stored {
        Some(dir) => div()
            .text_sm()
            .font_family(cx.theme().mono_font_family.clone())
            .child(dir.display().to_string())
            .into_any_element(),
        None => muted_note("Home folder", cx),
    };
    h_flex()
        .gap_2()
        .items_center()
        .child(shown)
        .child(
            Button::new("choose-export-folder")
                .ghost()
                .small()
                .icon(Icon::new(IconName::FolderOpen))
                .label("Choose…")
                .on_click(|_, _, cx| choose_export_folder(cx)),
        )
        .when(stored.is_some(), |row| {
            row.child(
                Button::new("use-home-export-folder")
                    .ghost()
                    .small()
                    .label("Use home folder")
                    .on_click(|_, _, cx| {
                        AppSettings::update(cx, |settings| settings.general.export_dir = None);
                    }),
            )
        })
        .into_any_element()
}

/// Opens the folder picker and saves the folder it returns; a cancel changes nothing.
fn choose_export_folder(cx: &mut App) {
    let picked = cx.prompt_for_paths(PathPromptOptions {
        files: false,
        directories: true,
        multiple: false,
        prompt: Some("Export folder".into()),
    });
    cx.spawn(async move |cx| {
        let Ok(Ok(Some(paths))) = picked.await else {
            return;
        };
        let Some(folder) = paths.into_iter().next() else {
            return;
        };
        cx.update(|cx| {
            AppSettings::update(cx, |settings| settings.general.export_dir = Some(folder));
        });
    })
    .detach();
}

/// A dropdown over an option table: `get` reads the stored value, `set` stores the picked one.
/// A stored value outside the table shows as `unlisted` says. The page's Reset puts back the
/// value of `SettingsData::default()`.
fn table_dropdown<T: Copy + PartialEq + 'static>(
    table: &'static OptionTable<T>,
    get: fn(&SettingsData) -> T,
    set: fn(&mut SettingsData, T),
    unlisted: fn(T) -> String,
) -> SettingField<SharedString> {
    let default = get(&SettingsData::default());
    SettingField::dropdown(
        table.choices(),
        move |cx| {
            let value = get(AppSettings::get(cx));
            table.label(value, || unlisted(value))
        },
        move |label, cx| {
            let value = table.value(&label);
            AppSettings::update(cx, |settings| set(settings, value));
        },
    )
    .default_value(table.label(default, || unlisted(default)))
}

/// Defaults for new log tabs; a tab can still change them from its toolbar.
fn logs_page() -> SettingPage {
    let tail = table_dropdown(
        &TAIL_OPTIONS,
        |settings| settings.logs.tail_lines,
        |settings, lines| settings.logs.tail_lines = lines,
        |lines| format!("{} lines", group_digits(lines as usize)),
    );
    let timestamps = SettingField::switch(
        |cx| AppSettings::get(cx).logs.show_timestamps,
        |value, cx| AppSettings::update(cx, |settings| settings.logs.show_timestamps = value),
    )
    .default_value(SettingsData::default().logs.show_timestamps);
    let wrap = SettingField::switch(
        |cx| AppSettings::get(cx).logs.wrap_lines,
        |value, cx| AppSettings::update(cx, |settings| settings.logs.wrap_lines = value),
    )
    .default_value(SettingsData::default().logs.wrap_lines);
    let json = SettingField::switch(
        |cx| AppSettings::get(cx).logs.show_json,
        |value, cx| AppSettings::update(cx, |settings| settings.logs.show_json = value),
    )
    .default_value(SettingsData::default().logs.show_json);
    SettingPage::new(SettingsPage::Logs.title()).group(
        SettingGroup::new()
            .title("New log tabs")
            .description("Each tab can still change these from its toolbar.")
            .item(SettingItem::new("Lines loaded at open", tail))
            .item(SettingItem::new("Timestamps", timestamps))
            .item(SettingItem::new("Wrap long lines", wrap))
            .item(SettingItem::new("Show JSON as message and fields", json)),
    )
}

/// The shell a new tab runs, and how the terminal keeps and draws its text.
fn terminal_page() -> SettingPage {
    let shell = table_dropdown(
        &SHELL_OPTIONS,
        |settings| settings.terminal.default_shell,
        |settings, shell| settings.terminal.default_shell = shell,
        |_| "Auto".to_owned(),
    );
    let scrollback = table_dropdown(
        &SCROLLBACK_OPTIONS,
        |settings| settings.terminal.scrollback_lines,
        |settings, lines| settings.terminal.scrollback_lines = lines,
        |lines| format!("{} lines", group_digits(lines as usize)),
    );
    let font = table_dropdown(
        &FONT_SIZE_OPTIONS,
        |settings| settings.terminal.font_size,
        |settings, size| settings.terminal.font_size = size,
        |size| size.map_or_else(|| "Theme size".to_owned(), |size| format!("{size} px")),
    );
    SettingPage::new(SettingsPage::TerminalAndShell.title())
        .group(
            SettingGroup::new()
                .title("Shell")
                .description("Applies to new shell tabs.")
                .item(
                    SettingItem::new("Default shell", shell)
                        .description("Used by Open shell. The tab can still pick another."),
                ),
        )
        .group(
            SettingGroup::new()
                .title("Terminal")
                .description("Font size applies at once.")
                .item(SettingItem::new("Scrollback", scrollback))
                .item(SettingItem::new("Font size", font)),
        )
}

/// The Mode dropdown: saves the choice and re-themes every window now.
fn change_theme(label: &str, cx: &mut App) {
    let theme = theme_from_label(label);
    AppSettings::update(cx, |settings| settings.theme = theme);
    theme.apply(AppSettings::get(cx).appearance.color_theme, cx);
}

/// The Theme dropdown: saves the colour family and re-themes every window now.
fn change_color_theme(label: &str, cx: &mut App) {
    let colors = COLOR_THEME_OPTIONS.value(label);
    AppSettings::update(cx, |settings| settings.appearance.color_theme = colors);
    AppSettings::get(cx).theme.apply(colors, cx);
}

fn appearance_page() -> SettingPage {
    let colors = SettingField::dropdown(
        COLOR_THEME_OPTIONS.choices(),
        |cx| {
            let colors = AppSettings::get(cx).appearance.color_theme;
            COLOR_THEME_OPTIONS.label(colors, || "Default".to_owned())
        },
        |label, cx| change_color_theme(&label, cx),
    )
    .default_value(
        COLOR_THEME_OPTIONS.label(SettingsData::default().appearance.color_theme, || {
            "Default".to_owned()
        }),
    );
    let mode = SettingField::dropdown(
        theme_choices(),
        |cx| theme_label(AppSettings::get(cx).theme).into(),
        |label, cx| change_theme(&label, cx),
    )
    .default_value(theme_label(SettingsData::default().theme));
    let density = table_dropdown(
        &DENSITY_OPTIONS,
        |settings| settings.appearance.density,
        |settings, density| settings.appearance.density = density,
        |_| "Compact (28 px)".to_owned(),
    );
    SettingPage::new(SettingsPage::Appearance.title())
        .group(
            SettingGroup::new()
                .title("Colours")
                .description("Applies to every window at once.")
                .item(
                    SettingItem::new("Theme", colors)
                        .description("Zed One is One Light or One Dark, following Mode."),
                )
                .item(SettingItem::new("Mode", mode)),
        )
        .group(
            SettingGroup::new()
                .title("Fonts")
                .item(SettingItem::render(|_, _, cx| {
                    Label::new("The interface, monospace text and the terminal use Lilex, built into the app.")
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                })),
        )
        .group(
            SettingGroup::new().title("Tables").item(
                SettingItem::new("Row density", density).description(
                    "The height of every table row, header included. Applies at once.",
                ),
            ),
        )
}

/// The same grid as the `?` sheet, read-only: the keymap is fixed, so there is nothing to edit.
fn keyboard_shortcuts_page() -> SettingPage {
    SettingPage::new(SettingsPage::KeyboardShortcuts.title())
        .resettable(false)
        .group(SettingGroup::new().item(SettingItem::render(|_, _, cx| shortcut_sheet(cx))))
}

/// One row of the tier table: the environments that share a confirm tier, and what each risk asks.
#[derive(Clone, Debug, PartialEq, Eq)]
struct TierRow {
    environments: String,
    change: String,
    destructive: String,
    /// The node shell: the same in every row.
    privileged: String,
}

/// The two tiers, read from the same rules the dialog uses: the environments are grouped by
/// `ConfirmMode::for_tier`, and each cell is what `confirm_step` asks. Custom environments list
/// after the built-ins of their tier.
fn tier_rows(custom: &[CustomEnvironment]) -> Vec<TierRow> {
    [ConfirmMode::TypeName, ConfirmMode::Click]
        .into_iter()
        .map(|mode| {
            let mut environments: Vec<&str> = EnvironmentTier::ALL
                .into_iter()
                .filter(|tier| ConfirmMode::for_tier(*tier) == mode)
                .map(EnvironmentTier::name)
                .collect();
            environments.extend(
                usable_environments(custom)
                    .filter(|environment| ConfirmMode::for_tier(environment.tier) == mode)
                    .map(|environment| environment.name.as_str()),
            );
            TierRow {
                environments: environments.join(", "),
                change: tier_cell(mode, ActionRisk::Change),
                destructive: tier_cell(mode, ActionRisk::Destructive),
                privileged: tier_cell(mode, ActionRisk::Privileged),
            }
        })
        .collect()
}

/// What the confirm dialog of `mode` asks for `risk`; a destructive action also gets a danger
/// button.
pub(crate) fn tier_cell(mode: ConfirmMode, risk: ActionRisk) -> String {
    let expected = match risk {
        ActionRisk::Privileged => "the node name",
        ActionRisk::Change | ActionRisk::Destructive => {
            "the object name (one object) or the cluster name (several)"
        }
    };
    let how = match confirm_step(mode, risk, expected) {
        DialogConfirm::TypeName { expected } => format!("Type {expected}"),
        DialogConfirm::Click => "Click Confirm".to_owned(),
    };
    match risk {
        ActionRisk::Change => how,
        ActionRisk::Destructive | ActionRisk::Privileged => format!("{how}, danger button"),
    }
}

fn safety_page() -> SettingPage {
    let table = SettingGroup::new()
        .title("Confirming changes")
        .item(SettingItem::render(|_, _, cx| tier_table(cx)));
    let audit = SettingGroup::new()
        .title("Audit log")
        .item(about_row("Audit file", audit_file));
    SettingPage::new(SettingsPage::Safety.title())
        .resettable(false)
        .group(table)
        .group(audit)
}

/// Every guarded action opens a confirm dialog; the tier only picks how it is confirmed. Each
/// cluster can override its tier under Clusters.
fn tier_table(cx: &App) -> AnyElement {
    let theme = cx.theme();
    let cell = |text: String| div().flex_1().min_w_0().text_sm().child(text);
    let header = h_flex()
        .w_full()
        .gap_3()
        .pb_1()
        .border_b_1()
        .border_color(theme.border)
        .text_xs()
        .text_color(theme.muted_foreground)
        .child(cell("Environment".to_owned()))
        .child(cell("Change".to_owned()))
        .child(cell("Destructive".to_owned()))
        .child(cell("Node shell".to_owned()));
    v_flex()
        .w_full()
        .gap_2()
        .child(
            div()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child("Every change opens a confirm dialog. How it is confirmed follows the environment, and each cluster can override it under Clusters."),
        )
        .child(header)
        .children(tier_rows(&AppSettings::get(cx).registry.environments).into_iter().map(|row| {
            h_flex()
                .w_full()
                .gap_3()
                .child(cell(row.environments))
                .child(cell(row.change))
                .child(cell(row.destructive))
                .child(cell(row.privileged))
        }))
        .into_any_element()
}

/// The notice of the terminal engine, which the Apache License asks the app to carry (its NOTICE
/// file, shortened to what applies to the part of it that is linked).
const ONETERM_NOTICE: &str = "oneterm-vt, the terminal engine of the shell tab. Copyright 2026 The OneTerm authors (https://github.com/vnStrawHat/OneTerm), Apache-2.0. Its column reflow follows the algorithm of avt (https://github.com/asciinema/avt), Apache-2.0.";

fn about_page(cx: &App) -> SettingPage {
    let mut group = SettingGroup::new()
        .item(about_row("Version", |_, _| {
            format!("k8sBoard {}", env!("CARGO_PKG_VERSION")).into_any_element()
        }))
        .item(about_row("License", |_, _| "Apache-2.0".into_any_element()))
        .item(about_row("Third-party software", |_, cx| {
            div()
                .max_w(px(420.))
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(ONETERM_NOTICE)
                .into_any_element()
        }))
        .item(about_row("Settings folder", settings_folder))
        .item(about_row("Audit log", audit_file));
    if let Some(notice) = AppSettings::notice(cx) {
        let text = notice.to_string();
        group = group.item(about_row("Settings notice", move |_, cx| {
            div()
                .text_sm()
                .text_color(cx.theme().warning)
                .child(text.clone())
                .into_any_element()
        }));
    }
    SettingPage::new(SettingsPage::About.title())
        .resettable(false)
        .group(group)
}

/// The settings folder as monospace text with a button that reveals it; without a folder,
/// settings are not saved.
fn settings_folder(_: &mut Window, cx: &mut App) -> AnyElement {
    let Some(dir) = AppSettings::config_dir(cx).map(std::path::Path::to_path_buf) else {
        return muted_note("Not saved this session", cx);
    };
    let shown = dir.display().to_string();
    path_with_reveal("reveal-settings-folder", shown, dir, cx)
}

/// The audit file as monospace text with a button that reveals its folder. Without a settings
/// folder no line is written.
fn audit_file(_: &mut Window, cx: &mut App) -> AnyElement {
    let Some(dir) = AppSettings::config_dir(cx).map(std::path::Path::to_path_buf) else {
        return muted_note("Audit file unavailable: changes are not logged", cx);
    };
    let shown = audit_path(&dir).display().to_string();
    path_with_reveal("reveal-audit-folder", shown, dir, cx)
}

fn muted_note(text: &'static str, cx: &App) -> AnyElement {
    div()
        .text_sm()
        .text_color(cx.theme().muted_foreground)
        .child(text)
        .into_any_element()
}

/// `shown` in monospace, and a button that reveals `folder`.
fn path_with_reveal(
    id: &'static str,
    shown: String,
    folder: std::path::PathBuf,
    cx: &App,
) -> AnyElement {
    h_flex()
        .gap_2()
        .items_center()
        .child(
            div()
                .text_sm()
                .font_family(cx.theme().mono_font_family.clone())
                .child(shown),
        )
        .child(
            Button::new(id)
                .ghost()
                .small()
                .icon(Icon::new(IconName::FolderOpen))
                .label("Show in folder")
                .on_click(move |_, _, cx| cx.reveal_path(&folder)),
        )
        .into_any_element()
}

/// A label on the left and a value element on the right.
fn about_row(
    label: &'static str,
    value: impl Fn(&mut Window, &mut App) -> AnyElement + 'static,
) -> SettingItem {
    SettingItem::render(move |_, window, cx| {
        h_flex()
            .w_full()
            .justify_between()
            .items_center()
            .gap_3()
            .child(Label::new(label).text_sm())
            .child(value(window, cx))
    })
}

#[cfg(test)]
#[path = "settings_window_tests.rs"]
mod settings_window_tests;
