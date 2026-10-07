//! The workspace region of the window: header, banner, table or state view, and the drawer.

use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::base::{ResizeHandleRenderer, ResizeHandleState};
use gpui_kit::component::alert::Alert;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::resizable::{
    ResizableState, resizable_panel, resize_handle_appearance, v_resizable,
};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::table::DataTable;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, Selectable as _, Sizable as _, Size, StyledExt as _,
    h_flex, v_flex,
};
use gpui_kit::{
    AnyElement, App, Bounds, Context, Entity, Hsla, InteractiveElement as _, IntoElement,
    MouseButton, ParentElement as _, Pixels, SharedString, Styled as _, Window, canvas, deferred,
    div, fill, point, prelude::FluentBuilder as _, px, size,
};

use cluster::{EVENT_LIMIT, EventFilter, NamespaceScope, ObjectKind};

use super::{AppShell, KubeconfigState, Screen};
use crate::cluster_session::{FlowState, LiveCluster, SessionPhase};
use crate::clusters_page::ClustersPage;
use crate::dock::{
    DEFAULT_DOCK_HEIGHT, DockMode, MIN_DOCK_HEIGHT, dock_max_height, initial_dock_height,
    max_line_offset, saved_dock_height,
};
use crate::drawer::{ClickHandler, DrawerChrome};
use crate::file_export::ExportState;
use crate::filter_bar::{ToolkitState, filter_bar};
use crate::issue_board::IssueSummary;
use crate::issue_table::coverage_status;
use crate::kind_drawer::kind_drawer;
use crate::navigation::{SIDEBAR_WIDTH, screen_icon};
use crate::node_drawer::node_drawer;
use crate::node_summary::role_counts;
use crate::overview::{
    OverviewData, change_window_button, headline_text, overview_body, stats_line,
};
use crate::pod_drawer::pod_drawer;
use crate::port_forward_menu::PortButtons;
use crate::resource_actions::{ActionAvailability, ResourceAction, RowAction, action_availability};
use crate::resource_kind::ResourceKind;
use crate::row_context::RowContext;
use crate::row_selection::{selection_bar, selection_bar_clearance, unticked_notice};
use crate::settings::AppSettings;
use crate::settings_window::{ClusterAddition, add_cluster, manage_clusters};
use crate::table_filter::FilterPreset;
use crate::table_selection::ResourceKey;
use crate::usage_format::group_digits;

/// The row height of every table, from the saved density. The kit uses the same size for the
/// header row, so the header follows (accepted: the Tokens page shows one row height).
fn row_size(cx: &App) -> Size {
    Size::Size(px(AppSettings::get(cx).appearance.density.row_height()))
}

impl AppShell {
    /// The tables have fixed pixel columns, so one column is resized to fill the workspace
    /// whenever the window size changes.
    pub(super) fn fit_table_widths(&self, window: &Window, cx: &mut Context<Self>) {
        let table_width = window.viewport_size().width - SIDEBAR_WIDTH;
        self.pod_table.update(cx, |table, cx| {
            if table.delegate_mut().fit_width(table_width) {
                table.refresh(cx);
            }
        });
        self.node_table.update(cx, |table, cx| {
            if table.delegate_mut().fit_width(table_width) {
                table.refresh(cx);
            }
        });
        self.issue_table.update(cx, |table, cx| {
            if table.delegate_mut().fit_width(table_width) {
                table.refresh(cx);
            }
        });
        self.kind_table.update(cx, |table, cx| {
            if table.delegate_mut().fit_width(table_width) {
                table.refresh(cx);
            }
        });
    }

    /// The region right of the sidebar: the list region with the log dock below it, or the
    /// dock alone over the whole region when zoomed.
    pub(super) fn render_workspace(&self, window: &Window, cx: &Context<Self>) -> impl IntoElement {
        self.drawer
            .set_workspace_width(window.viewport_size().width - SIDEBAR_WIDTH);
        let region = v_flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .bg(cx.theme().background);
        let dock = self.dock.read(cx);
        if !dock.has_tabs() {
            return region.child(self.render_upper(cx));
        }
        match dock.mode() {
            DockMode::Zoomed => region.child(self.dock.clone()),
            DockMode::Minimized => region.child(self.render_upper(cx)).child(self.dock.clone()),
            DockMode::Normal => {
                let max_height = dock_max_height(self.dock_split.read(cx).container_size());
                // The kit clamps from the second frame on; the first frame has no measured
                // workspace yet, so the viewport caps it.
                let height = initial_dock_height(
                    AppSettings::get(cx).dock.height,
                    window.viewport_size().height,
                );
                region.child(
                    v_resizable("workspace-split")
                        .with_handle_appearance(dock_handle_appearance(self.dock_split.clone()))
                        .with_state(&self.dock_split)
                        .child(resizable_panel().child(self.render_upper(cx)))
                        .child(
                            resizable_panel()
                                .size(height)
                                .flex_none()
                                .size_range(MIN_DOCK_HEIGHT..max_height)
                                .child(self.dock.clone()),
                        ),
                )
            }
        }
    }

    /// Stores the dock height when a resize ends; a reset to the default forgets it.
    pub(super) fn save_dock_height(split: &Entity<ResizableState>, cx: &mut App) {
        // The dock is the second panel; with one panel there is nothing to save.
        let Some(size) = split.read(cx).sizes().get(1).copied() else {
            return;
        };
        AppSettings::update(cx, |settings| {
            settings.dock.height = saved_dock_height(size)
        });
    }

