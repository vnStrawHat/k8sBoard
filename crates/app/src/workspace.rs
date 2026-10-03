//! The workspace region of the window: header, banner, table or state view, and the drawer.

use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::component::alert::Alert;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::resizable::{resizable_panel, v_resizable};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::table::DataTable;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, Selectable as _, Sizable as _, StyledExt as _,
    h_flex, v_flex,
};
use gpui_kit::{
    AnyElement, App, Context, InteractiveElement as _, IntoElement, ParentElement as _,
    Styled as _, Window, div, prelude::FluentBuilder as _, px,
};

use cluster::{AccessCheck, EVENT_LIMIT, EventFilter};

use super::{AppShell, KubeconfigState, Screen};
use crate::cluster_registry::ClusterRef;
use crate::cluster_rows::RowContext;
use crate::cluster_session::{AccessState, FlowState, LiveCluster, SessionPhase};
use crate::dock::{DEFAULT_DOCK_HEIGHT, DockMode, MIN_DOCK_HEIGHT, dock_max_height};
use crate::drawer::ClickHandler;
use crate::file_export::ExportState;
use crate::filter_bar::{ToolkitState, filter_bar};
use crate::issue_board::IssueSummary;
use crate::issue_table::coverage_status;
use crate::kind_drawer::kind_drawer;
use crate::navigation::{SIDEBAR_WIDTH, sum_known};
use crate::node_drawer::node_drawer;
use crate::node_summary::role_counts;
use crate::overview::{
    OverviewData, change_window_button, headline_text, overview_body, stats_line,
};
use crate::pod_drawer::pod_drawer;
use crate::resource_kind::ResourceKind;
use crate::row_selection::selection_bar;
use crate::table_filter::FilterPreset;
use crate::table_selection::ResourceKey;
use crate::usage_format::group_digits;

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
    pub(super) fn render_workspace(&self, cx: &Context<Self>) -> impl IntoElement {
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
                region.child(
                    v_resizable("workspace-split")
                        .with_state(&self.dock_split)
                        .child(resizable_panel().child(self.render_upper(cx)))
                        .child(
                            resizable_panel()
                                .size(DEFAULT_DOCK_HEIGHT)
                                .flex_none()
                                .size_range(MIN_DOCK_HEIGHT..max_height)
                                .child(self.dock.clone()),
                        ),
                )
            }
        }
    }

    /// Header, banner, body, and the drawer overlay. The drawer covers this region only, so
    /// it never covers the dock.
    fn render_upper(&self, cx: &Context<Self>) -> impl IntoElement {
        // One read of the table view for everything drawn from it in this frame.
        let toolkit = self.toolkit_state(cx);
        let toolkit = toolkit.as_ref();
        v_flex()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .relative()
            .child(self.render_header(toolkit, cx))
            .children(self.render_slot_notices(cx))
            .children(self.render_filter_bar(toolkit, cx))
            .children(self.render_overview_stats(cx))
            .children(self.render_overview_export_error())
            .children(self.render_interruption_banner(cx))
            .child(div().flex_1().min_h_0().child(self.render_body(cx)))
            .children(self.render_selection_bar(toolkit, cx))
            .children(self.render_value_popover(toolkit))
            .children(self.render_drawer(cx))
    }

    fn render_header(
        &self,
        toolkit: Option<&ToolkitState>,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let live = self.live(cx);
        let multi_count = self.multi_header_count(cx);
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
                multi_count.or_else(|| {
                    live.and_then(|live| {
                        live.pods.ready_count().map(|count| {
                            format!(
                                "{} · {}",
                                count_label(count, "pod", "pods"),
                                live.scope_label()
                            )
                        })
                    })
                }),
            ),
            Screen::Nodes => (
                "Nodes",
                multi_count.or_else(|| {
                    live.and_then(|live| {
                        let count = live.nodes.ready_count()?;
                        Some(nodes_count_text(count, &role_counts(live.nodes.items())))
                    })
                }),
            ),
            Screen::Issues => (
                "Issues",
                self.issue_summary(cx)
                    .map(|summary| count_label(summary.total, "issue", "issues")),
            ),
            Screen::Topology => ("Topology", self.topology.read(cx).header_count()),
            Screen::Kind(kind) => (
                kind.label(),
                multi_count.or_else(|| {
                    live.and_then(|live| {
                        let count = live.kind_list(kind)?.list.ready_count()?;
                        let label = count_label(count, kind.singular(), kind.plural());
                        let text = if kind.is_namespaced() {
                            format!("{label} · {}", live.scope_label())
                        } else {
                            label
                        };
                        // The events store keeps only the newest ones, so say so at the cap.
                        Some(if kind == ResourceKind::Events && count >= EVENT_LIMIT {
                            format!("{text} · newest {}", group_digits(EVENT_LIMIT))
                        } else {
                            text
                        })
                    })
                }),
            ),
        };
        // A filter replaces the total with how many rows match it.
        let count = match (toolkit, count) {
            (Some(state), Some(_)) if state.is_filtering => {
                Some(match_count_label(state.shown, state.total))
            }
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
            .child(div().text_lg().font_semibold().child(title))
            .children(count.map(|count| {
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(count)
            }))
            .children(self.render_header_actions(toolkit, cx))
    }

    /// The bar over the bottom of the table while rows are ticked. It sits left of an open
    /// drawer, and the drawer is drawn after it.
    fn render_selection_bar(
        &self,
        state: Option<&ToolkitState>,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        self.any_live(cx).then_some(())?;
        let state = state?;
        if state.checked == 0 {
            return None;
        }
        let (singular, plural) = match self.screen {
            Screen::Overview | Screen::Topology => return None,
            Screen::Pods => ("pod", "pods"),
            Screen::Nodes => ("node", "nodes"),
            Screen::Issues => ("issue", "issues"),
            Screen::Kind(kind) => (kind.singular(), kind.plural()),
        };
        let text = format!("{} selected", count_label(state.checked, singular, plural));
        let bar = selection_bar(text, self.bulk_buttons(cx), &cx.weak_entity(), cx);
        let right = if self.drawer_subject().is_some() {
            self.drawer.width()
        } else {
            px(0.)
        };
        Some(
            div()
                .absolute()
                .bottom_4()
                .left_0()
                .right(right)
                .flex()
                .justify_center()
                .child(bar)
                .into_any_element(),
        )
    }

    /// The value popover, over the bottom of the table and above the selection bar while rows are
    /// ticked. It sits left of an open drawer, like the bar.
    fn render_value_popover(&self, state: Option<&ToolkitState>) -> Option<AnyElement> {
        let popover = self.value_popover()?.clone();
        let bar_height = if state.is_some_and(|state| state.checked > 0) {
            px(64.)
        } else {
            px(0.)
        };
        let right = if self.drawer_subject().is_some() {
            self.drawer.width()
        } else {
            px(0.)
        };
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

    /// The Topology header, right-aligned: the saved file name, Fit, and Export PNG. Both buttons
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
        let fit = Button::new("topology-fit")
            .ghost()
            .small()
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

    /// The per-screen toggles, right-aligned in the header: Hide inactive on ReplicaSets, and
    /// Warnings only with Pause stream on Events.
    fn render_header_actions(
        &self,
        toolkit: Option<&ToolkitState>,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let buttons: Vec<AnyElement> = match self.screen {
            Screen::Overview => self.overview_header_buttons(cx),
            Screen::Topology => self.topology_header_buttons(cx),
            Screen::Issues => return self.render_issues_status(cx),
            Screen::Kind(ResourceKind::ReplicaSets) => {
                self.render_hide_inactive(toolkit, cx).into_iter().collect()
            }
            Screen::Kind(ResourceKind::NetworkPolicies) => {
                self.render_test_traffic(cx).into_iter().collect()
            }
            Screen::Kind(ResourceKind::ServiceAccounts) => {
                self.render_check_permissions(cx).into_iter().collect()
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
            _ => return None,
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
            .label("Test traffic")
            .small()
            .outline();
        let button = if self.live(cx).is_some() {
            button
                .tooltip("Check whether NetworkPolicies allow a connection")
                .on_click(cx.listener(|shell, _, window, cx| {
                    let Some(cluster) = shell.primary_cluster() else {
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
            .label("Check permissions")
            .small()
            .outline();
        let button = if self.live(cx).is_some() {
            button
                .tooltip("See what You or a service account can do")
                .on_click(cx.listener(|shell, _, window, cx| {
                    // The drawer's cluster when an account is open, else the primary one.
                    let Some(cluster) = shell.context_cluster() else {
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

    /// Opens the Who can… dialog on the namespace the scope starts in.
    fn render_who_can(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let button = Button::new("who-can").label("Who can…").small().outline();
        let button = if self.live(cx).is_some() {
            button
                .tooltip("Find the subjects that can do something")
                .on_click(cx.listener(|shell, _, window, cx| {
                    let Some(cluster) = shell.primary_cluster() else {
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

    /// ReplicaSets scaled to zero are hidden while it is on, which is the default.
    fn render_hide_inactive(
        &self,
        toolkit: Option<&ToolkitState>,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let is_on = toolkit?.preset == Some(FilterPreset::HideInactive);
        let next = if is_on {
            None
        } else {
            Some(FilterPreset::HideInactive)
        };
        Some(
            toggle_button("hide-inactive", "Hide inactive", is_on)
                .tooltip("Hide ReplicaSets scaled to zero")
                .on_click(cx.listener(move |shell, _, _, cx| shell.set_preset(next.clone(), cx)))
                .into_any_element(),
        )
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
        let button = toggle_button("warnings-only", "Warnings only", is_on)
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
    fn render_body(&self, cx: &Context<Self>) -> AnyElement {
        match self.kubeconfig_state(cx) {
            KubeconfigState::Loading => return busy_view("Loading kubeconfig…", cx),
            KubeconfigState::Failed(message) => {
                return error_view("Cannot load the kubeconfig", &message, None, None, None, cx);
            }
            KubeconfigState::Loaded => {}
        }
        if self.view.is_multi() {
            return self.render_multi_body(cx);
        }
        let Some(session) = self.session() else {
            // Between the release of the old session and the deferred connect of the new one.
            if let Some(label) = self.active_label(cx) {
                return busy_view(&format!("Connecting to {label}…"), cx);
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
            Screen::Overview => match (self.session(), self.primary_row_context(cx)) {
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
                .bordered(false)
                .into_any_element(),
            Screen::Nodes => DataTable::new(&self.node_table)
                .bordered(false)
                .into_any_element(),
            Screen::Issues => self.render_issues(cx),
            Screen::Topology => self.topology.clone().into_any_element(),
            Screen::Kind(_) => DataTable::new(&self.kind_table)
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
    fn render_drawer(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let object = self.drawer_subject()?;
        let key = &object.key;
        // The drawer reads the cluster of its subject, never the primary.
        let session = self.slot_session(&object.cluster)?;
        let live = session.read(cx).live()?;
        let row = self.slot_row_context(&object.cluster, cx)?;
        match key {
            ResourceKey::Pod { .. } => {
                let pod = live.pods.items().iter().find(|pod| key.is_pod(pod))?;
                Some(pod_drawer(
                    pod,
                    &self.drawer,
                    session,
                    &row,
                    &self.dock.downgrade(),
                    cx,
                ))
            }
            ResourceKey::Node { .. } => {
                let node = live.nodes.items().iter().find(|node| key.is_node(node))?;
                Some(node_drawer(node, &self.drawer, session, &row, cx))
            }
            ResourceKey::Kind { kind, .. } => {
                // Over Topology the row comes from its feeds, not the explorer.
                let live_row = live.row_of(key)?;
                Some(kind_drawer(
                    *kind,
                    live_row,
                    &self.drawer,
                    live,
                    session,
                    &row,
                    cx,
                ))
            }
        }
    }
}

impl AppShell {
    /// Whether any viewed cluster is live: the filter bar and the selection bar need rows.
    fn any_live(&self, cx: &App) -> bool {
        self.view
            .slots()
            .iter()
            .any(|slot| slot.session.read(cx).live().is_some())
    }

    /// What a row menu keeps of the primary cluster.
    fn primary_row_context(&self, cx: &App) -> Option<RowContext> {
        self.slot_row_context(self.view.primary_cluster()?, cx)
    }

    /// `{n} clusters · {count} {plural}` of the shown list over the viewed clusters that know it;
    /// `None` while one cluster is viewed, on screens without a table, and while none knows.
    pub(super) fn multi_header_count(&self, cx: &App) -> Option<String> {
        if !self.view.is_multi() {
            return None;
        }
        let lives = self
            .view
            .slots()
            .iter()
            .filter_map(|slot| slot.session.read(cx).live());
        let (total, singular, plural) = match self.screen {
            Screen::Pods => (
                sum_known(lives.map(|live| live.pods.ready_count())),
                "pod",
                "pods",
            ),
            Screen::Nodes => (
                sum_known(lives.map(|live| live.nodes.ready_count())),
                "node",
                "nodes",
            ),
            Screen::Kind(kind) => (
                sum_known(lives.map(|live| live.kind_list(kind)?.list.ready_count())),
                kind.singular(),
                kind.plural(),
            ),
            Screen::Overview | Screen::Issues | Screen::Topology => return None,
        };
        Some(clusters_count_text(
            self.view.slots().len(),
            total?,
            singular,
            plural,
        ))
    }

    /// What each viewed cluster tells for the shown screen, as banners and notes over the rows of
    /// the others (the 0027 per-slot states). One cluster viewed: none, its state has the screens
    /// of its own.
    fn render_slot_notices(&self, cx: &Context<Self>) -> Vec<AnyElement> {
        let mut notices: Vec<AnyElement> = self
            .slot_notice_list(cx)
            .into_iter()
            .enumerate()
            .map(|(index, notice)| render_slot_notice(index, notice, cx))
            .collect();
        // Overview, Issues, and Topology draw the primary cluster alone.
        if self.view.is_multi()
            && matches!(
                self.screen,
                Screen::Overview | Screen::Issues | Screen::Topology
            )
            && let Some(primary) = self.view.primary()
        {
            let text = format!("Showing {} only", primary.label);
            notices.push(muted_notice(text, cx));
        }
        notices
    }

    /// The notices of the viewed clusters for the shown screen; none while one cluster is viewed.
    pub(super) fn slot_notice_list(&self, cx: &App) -> Vec<SlotNotice> {
        if !self.view.is_multi() {
            return Vec::new();
        }
        let plural = self.list_plural();
        let statuses: Vec<SlotStatus> = self
            .view
            .slots()
            .iter()
            .map(|slot| {
                let state = match slot.session.read(cx).phase() {
                    SessionPhase::Connecting { .. } => SlotState::Connecting,
                    SessionPhase::Failed { message } => SlotState::Failed(message),
                    SessionPhase::Live(live) => SlotState::Live {
                        is_interrupted: live.has_problem(),
                        list_failure: self.list_problem(live),
                    },
                };
                SlotStatus {
                    cluster: &slot.cluster,
                    label: &slot.label,
                    state,
                }
            })
            .collect();
        slot_notices(&statuses, plural)
    }

    /// The plural of what the shown screen lists; `None` on the screens that list no kind.
    fn list_plural(&self) -> Option<&'static str> {
        match self.screen {
            Screen::Pods => Some("pods"),
            Screen::Nodes => Some("nodes"),
            Screen::Kind(kind) => Some(kind.plural()),
            Screen::Overview | Screen::Issues | Screen::Topology => None,
        }
    }

    /// Why the list of the shown screen is not there in `live`, when its watch failed.
    fn list_failure<'a>(&self, live: &'a LiveCluster) -> Option<&'a str> {
        match self.screen {
            Screen::Pods => live.pods.failure(),
            Screen::Nodes => live.nodes.failure(),
            Screen::Kind(kind) => live.kind_list(kind)?.list.failure(),
            Screen::Overview | Screen::Issues | Screen::Topology => None,
        }
    }

    /// The failed list of the shown screen in `live`, and whether the access review says the list
    /// is not permitted (else it is some other failure, such as a CRD the cluster does not have).
    fn list_problem<'a>(&self, live: &'a LiveCluster) -> Option<ListFailure<'a>> {
        let message = self.list_failure(live)?;
        let check = match self.screen {
            Screen::Pods => Some(AccessCheck::ListPods),
            Screen::Nodes => Some(AccessCheck::ListNodes),
            Screen::Kind(kind) => kind.access_check(),
            Screen::Overview | Screen::Issues | Screen::Topology => None,
        };
        let is_denied = match (&live.access, check) {
            (AccessState::Known(report), Some(check)) => !report.is_allowed(check),
            _ => false,
        };
        Some(ListFailure { message, is_denied })
    }

    /// The workspace while several clusters are viewed: the rows of the clusters that answer, with
    /// the others as banners above (`render_slot_notices`). The error view only when every cluster
    /// failed, with Retry all.
    fn render_multi_body(&self, cx: &Context<Self>) -> AnyElement {
        let slots = self.view.slots();
        let is_failed = |slot: &&crate::cluster_view::ViewSlot| {
            matches!(slot.session.read(cx).phase(), SessionPhase::Failed { .. })
        };
        let primary_label = self
            .view
            .primary()
            .map_or_else(String::new, |slot| slot.label.clone());
        if slots.iter().all(|slot| is_failed(&slot)) {
            let message = self
                .view
                .primary()
                .and_then(|slot| match slot.session.read(cx).phase() {
                    SessionPhase::Failed { message } => Some(message.clone()),
                    _ => None,
                })
                .unwrap_or_default();
            let retry_all: ClickHandler = Rc::new(cx.listener(|shell, _, _, cx| shell.retry(cx)));
            return v_flex()
                .child(error_view(
                    &format!("Cannot connect to {primary_label}"),
                    &message,
                    Some("Every viewed cluster failed to connect."),
                    None,
                    None,
                    cx,
                ))
                .child(
                    div().px_4().child(
                        Button::new("retry-all")
                            .label("Retry all")
                            .small()
                            .on_click(move |event, window, cx| retry_all(event, window, cx)),
                    ),
                )
                .into_any_element();
        }
        if !self.any_live(cx) {
            let labels: Vec<&str> = slots.iter().map(|slot| slot.label.as_str()).collect();
            return busy_view(&format!("Connecting to {}…", labels.join(", ")), cx);
        }
        // Overview, Issues, and Topology draw the primary cluster alone.
        if matches!(
            self.screen,
            Screen::Overview | Screen::Issues | Screen::Topology
        ) {
            return match self
                .view
                .primary()
                .map(|slot| (&slot.cluster, slot.session.read(cx).phase()))
            {
                Some((_, SessionPhase::Live(live))) => self.render_list(live, cx),
                Some((cluster, SessionPhase::Failed { message })) => {
                    let cluster = cluster.clone();
                    let retry: ClickHandler = Rc::new(cx.listener(move |shell, _, _, cx| {
                        shell.retry_cluster(&cluster, cx);
                    }));
                    error_view(
                        &format!("Cannot connect to {primary_label}"),
                        message,
                        Some("This screen draws the primary cluster only."),
                        Some(retry),
                        None,
                        cx,
                    )
                }
                _ => busy_view(&format!("Connecting to {primary_label}…"), cx),
            };
        }
        // A cluster whose list failed is a note of its own; the error view needs every live
        // cluster to have failed.
        let mut failures = slots
            .iter()
            .filter_map(|slot| slot.session.read(cx).live())
            .map(|live| self.list_failure(live))
            .peekable();
        if failures.peek().is_some() && failures.all(|failure| failure.is_some()) {
            let message = slots
                .iter()
                .filter_map(|slot| slot.session.read(cx).live())
                .find_map(|live| self.list_failure(live))
                .unwrap_or_default();
            let title = match self.list_plural() {
                Some(plural) => format!("{} are unavailable", capitalized(plural)),
                None => "The list is unavailable".to_owned(),
            };
            return error_view(
                &title,
                message,
                Some("Retrying automatically."),
                None,
                None,
                cx,
            );
        }
        match self.screen {
            Screen::Pods => DataTable::new(&self.pod_table)
                .bordered(false)
                .into_any_element(),
            Screen::Nodes => DataTable::new(&self.node_table)
                .bordered(false)
                .into_any_element(),
            _ => DataTable::new(&self.kind_table)
                .bordered(false)
                .into_any_element(),
        }
    }
}

/// How one viewed cluster stands, for the banners.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum SlotState<'a> {
    Connecting,
    /// The connect failed with this message.
    Failed(&'a str),
    Live {
        /// A watch of the session is interrupted and retrying.
        is_interrupted: bool,
        /// Why the list of the shown screen is not there, when its watch failed.
        list_failure: Option<ListFailure<'a>>,
    },
}

/// A list that failed, and whether the access review denies it.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct ListFailure<'a> {
    pub(super) message: &'a str,
    pub(super) is_denied: bool,
}

pub(super) struct SlotStatus<'a> {
    pub(super) cluster: &'a ClusterRef,
    pub(super) label: &'a str,
    pub(super) state: SlotState<'a>,
}

/// One banner or note over the rows of a multi view.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum SlotNotice {
    /// The connect failed: Retry and Remove from view.
    Failed {
        cluster: ClusterRef,
        text: String,
    },
    /// A watch is interrupted; it recovers on its own, so there are no buttons.
    Interrupted {
        text: String,
    },
    Connecting {
        text: String,
    },
    /// The list of the shown screen is not permitted in this cluster.
    Denied {
        text: String,
    },
    /// The list of the shown screen failed for another reason, such as a CRD missing here.
    ListFailed {
        text: String,
    },
}

/// The notices of the viewed clusters, in slot order. `plural` is what the shown screen lists.
pub(super) fn slot_notices(slots: &[SlotStatus], plural: Option<&str>) -> Vec<SlotNotice> {
    let mut notices = Vec::new();
    for slot in slots {
        let label = slot.label;
        match &slot.state {
            SlotState::Connecting => notices.push(SlotNotice::Connecting {
                text: format!("Connecting to {label}…"),
            }),
            SlotState::Failed(message) => notices.push(SlotNotice::Failed {
                cluster: slot.cluster.clone(),
                text: format!("Cannot connect to {label}: {message}"),
            }),
            SlotState::Live {
                is_interrupted,
                list_failure,
            } => {
                if *is_interrupted {
                    notices.push(SlotNotice::Interrupted {
                        text: format!("Live updates interrupted in {label}"),
                    });
                }
                if let (Some(failure), Some(plural)) = (list_failure, plural) {
                    notices.push(if failure.is_denied {
                        SlotNotice::Denied {
                            text: format!("Not permitted in {label}: list {plural}"),
                        }
                    } else {
                        SlotNotice::ListFailed {
                            text: format!("Cannot list {plural} in {label}: {}", failure.message),
                        }
                    });
                }
            }
        }
    }
    notices
}

fn render_slot_notice(index: usize, notice: SlotNotice, cx: &Context<AppShell>) -> AnyElement {
    let theme = cx.theme();
    let row = h_flex()
        .flex_shrink_0()
        .items_center()
        .gap_2()
        .px_4()
        .py_1()
        .text_sm();
    match notice {
        SlotNotice::Failed { cluster, text } => {
            let (retry, remove) = (cluster.clone(), cluster);
            row.id(("slot-failed", index))
                .text_color(theme.danger)
                .child(div().flex_1().min_w_0().truncate().child(text))
                .child(
                    Button::new(("slot-retry", index))
                        .ghost()
                        .small()
                        .label("Retry")
                        .on_click(cx.listener(move |shell, _, _, cx| {
                            shell.retry_cluster(&retry, cx);
                        })),
                )
                .child(
                    Button::new(("slot-remove", index))
                        .ghost()
                        .small()
                        .label("Remove from view")
                        .on_click(cx.listener(move |shell, _, _, cx| {
                            shell.remove_from_view(&remove, cx);
                        })),
                )
                .into_any_element()
        }
        SlotNotice::Interrupted { text } => row
            .id(("slot-interrupted", index))
            .text_color(theme.warning)
            .child(text)
            .into_any_element(),
        SlotNotice::Connecting { text }
        | SlotNotice::Denied { text }
        | SlotNotice::ListFailed { text } => muted_notice(text, cx),
    }
}

/// A muted line under the header.
fn muted_notice(text: String, cx: &App) -> AnyElement {
    div()
        .flex_shrink_0()
        .px_4()
        .py_1()
        .text_sm()
        .text_color(cx.theme().muted_foreground)
        .child(text)
        .into_any_element()
}

/// `Pods` for `pods`; the plurals of the kinds are lowercase words.
fn capitalized(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// `2 clusters · 2,431 pods`.
fn clusters_count_text(clusters: usize, count: usize, singular: &str, plural: &str) -> String {
    format!(
        "{clusters} clusters · {}",
        count_label(count, singular, plural)
    )
}

/// `38 of 1,284 match`.
fn match_count_label(shown: usize, total: usize) -> String {
    format!("{} of {} match", group_digits(shown), group_digits(total))
}

/// `4 nodes`, then up to three roles with their counts, such as `4 nodes · 1 control-plane`.
fn nodes_count_text(count: usize, roles: &[(String, usize)]) -> String {
    let mut text = count_label(count, "node", "nodes");
    for (role, number) in roles.iter().take(3) {
        text.push_str(&format!(" · {number} {role}"));
    }
    text
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn count_label_pluralizes() {
        assert_eq!(count_label(1, "pod", "pods"), "1 pod");
        assert_eq!(count_label(0, "pod", "pods"), "0 pods");
        assert_eq!(count_label(104, "node", "nodes"), "104 nodes");
        assert_eq!(count_label(1, "ingress", "ingresses"), "1 ingress");
        assert_eq!(count_label(2, "ingress", "ingresses"), "2 ingresses");
    }

    #[test]
    fn match_count_label_groups_both_numbers() {
        assert_eq!(match_count_label(38, 1_284), "38 of 1,284 match");
        assert_eq!(match_count_label(0, 12), "0 of 12 match");
    }

    #[test]
    fn header_counts_all_clusters() {
        assert_eq!(
            clusters_count_text(2, 2_431, "pod", "pods"),
            "2 clusters · 2,431 pods"
        );
        assert_eq!(
            clusters_count_text(3, 1, "node", "nodes"),
            "3 clusters · 1 node"
        );
    }

    fn cluster(context: &str) -> ClusterRef {
        ClusterRef {
            kubeconfig: std::path::PathBuf::from("kube.yaml"),
            context: context.to_owned(),
        }
    }

    /// The notices of one cluster in `state`, for a screen that lists pods.
    fn notices_of(state: SlotState) -> Vec<SlotNotice> {
        let cluster = cluster("a");
        let status = SlotStatus {
            cluster: &cluster,
            label: "prod-eu",
            state,
        };
        slot_notices(&[status], Some("pods"))
    }

    fn live_with(is_interrupted: bool, list_failure: Option<ListFailure>) -> SlotState {
        SlotState::Live {
            is_interrupted,
            list_failure,
        }
    }

    #[test]
    fn a_failed_connect_has_a_banner_with_its_cluster() {
        assert_eq!(
            notices_of(SlotState::Failed("connection refused")),
            [SlotNotice::Failed {
                cluster: cluster("a"),
                text: "Cannot connect to prod-eu: connection refused".to_owned(),
            }]
        );
    }

    #[test]
    fn an_interrupted_watch_has_a_banner_without_buttons() {
        assert_eq!(
            notices_of(live_with(true, None)),
            [SlotNotice::Interrupted {
                text: "Live updates interrupted in prod-eu".to_owned(),
            }]
        );
    }

    #[test]
    fn a_connecting_cluster_has_a_muted_line() {
        assert_eq!(
            notices_of(SlotState::Connecting),
            [SlotNotice::Connecting {
                text: "Connecting to prod-eu…".to_owned(),
            }]
        );
    }

    #[test]
    fn a_denied_list_says_not_permitted() {
        let failure = ListFailure {
            message: "forbidden",
            is_denied: true,
        };
        assert_eq!(
            notices_of(live_with(false, Some(failure))),
            [SlotNotice::Denied {
                text: "Not permitted in prod-eu: list pods".to_owned(),
            }]
        );
    }

    #[test]
    fn a_list_that_failed_otherwise_says_cannot_list_with_the_message() {
        // A CRD missing in this cluster is a 404, not a denial.
        let failure = ListFailure {
            message: "the server could not find the requested resource",
            is_denied: false,
        };
        assert_eq!(
            notices_of(live_with(false, Some(failure))),
            [SlotNotice::ListFailed {
                text:
                    "Cannot list pods in prod-eu: the server could not find the requested resource"
                        .to_owned(),
            }]
        );
    }

    #[test]
    fn a_screen_without_a_list_has_no_list_note() {
        let cluster = cluster("a");
        let failure = ListFailure {
            message: "forbidden",
            is_denied: true,
        };
        let status = SlotStatus {
            cluster: &cluster,
            label: "prod-eu",
            state: live_with(false, Some(failure)),
        };
        assert!(slot_notices(&[status], None).is_empty());
    }

    #[test]
    fn capitalized_uppercases_the_first_letter() {
        assert_eq!(capitalized("pods"), "Pods");
        assert_eq!(capitalized(""), "");
    }

    #[test]
    fn count_label_groups_digits() {
        assert_eq!(count_label(2_000, "event", "events"), "2,000 events");
    }
}
