//! The Deployment revision diff (spec 0039, wireframe W7 Deployments): a read-only dialog with the
//! line diff of the pod templates of two ReplicaSets. The cluster crate masks the templates like the
//! YAML tab, so env literals stay hidden until the toggle asks for them, and nothing here logs or
//! keeps the text beyond the diff rows it draws.

use std::ops::Range;
use std::rc::Rc;

use cluster::{ClusterConnection, EnvValues, ObjectKind, ObjectRef, ReplicaSetSummary};
use gpui_kit::component::button::Button;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, Context, Div, IntoElement, ParentElement as _, Render, SharedString,
    Styled as _, Task, UniformListScrollHandle, Window, div, prelude::FluentBuilder as _,
    uniform_list,
};

use crate::cluster_runtime::ClusterRuntime;
use crate::cluster_session::error_text;
use crate::table_selection::ResourceKey;
use crate::workload_actions::image_tag;
use crate::yaml_diff::{DiffRow, DiffRowKind, diff_rows};
use crate::yaml_edit::yaml_edit_panels::diff_row_element;
use crate::yaml_view::shows_env_toggle;

/// The dialog is a share of the window high, so the diff list has room to scroll.
const HEIGHT_SHARE: f32 = 0.7;

const SAME_NOTE: &str = "The pod templates of the two revisions are the same.";
const HIDDEN_SAME_NOTE: &str = "No visible difference; env values are hidden";

/// One side of the diff: a ReplicaSet of the Deployment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RevisionSide {
    pub(crate) replica_set: String,
    /// The numeric `deployment.kubernetes.io/revision`; `None` when the annotation is missing.
    pub(crate) revision: Option<u64>,
    /// The tag of the first container image.
    pub(crate) tag: Option<String>,
    /// The revision the Deployment runs now.
    pub(crate) is_current: bool,
}

impl RevisionSide {
    pub(crate) fn of(replica_set: &ReplicaSetSummary, is_current: bool) -> Self {
        Self {
            replica_set: replica_set.name.clone(),
            revision: replica_set
                .revision
                .as_deref()
                .and_then(|text| text.parse().ok()),
            tag: replica_set
                .containers
                .first()
                .map(|container| image_tag(&container.image))
                .filter(|tag| !tag.is_empty())
                .map(str::to_owned),
            is_current,
        }
    }

    /// `rev 12 · v2.1`, with `(current)` after the side the Deployment runs.
    fn label(&self) -> String {
        let revision = self
            .revision
            .map_or_else(|| "—".to_owned(), |number| number.to_string());
        let mut label = format!("rev {revision}");
        if let Some(tag) = &self.tag {
            label.push_str(&format!(" · {tag}"));
        }
        if self.is_current {
            label.push_str(" (current)");
        }
        label
    }

    fn name_in_error(&self) -> String {
        self.revision.map_or_else(
            || self.replica_set.clone(),
            |number| format!("rev {number}"),
        )
    }
}

/// The two ReplicaSets to compare, older on the left whichever row was clicked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RevisionDiffRequest {
    /// The Deployment of the open drawer.
    pub(crate) deployment: ResourceKey,
    pub(crate) older: RevisionSide,
    pub(crate) newer: RevisionSide,
}

/// Orders the clicked revision and the current one into (older, newer). After a roll back the
/// current revision can be the older one. Pure.
pub(crate) fn diff_request(
    deployment: ResourceKey,
    clicked: RevisionSide,
    current: RevisionSide,
) -> RevisionDiffRequest {
    let (older, newer) = if clicked.revision <= current.revision {
        (clicked, current)
    } else {
        (current, clicked)
    };
    RevisionDiffRequest {
        deployment,
        older,
        newer,
    }
}

impl RevisionDiffRequest {
    /// `rev 12 · v2.1 → rev 14 · v2.3 (current)`.
    pub(crate) fn subtitle(&self) -> String {
        format!("{} → {}", self.older.label(), self.newer.label())
    }

