//! The Clusters page of the Settings window: the list of clusters grouped by environment and the
//! form of the selected one. The page owns the view state only; the rules live in
//! `cluster_form.rs`, and the file writes in `cluster_catalog.rs`.
//!
//! `ClustersPage` has no `Debug`: it can hold the clipboard text of a pending paste.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use cluster::{Kubeconfig, MetricsSourceFields};
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariant, ButtonVariants as _};
use gpui_kit::component::dialog::DialogButtonProps;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, Sizable as _, StyledExt as _, WindowExt as _, h_flex,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, ElementId, Entity, InteractiveElement as _,
    IntoElement, ParentElement as _, PathPromptOptions, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, Task, Window, div, px,
};

use crate::active_session::ActiveConnection;
use crate::app_shell::find_cluster;
use crate::cluster_catalog::{CatalogNotice, ClusterCatalog, PasteStatus};
use crate::cluster_form::{
    ClusterGroup, ClusterRow, FieldError, MoveStep, ProxyMode, TEST_CONNECTION_TIMEOUT, TestState,
    add_watched_folder, color_to_store, count_text, edit_entry, filter_groups, is_proxy_pending,
    move_cluster, proxy_input_prefill, proxy_mode, proxy_mode_label, remove_block_reason,
    remove_dialog_text, reset_entry, resolve_selection, step_cluster, stop_watching_folder,
    test_connection, validate_display_name, validate_namespace, validate_proxy_url,
};
use crate::cluster_registry::{ClusterEntry, ClusterProxy, ClusterRef};
use crate::cluster_runtime::ClusterRuntime;
use crate::cluster_switcher_rows::normalize_query;
use crate::drawer::truncated_text;
use crate::environment::{ClusterColor, Environment, cluster_color, environment_badge};
use crate::resource_actions::disabled_menu_item;
use crate::settings::AppSettings;
use crate::settings_window::{ImportKubeconfig, show_metrics_page};
use crate::write_guard::ConfirmMode;

#[path = "clusters_page_import.rs"]
mod import;

#[cfg(test)]
#[path = "clusters_page_tests.rs"]
mod clusters_page_tests;

const LIST_WIDTH: f32 = 300.;
const LIST_MAX_HEIGHT: f32 = 420.;
const FIELD_LABEL_WIDTH: f32 = 150.;
/// The height of a one-line control (an input): the label and the other controls are centered in
/// a band of this height so they line up with it.
const CONTROL_HEIGHT: f32 = 32.;
const LATER_VERSION: &str = "Comes in a later version";
const PROXY_PLACEHOLDER: &str = "http://proxy.example:3128";
const PROXY_HINT: &str = "Applies the next time k8sBoard connects. HTTPS_PROXY and NO_PROXY are not read, but exec credential plugins (aws, gcloud, …) inherit them from the environment.";

pub(crate) struct ClustersPage {
    catalog: Entity<ClusterCatalog>,
    selected: Option<ClusterRef>,
    form: Option<ClusterForm>,
    test: TestState,
    /// Dropping it cancels the test, and its request with it.
    test_task: Option<Task<()>>,
    /// The clipboard text of a paste whose preview is open. It is set only after the text
    /// parsed, and cleared on Save and add (moved out), Cancel, Esc, overlay click, and a preview
    /// error; closing the window drops the page.
    paste_text: Option<String>,
    /// A file whose first row becomes the selection once it has loaded.
    pending_select: Option<PathBuf>,
    /// The search box of the page header; it is not saved.
    search: Entity<InputState>,
    _observers: Vec<Subscription>,
}

/// The inputs of the selected cluster, recreated with the stored values when the selection
/// changes.
struct ClusterForm {
    cluster: ClusterRef,
    name: Entity<InputState>,
    namespace: Entity<InputState>,
    proxy_url: Entity<InputState>,
    name_error: Option<FieldError>,
    namespace_error: Option<FieldError>,
    proxy_error: Option<FieldError>,
    /// Custom URL was picked, so its input shows before a URL is stored.
    is_custom_proxy: bool,
    _subscriptions: Vec<Subscription>,
}

