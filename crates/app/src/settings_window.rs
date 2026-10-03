//! The Settings window: a second OS window, one instance, over the shared `AppSettings` global
//! and the `ClusterCatalog`. The pages are the kit's `Settings` component.

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::label::Label;
use gpui_kit::component::setting::{
    SelectIndex, SettingField, SettingGroup, SettingItem, SettingPage, Settings,
};
use gpui_kit::component::{
    ActiveTheme as _, Icon, Sizable as _, StyledExt as _, TitleBar, h_flex, v_flex,
};
use gpui_kit::{
    AnyElement, AnyWindowHandle, App, AppContext as _, Bounds, Context, Entity, FocusHandle,
    Focusable, Global, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    Styled as _, Subscription, WeakEntity, Window, WindowBounds, WindowId, WindowOptions, div, px,
    size,
};

use crate::cluster_catalog::CatalogHandle;
use crate::clusters_page::{ClustersPage, add_cluster_button};
use crate::settings::{AppSettings, theme_choices, theme_from_label, theme_label};
use crate::shortcut_sheet::shortcut_sheet;

gpui_kit::actions!(k8sboard, [OpenSettings, ManageClusters, ImportKubeconfig]);

const WINDOW_WIDTH: f32 = 1000.;
const WINDOW_HEIGHT: f32 = 620.;
/// For a screenshot of the whole Clusters form, which is taller than the standard window.
const TALL_WINDOW_HEIGHT: f32 = 900.;
const SIDEBAR_WIDTH: f32 = 200.;

/// The pages in W2 nav order, keeping only those with content. A later spec inserts its page
/// at its W2 position.
const PAGES: [SettingsPage; 4] = [
    SettingsPage::Clusters,
    SettingsPage::Appearance,
    SettingsPage::KeyboardShortcuts,
    SettingsPage::About,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SettingsPage {
    Clusters,
    Appearance,
    KeyboardShortcuts,
    About,
}

impl SettingsPage {
    fn title(self) -> &'static str {
        match self {
            Self::Clusters => "Clusters",
            Self::Appearance => "Appearance",
            Self::KeyboardShortcuts => "Keyboard Shortcuts",
            Self::About => "About",
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
    if let Some(open) = open_window_of(cx) {
        let _ = open
            .view
            .update(cx, |view, cx| view.show_page(SettingsPage::Clusters, cx));
    }
    open_settings_window(SettingsPage::Clusters, SettingsSize::Standard, cx);
}

fn open_window_of(cx: &App) -> Option<OpenWindow> {
    cx.try_global::<SettingsWindowHandle>()
        .and_then(|handle| handle.0.clone())
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
    /// Changes with each `show_page`: a new key gives the kit `Settings` a fresh selection.
    page_generation: usize,
    clusters: Entity<ClustersPage>,
    focus_handle: FocusHandle,
    _observers: Vec<Subscription>,
}

impl SettingsWindow {
    fn new(first_page: SettingsPage, _: &mut Window, cx: &mut Context<Self>) -> Self {
        let catalog = CatalogHandle::of(cx);
        // A status left over from a closed window must not show here.
        catalog.update(cx, |catalog, cx| catalog.reset_paste_status(cx));
        let clusters = cx.new(|cx| ClustersPage::new(catalog.clone(), cx));
        Self {
            first_page,
            page_generation: 0,
            clusters,
            focus_handle: cx.focus_handle(),
            _observers: vec![
                cx.observe_global::<AppSettings>(|_, cx| cx.notify()),
                cx.observe(&catalog, |_, _, cx| cx.notify()),
            ],
        }
    }

    fn show_page(&mut self, page: SettingsPage, cx: &mut Context<Self>) {
        self.first_page = page;
        self.page_generation += 1;
        cx.notify();
    }

    fn pages(&self, cx: &App) -> Vec<SettingPage> {
        PAGES
            .iter()
            .map(|page| match page {
                SettingsPage::Clusters => clusters_page(&self.clusters, cx),
                SettingsPage::Appearance => appearance_page(),
                SettingsPage::KeyboardShortcuts => keyboard_shortcuts_page(),
                SettingsPage::About => about_page(cx),
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
            .bg(theme.background)
            .text_color(theme.foreground)
            .child(TitleBar::new().child(div().font_semibold().child("Settings")))
            .child(
                div().flex_1().min_h_0().child(
                    Settings::new(("settings", self.page_generation))
                        .sidebar_width(px(SIDEBAR_WIDTH))
                        .default_selected_index(SelectIndex {
                            page_ix: self.first_page.index(),
                            group_ix: None,
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
    let blocked = ClustersPage::paste_blocked_reason(cx);
    SettingPage::new(SettingsPage::Clusters.title())
        .resettable(false)
        .description(page.read(cx).description(cx))
        .title_suffix(move |_, _| add_cluster_button(add.clone(), blocked))
        .group(SettingGroup::new().item(SettingItem::render(move |_, _, _| body.clone())))
}

/// The theme dropdown: saves the choice and re-themes every window now.
fn change_theme(label: &str, cx: &mut App) {
    let theme = theme_from_label(label);
    AppSettings::update(cx, |settings| settings.theme = theme);
    theme.apply(cx);
}

fn appearance_page() -> SettingPage {
    let theme = SettingField::dropdown(
        theme_choices(),
        |cx| theme_label(AppSettings::get(cx).theme).into(),
        |label, cx| change_theme(&label, cx),
    );
    SettingPage::new(SettingsPage::Appearance.title())
        .resettable(false)
        .group(
            SettingGroup::new().title("Theme").item(
                SettingItem::new("Theme", theme).description("Applies to every window at once."),
            ),
        )
}

/// The same grid as the `?` sheet, read-only: the keymap is fixed, so there is nothing to edit.
fn keyboard_shortcuts_page() -> SettingPage {
    SettingPage::new(SettingsPage::KeyboardShortcuts.title())
        .resettable(false)
        .group(SettingGroup::new().item(SettingItem::render(|_, _, cx| shortcut_sheet(cx))))
}

fn about_page(cx: &App) -> SettingPage {
    let mut group = SettingGroup::new()
        .item(about_row("Version", |_, _| {
            format!("k8sBoard {}", env!("CARGO_PKG_VERSION")).into_any_element()
        }))
        .item(about_row("License", |_, _| "Apache-2.0".into_any_element()))
        .item(about_row("Settings folder", settings_folder));
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
        return div()
            .text_sm()
            .text_color(cx.theme().muted_foreground)
            .child("Not saved this session")
            .into_any_element();
    };
    h_flex()
        .gap_2()
        .items_center()
        .child(
            div()
                .text_sm()
                .font_family(cx.theme().mono_font_family.clone())
                .child(dir.display().to_string()),
        )
        .child(
            Button::new("reveal-settings-folder")
                .ghost()
                .small()
                .icon(Icon::new(IconName::FolderOpen))
                .label("Show in folder")
                .on_click(move |_, _, cx| cx.reveal_path(&dir)),
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