    /// Header, banner, body, and the drawer overlay. The drawer covers this region only, so
    /// it never covers the dock.
    fn render_upper(&self, cx: &Context<Self>) -> AnyElement {
        // The Edit YAML view takes the place of the table and the drawer (spec 0031). The cursor and
        // the drawer flag are kept, so closing it shows the workspace as it was.
        if let Some(edit) = &self.edit {
            return v_flex()
                .flex_1()
                .min_w_0()
                .min_h_0()
                .child(edit.element())
                .into_any_element();
        }
        // One read of the table view for everything drawn from it in this frame.
        let toolkit = self.toolkit_state(cx);
        let toolkit = toolkit.as_ref();
        v_flex()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .relative()
            .child(self.render_header(toolkit, cx))
            .children(self.render_filter_bar(toolkit, cx))
            .children(self.render_overview_stats(cx))
            .children(self.render_overview_export_error())
            .children(self.render_interruption_banner(cx))
            // The floating selection bar must not cover the last rows: pad the body by its height.
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .pb(selection_bar_clearance(
                        toolkit.map_or(0, |state| state.checked),
                    ))
                    .child(self.render_body(cx)),
            )
            .children(self.render_selection_bar(toolkit, cx))
            .children(self.render_value_popover(toolkit))
            .children(self.render_drawer(cx))
            .into_any_element()
    }

    fn render_header(
        &self,
        toolkit: Option<&ToolkitState>,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let live = self.live(cx);
        let (title, count) = match self.screen {
            Screen::Overview => (
                "Overview",
                self.session().zip(live).map(|(session, live)| {
                    headline_text(
                        session.read(cx).context(),
                        &live.server_version,
                        live.nodes.ready_items(),
                    )
                }),
            ),
            Screen::Pods => (
                "Pods",
                live.and_then(|live| {
                    live.pods.ready_count().map(|count| {
                        format!(
                            "{} · {}",
                            count_label(count, "pod", "pods"),
                            live.scope_label()
                        )
                    })
                }),
            ),
            Screen::Nodes => (
                "Nodes",
                live.and_then(|live| {
                    let count = live.nodes.ready_count()?;
                    let text = nodes_count_text(count, &role_counts(live.nodes.items()));
                    Some(cluster_wide_text(text, &live.scope))
                }),
            ),
            Screen::Issues => (
                "Issues",
                self.issue_summary(cx)
                    .map(|summary| count_label(summary.total, "issue", "issues")),
            ),
            Screen::Topology => ("Topology", self.topology.read(cx).header_count(cx)),
            Screen::PortForwarding => ("Port Forwarding", Some(self.port_forward_header_count(cx))),
            Screen::Kind(kind) => (
                kind.label(),
                live.and_then(|live| {
                    let count = live.kind_list(kind)?.list.ready_count()?;
                    let label = count_label(count, kind.singular(), kind.plural());
                    let text = if kind.is_namespaced() {
                        format!("{label} · {}", live.scope_label())
                    } else {
                        cluster_wide_text(label, &live.scope)
                    };
                    // The events store keeps only the newest ones, so say so at the cap.
                    Some(if kind == ResourceKind::Events && count >= EVENT_LIMIT {
                        format!("{text} · newest {}", group_digits(EVENT_LIMIT))
                    } else {
                        text
                    })
                }),
            ),
        };
        // A filter replaces the total with how many rows match it.
        let count = match (toolkit, count) {
            (Some(state), Some(_)) if state.is_filtering => Some(filtered_count_label(state)),
            (_, count) => count,
        };
        let count = count.map(|count| match live.and_then(LiveCluster::explorer_flow) {
            Some(FlowState::Paused { has_held })
                if self.screen == Screen::Kind(ResourceKind::Events) =>
            {
                paused_text(&count, has_held)
            }
            _ => count,
        });
        h_flex()
            .flex_shrink_0()
            .gap_3()
            .items_baseline()
            .px_4()
            .py_2()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .child(
                        Icon::new(screen_icon(self.screen))
                            .size_4()
                            .text_color(cx.theme().muted_foreground),
                    )
                    .child(div().text_lg().font_semibold().child(title)),
            )
            .children(count.map(|count| {
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(count)
            }))
            .children(self.render_header_actions(toolkit, cx))
    }

    /// The bar over the bottom of the table while rows are ticked, and the notice of rows a filter
    /// unticked above it (alone when nothing is ticked any more). It sits left of an open
    /// drawer, and the drawer is drawn after it.
    fn render_selection_bar(
        &self,
        state: Option<&ToolkitState>,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        self.any_live(cx).then_some(())?;
        let state = state?;
        let bar = self.selection_bar_pill(state, cx);
        let notice = self
            .unticked_notice
            .as_ref()
            .map(|notice| unticked_notice(notice.count, &cx.weak_entity(), cx));
        if bar.is_none() && notice.is_none() {
            return None;
        }
        let right = self.open_drawer_width();
        Some(
            div()
                .absolute()
                .bottom_4()
                .left_0()
                .right(right)
                .flex()
                .flex_col()
                .items_center()
                .gap_2()
                .children(notice)
                .children(bar)
                .into_any_element(),
        )
    }

    /// The count and the bulk buttons of the ticked rows; `None` when none is ticked.
    fn selection_bar_pill(&self, state: &ToolkitState, cx: &Context<Self>) -> Option<AnyElement> {
        if state.checked == 0 {
            return None;
        }
        let (singular, plural) = match self.screen {
            Screen::Overview | Screen::Topology | Screen::PortForwarding => return None,
            Screen::Pods => ("pod", "pods"),
            Screen::Nodes => ("node", "nodes"),
            Screen::Issues => ("issue", "issues"),
            Screen::Kind(kind) => (kind.singular(), kind.plural()),
        };
        let text = format!("{} selected", count_label(state.checked, singular, plural));
        Some(selection_bar(
            text,
            self.bulk_buttons(cx),
            &cx.weak_entity(),
            cx,
        ))
    }

    /// The value popover, over the bottom of the table and above the selection bar while rows are
    /// ticked. It sits left of an open drawer, like the bar.
    fn render_value_popover(&self, state: Option<&ToolkitState>) -> Option<AnyElement> {
        let popover = self.value_popover()?.clone();
        let bar_height = selection_bar_clearance(state.map_or(0, |state| state.checked));
        let right = self.open_drawer_width();
        Some(
            div()
                .absolute()
                .bottom_4()
                .mb(bar_height)
                .left_0()
                .right(right)
                .flex()
                .justify_center()
                .child(popover)
                .into_any_element(),
        )
    }

    /// The Overview header, right-aligned: the saved file name, the range, and Export report. The
    /// range needs a live session; Export report is disabled without one and while an export runs.
    fn overview_header_buttons(&self, cx: &Context<Self>) -> Vec<AnyElement> {
        let is_live = self.live(cx).is_some();
        let export = &self.overview.export;
        let saved = match export {
            ExportState::Saved { file_name } => Some(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!("Saved to {file_name}"))
                    .into_any_element(),
            ),
            _ => None,
        };
        let range = is_live.then(|| change_window_button(self.overview.window, cx));
        let button = Button::new("export-report")
            .ghost()
            .small()
            .icon(Icon::new(IconName::Download))
            .label("Export report")
            .tooltip("Save the Overview as a Markdown file…")
            .disabled(!is_live || export.is_busy())
            .on_click(cx.listener(|shell, _, _, cx| shell.export_overview_report(cx)));
        saved
            .into_iter()
            .chain(range)
            .chain([button.into_any_element()])
            .collect()
    }

    /// The Topology header, right-aligned: the saved file name, the partial-view hint, Fit, and Export PNG. Both buttons
    /// need a graph; Export PNG is also disabled while an export runs.
    fn topology_header_buttons(&self, cx: &Context<Self>) -> Vec<AnyElement> {
        let topology = self.topology.read(cx);
        let has_graph = topology.has_graph();
        let saved = topology.export_detail().map(|detail| {
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(detail)
                .into_any_element()
        });
        let partial = topology.partial_view_hint().map(|hint| {
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(hint)
                .into_any_element()
        });
        let fit = Button::new("topology-fit")
            .ghost()
            .small()
            .icon(Icon::new(IconName::Maximize))
            .label("Fit")
            .tooltip("Fit the whole graph in view")
            .disabled(!has_graph)
            .on_click(cx.listener(|shell, _, _, cx| {
                shell.topology.update(cx, |view, cx| view.fit(cx));
            }));
        let export = Button::new("topology-export")
            .ghost()
            .small()
            .icon(Icon::new(IconName::Download))
            .label("Export PNG")
            .tooltip("Save the graph as a PNG or SVG file…")
            .disabled(!has_graph || topology.export_state().is_busy())
            .on_click(cx.listener(|shell, _, _, cx| {
                shell.topology.update(cx, |view, cx| view.export(cx));
            }));
        saved
            .into_iter()
            .chain(partial)
            .chain([fit.into_any_element(), export.into_any_element()])
            .collect()
    }

    /// Under the Overview header: why the last export failed.
    fn render_overview_export_error(&self) -> Option<AnyElement> {
        let ExportState::Failed { message } = &self.overview.export else {
            return None;
        };
        if self.screen != Screen::Overview {
            return None;
        }
        Some(
            div()
                .flex_shrink_0()
                .px_4()
                .py_1()
                .child(Alert::error("overview-export-error", message.clone()))
                .into_any_element(),
        )
    }

    /// The counts under the Overview header.
    fn render_overview_stats(&self, cx: &Context<Self>) -> Option<AnyElement> {
        if self.screen != Screen::Overview {
            return None;
        }
        let live = self.live(cx)?;
        Some(stats_line(live, cx).into_any_element())
    }

    /// The filter bar under the header, once the session is live.
    fn render_filter_bar(
        &self,
        state: Option<&ToolkitState>,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        self.any_live(cx).then_some(())?;
        Some(filter_bar(state?, self, &self.quick_filter, cx))
    }

    /// The per-screen toggles, right-aligned in the header: Warnings only with Pause stream on
    /// Events, and Hide system on the RBAC kinds.
    fn render_header_actions(
        &self,
        toolkit: Option<&ToolkitState>,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let buttons: Vec<AnyElement> = match self.screen {
            Screen::Overview => self.overview_header_buttons(cx),
            Screen::Topology => self.topology_header_buttons(cx),
            Screen::PortForwarding => self.port_forward_header_buttons(cx),
            Screen::Issues => return self.render_issues_status(cx),
            Screen::Nodes => self.node_header_buttons(cx),
            Screen::Kind(ResourceKind::NetworkPolicies) => {
                self.render_test_traffic(cx).into_iter().collect()
            }
            Screen::Kind(ResourceKind::ServiceAccounts) => {
                self.render_check_permissions(cx).into_iter().collect()
            }
            Screen::Kind(kind @ ResourceKind::Custom(custom))
                if custom.is_cert_manager_certificate() =>
            {
                self.render_renew(kind, cx).into_iter().collect()
            }
            Screen::Kind(ResourceKind::Roles) => self.render_who_can(cx).into_iter().collect(),
            Screen::Kind(ResourceKind::ClusterRoles) => [
                self.render_who_can(cx),
                self.render_hide_system(toolkit, cx),
            ]
            .into_iter()
            .flatten()
            .collect(),
            Screen::Kind(ResourceKind::ClusterRoleBindings) => {
                self.render_hide_system(toolkit, cx).into_iter().collect()
            }
            Screen::Kind(ResourceKind::Events) => {
                [self.render_warnings_only(cx), self.render_pause_stream(cx)]
                    .into_iter()
                    .flatten()
                    .collect()
            }
            screen => {
                let (kind, label) = new_button_of(screen)?;
                vec![self.render_new_button(kind, label, cx)]
            }
        };
        Some(
            h_flex()
                .ml_auto()
                .gap_2()
                .children(buttons)
                .into_any_element(),
        )
    }

    /// The numbers behind the title bar flag; `None` before pods and nodes have loaded.
    fn issue_summary(&self, cx: &App) -> Option<IssueSummary> {
        self.live(cx)?;
        self.session()?.read(cx).issues().summary()
    }

    /// Right of the Issues header: how the issues were found, and what could not be checked. A
    /// gap shows as `Partial coverage` in Warn with the note as its tooltip; a feed that is only
    /// limited by design shows muted, untoned.
    fn render_issues_status(&self, cx: &Context<Self>) -> Option<AnyElement> {
        self.live(cx)?;
        let board = self.session()?.read(cx).issues();
        Some(
            h_flex()
                .ml_auto()
                .text_sm()
                .child(coverage_status(board.coverage(), cx))
                .into_any_element(),
        )
    }

    /// Opens Test traffic on the defaults for the first pods.
    fn render_test_traffic(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let button = Button::new("test-traffic")
            .icon(Icon::new(IconName::Activity))
            .label("Test traffic")
            .small()
            .outline();
        let button = if self.live(cx).is_some() {
            button
                .tooltip("Check whether NetworkPolicies allow a connection")
                .on_click(cx.listener(|shell, _, window, cx| {
                    let Some(cluster) = shell.active_cluster() else {
                        return;
                    };
                    shell.open_traffic_test(&cluster, None, false, window, cx);
                }))
        } else {
            button.disabled(true).tooltip("Not connected")
        };
        Some(button.into_any_element())
    }

    /// Opens Check permissions for the account whose drawer is open, else for You.
    fn render_check_permissions(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let button = Button::new("check-permissions")
            .icon(Icon::new(IconName::ShieldQuestionMark))
            .label("Check permissions")
            .small()
            .outline();
        let button = if self.live(cx).is_some() {
            button
                .tooltip("See what You or a service account can do")
                .on_click(cx.listener(|shell, _, window, cx| {
                    // The one open cluster.
                    let Some(cluster) = shell.active_cluster() else {
                        return;
                    };
                    let (subject, namespace) = match shell.drawer_account() {
                        Some((subject, namespace)) => (Some(subject), Some(namespace)),
                        None => (None, shell.tool_namespace(cx)),
                    };
                    shell.open_permissions(&cluster, subject, namespace, true, window, cx);
                }))
        } else {
            button.disabled(true).tooltip("Not connected")
        };
        Some(button.into_any_element())
    }

    /// The `New` button of a screen whose kind can be created (spec 0042). Off with the gate's
    /// reason: the permissions still checking, `Not permitted: create {resource}`, or the lock.
    fn render_new_button(
        &self,
        kind: ObjectKind,
        label: &'static str,
        cx: &Context<Self>,
    ) -> AnyElement {
        let button = Button::new("new-object")
            .icon(Icon::new(IconName::Plus))
            .label(label)
            .small()
            .outline();
        match self.new_object_block(kind, cx) {
            Some(reason) => button.disabled(true).tooltip(reason),
            None => button
                .tooltip(format!("Create a {}", kind.name()))
                .on_click(cx.listener(move |shell, _, window, cx| {
                    shell.open_create(kind, window, cx);
                })),
        }
        .into_any_element()
    }

    /// Why `New` of `kind` is off on the open cluster, `None` when it is on.
    pub(super) fn new_object_block(&self, kind: ObjectKind, cx: &App) -> Option<SharedString> {
        let guard = self
            .active_cluster()
            .and_then(|cluster| self.guard_for(&cluster, cx));
        let Some(guard) = guard else {
            return Some("Not connected".into());
        };
        match action_availability(ResourceAction::CreateObject(kind), &guard) {
            ActionAvailability::Enabled => None,
            ActionAvailability::Disabled { reason } => Some(reason),
        }
    }

    /// Renew on the Certificates header: renews the cursor Certificate through the same gate as the
    /// key, so the reason it is off is the one the key would give.
    fn render_renew(&self, kind: ResourceKind, cx: &Context<Self>) -> Option<AnyElement> {
        let button = Button::new("renew-certificate")
            .icon(Icon::new(IconName::RefreshCw))
            .label("Renew")
            .small()
            .outline();
        let button = match self.renew_header_state(kind, cx) {
            Ok(()) => button
                .tooltip("Request a new certificate for the selected Certificate now")
                .on_click(cx.listener(|shell, _, window, cx| {
                    shell.run_row_key(RowAction::RenewCertificate, window, cx);
                })),
            Err(reason) => button.disabled(true).tooltip(reason),
        };
        Some(button.into_any_element())
    }

    /// Opens the Who can… dialog on the namespace the scope starts in.
    fn render_who_can(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let button = Button::new("who-can")
            .icon(Icon::new(IconName::UserSearch))
            .label("Who can…")
            .small()
            .outline();
        let button = if self.live(cx).is_some() {
            button
                .tooltip("Find the subjects that can do something")
                .on_click(cx.listener(|shell, _, window, cx| {
                    let Some(cluster) = shell.active_cluster() else {
                        return;
                    };
                    let namespace = shell.tool_namespace(cx);
                    shell.open_who_can(&cluster, None, namespace, false, window, cx);
                }))
        } else {
            button.disabled(true).tooltip("Not connected")
        };
        Some(button.into_any_element())
    }

    /// Objects named `system:*` are hidden while it is on, which is the default.
    fn render_hide_system(
        &self,
        toolkit: Option<&ToolkitState>,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let is_on = toolkit?.preset == Some(FilterPreset::HideSystem);
        let next = if is_on {
            None
        } else {
            Some(FilterPreset::HideSystem)
        };
        Some(
            toggle_button("hide-system", "Hide system", is_on)
                .tooltip("Hide objects named system:*")
                .on_click(cx.listener(move |shell, _, _, cx| shell.set_preset(next.clone(), cx)))
                .into_any_element(),
        )
    }

    /// Holds the Events list still so rows stop moving; Resume shows what arrived meanwhile.
    fn render_pause_stream(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let live = self.live(cx)?;
        let flow = live.explorer_flow()?;
        let is_paused = matches!(flow, FlowState::Paused { .. });
        let label = if is_paused { "Resume" } else { "Pause stream" };
        let button = toggle_button("pause-stream", label, is_paused);
        // Only a loaded list can be held.
        let can_pause = live
            .kind_list(ResourceKind::Events)
            .is_some_and(|explorer| explorer.list.ready_count().is_some());
        let button = if can_pause {
            button
                .tooltip("Hold the list still; new events wait")
                .on_click(cx.listener(|shell, _, _, cx| shell.toggle_explorer_paused(cx)))
        } else {
            button.disabled(true).tooltip("Nothing to pause yet")
        };
        Some(button.into_any_element())
    }

    /// The Events screen's server-side filter toggle.
    fn render_warnings_only(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let session = self.session()?;
        let is_on = session.read(cx).event_filter() == EventFilter::WarningsOnly;
        // The check says "on" without relying on the fill colour alone.
        let button = toggle_button("warnings-only", "Warnings only", is_on)
            .when(is_on, |button| button.icon(IconName::Check))
            .tooltip("Show only Warning events")
            .on_click(cx.listener(|shell, _, _, cx| shell.toggle_warnings_only(cx)));
        Some(button.into_any_element())
    }

    fn render_interruption_banner(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let live = self.live(cx)?;
        let message = match self.screen {
            Screen::Overview => live
                .pods
                .interruption()
                .or_else(|| live.nodes.interruption()),
            Screen::Pods => live.pods.interruption(),
            Screen::Nodes => live.nodes.interruption(),
            // The problem is in the lists the issues come from.
            Screen::Issues => live
                .pods
                .interruption()
                .or_else(|| live.nodes.interruption()),
            // The graph is drawn from the pods and the feeds of the namespace.
            Screen::Topology => live.pods.interruption(),
            // A local list: no cluster list can interrupt it.
            Screen::PortForwarding => return None,
            Screen::Kind(kind) => live.kind_list(kind)?.list.interruption(),
        }?;
        Some(
            div()
                .flex_shrink_0()
                .px_4()
                .py_1()
                .text_sm()
                .text_color(cx.theme().warning)
                .child(format!("Live updates interrupted: {message}. Retrying…"))
                .into_any_element(),
        )
    }

    /// The first matching state of the priority list in the shell-layout spec.
    #[cfg_attr(
        feature = "hotpath-profiling",
        hotpath::measure(impl_type = "AppShell")
    )]
    fn render_body(&self, cx: &Context<Self>) -> AnyElement {
        // The forwards are a local list: they show with no cluster, and while the kubeconfig loads.
        if self.screen == Screen::PortForwarding {
            return self.render_port_forwards(cx);
        }
        match self.kubeconfig_state(cx) {
            KubeconfigState::Loading => return busy_view("Loading kubeconfig…", cx),
            KubeconfigState::Empty(detail) => return no_clusters_view(&detail, cx),
            KubeconfigState::Failed(message) => {
                return v_flex()
                    .child(error_view(
                        "Cannot load the kubeconfig",
                        &message,
                        None,
                        None,
                        None,
                        cx,
                    ))
                    .child(h_flex().px_4().child(open_clusters_settings_button()))
                    .into_any_element();
            }
            KubeconfigState::Loaded => {}
        }
        let Some(session) = self.session() else {
            // Between the release of the old session and the deferred connect of the new one.
            if let Some(label) = self.active_label(cx) {
                return busy_view(&format!("Connecting to {label}…"), cx);
            }
            if self.needs_pick {
                return pick_cluster_view(cx);
            }
            let message = self
                .context_error
                .as_deref()
                .unwrap_or("No context selected");
            return error_view(
                "Cannot open a context",
                message,
                Some("Pick a context from the cluster menu."),
                None,
                self.back_action(cx),
                cx,
            );
        };
        let session = session.read(cx);
        let label = self
            .active_label(cx)
            .unwrap_or_else(|| session.context().to_owned());
        match session.phase() {
            SessionPhase::Connecting { .. } => busy_view(&format!("Connecting to {label}…"), cx),
            SessionPhase::Failed { message } => error_view(
                &format!("Cannot connect to {label}"),
                message,
                None,
                Some(Rc::new(cx.listener(|shell, _, _, cx| shell.retry(cx)))),
                self.back_action(cx),
                cx,
            ),
            SessionPhase::Live(live) => self.render_list(live, cx),
        }
    }

    /// "Back to {previous}", offered only while the previous cluster still resolves.
    fn back_action(&self, cx: &Context<Self>) -> Option<(String, ClickHandler)> {
        let previous = self.previous_label(cx)?;
        let back: ClickHandler = Rc::new(cx.listener(|shell, _, _, cx| shell.back_to_previous(cx)));
        Some((format!("Back to {previous}"), back))
    }

    fn render_list(&self, live: &LiveCluster, cx: &Context<Self>) -> AnyElement {
        let (title, failure) = match self.screen {
            // A list that failed shows inside its panels.
            Screen::Overview => ("Overview".to_owned(), None),
            Screen::Pods => ("Pods".to_owned(), live.pods.failure()),
            Screen::Nodes => ("Nodes".to_owned(), live.nodes.failure()),
            // A list that failed is a gap in the coverage, not a failure of this screen.
            Screen::Issues => ("Issues".to_owned(), None),
            // The graph needs the pods; a feed that failed is a gap the coverage note names.
            Screen::Topology => ("Pods".to_owned(), live.pods.failure()),
            Screen::PortForwarding => ("Port Forwarding".to_owned(), None),
            Screen::Kind(kind) => (
                kind.label().to_owned(),
                live.kind_list(kind)
                    .and_then(|explorer| explorer.list.failure()),
            ),
        };
        if let Some(message) = failure {
            return error_view(
                &format!("{title} are unavailable"),
                message,
                Some("Retrying automatically."),
                None,
                None,
                cx,
            );
        }
        match self.screen {
            Screen::Overview => match (self.session(), self.open_row_context(cx)) {
                (Some(session), Some(row)) => overview_body(
                    &OverviewData {
                        live,
                        board: session.read(cx).issues(),
                        window: self.overview.window,
                        dock: &self.dock.downgrade(),
                        row: &row,
                    },
                    cx,
                ),
                _ => div().into_any_element(),
            },
            Screen::Pods => DataTable::new(&self.pod_table)
                .with_size(row_size(cx))
                .bordered(false)
                .into_any_element(),
            Screen::Nodes => DataTable::new(&self.node_table)
                .with_size(row_size(cx))
                .bordered(false)
                .into_any_element(),
            Screen::Issues => self.render_issues(cx),
            Screen::Topology => self.topology.clone().into_any_element(),
            Screen::PortForwarding => self.render_port_forwards(cx),
            Screen::Kind(_) => DataTable::new(&self.kind_table)
                .with_size(row_size(cx))
                .bordered(false)
                .into_any_element(),
        }
    }

    /// The Issues table, or what stands in for it: a spinner until pods and nodes have loaded, and
    /// a calm message when nothing was found.
    fn render_issues(&self, cx: &Context<Self>) -> AnyElement {
        let Some(summary) = self.issue_summary(cx) else {
            return busy_view("Checking the cluster…", cx);
        };
        if summary.total > 0 {
            return DataTable::new(&self.issue_table)
                .with_size(row_size(cx))
                .bordered(false)
                .into_any_element();
        }
        let note = self
            .session()
            .and_then(|session| session.read(cx).issues().coverage().note());
        let text = if summary.is_partial {
            "No issues found in what k8sBoard watches."
        } else {
            "No issues found."
        };
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_1()
            .text_sm()
            .text_color(cx.theme().muted_foreground)
            .child(text)
            .children(note)
            .into_any_element()
    }

    /// An overlay on the workspace only, so it never covers the title bar, the sidebar, or
    /// the status bar. `None` when nothing is selected or the subject is not in the list.
    #[cfg_attr(
        feature = "hotpath-profiling",
        hotpath::measure(impl_type = "AppShell")
    )]
    fn render_drawer(&self, cx: &Context<Self>) -> Option<AnyElement> {
        if self.screen == Screen::PortForwarding {
            return self.render_port_forward_drawer(cx);
        }
        let object = self.drawer_subject()?;
        let key = &object.key;
        // The drawer reads the cluster of its subject, never the primary.
        let session = self.session_of(&object.cluster)?;
        let live = session.read(cx).live()?;
        let row = self.row_context_of(&object.cluster, cx)?;
        // The Forward buttons read the running forwards and the gate of the subject's own cluster.
        let gate = self.forward_start_gate(&object.cluster, cx)?;
        let forward = PortButtons {
            forwards: self.port_forwards.read(cx),
            cluster: &object.cluster,
            gate,
        };
        let navigation = self.drawer_navigation(cx);
        match key {
            ResourceKey::Pod { .. } => {
                let pod = live.pods.items().iter().find(|pod| key.is_pod(pod))?;
                Some(pod_drawer(
                    pod,
                    &self.drawer,
                    session,
                    &row,
                    &self.dock.downgrade(),
                    DrawerChrome {
                        forward: &forward,
                        navigation,
                    },
                    cx,
                ))
            }
            ResourceKey::Node { .. } => {
                let node = live.nodes.items().iter().find(|node| key.is_node(node))?;
                Some(node_drawer(
                    node,
                    &self.drawer,
                    session,
                    &row,
                    navigation,
                    cx,
                ))
            }
            ResourceKey::Kind { kind, .. } => {
                // Over Topology the row comes from its feeds, not the explorer.
                let live_row = live.row_of(key)?;
                Some(kind_drawer(
                    *kind,
                    live_row,
                    &self.drawer,
                    live,
                    DrawerChrome {
                        forward: &forward,
                        navigation,
                    },
                    &row,
                    cx,
                ))
            }
        }
    }
}