impl ClustersPage {
    pub(crate) fn new(
        catalog: Entity<ClusterCatalog>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search clusters"));
        let observers = vec![
            cx.observe_global::<AppSettings>(|_, cx| cx.notify()),
            cx.observe(&catalog, |page, _, cx| page.on_catalog_changed(cx)),
            cx.subscribe(&search, |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }),
        ];
        Self {
            catalog,
            selected: None,
            form: None,
            test: TestState::Idle,
            test_task: None,
            paste_text: None,
            pending_select: None,
            search,
            _observers: observers,
        }
    }

    /// `1 cluster · 1 kubeconfig file`, for the page header.
    pub(crate) fn description(&self, cx: &App) -> String {
        let catalog = self.catalog.read(cx);
        let clusters = catalog
            .kubeconfigs()
            .map(|kubeconfig| kubeconfig.contexts().len())
            .sum();
        let files = catalog
            .kubeconfigs()
            .map(|kubeconfig| kubeconfig.sources().len())
            .sum();
        count_text(clusters, files)
    }

    /// The search box for the page header (W2 places it before Add cluster).
    pub(crate) fn search_input(&self) -> Entity<InputState> {
        self.search.clone()
    }

    /// The search text; empty (or only blanks) means the list is not filtered.
    fn search_text(&self, cx: &App) -> SharedString {
        self.search.read(cx).value()
    }

    fn is_searching(&self, cx: &App) -> bool {
        !normalize_query(&self.search_text(cx)).is_empty()
    }

    /// `Alt ↑` and `Alt ↓`: moves the selected row one place inside its group. Off while
    /// searching, like the drag, because hidden rows make the place ambiguous.
    pub(crate) fn step_selected(&mut self, step: MoveStep, cx: &mut Context<Self>) {
        let Some(cluster) = self.selected.clone() else {
            return;
        };
        if self.is_searching(cx) {
            return;
        }
        let groups = self.groups(cx);
        let Some(group) = groups
            .iter()
            .find(|group| group.rows.iter().any(|row| row.cluster == cluster))
        else {
            return;
        };
        AppSettings::update(cx, |settings| {
            step_cluster(&mut settings.registry, group, &cluster, step);
        });
    }

    /// A row dropped on `target`: it takes that place when both are in the dragged row's group;
    /// a drop on another group changes nothing (the environment decides the group).
    fn drop_cluster(
        &mut self,
        dragged: &DraggedCluster,
        target: &ClusterRef,
        cx: &mut Context<Self>,
    ) {
        let groups = self.groups(cx);
        let Some(group) = groups
            .iter()
            .find(|group| group.title == dragged.group_title)
        else {
            return;
        };
        AppSettings::update(cx, |settings| {
            move_cluster(&mut settings.registry, group, &dragged.cluster, target);
        });
    }

    fn groups(&self, cx: &App) -> Vec<ClusterGroup> {
        self.catalog.read(cx).groups(cx)
    }

    fn rows(&self, cx: &App) -> Vec<ClusterRow> {
        self.groups(cx)
            .into_iter()
            .flat_map(|group| group.rows)
            .collect()
    }

    /// Selects the first row of a file that was just added, once its rows exist.
    fn on_catalog_changed(&mut self, cx: &mut Context<Self>) {
        let added = match self.catalog.read(cx).paste_status() {
            PasteStatus::Added(path) => Some(path.clone()),
            PasteStatus::Idle | PasteStatus::Saving | PasteStatus::Failed => None,
        };
        if let Some(path) = added {
            self.pending_select = Some(path);
            self.catalog
                .update(cx, |catalog, cx| catalog.reset_paste_status(cx));
        }
        if let Some(path) = self.pending_select.clone() {
            let first = self
                .rows(cx)
                .into_iter()
                .find(|row| row.cluster.kubeconfig == path);
            if let Some(row) = first {
                self.pending_select = None;
                self.select(row.cluster, cx);
            }
        }
        cx.notify();
    }

    fn select(&mut self, cluster: ClusterRef, cx: &mut Context<Self>) {
        if self.selected.as_ref() == Some(&cluster) {
            return;
        }
        self.selected = Some(cluster);
        self.forget_form_and_test();
        cx.notify();
    }

    /// A selection change drops the inputs, the errors, and a running test.
    fn forget_form_and_test(&mut self) {
        self.form = None;
        self.test = TestState::Idle;
        self.test_task = None;
    }

    fn sync_form(&mut self, row: &ClusterRow, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .form
            .as_ref()
            .is_some_and(|form| form.cluster == row.cluster)
        {
            return;
        }
        let entry = stored_entry(&row.cluster, cx);
        let name_text = entry
            .as_ref()
            .and_then(|entry| entry.display_name.clone())
            .unwrap_or_default();
        let namespace_text = entry
            .as_ref()
            .and_then(|entry| entry.default_namespace.clone())
            .unwrap_or_default();
        let placeholder = row.cluster.context.clone();
        let name = cx.new(|cx| {
            let mut state = InputState::new(window, cx).placeholder(placeholder);
            state.set_value(name_text, window, cx);
            state
        });
        let namespace = cx.new(|cx| {
            let mut state = InputState::new(window, cx).placeholder("none");
            state.set_value(namespace_text, window, cx);
            state
        });
        let proxy_text = proxy_input_prefill(entry.as_ref().and_then(|entry| entry.proxy.as_ref()));
        let proxy_url = cx.new(|cx| {
            let mut state = InputState::new(window, cx).placeholder(PROXY_PLACEHOLDER);
            state.set_value(proxy_text, window, cx);
            state
        });
        let subscriptions = vec![
            cx.subscribe_in(&name, window, Self::on_name_event),
            cx.subscribe_in(&namespace, window, Self::on_namespace_event),
            cx.subscribe_in(&proxy_url, window, Self::on_proxy_event),
        ];
        self.form = Some(ClusterForm {
            cluster: row.cluster.clone(),
            name,
            namespace,
            proxy_url,
            name_error: None,
            namespace_error: None,
            proxy_error: None,
            is_custom_proxy: false,
            _subscriptions: subscriptions,
        });
    }

    fn on_name_event(
        &mut self,
        input: &Entity<InputState>,
        event: &InputEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !matches!(event, InputEvent::Change) {
            return;
        }
        let Some(cluster) = self.form.as_ref().map(|form| form.cluster.clone()) else {
            return;
        };
        let text = input.read(cx).value();
        let result = validate_display_name(&text, &cluster, &self.rows(cx));
        let error = match result {
            Ok(value) => {
                AppSettings::update(cx, |settings| {
                    edit_entry(&mut settings.registry, &cluster, |entry| {
                        entry.display_name = value;
                    });
                });
                None
            }
            Err(error) => Some(error),
        };
        if let Some(form) = &mut self.form {
            form.name_error = error;
        }
        cx.notify();
    }

    fn on_namespace_event(
        &mut self,
        input: &Entity<InputState>,
        event: &InputEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !matches!(event, InputEvent::Change) {
            return;
        }
        let Some(cluster) = self.form.as_ref().map(|form| form.cluster.clone()) else {
            return;
        };
        let text = input.read(cx).value();
        let error = match validate_namespace(&text) {
            Ok(value) => {
                AppSettings::update(cx, |settings| {
                    edit_entry(&mut settings.registry, &cluster, |entry| {
                        entry.default_namespace = value;
                    });
                });
                None
            }
            Err(error) => Some(error),
        };
        if let Some(form) = &mut self.form {
            form.namespace_error = error;
        }
        cx.notify();
    }

    /// The URL is stored on Enter or blur, never per keystroke: a half-typed URL must not apply.
    fn on_proxy_event(
        &mut self,
        _: &Entity<InputState>,
        event: &InputEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::PressEnter { .. } | InputEvent::Blur => self.commit_proxy(cx),
            // Only the `Not applied` note follows the typing.
            InputEvent::Change => cx.notify(),
            InputEvent::Focus => {}
        }
    }

    fn commit_proxy(&mut self, cx: &mut Context<Self>) {
        let Some(form) = &self.form else {
            return;
        };
        let cluster = form.cluster.clone();
        let text = form.proxy_url.read(cx).value();
        let error = match validate_proxy_url(&text) {
            Ok(Some(proxy)) => {
                AppSettings::update(cx, |settings| {
                    edit_entry(&mut settings.registry, &cluster, |entry| {
                        entry.proxy = Some(proxy);
                    });
                });
                None
            }
            // Blank text stores nothing and says nothing.
            Ok(None) => None,
            Err(error) => Some(error),
        };
        if let Some(form) = &mut self.form {
            form.proxy_error = error;
        }
        cx.notify();
    }

    /// From kubeconfig and None store at once; Custom URL only shows the input, and stores nothing
    /// until a URL is committed.
    fn pick_proxy_mode(&mut self, mode: ProxyMode, cx: &mut Context<Self>) {
        let Some(form) = &mut self.form else {
            return;
        };
        let cluster = form.cluster.clone();
        form.proxy_error = None;
        form.is_custom_proxy = mode == ProxyMode::Custom;
        let stored = match mode {
            ProxyMode::FromKubeconfig => Some(None),
            ProxyMode::Direct => Some(Some(ClusterProxy::Direct)),
            ProxyMode::Custom => None,
        };
        if let Some(stored) = stored {
            AppSettings::update(cx, |settings| {
                edit_entry(&mut settings.registry, &cluster, |entry| {
                    entry.proxy = stored
                });
            });
        }
        cx.notify();
    }

    fn start_test(&mut self, cx: &mut Context<Self>) {
        let Some(cluster) = self.selected.clone() else {
            return;
        };
        let kubeconfigs: Vec<Arc<Kubeconfig>> =
            self.catalog.read(cx).kubeconfigs().cloned().collect();
        let Some((kubeconfig, summary)) = find_cluster(&kubeconfigs, &cluster) else {
            return;
        };
        self.test = TestState::Running;
        // The stored choice, read now: a proxy typed but not committed is not tested.
        let proxy = AppSettings::get(cx).registry.profile(&summary).proxy;
        let running = cx.global::<ClusterRuntime>().spawn(test_connection(
            kubeconfig,
            summary.name,
            proxy,
            TEST_CONNECTION_TIMEOUT,
        ));
        self.test_task = Some(cx.spawn(async move |this, cx| {
            let state = running.await.unwrap_or_else(|_| {
                TestState::Failed("The connection test was interrupted.".into())
            });
            let _ = this.update(cx, |page, cx| {
                page.test = state;
                page.test_task = None;
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn reset_selected(&mut self, cx: &mut Context<Self>) {
        let Some(cluster) = self.selected.clone() else {
            return;
        };
        AppSettings::update(cx, |settings| reset_entry(&mut settings.registry, &cluster));
        // The inputs show the stored values, which are the defaults again.
        self.forget_form_and_test();
        cx.notify();
    }

    fn confirm_remove(&self, row: &ClusterRow, window: &mut Window, cx: &mut Context<Self>) {
        let (title, body) = remove_dialog_text(&row.cluster.kubeconfig, &self.rows(cx), row.origin);
        let path = row.cluster.kubeconfig.clone();
        let catalog = self.catalog.clone();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let path = path.clone();
            let catalog = catalog.clone();
            alert
                .title(title.clone())
                .description(body.clone())
                .confirm()
                .button_props(
                    DialogButtonProps::default()
                        .ok_text("Remove")
                        .ok_variant(ButtonVariant::Danger)
                        .show_cancel(true),
                )
                .on_ok(move |_, _, cx| {
                    let path = path.clone();
                    catalog.update(cx, |catalog, cx| catalog.remove_kubeconfig(path, cx));
                    true
                })
        });
    }
}

fn stored_entry(cluster: &ClusterRef, cx: &App) -> Option<ClusterEntry> {
    AppSettings::get(cx)
        .registry
        .clusters
        .iter()
        .find(|entry| entry.cluster == *cluster)
        .cloned()
}

impl Render for ClustersPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let groups = self.groups(cx);
        let preferred = AppSettings::get(cx).registry.last_used.clone();
        let resolved = resolve_selection(self.selected.as_ref(), preferred.as_ref(), &groups);
        if resolved != self.selected {
            self.selected = resolved;
            self.forget_form_and_test();
        }
        let selected_row = groups
            .iter()
            .flat_map(|group| &group.rows)
            .find(|row| Some(&row.cluster) == self.selected.as_ref())
            .cloned();
        if let Some(row) = &selected_row {
            self.sync_form(row, window, cx);
        }
        let form = match &selected_row {
            Some(row) => self.render_form(row, cx),
            None => muted_text("Add a kubeconfig to see its clusters here.", cx).into_any_element(),
        };
        v_flex()
            .w_full()
            .gap_3()
            .children(self.render_notices(cx))
            .children(self.render_folder_lines(cx))
            .children(self.render_paste_status(cx))
            .child(
                h_flex()
                    .w_full()
                    .gap_4()
                    .items_start()
                    .child(self.render_list(&groups, cx))
                    .child(div().flex_1().min_w_0().child(form)),
            )
    }
}

impl ClustersPage {
    /// One line per watched folder, with the way to stop watching it. Stopping only edits the
    /// settings: nothing in the folder is touched, so there is no dialog.
    fn render_folder_lines(&self, cx: &Context<Self>) -> Vec<AnyElement> {
        let muted = cx.theme().muted_foreground;
        self.catalog
            .read(cx)
            .folder_summaries()
            .into_iter()
            .enumerate()
            .map(|(index, summary)| {
                let folder = summary.path.clone();
                h_flex()
                    .gap_2()
                    .items_center()
                    .text_sm()
                    .text_color(muted)
                    .child(Icon::new(IconName::FolderOpen))
                    .child(summary.line_text())
                    .child(
                        Button::new(("stop-watching", index))
                            .ghost()
                            .small()
                            .label("Stop watching")
                            .on_click(cx.listener(move |page, _, _, cx| {
                                page.stop_watching(&folder, cx);
                            })),
                    )
                    .into_any_element()
            })
            .collect()
    }

    /// Watch a kubeconfig folder…: the folder picker, then the folder joins the registry unless it
    /// is there already.
    fn watch_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Watch folder".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = picked.await else {
                return;
            };
            let Some(folder) = paths.into_iter().next() else {
                return;
            };
            let _ = this.update(cx, |_, cx| {
                AppSettings::update(cx, |settings| {
                    add_watched_folder(&mut settings.registry, folder);
                });
            });
        })
        .detach();
    }

    fn stop_watching(&mut self, folder: &Path, cx: &mut Context<Self>) {
        AppSettings::update(cx, |settings| {
            stop_watching_folder(&mut settings.registry, folder);
        });
    }

    fn render_notices(&self, cx: &Context<Self>) -> Vec<AnyElement> {
        let warning = cx.theme().warning;
        self.catalog
            .read(cx)
            .notices()
            .iter()
            .enumerate()
            .map(|(index, notice)| {
                let line = h_flex()
                    .gap_2()
                    .items_center()
                    .text_sm()
                    .text_color(warning)
                    .child(Icon::new(IconName::TriangleAlert))
                    .child(notice.to_string());
                match notice {
                    CatalogNotice::Unregistered(path) => {
                        let path = path.clone();
                        let catalog = self.catalog.clone();
                        line.child(
                            Button::new(("remove-unregistered", index))
                                .ghost()
                                .small()
                                .label("Remove")
                                .on_click(move |_, _, cx| {
                                    let path = path.clone();
                                    catalog.update(cx, |catalog, cx| {
                                        catalog.remove_kubeconfig(path, cx);
                                    });
                                }),
                        )
                        .into_any_element()
                    }
                    CatalogNotice::Skipped { .. }
                    | CatalogNotice::DeleteFailed(_)
                    | CatalogNotice::SaveFailed(_)
                    | CatalogNotice::FolderMissing { .. } => line.into_any_element(),
                }
            })
            .collect()
    }

    fn render_paste_status(&self, cx: &Context<Self>) -> Option<AnyElement> {
        match self.catalog.read(cx).paste_status() {
            PasteStatus::Saving => {
                Some(muted_text("Saving the pasted kubeconfig…", cx).into_any_element())
            }
            PasteStatus::Idle | PasteStatus::Added(_) | PasteStatus::Failed => None,
        }
    }

    fn render_list(&self, groups: &[ClusterGroup], cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (border, muted) = (theme.border, theme.muted_foreground);
        let search = self.search_text(cx);
        let is_searching = self.is_searching(cx);
        let visible = if is_searching {
            filter_groups(groups, &search)
        } else {
            groups.to_vec()
        };
        let mut list = v_flex()
            .id("cluster-list")
            .w_full()
            .flex_none()
            .max_h(px(LIST_MAX_HEIGHT))
            .overflow_y_scroll()
            .p_1()
            .gap_1();
        if groups.is_empty() {
            list = list.child(muted_text("No clusters yet.", cx));
        } else {
            list = list
                .border_1()
                .border_color(border)
                .rounded(cx.theme().radius);
        }
        if !groups.is_empty() && visible.is_empty() {
            let text = format!("No clusters match '{}'.", search.trim());
            list = list.child(muted_text(text, cx));
        }
        let mut index = 0;
        for group in &visible {
            list = list.child(
                h_flex()
                    .px_2()
                    .pt_1()
                    .gap_1()
                    .text_xs()
                    .font_semibold()
                    .text_color(muted)
                    .child(group.title)
                    .child(group.rows.len().to_string()),
            );
            for row in &group.rows {
                list = list.child(self.render_row(index, group.title, row, is_searching, cx));
                index += 1;
            }
        }
        let hint = if is_searching {
            "Clear the search to reorder."
        } else {
            "Drag to reorder inside a group; the order sets Ctrl 1–9."
        };
        v_flex()
            .w(px(LIST_WIDTH))
            .flex_none()
            .gap_1()
            .child(list)
            .child(muted_text(hint, cx).text_xs())
            .into_any_element()
    }

    fn render_row(
        &self,
        index: usize,
        group_title: &'static str,
        row: &ClusterRow,
        is_searching: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let is_selected = self.selected.as_ref() == Some(&row.cluster);
        let theme = cx.theme();
        let (active, hover, muted) = (theme.list_active, theme.list_hover, theme.muted_foreground);
        let mono = theme.mono_font_family.clone();
        let cluster = row.cluster.clone();
        let target = row.cluster.clone();
        let dragged = DraggedCluster {
            cluster: row.cluster.clone(),
            group_title,
            label: row.label.clone().into(),
        };
        h_flex()
            .id(ElementId::from(("cluster-row", index)))
            .w_full()
            .gap_2()
            .px_2()
            .py_1()
            .items_center()
            .rounded(cx.theme().radius)
            .cursor_pointer()
            .when(is_selected, |this| this.bg(active))
            .hover(|style| style.bg(hover))
            .on_click(cx.listener(move |page, _, _, cx| page.select(cluster.clone(), cx)))
            .when(!is_searching, |this| {
                this.on_drag(dragged, |dragged, _, _, cx| cx.new(|_| dragged.clone()))
            })
            // The tint shows only on a row of the dragged row's own group.
            .drag_over::<DraggedCluster>(move |style, dragged, _, cx| {
                if dragged.group_title == group_title {
                    style.bg(cx.theme().drop_target)
                } else {
                    style
                }
            })
            .on_drop(cx.listener(move |page, dragged: &DraggedCluster, _, cx| {
                page.drop_cluster(dragged, &target, cx);
            }))
            .child(environment_badge(row.profile.environment, cx))
            .child(
                v_flex()
                    .min_w_0()
                    .child(
                        div()
                            .text_sm()
                            .font_family(mono)
                            .truncate()
                            .child(row.label.clone()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .truncate()
                            .child(row.meta.clone()),
                    ),
            )
            .into_any_element()
    }

    fn render_form(&self, row: &ClusterRow, cx: &mut Context<Self>) -> AnyElement {
        let Some(form) = &self.form else {
            return div().into_any_element();
        };
        let entry = stored_entry(&row.cluster, cx);
        let kubeconfigs: Vec<Arc<Kubeconfig>> =
            self.catalog.read(cx).kubeconfigs().cloned().collect();
        let info = find_cluster(&kubeconfigs, &row.cluster)
            .map(|(kubeconfig, summary)| kubeconfig.connection_info(&summary));
        let server = info
            .as_ref()
            .and_then(|info| info.server.clone())
            .unwrap_or_else(|| "—".to_owned());
        let auth = info
            .as_ref()
            .map_or_else(|| "—".to_owned(), |info| info.auth.to_string());
        let source = format!(
            "{} · context {}",
            row.cluster.kubeconfig.display(),
            row.cluster.context
        );
        let mono = cx.theme().mono_font_family.clone();
        let danger = cx.theme().danger;
        let name_row = form_row(
            "Display name",
            v_flex()
                .gap_1()
                .child(Input::new(&form.name))
                .children(
                    form.name_error
                        .as_ref()
                        .map(|error| error_text(error, danger)),
                )
                .into_any_element(),
            cx,
        );
        let namespace_row = form_row(
            "Default namespace",
            v_flex()
                .gap_1()
                .child(Input::new(&form.namespace))
                .children(
                    form.namespace_error
                        .as_ref()
                        .map(|error| error_text(error, danger)),
                )
                .into_any_element(),
            cx,
        );
        let environment_row = form_row(
            "Environment",
            centered(environment_menu(row, entry.as_ref())),
            cx,
        );
        let color_row = form_row("Color", color_swatches(row, cx), cx);
        let cluster = row.cluster.clone();
        let read_only_row = form_row(
            "Open as read-only",
            v_flex()
                .gap_1()
                .child(centered(
                    Switch::new("read-only")
                        .checked(row.profile.read_only)
                        .on_click(move |checked, _, cx| {
                            let checked = *checked;
                            AppSettings::update(cx, |settings| {
                                edit_entry(&mut settings.registry, &cluster, |entry| {
                                    entry.read_only = Some(checked);
                                });
                            });
                        }),
                ))
                .child(muted_text(
                    "Default: on for Production. Editing actions stay off while a cluster is read-only.",
                    cx,
                ))
                .into_any_element(),
            cx,
        );
        let confirm_row = form_row(
            "Confirm changes by",
            centered(confirm_menu(row, entry.as_ref())),
            cx,
        );
        let node_shell_cluster = row.cluster.clone();
        let node_shell_row = form_row(
            "Allow node shell",
            v_flex()
                .gap_1()
                .child(centered(
                    Switch::new("allow-node-shell")
                        .checked(row.profile.allow_node_shell)
                        .on_click(move |checked, _, cx| {
                            set_allow_node_shell(&node_shell_cluster, Some(*checked), cx);
                        }),
                ))
                .child(muted_text(NODE_SHELL_HINT, cx))
                .into_any_element(),
            cx,
        );
        let metrics_row = form_row(
            "Source",
            v_flex()
                .gap_1()
                .child(centered(metrics_menu(row, entry.as_ref(), cx)))
                .child(muted_text(METRICS_HINT, cx))
                .into_any_element(),
            cx,
        );
        let proxy_row = self.render_proxy(
            entry.as_ref(),
            info.as_ref().and_then(|info| info.proxy.as_deref()),
            cx,
        );
        let folder = self
            .catalog
            .read(cx)
            .watched_folder_of(&row.cluster.kubeconfig);
        let block_reason = remove_block_reason(row, folder);
        let can_remove = block_reason.is_none();
        let remove_row = row.clone();
        v_flex()
            .w_full()
            .gap_3()
            .child(section(
                "General",
                [name_row, environment_row, color_row, namespace_row],
                cx,
            ))
            .child(section(
                "Connection",
                [
                    form_row("Source", mono_line("source-path", source, &mono), cx),
                    form_row("Server", mono_line("server-text", server, &mono), cx),
                    form_row("Authentication", centered(div().text_sm().child(auth)), cx),
                    form_row("Proxy", proxy_row, cx),
                    form_row(
                        "Connection test",
                        v_flex()
                            .gap_1()
                            .child(centered(self.render_test(cx)))
                            .children(row.trust_note.clone().map(|note| muted_text(note, cx)))
                            .into_any_element(),
                        cx,
                    ),
                ],
                cx,
            ))
            .child(section(
                "Safety",
                [read_only_row, confirm_row, node_shell_row],
                cx,
            ))
            .child(section("Metrics", [metrics_row], cx))
            .child(
                v_flex()
                    .gap_2()
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(
                                Button::new("reset-cluster")
                                    .ghost()
                                    .small()
                                    .label("Reset to defaults")
                                    .tooltip("Clears the overrides and the place in the list.")
                                    .disabled(entry.is_none())
                                    .on_click(
                                        cx.listener(|page, _, _, cx| page.reset_selected(cx)),
                                    ),
                            )
                            .child(
                                Button::new("remove-cluster")
                                    .danger()
                                    .small()
                                    .label("Remove from k8sBoard")
                                    .disabled(!can_remove)
                                    .on_click(cx.listener(move |page, _, window, cx| {
                                        page.confirm_remove(&remove_row, window, cx);
                                    })),
                            ),
                    )
                    .children(block_reason.map(|reason| muted_text(reason, cx))),
            )
            .into_any_element()
    }

    /// The Proxy control: the stored choice, the URL input of Custom with its message, and the note.
    fn render_proxy(
        &self,
        entry: Option<&ClusterEntry>,
        kubeconfig_proxy: Option<&str>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(form) = &self.form else {
            return div().into_any_element();
        };
        let stored = entry.and_then(|entry| entry.proxy.as_ref());
        let mode = proxy_mode(stored, form.is_custom_proxy);
        let is_pending =
            mode == ProxyMode::Custom && is_proxy_pending(stored, &form.proxy_url.read(cx).value());
        let danger = cx.theme().danger;
        let page = cx.entity();
        let kubeconfig_proxy = kubeconfig_proxy.map(str::to_owned);
        let label = proxy_mode_label(mode, kubeconfig_proxy.as_deref());
        let menu = Button::new("proxy-mode")
            .small()
            .outline()
            .label(label)
            .dropdown_caret(true)
            .dropdown_menu(move |menu, _, _| {
                [
                    ProxyMode::FromKubeconfig,
                    ProxyMode::Direct,
                    ProxyMode::Custom,
                ]
                .into_iter()
                .fold(menu, |menu, choice| {
                    let page = page.clone();
                    menu.item(
                        PopupMenuItem::new(proxy_mode_label(choice, kubeconfig_proxy.as_deref()))
                            .checked(choice == mode)
                            .on_click(move |_, _, cx| {
                                page.update(cx, |page, cx| page.pick_proxy_mode(choice, cx));
                            }),
                    )
                })
            });
        v_flex()
            .gap_1()
            .child(centered(
                h_flex()
                    .gap_3()
                    .items_center()
                    .child(menu)
                    .children(is_pending.then(|| muted_text("Not applied", cx))),
            ))
            .when(mode == ProxyMode::Custom, |column| {
                column.child(Input::new(&form.proxy_url)).children(
                    form.proxy_error
                        .as_ref()
                        .map(|error| error_text(error, danger)),
                )
            })
            .child(muted_text(PROXY_HINT, cx))
            .into_any_element()
    }

    fn render_test(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (success, danger) = (theme.success, theme.danger);
        let result = match &self.test {
            TestState::Idle => None,
            TestState::Running => Some(muted_text("Testing…", cx).into_any_element()),
            TestState::Connected {
                version,
                latency_ms,
            } => Some(
                div()
                    .text_sm()
                    .text_color(success)
                    .child(format!("Connected · {version} · {latency_ms} ms"))
                    .into_any_element(),
            ),
            TestState::Failed(message) => Some(
                div()
                    .text_sm()
                    .text_color(danger)
                    .child(format!("Failed: {message}"))
                    .into_any_element(),
            ),
        };
        h_flex()
            .gap_3()
            .items_center()
            .child(
                Button::new("test-connection")
                    .small()
                    .label("Test connection")
                    .disabled(self.test == TestState::Running)
                    .on_click(cx.listener(|page, _, _, cx| page.start_test(cx))),
            )
            .children(result)
            .into_any_element()
    }
}

