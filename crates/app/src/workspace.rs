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
    AnyElement, App, Context, IntoElement, ParentElement as _, Styled as _, Window, div,
    prelude::FluentBuilder as _, px,
};

use cluster::{EVENT_LIMIT, EventFilter};

use super::{AppShell, KubeconfigState, Screen};
use crate::cluster_session::{FlowState, LiveCluster, SessionPhase};
use crate::drawer::ClickHandler;
use crate::file_export::ExportState;
use crate::filter_bar::{ToolkitState, filter_bar};
use crate::issue_board::IssueSummary;
use crate::issue_table::coverage_status;
use crate::kind_drawer::kind_drawer;
use crate::log_dock::{DEFAULT_DOCK_HEIGHT, DockMode, MIN_DOCK_HEIGHT, dock_max_height};
use crate::navigation::SIDEBAR_WIDTH;
use crate::node_drawer::node_drawer;
use crate::node_summary::role_counts;
use crate::overview::{
    OverviewData, change_window_button, headline_text, overview_body, stats_line,
};
use crate::pod_drawer::pod_drawer;
use crate::resource_kind::ResourceKind;
use crate::row_selection::{bulk_actions, selection_bar};
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
        let dock = self.log_dock.read(cx);
        if !dock.has_tabs() {
            return region.child(self.render_upper(cx));
        }
        match dock.mode() {
            DockMode::Zoomed => region.child(self.log_dock.clone()),
            DockMode::Minimized => region
                .child(self.render_upper(cx))
                .child(self.log_dock.clone()),
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
                                .child(self.log_dock.clone()),
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
            .children(self.render_filter_bar(toolkit, cx))
            .children(self.render_overview_stats(cx))
            .children(self.render_overview_export_error())
            .children(self.render_interruption_banner(cx))
            .child(div().flex_1().min_h_0().child(self.render_body(cx)))
            .children(self.render_selection_bar(toolkit, cx))
            .children(self.render_drawer(cx))
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
                self.session.as_ref().zip(live).map(|(session, live)| {
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
                    Some(nodes_count_text(count, &role_counts(live.nodes.items())))
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
        self.live(cx)?;
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
        let bar = selection_bar(text, bulk_actions(self.screen), &cx.weak_entity(), cx);
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
        self.live(cx)?;
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
        self.session.as_ref()?.read(cx).issues().summary()
    }

    /// Right of the Issues header: how the issues were found, and what could not be checked. A
    /// gap shows as `Partial coverage` in Warn with the note as its tooltip; a feed that is only
    /// limited by design shows muted, untoned.
    fn render_issues_status(&self, cx: &Context<Self>) -> Option<AnyElement> {
        self.live(cx)?;
        let board = self.session.as_ref()?.read(cx).issues();
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
                    shell.open_traffic_test(None, false, window, cx);
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
                    let (subject, namespace) = match shell.drawer_account() {
                        Some((subject, namespace)) => (Some(subject), Some(namespace)),
                        None => (None, shell.tool_namespace(cx)),
                    };
                    shell.open_permissions(subject, namespace, true, window, cx);
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
                    let namespace = shell.tool_namespace(cx);
                    shell.open_who_can(None, namespace, false, window, cx);
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
        let session = self.session.as_ref()?;
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
        let Some(session) = &self.session else {
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
            Screen::Overview => match self.session.as_ref() {
                Some(session) => overview_body(
                    &OverviewData {
                        live,
                        board: session.read(cx).issues(),
                        window: self.overview.window,
                        dock: &self.log_dock.downgrade(),
                    },
                    cx,
                ),
                None => div().into_any_element(),
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
            .session
            .as_ref()
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
        let key = self.drawer_subject()?;
        let session = self.session.as_ref()?;
        let live = session.read(cx).live()?;
        match key {
            ResourceKey::Pod { .. } => {
                let pod = live.pods.items().iter().find(|pod| key.is_pod(pod))?;
                Some(pod_drawer(
                    pod,
                    &self.drawer,
                    session,
                    &self.log_dock.downgrade(),
                    cx,
                ))
            }
            ResourceKey::Node { .. } => {
                let node = live.nodes.items().iter().find(|node| key.is_node(node))?;
                Some(node_drawer(node, &self.drawer, session, cx))
            }
            ResourceKey::Kind { kind, .. } => {
                // Over Topology the row comes from its feeds, not the explorer.
                let row = live.row_of(key)?;
                Some(kind_drawer(*kind, row, &self.drawer, live, session, cx))
            }
        }
    }
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
    fn count_label_groups_digits() {
        assert_eq!(count_label(2_000, "event", "events"), "2,000 events");
    }
}