impl AppShell {
    /// Whether the open cluster is live: the filter bar and the selection bar need rows.
    fn any_live(&self, cx: &App) -> bool {
        self.live(cx).is_some()
    }

    /// What a row menu keeps of the open cluster.
    fn open_row_context(&self, cx: &App) -> Option<RowContext> {
        self.row_context_of(&self.active_cluster()?, cx)
    }
}

/// `38 of 1,284 match`.
fn match_count_label(shown: usize, total: usize) -> String {
    format!("{} of {} match", group_digits(shown), group_digits(total))
}

/// The header count of a filtered list. A list that only a default preset trims says what was hidden
/// (`95 total, 62 hidden (system:*)`), because a plain `33 of 95 match` hides why someone's row is gone.
fn filtered_count_label(state: &ToolkitState) -> String {
    let only_preset = state.text.trim().is_empty() && state.chips.is_empty();
    let reason = match state.preset {
        Some(FilterPreset::HideSystem) => "system:*",
        Some(FilterPreset::HideInactive) => "inactive",
        Some(FilterPreset::Changes) => "not changes",
        Some(FilterPreset::Nodes(_)) | None => "",
    };
    if only_preset && !reason.is_empty() {
        return format!(
            "{} total, {} hidden ({reason})",
            group_digits(state.total),
            group_digits(state.total.saturating_sub(state.shown)),
        );
    }
    match_count_label(state.shown, state.total)
}