/// The Add cluster button of the page header. `blocked` is why pasting is off.
pub(crate) fn add_cluster_button(
    page: Entity<ClustersPage>,
    blocked: Option<&'static str>,
) -> impl IntoElement {
    Button::new("add-cluster")
        .small()
        .icon(Icon::new(IconName::Plus))
        .label("Add cluster")
        .dropdown_caret(true)
        .dropdown_menu(move |menu, _, _| {
            let import_page = page.clone();
            let paste_page = page.clone();
            let import = PopupMenuItem::new("Import kubeconfig file…")
                .action(Box::new(ImportKubeconfig))
                .on_click(move |_, window, cx| {
                    import_page.update(cx, |page, cx| page.import_file(window, cx));
                });
            let paste = match blocked {
                Some(reason) => disabled_menu_item("Paste kubeconfig YAML…", reason.into()),
                None => {
                    PopupMenuItem::new("Paste kubeconfig YAML…").on_click(move |_, window, cx| {
                        paste_page.update(cx, |page, cx| page.start_paste(window, cx));
                    })
                }
            };
            let folder_page = page.clone();
            let watch =
                PopupMenuItem::new("Watch a kubeconfig folder…").on_click(move |_, window, cx| {
                    folder_page.update(cx, |page, cx| page.watch_folder(window, cx));
                });
            let later = ["Scan AWS EKS", "Scan Google GKE", "Scan Azure AKS"];
            later.into_iter().fold(
                menu.item(import).item(paste).item(watch).separator(),
                |menu, label| menu.item(disabled_menu_item(label, LATER_VERSION.into())),
            )
        })
}

