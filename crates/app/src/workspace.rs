//! The workspace region of the window: header, banner, table or state view, and the drawer.

use std::rc::Rc;

use gpui_kit::component::alert::Alert;
use gpui_kit::component::button::Button;
use gpui_kit::component::resizable::{resizable_panel, v_resizable};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::table::DataTable;
use gpui_kit::component::{ActiveTheme as _, Sizable as _, StyledExt as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, Context, IntoElement, ParentElement as _, Styled as _, Window, div,
    prelude::FluentBuilder as _,
};

use super::{AppShell, KubeconfigState, Screen};
use crate::cluster_session::{LiveCluster, SessionPhase};
use crate::drawer::ClickHandler;
use crate::log_dock::{DEFAULT_DOCK_HEIGHT, DockMode, MIN_DOCK_HEIGHT, dock_max_height};
use crate::navigation::SIDEBAR_WIDTH;
use crate::node_drawer::node_drawer;
use crate::pod_drawer::pod_drawer;
use crate::table_selection::ResourceKey;

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
        v_flex()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .relative()
            .child(self.render_header(cx))
            .children(self.render_interruption_banner(cx))
            .child(div().flex_1().min_h_0().child(self.render_body(cx)))
            .children(self.render_drawer(cx))
    }

    fn render_header(&self, cx: &Context<Self>) -> impl IntoElement {
        let live = self.live(cx);
        let (title, count) = match self.screen {
            Screen::Pods => (
                "Pods",
                live.and_then(|live| {
                    live.pods.ready_count().map(|count| {
                        format!("{} · {}", count_label(count, "pod"), live.scope_label())
                    })
                }),
            ),
            Screen::Nodes => (
                "Nodes",
                live.and_then(|live| {
                    live.nodes
                        .ready_count()
                        .map(|count| count_label(count, "node"))
                }),
            ),
        };
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
    }

    fn render_interruption_banner(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let live = self.live(cx)?;
        let message = match self.screen {
            Screen::Pods => live.pods.interruption(),
            Screen::Nodes => live.nodes.interruption(),
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
        match &self.kubeconfig {
            KubeconfigState::Loading => return busy_view("Loading kubeconfig…", cx),
            KubeconfigState::Failed(message) => {
                return error_view("Cannot load the kubeconfig", message, None, None, cx);
            }
            KubeconfigState::Loaded(_) => {}
        }
        let Some(session) = &self.session else {
            let message = self
                .context_error
                .as_deref()
                .unwrap_or("No context selected");
            return error_view(
                "Cannot open a context",
                message,
                Some("Pick a context from the cluster menu."),
                None,
                cx,
            );
        };
        let session = session.read(cx);
        match session.phase() {
            SessionPhase::Connecting { .. } => {
                busy_view(&format!("Connecting to {}…", session.context()), cx)
            }
            SessionPhase::Failed { message } => error_view(
                &format!("Cannot connect to {}", session.context()),
                message,
                None,
                Some(Rc::new(cx.listener(|shell, _, _, cx| shell.retry(cx)))),
                cx,
            ),
            SessionPhase::Live(live) => self.render_list(live, cx),
        }
    }

    fn render_list(&self, live: &LiveCluster, cx: &Context<Self>) -> AnyElement {
        let (title, failure) = match self.screen {
            Screen::Pods => ("Pods are unavailable", live.pods.failure()),
            Screen::Nodes => ("Nodes are unavailable", live.nodes.failure()),
        };
        if let Some(message) = failure {
            return error_view(title, message, Some("Retrying automatically."), None, cx);
        }
        match self.screen {
            Screen::Pods => DataTable::new(&self.pod_table)
                .bordered(false)
                .into_any_element(),
            Screen::Nodes => DataTable::new(&self.node_table)
                .bordered(false)
                .into_any_element(),
        }
    }

    /// An overlay on the workspace only, so it never covers the title bar, the sidebar, or
    /// the status bar. `None` when nothing is selected or the subject is not in the list.
    fn render_drawer(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let key = self.selected.as_ref()?;
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
                Some(node_drawer(node, session, cx))
            }
        }
    }
}

fn count_label(count: usize, noun: &str) -> String {
    if count == 1 {
        format!("1 {noun}")
    } else {
        format!("{count} {noun}s")
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

/// An error alert, with optional follow-up help and a Retry button.
fn error_view(
    title: &str,
    message: &str,
    hint: Option<&'static str>,
    retry: Option<ClickHandler>,
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
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn count_label_pluralizes() {
        assert_eq!(count_label(1, "pod"), "1 pod");
        assert_eq!(count_label(0, "pod"), "0 pods");
        assert_eq!(count_label(104, "node"), "104 nodes");
    }
}