/// `4 nodes`, then up to three roles with their counts, such as `4 nodes · 1 control-plane`.
fn nodes_count_text(count: usize, roles: &[(String, usize)]) -> String {
    let mut text = count_label(count, "node", "nodes");
    for (role, number) in roles.iter().take(3) {
        text.push_str(&format!(" · {number} {role}"));
    }
    text
}

/// The header count of a cluster-scoped list, which ignores the namespace scope: with a scope
/// set, the extra `cluster-wide` keeps the list from reading as filtered by it.
fn cluster_wide_text(count: String, scope: &NamespaceScope) -> String {
    match scope {
        NamespaceScope::All => count,
        NamespaceScope::Named(_) | NamespaceScope::Several(_) => format!("{count} · cluster-wide"),
    }
}

/// The count text of a paused list, with a note when new rows are waiting.
fn paused_text(count: &str, has_held: bool) -> String {
    if has_held {
        format!("{count} · paused · new events waiting")
    } else {
        format!("{count} · paused")
    }
}

/// A header toggle: primary when on, outline when off, like Warnings only.
pub(crate) fn toggle_button(id: &'static str, label: &'static str, is_on: bool) -> Button {
    Button::new(id)
        .label(label)
        .small()
        .map(|button| {
            if is_on {
                button.primary()
            } else {
                button.outline()
            }
        })
        .selected(is_on)
        .toggled(is_on)
}