fn environment_menu(row: &ClusterRow, entry: Option<&ClusterEntry>) -> impl IntoElement {
    let current = entry.and_then(|entry| entry.environment);
    let auto = format!("Auto ({})", row.guessed.badge());
    let label = current.map_or_else(|| auto.clone(), |environment| environment.name().to_owned());
    let cluster = row.cluster.clone();
    Button::new("environment")
        .small()
        .outline()
        .label(label)
        .dropdown_caret(true)
        .dropdown_menu(move |menu, _, _| {
            let choices = [
                (auto.clone(), None),
                (
                    Environment::Production.name().to_owned(),
                    Some(Environment::Production),
                ),
                (
                    Environment::Staging.name().to_owned(),
                    Some(Environment::Staging),
                ),
                (
                    Environment::Development.name().to_owned(),
                    Some(Environment::Development),
                ),
                (
                    Environment::Local.name().to_owned(),
                    Some(Environment::Local),
                ),
            ];
            choices.into_iter().fold(menu, |menu, (label, value)| {
                let cluster = cluster.clone();
                menu.item(
                    PopupMenuItem::new(label)
                        .checked(value == current)
                        .on_click(move |_, _, cx| {
                            AppSettings::update(cx, |settings| {
                                edit_entry(&mut settings.registry, &cluster, |entry| {
                                    entry.environment = value;
                                });
                            });
                        }),
                )
            })
        })
}