    /// `Revision diff · deployment/api`.
    pub(crate) fn title(&self) -> String {
        let name = match &self.deployment {
            ResourceKey::Kind { name, .. } => name.as_str(),
            ResourceKey::Pod { name, .. } => name.as_str(),
            ResourceKey::Node { name } => name.as_str(),
        };
        format!("Revision diff · deployment/{name}")
    }

    fn replica_set_ref(&self, side: &RevisionSide) -> Option<ObjectRef> {
        let ResourceKey::Kind { namespace, .. } = &self.deployment else {
            return None;
        };
        ObjectRef::new(
            ObjectKind::ReplicaSet,
            namespace.clone(),
            side.replica_set.clone(),
        )
    }
}

enum DiffState {
    /// Dropping the task aborts both GETs and the diff.
    Loading {
        _task: Task<()>,
    },
    Failed(SharedString),
    Ready {
        rows: Rc<[DiffRow]>,
        /// Env literals hidden on either side.
        hidden_env_values: usize,
    },
}

/// What the Ready body says when no row changed, `None` when there is a diff to draw. Hidden env
/// values may differ, so they get their own words. Pure.
fn same_note(rows: &[DiffRow], hidden_env_values: usize) -> Option<&'static str> {
    let has_change = rows
        .iter()
        .any(|row| matches!(row.kind, DiffRowKind::Added | DiffRowKind::Removed));
    match (has_change, hidden_env_values) {
        (true, _) => None,
        (false, 0) => Some(SAME_NOTE),
        (false, _) => Some(HIDDEN_SAME_NOTE),
    }
}

/// The dialog child: fetches the two templates when opened and shows their diff. It is dropped with
/// the dialog, and nothing is cached.
pub(crate) struct RevisionDiffView {
    request: RevisionDiffRequest,
    /// `None` for the screenshot fixture, which never fetches.
    connection: Option<ClusterConnection>,
    env: EnvValues,
    state: DiffState,
    scroll: UniformListScrollHandle,
}

impl RevisionDiffView {
    pub(crate) fn new(
        request: RevisionDiffRequest,
        connection: ClusterConnection,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut view = Self {
            request,
            connection: Some(connection),
            env: EnvValues::Hidden,
            state: DiffState::Loading {
                _task: Task::ready(()),
            },
            scroll: UniformListScrollHandle::new(),
        };
        view.load(EnvValues::Hidden, cx);
        view
    }

    /// `--screen revision-diff`: a loaded diff of two fixed templates; no request is ever made.
    #[cfg(feature = "screenshot")]
    pub(crate) fn fixture(
        request: RevisionDiffRequest,
        older: &str,
        newer: &str,
        hidden_env_values: usize,
    ) -> Self {
        Self {
            request,
            connection: None,
            env: EnvValues::Hidden,
            state: DiffState::Ready {
                rows: diff_rows(older, newer).into(),
                hidden_env_values,
            },
            scroll: UniformListScrollHandle::new(),
        }
    }

    /// Fetches both templates at once on the cluster runtime, then diffs them on the background
    /// executor. A running load is replaced, which drops it.
    fn load(&mut self, env: EnvValues, cx: &mut Context<Self>) {
        let Some(connection) = self.connection.clone() else {
            return;
        };
        let (Some(older), Some(newer)) = (
            self.request.replica_set_ref(&self.request.older),
            self.request.replica_set_ref(&self.request.newer),
        ) else {
            self.state = DiffState::Failed("The Deployment has no namespace".into());
            return;
        };
        let (older_name, newer_name) = (
            self.request.older.name_in_error(),
            self.request.newer.name_in_error(),
        );
        let runtime = cx.global::<ClusterRuntime>().clone();
        let task = cx.spawn(async move |this, cx| {
            let fetched = runtime
                .spawn(async move {
                    let read = |side: ObjectRef, name: String| {
                        let connection = connection.clone();
                        async move {
                            connection
                                .pod_template_yaml(&side, env)
                                .await
                                .map_err(|error| {
                                    format!("Could not load {name}: {}", error_text(&error))
                                })
                        }
                    };
                    futures::future::try_join(read(older, older_name), read(newer, newer_name))
                        .await
                })
                .await;
            let state = match fetched {
                Ok(Ok((older, newer))) => {
                    let hidden_env_values = older.hidden_env_values + newer.hidden_env_values;
                    let rows = cx
                        .background_executor()
                        .spawn(async move { diff_rows(&older.text, &newer.text) })
                        .await;
                    DiffState::Ready {
                        rows: rows.into(),
                        hidden_env_values,
                    }
                }
                Ok(Err(message)) => DiffState::Failed(message.into()),
                Err(_) => DiffState::Failed("The request stopped before it finished".into()),
            };
            let _ = this.update(cx, |view, cx| {
                view.state = state;
                cx.notify();
            });
        });
        self.env = env;
        self.state = DiffState::Loading { _task: task };
    }