fn count_label(count: usize, singular: &str, plural: &str) -> String {
    if count == 1 {
        format!("1 {singular}")
    } else {
        format!("{} {plural}", group_digits(count))
    }
}

/// Shown while nothing may start on its own and the user has not picked a cluster yet.
fn pick_cluster_view(cx: &App) -> AnyElement {
    v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .text_sm()
        .text_color(cx.theme().muted_foreground)
        .child(NO_CLUSTER_SELECTED)
        .into_any_element()
}

pub(crate) const NO_CLUSTER_SELECTED: &str = "No cluster selected. Pick one in the switcher.";

fn open_clusters_settings_button() -> Button {
    Button::new("open-clusters-settings")
        .label("Open Settings › Clusters")
        .primary()
        .small()
        .on_click(|_, _, cx| manage_clusters(cx))
}

/// First run: no kubeconfig file exists yet. `detail` is the reason, kept as a muted line.
/// Import and Paste live on the Clusters page, which owns their dialogs; the buttons open that
/// page and start the flow there instead of duplicating it here.
fn no_clusters_view(detail: &str, cx: &App) -> AnyElement {
    v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .gap_3()
        .child(div().text_lg().font_semibold().child("No clusters yet"))
        .child(
            div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child("Add a kubeconfig on the Clusters page to start."),
        )
        .child(
            h_flex()
                .gap_2()
                .child(open_clusters_settings_button())
                .child(
                    Button::new("first-run-import")
                        .label("Import kubeconfig…")
                        .outline()
                        .small()
                        .on_click(|_, _, cx| add_cluster(ClusterAddition::ImportFile, cx)),
                )
                .child(paste_kubeconfig_button(cx)),
        )
        .child(
            div()
                .max_w(px(640.))
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(detail.to_owned()),
        )
        .into_any_element()
}