/// The select text of a confirm mode, which the Auto entry repeats for the environment default.
fn confirm_label(mode: ConfirmMode) -> &'static str {
    match mode {
        ConfirmMode::TypeName => "Typing the cluster name",
        ConfirmMode::Click => "Clicking Confirm",
    }
}

/// Stores the confirm mode of `cluster`; `None` is Auto and drops the key (`edit_entry` drops an
/// entry left with no override).
pub(crate) fn set_confirm(cluster: &ClusterRef, mode: Option<ConfirmMode>, cx: &mut App) {
    AppSettings::update(cx, |settings| {
        edit_entry(&mut settings.registry, cluster, |entry| {
            entry.confirm = mode;
        });
    });
}

/// The hint under the Allow node shell switch (wireframe W2).
const NODE_SHELL_HINT: &str =
    "Creates a privileged debug pod on the node. Off by default for production.";

/// Stores the Allow node shell switch of `cluster`; `None` follows the environment again.
pub(crate) fn set_allow_node_shell(cluster: &ClusterRef, allowed: Option<bool>, cx: &mut App) {
    AppSettings::update(cx, |settings| {
        edit_entry(&mut settings.registry, cluster, |entry| {
            entry.allow_node_shell = allowed;
        });
    });
}

const METRICS_HINT: &str = "Used for 7- and 30-day Monitor ranges and Topology traffic.";
const METRICS_SERVER_ONLY: &str = "metrics-server only";