    fn toggle_env_values(&mut self, cx: &mut Context<Self>) {
        let flipped = match self.env {
            EnvValues::Hidden => EnvValues::Shown,
            EnvValues::Shown => EnvValues::Hidden,
        };
        self.load(flipped, cx);
        cx.notify();
    }

    fn hidden_env_values(&self) -> usize {
        match &self.state {
            DiffState::Ready {
                hidden_env_values, ..
            } => *hidden_env_values,
            DiffState::Loading { .. } | DiffState::Failed(_) => 0,
        }
    }

    fn render_toolbar(&self, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let is_loading = matches!(self.state, DiffState::Loading { .. });
        let is_shown = self.env == EnvValues::Shown;
        // The toggle stays while a load runs, so a click never makes it vanish under the pointer.
        let has_toggle =
            shows_env_toggle(self.env, self.hidden_env_values()) || (is_loading && is_shown);
        h_flex()
            .flex_shrink_0()
            .gap_2()
            .pb_2()
            .items_center()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(self.request.subtitle()),
            )
            .when(has_toggle, |bar| {
                bar.child(
                    Button::new("revision-diff-env")
                        .label(if is_shown {
                            "Hide env values"
                        } else {
                            "Show env values"
                        })
                        .small()
                        .outline()
                        .disabled(is_loading)
                        .on_click(cx.listener(|view, _, _, cx| view.toggle_env_values(cx))),
                )
            })
            .into_any_element()
    }

    fn render_body(&self, cx: &Context<Self>) -> AnyElement {
        let muted = cx.theme().muted_foreground;
        match &self.state {
            DiffState::Loading { .. } => centered(
                v_flex().items_center().gap_3().child(Spinner::new()).child(
                    div()
                        .text_sm()
                        .text_color(muted)
                        .child("Loading revisions…"),
                ),
            ),
            DiffState::Failed(message) => centered(
                div()
                    .text_sm()
                    .text_color(cx.theme().danger)
                    .child(message.clone()),
            ),
            DiffState::Ready {
                rows,
                hidden_env_values,
            } => match same_note(rows, *hidden_env_values) {
                Some(note) => centered(div().text_sm().text_color(muted).child(note)),
                None => {
                    let rows = Rc::clone(rows);
                    uniform_list(
                        "revision-diff-rows",
                        rows.len(),
                        move |range: Range<usize>, _: &mut Window, cx: &mut App| {
                            rows_in(&rows, range, cx)
                        },
                    )
                    .track_scroll(&self.scroll)
                    .size_full()
                    .into_any_element()
                }
            },
        }
    }
}

/// The rows `range` of the diff, drawn on demand by the list.
fn rows_in(rows: &[DiffRow], range: Range<usize>, cx: &App) -> Vec<Div> {
    rows.get(range)
        .unwrap_or_default()
        .iter()
        .map(|row| diff_row_element(row, cx))
        .collect()
}

fn centered(content: impl IntoElement) -> AnyElement {
    v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .px_4()
        .child(content)
        .into_any_element()
}

impl Render for RevisionDiffView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .w_full()
            .h(window.viewport_size().height * HEIGHT_SHARE)
            .child(self.render_toolbar(cx))
            .child(div().flex_1().min_h_0().child(self.render_body(cx)))
    }
}

#[cfg(test)]
#[path = "revision_diff_tests.rs"]
mod revision_diff_tests;