/// `Paste YAML…`, off with the reason the Clusters page gives when pasting cannot work.
fn paste_kubeconfig_button(cx: &App) -> Button {
    let button = Button::new("first-run-paste")
        .label("Paste YAML…")
        .outline()
        .small();
    match ClustersPage::paste_blocked_reason(cx) {
        Some(reason) => button.disabled(true).tooltip(reason),
        None => button.on_click(|_, _, cx| add_cluster(ClusterAddition::PasteYaml, cx)),
    }
}

fn busy_view(text: &str, cx: &App) -> AnyElement {
    v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .gap_3()
        .child(Spinner::new())
        .child(
            div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(text.to_owned()),
        )
        .into_any_element()
}

/// An error alert, with optional follow-up help, a Retry button, and a second button (Back to …).
fn error_view(
    title: &str,
    message: &str,
    hint: Option<&'static str>,
    retry: Option<ClickHandler>,
    back: Option<(String, ClickHandler)>,
    cx: &App,
) -> AnyElement {
    v_flex()
        .p_4()
        .gap_3()
        .child(Alert::error("workspace-error", message.to_owned()).title(title.to_owned()))
        .children(hint.map(|text| {
            div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(text)
        }))
        .when_some(retry, |this, on_retry| {
            this.child(
                Button::new("retry")
                    .label("Retry")
                    .small()
                    .on_click(move |event, window, cx| on_retry(event, window, cx)),
            )
        })
        .when_some(back, |this, (label, on_back)| {
            this.child(
                Button::new("back")
                    .label(label)
                    .small()
                    .on_click(move |event, window, cx| on_back(event, window, cx)),
            )
        })
        .into_any_element()
}