/// Stores the metrics source of `cluster`; `None` is metrics-server only.
pub(crate) fn set_metrics_source(
    cluster: &ClusterRef,
    source: Option<MetricsSourceFields>,
    cx: &mut App,
) {
    AppSettings::update(cx, |settings| {
        edit_entry(&mut settings.registry, cluster, |entry| {
            entry.metrics = source;
        });
    });
}

/// What the Source button shows: `metrics-server only` or the saved service.
fn metrics_source_label(stored: Option<&MetricsSourceFields>) -> String {
    match stored {
        None => METRICS_SERVER_ONLY.to_owned(),
        Some(fields) => format!(
            "Prometheus-compatible · {}/{}:{}",
            fields.namespace, fields.service, fields.port
        ),
    }
}

/// The Metrics section's Source dropdown (W2 `Metrics · Source ▾`).
fn metrics_menu(row: &ClusterRow, entry: Option<&ClusterEntry>, cx: &App) -> impl IntoElement {
    let stored = entry.and_then(|entry| entry.metrics.clone());
    let is_active = cx
        .try_global::<ActiveConnection>()
        .is_some_and(|active| active.cluster == row.cluster);
    let cluster = row.cluster.clone();
    Button::new("metrics-source")
        .small()
        .outline()
        .label(metrics_source_label(stored.as_ref()))
        .dropdown_caret(true)
        .dropdown_menu(move |menu, _, _| {
            let cluster = cluster.clone();
            let clear = PopupMenuItem::new(METRICS_SERVER_ONLY)
                .checked(stored.is_none())
                .on_click(move |_, _, cx| set_metrics_source(&cluster, None, cx));
            let menu = menu.item(clear);
            let menu = match &stored {
                Some(fields) => {
                    menu.item(PopupMenuItem::new(metrics_source_label(Some(fields))).checked(true))
                }
                None => menu,
            };
            let choose = if is_active {
                PopupMenuItem::new("Choose on the Metrics page…")
                    .on_click(|_, _, cx| show_metrics_page(cx))
            } else {
                disabled_menu_item(
                    "Choose on the Metrics page…",
                    "Connect to this cluster first".into(),
                )
            };
            menu.separator().item(choose)
        })
}

