//! The Clusters page of the Settings window: the list of clusters grouped by environment and the
//! form of the selected one. The page owns the view state only; the rules live in
//! `cluster_form.rs`, and the file writes in `cluster_catalog.rs`.
//!
//! `ClustersPage` has no `Debug`: it can hold the clipboard text of a pending paste.

use std::path::PathBuf;
use std::sync::Arc;

use cluster::Kubeconfig;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariant, ButtonVariants as _};
use gpui_kit::component::dialog::DialogButtonProps;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, Sizable as _, StyledExt as _, WindowExt as _, h_flex,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, ElementId, Entity, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, SharedString, StatefulInteractiveElement as _,
    Styled as _, Subscription, Task, Window, div, px,
};

use crate::app_shell::find_cluster;
use crate::cluster_catalog::{CatalogNotice, ClusterCatalog, PasteStatus};
use crate::cluster_form::{
    ClusterGroup, ClusterRow, FieldError, RowOrigin, TEST_CONNECTION_TIMEOUT, TestState,
    count_text, edit_entry, remove_dialog_text, reset_entry, resolve_selection, test_connection,
    validate_display_name, validate_namespace,
};
use crate::cluster_registry::{ClusterEntry, ClusterRef};
use crate::cluster_runtime::ClusterRuntime;
use crate::drawer::truncated_text;
use crate::environment::{Environment, environment_badge};
use crate::resource_actions::disabled_menu_item;
use crate::settings::AppSettings;
use crate::settings_window::ImportKubeconfig;

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
const CHAIN_ROW_REASON: &str = "Comes from KUBECONFIG or ~/.kube/config; edit that instead.";

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
    _observers: Vec<Subscription>,
}

/// The inputs of the selected cluster, recreated with the stored values when the selection
/// changes.
struct ClusterForm {
    cluster: ClusterRef,
    name: Entity<InputState>,
    namespace: Entity<InputState>,
    name_error: Option<FieldError>,
    namespace_error: Option<FieldError>,
    _subscriptions: Vec<Subscription>,
}

impl ClustersPage {
    pub(crate) fn new(catalog: Entity<ClusterCatalog>, cx: &mut Context<Self>) -> Self {
        let observers = vec![
            cx.observe_global::<AppSettings>(|_, cx| cx.notify()),
            cx.observe(&catalog, |page, _, cx| page.on_catalog_changed(cx)),
        ];
        Self {
            catalog,
            selected: None,
            form: None,
            test: TestState::Idle,
            test_task: None,
            paste_text: None,
            pending_select: None,
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
        let subscriptions = vec![
            cx.subscribe_in(&name, window, Self::on_name_event),
            cx.subscribe_in(&namespace, window, Self::on_namespace_event),
        ];
        self.form = Some(ClusterForm {
            cluster: row.cluster.clone(),
            name,
            namespace,
            name_error: None,
            namespace_error: None,
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
        let running = cx.global::<ClusterRuntime>().spawn(test_connection(
            kubeconfig,
            summary.name,
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
                    | CatalogNotice::SaveFailed(_) => line.into_any_element(),
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
        let mut list = v_flex()
            .id("cluster-list")
            .w(px(LIST_WIDTH))
            .flex_none()
            .max_h(px(LIST_MAX_HEIGHT))
            .overflow_y_scroll()
            .p_1()
            .gap_1()
            .border_1()
            .border_color(border)
            .rounded(cx.theme().radius);
        if groups.is_empty() {
            list = list.child(muted_text("No clusters yet.", cx));
        }
        let mut index = 0;
        for group in groups {
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
                list = list.child(self.render_row(index, row, cx));
                index += 1;
            }
        }
        list.into_any_element()
    }

    fn render_row(&self, index: usize, row: &ClusterRow, cx: &mut Context<Self>) -> AnyElement {
        let is_selected = self.selected.as_ref() == Some(&row.cluster);
        let theme = cx.theme();
        let (active, hover, muted) = (theme.list_active, theme.list_hover, theme.muted_foreground);
        let mono = theme.mono_font_family.clone();
        let cluster = row.cluster.clone();
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
                    "Default: on for Production. Applies when editing actions arrive (0030).",
                    cx,
                ))
                .into_any_element(),
            cx,
        );
        let can_remove = row.origin != RowOrigin::Chain;
        let remove_row = row.clone();
        v_flex()
            .w_full()
            .gap_3()
            .child(section(
                "General",
                [name_row, environment_row, namespace_row],
                cx,
            ))
            .child(section(
                "Connection",
                [
                    form_row("Source", mono_line("source-path", source, &mono), cx),
                    form_row("Server", mono_line("server-text", server, &mono), cx),
                    form_row("Authentication", centered(div().text_sm().child(auth)), cx),
                    form_row("Connection test", centered(self.render_test(cx)), cx),
                ],
                cx,
            ))
            .child(section("Safety", [read_only_row], cx))
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
                    .children((!can_remove).then(|| muted_text(CHAIN_ROW_REASON, cx))),
            )
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
            let later = [
                "Watch a kubeconfig folder…",
                "Scan AWS EKS",
                "Scan Google GKE",
                "Scan Azure AKS",
            ];
            later
                .into_iter()
                .fold(menu.item(import).item(paste).separator(), |menu, label| {
                    menu.item(disabled_menu_item(label, LATER_VERSION.into()))
                })
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