/// The reach of the double-click area around the 1 px divider, like the kit's own band.
const HANDLE_HIT_PADDING: Pixels = px(4.);
/// The dashes of the 60 % line.
const DASH_LENGTH: Pixels = px(6.);
const DASH_GAP: Pixels = px(4.);

/// The kit's divider, plus the double-click reset and, while dragging, the dashed 60 % line.
fn dock_handle_appearance(split: Entity<ResizableState>) -> ResizeHandleRenderer {
    let kit_line = resize_handle_appearance();
    Rc::new(move |handle, window, cx| {
        let line = kit_line(handle, window, cx)?;
        let reset_split = split.clone();
        // Does not occlude, so the press still reaches the kit handle and starts the drag.
        let hit_area = div()
            .id("dock-handle-reset")
            .debug_selector(|| "dock-handle-reset".into())
            .absolute()
            .left_0()
            .w_full()
            .top(-HANDLE_HIT_PADDING)
            .h(HANDLE_HIT_PADDING * 2. + px(1.))
            .on_mouse_down(MouseButton::Left, move |event, window, cx| {
                if event.click_count != 2 {
                    return;
                }
                reset_split.update(cx, |state, cx| {
                    state.resize_panel(1, DEFAULT_DOCK_HEIGHT, window, cx);
                });
            });
        let max_line = (handle.state() == ResizeHandleState::Dragging)
            .then(|| max_height_line(&split, cx))
            .flatten();
        Some(
            div()
                .relative()
                .flex_none()
                .w_full()
                .h(px(1.))
                .child(line)
                .child(hit_area)
                .children(max_line)
                .into_any_element(),
        )
    })
}

/// The dashed line at 60 % of the workspace and its label, above the handle. Deferred so it paints
/// over the upper panel; it exists only during a drag, when no popover is open.
fn max_height_line(split: &Entity<ResizableState>, cx: &App) -> Option<AnyElement> {
    let state = split.read(cx);
    let dock = *state.sizes().get(1)?;
    let offset = max_line_offset(state.container_size(), dock)?;
    let theme = cx.theme();
    Some(
        deferred(
            div()
                .absolute()
                .left_0()
                .w_full()
                .top(-offset)
                .h(px(1.))
                .child(dashed_rule(theme.muted_foreground))
                .child(
                    div()
                        .absolute()
                        .right_2()
                        .top(px(-18.))
                        .px_1()
                        .rounded_sm()
                        .bg(theme.background)
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child("max height · 60%"),
                ),
        )
        .into_any_element(),
    )
}