fn confirm_menu(row: &ClusterRow, entry: Option<&ClusterEntry>) -> impl IntoElement {
    let current = entry.and_then(|entry| entry.confirm);
    let auto = format!(
        "Auto ({})",
        confirm_label(ConfirmMode::for_environment(row.profile.environment))
    );
    let label = current.map_or_else(|| auto.clone(), |mode| confirm_label(mode).to_owned());
    let cluster = row.cluster.clone();
    Button::new("confirm-mode")
        .small()
        .outline()
        .label(label)
        .dropdown_caret(true)
        .dropdown_menu(move |menu, _, _| {
            let choices = [
                (auto.clone(), None),
                (
                    confirm_label(ConfirmMode::TypeName).to_owned(),
                    Some(ConfirmMode::TypeName),
                ),
                (
                    confirm_label(ConfirmMode::Click).to_owned(),
                    Some(ConfirmMode::Click),
                ),
            ];
            choices.into_iter().fold(menu, |menu, (label, mode)| {
                let cluster = cluster.clone();
                menu.item(
                    PopupMenuItem::new(label)
                        .checked(mode == current)
                        .on_click(move |_, _, cx| set_confirm(&cluster, mode, cx)),
                )
            })
        })
}

fn muted_text(text: impl Into<SharedString>, cx: &App) -> gpui_kit::Div {
    div()
        .text_sm()
        .text_color(cx.theme().muted_foreground)
        .child(text.into())
}