/// A one-pixel dashed rule across its box, painted dash by dash: a dashed border on a one-pixel
/// box draws nothing (the dash shader needs a box with room around the border).
fn dashed_rule(color: Hsla) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, (), window, _| {
            let mut x = bounds.left();
            while x < bounds.right() {
                let length = DASH_LENGTH.min(bounds.right() - x);
                let dash = Bounds::new(point(x, bounds.top()), size(length, bounds.size.height));
                window.paint_quad(fill(dash, color));
                x += DASH_LENGTH + DASH_GAP;
            }
        },
    )
    .absolute()
    .size_full()
}

/// The kind a screen's `New` button creates, and its label (spec 0042, W7): `New namespace` on
/// Namespaces, `New` on the four screens of a namespaced kind.
fn new_button_of(screen: Screen) -> Option<(ObjectKind, &'static str)> {
    match screen {
        Screen::Kind(ResourceKind::Namespaces) => Some((ObjectKind::Namespace, "New namespace")),
        Screen::Kind(
            kind @ (ResourceKind::ConfigMaps
            | ResourceKind::ResourceQuotas
            | ResourceKind::PodDisruptionBudgets
            | ResourceKind::RoleBindings),
        ) => kind.builtin_object().map(|object| (object, "New")),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_buttons_on_the_five_screens() {
        let button = |kind| new_button_of(Screen::Kind(kind));
        assert_eq!(
            button(ResourceKind::Namespaces),
            Some((ObjectKind::Namespace, "New namespace"))
        );
        for (kind, object) in [
            (ResourceKind::ConfigMaps, ObjectKind::ConfigMap),
            (ResourceKind::ResourceQuotas, ObjectKind::ResourceQuota),
            (
                ResourceKind::PodDisruptionBudgets,
                ObjectKind::PodDisruptionBudget,
            ),
            (ResourceKind::RoleBindings, ObjectKind::RoleBinding),
        ] {
            assert_eq!(button(kind), Some((object, "New")));
        }
        // Every other screen has none; Secrets keeps `Reveal all` only.
        let others = ResourceKind::ALL
            .into_iter()
            .filter(|kind| button(*kind).is_none())
            .count();
        assert_eq!(others, ResourceKind::ALL.len() - 5);
        assert_eq!(button(ResourceKind::Secrets), None);
        assert_eq!(new_button_of(Screen::Pods), None);
    }

    #[test]
    fn count_label_pluralizes() {
        assert_eq!(count_label(1, "pod", "pods"), "1 pod");
        assert_eq!(count_label(0, "pod", "pods"), "0 pods");
        assert_eq!(count_label(104, "node", "nodes"), "104 nodes");
        assert_eq!(count_label(1, "ingress", "ingresses"), "1 ingress");
        assert_eq!(count_label(2, "ingress", "ingresses"), "2 ingresses");
    }

    #[test]
    fn cluster_scoped_headers_say_cluster_wide_under_a_namespace_scope() {
        let named = NamespaceScope::Named("postgres".to_owned());
        let several = NamespaceScope::Several(vec!["a".to_owned(), "b".to_owned()]);
        assert_eq!(
            cluster_wide_text("4 nodes".to_owned(), &NamespaceScope::All),
            "4 nodes"
        );
        assert_eq!(
            cluster_wide_text("4 nodes".to_owned(), &named),
            "4 nodes · cluster-wide"
        );
        assert_eq!(
            cluster_wide_text("20 namespaces".to_owned(), &several),
            "20 namespaces · cluster-wide"
        );
    }

    #[test]
    fn match_count_label_groups_both_numbers() {
        assert_eq!(match_count_label(38, 1_284), "38 of 1,284 match");
        assert_eq!(match_count_label(0, 12), "0 of 12 match");
    }

    #[test]
    fn a_default_preset_names_what_it_hides() {
        let state = |text: &str, preset| ToolkitState {
            screen: Screen::Kind(ResourceKind::ClusterRoles),
            text: text.to_owned(),
            chips: Vec::new(),
            hidden: Default::default(),
            columns: Vec::new(),
            shown: 33,
            total: 95,
            is_filtering: true,
            checked: 0,
            preset,
            node_counts: None,
            scope: None,
        };
        assert_eq!(
            filtered_count_label(&state("", Some(FilterPreset::HideSystem))),
            "95 total, 62 hidden (system:*)"
        );
        assert_eq!(
            filtered_count_label(&state("", Some(FilterPreset::HideInactive))),
            "95 total, 62 hidden (inactive)"
        );
        // Typed text shares the blame, so the plain count stays.
        assert_eq!(
            filtered_count_label(&state("kube", Some(FilterPreset::HideSystem))),
            "33 of 95 match"
        );
        assert_eq!(filtered_count_label(&state("kube", None)), "33 of 95 match");
    }

    #[gpui_kit::test]
    fn density_change_resizes_the_table_rows(cx: &mut gpui_kit::TestAppContext) {
        use crate::settings::{RowDensity, Settings};
        use crate::settings_store::{LoadedSettings, WriteMode};
        cx.update(|cx| {
            AppSettings::install(
                LoadedSettings {
                    settings: Settings::default(),
                    writes: WriteMode::Disabled,
                    notice: None,
                },
                cx,
            );
            assert_eq!(row_size(cx), Size::Size(px(28.)));
            AppSettings::update(cx, |settings| {
                settings.appearance.density = RowDensity::Comfortable;
            });
            assert_eq!(row_size(cx), Size::Size(px(36.)));
        });
    }

    #[test]
    fn count_label_groups_digits() {
        assert_eq!(count_label(2_000, "event", "events"), "2,000 events");
    }
}