/// One monospace line, cut with an ellipsis; the full text is the tooltip.
fn mono_line(id: &'static str, text: String, mono: &SharedString) -> AnyElement {
    centered(truncated_text(id, text).text_xs().font_family(mono.clone()))
}

/// Centers a one-line control in a band as tall as an input.
fn centered(control: impl IntoElement) -> AnyElement {
    div()
        .h(px(CONTROL_HEIGHT))
        .flex()
        .items_center()
        .child(control)
        .into_any_element()
}

fn error_text(error: &FieldError, danger: gpui_kit::Hsla) -> AnyElement {
    div()
        .text_xs()
        .text_color(danger)
        .child(error.0.clone())
        .into_any_element()
}

/// A label on the left, the control on the right.
fn form_row(label: &'static str, control: AnyElement, cx: &App) -> AnyElement {
    h_flex()
        .w_full()
        .gap_3()
        .items_start()
        .child(
            div()
                .w(px(FIELD_LABEL_WIDTH))
                .flex_none()
                .h(px(CONTROL_HEIGHT))
                .flex()
                .items_center()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(label),
        )
        .child(div().flex_1().min_w_0().child(control))
        .into_any_element()
}

fn section<const N: usize>(
    title: &'static str,
    rows: [AnyElement; N],
    cx: &App,
) -> impl IntoElement {
    v_flex()
        .w_full()
        .gap_2()
        .child(
            div()
                .text_sm()
                .font_semibold()
                .pb_1()
                .border_b_1()
                .border_color(cx.theme().border)
                .child(title),
        )
        .children(rows)
}

/// The payload of a row drag, and the chip that follows the pointer (the `DraggedTab` pattern
/// of the dock).
#[derive(Clone)]
struct DraggedCluster {
    cluster: ClusterRef,
    /// A row only drops on its own group: the environment decides the group.
    group_title: &'static str,
    label: SharedString,
}

impl Render for DraggedCluster {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        div()
            .px_2()
            .py_1()
            .rounded_sm()
            .bg(theme.muted)
            .border_1()
            .border_color(theme.border)
            .font_family(theme.mono_font_family.clone())
            .text_xs()
            .child(self.label.clone())
    }
}

/// Stores the title-bar color of `cluster`; a color equal to its environment's is not stored, so
/// the cluster keeps following its environment.
pub(crate) fn set_cluster_color(
    cluster: &ClusterRef,
    color: ClusterColor,
    environment: Environment,
    cx: &mut App,
) {
    let stored = color_to_store(color, environment);
    AppSettings::update(cx, |settings| {
        edit_entry(&mut settings.registry, cluster, |entry| {
            entry.color = stored
        });
    });
}

/// Six round swatches, the current one ringed.
fn color_swatches(row: &ClusterRow, cx: &App) -> AnyElement {
    let ring = cx.theme().foreground;
    let mut swatches = h_flex().h(px(CONTROL_HEIGHT)).items_center().gap_2();
    for (index, color) in ClusterColor::ALL.into_iter().enumerate() {
        let is_current = row.profile.color == color;
        let (cluster, environment) = (row.cluster.clone(), row.profile.environment);
        swatches = swatches.child(
            div()
                .id(ElementId::from(("cluster-color", index)))
                .size(px(18.))
                .flex_none()
                .rounded_full()
                .bg(cluster_color(color, cx))
                .cursor_pointer()
                .when(is_current, |swatch| swatch.border_2().border_color(ring))
                .tooltip(move |window, cx| Tooltip::new(color.name()).build(window, cx))
                .on_click(move |_, _, cx| set_cluster_color(&cluster, color, environment, cx)),
        );
    }
    swatches.into_any_element()
}
