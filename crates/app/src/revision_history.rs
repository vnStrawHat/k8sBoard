//! The Revision history tab of Edit YAML for a Deployment (spec 0041, wireframe W10 tabs): the
//! Deployment's revisions newest first, and the 0039 revision diff of the selected one against the
//! current one. Read-only: one ReplicaSet LIST per editor, and the diff's own two GETs. The tab never
//! touches the editor text, and nothing here logs a template.

use cluster::{ClusterConnection, ClusterError, ObjectRef, ReplicaSetSummary};
use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _, Task,
    Window, div, prelude::FluentBuilder as _, px,
};

use crate::age::format_age;
use crate::cluster_runtime::ClusterRuntime;
use crate::cluster_session::error_text;
use crate::revision_diff::{
    RevisionDiffView, RevisionSide, RollBackOffer, diff_request, latest_pair, revision_list,
};
use crate::table_selection::ResourceKey;
use crate::yaml_edit::yaml_edit_panels::{busy, muted_center};

const LIST_WIDTH: f32 = 240.;
/// How strongly the `current` pill is tinted by its theme token.
const PILL_TINT: f32 = 0.14;

const CURRENT_NOTE: &str = "This is the current revision.";
const SINGLE_NOTE: &str = "No earlier revision kept (revisionHistoryLimit)";
const UNNUMBERED_NOTE: &str = "These ReplicaSets carry no revision number to compare.";
const DENIED_NOTE: &str = "Not permitted: list replicasets";

/// What the shell knows when the tab is first shown.
pub(crate) enum HistoryInputs {
    /// The session's access review denies `list replicasets`; nothing is sent.
    Denied,
    /// The list cannot be asked: the cluster is not open, or the Deployment is not loaded yet.
    Unavailable(SharedString),
    Ready {
        connection: ClusterConnection,
        /// The Deployment's selector as a label selector query; empty or `<invalid>` lists nothing.
        selector: String,
    },
}

enum HistoryState {
    /// Dropping the task cancels the LIST.
    Loading {
        _task: Task<()>,
    },
    Denied,
    Failed(SharedString),
    Ready(Vec<RevisionSide>),
}

pub(crate) struct RevisionHistory {
    deployment: ResourceKey,
    /// `None` for the screenshot fixture, which sends nothing.
    connection: Option<ClusterConnection>,
    state: HistoryState,
    selected: Option<usize>,
    /// The diff of the selected revision; dropping it cancels its GETs.
    diff: Option<Entity<RevisionDiffView>>,
    /// What the Roll back button of each older row does; `None` shows no button (the fixtures).
    roll_back: Option<RollBackOffer>,
}

impl RevisionHistory {
    /// Starts the one LIST of the Deployment's ReplicaSets, unless `inputs` says there is nothing to
    /// ask. `object` is the Deployment being edited.
    pub(crate) fn new(
        deployment: ResourceKey,
        object: ObjectRef,
        inputs: HistoryInputs,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut history = Self {
            deployment,
            connection: None,
            state: HistoryState::Denied,
            selected: None,
            diff: None,
            roll_back: None,
        };
        match inputs {
            HistoryInputs::Denied => {}
            HistoryInputs::Unavailable(reason) => history.state = HistoryState::Failed(reason),
            HistoryInputs::Ready {
                connection,
                selector,
            } => history.load(connection, object, selector, cx),
        }
        history
    }

    /// Adds a Roll back button to every revision that is not the current one.
    pub(crate) fn with_roll_back(mut self, offer: RollBackOffer) -> Self {
        self.roll_back = Some(offer);
        self
    }

    /// Whether the list failed or could not be asked, so showing the tab again should ask again.
    pub(crate) fn has_failed(&self) -> bool {
        matches!(self.state, HistoryState::Failed(_))
    }

    fn load(
        &mut self,
        connection: ClusterConnection,
        object: ObjectRef,
        selector: String,
        cx: &mut Context<Self>,
    ) {
        self.connection = Some(connection.clone());
        let runtime = cx.global::<ClusterRuntime>().clone();
        let task = cx.spawn(async move |this, cx| {
            let listed = runtime
                .spawn(async move { connection.deployment_revisions(&object, &selector).await })
                .await;
            let _ = this.update(cx, |history, cx| history.finish(listed.ok(), cx));
        });
        self.state = HistoryState::Loading { _task: task };
    }

    /// `None` when the task stopped before it answered.
    fn finish(
        &mut self,
        listed: Option<Result<Vec<ReplicaSetSummary>, ClusterError>>,
        cx: &mut Context<Self>,
    ) {
        match listed {
            Some(Ok(replica_sets)) => {
                let sides = revision_list(&replica_sets);
                // The previous revision is the likeliest one to look at: what the last rollout
                // replaced.
                let previous = latest_pair(&sides).map(|_| 1);
                self.state = HistoryState::Ready(sides);
                match previous {
                    Some(index) => self.select(index, cx),
                    None => self.selected = None,
                }
            }
            Some(Err(error)) => self.state = HistoryState::Failed(error_text(&error).into()),
            None => {
                self.state = HistoryState::Failed("The request stopped before it finished".into());
            }
        }
        cx.notify();
    }

    /// Shows the diff of revision `index` against the current one. The current row has none.
    fn select(&mut self, index: usize, cx: &mut Context<Self>) {
        let HistoryState::Ready(sides) = &self.state else {
            return;
        };
        let Some(clicked) = sides.get(index).cloned() else {
            return;
        };
        self.selected = Some(index);
        self.diff = None;
        if clicked.is_current {
            cx.notify();
            return;
        }
        let current = sides.iter().find(|side| side.is_current).cloned();
        if let (Some(current), Some(connection)) = (current, self.connection.clone()) {
            let request = diff_request(self.deployment.clone(), clicked, current);
            self.diff = Some(cx.new(|cx| RevisionDiffView::new(request, connection, cx)));
        }
        cx.notify();
    }

    fn render_list(&self, sides: &[RevisionSide], cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let now = jiff::Timestamp::now();
        let zone = jiff::tz::TimeZone::system();
        let (active, hover) = (theme.list_active, theme.list_hover);
        v_flex()
            .id("history-list")
            .w(px(LIST_WIDTH))
            .flex_shrink_0()
            .h_full()
            .p_2()
            .gap_1()
            .border_r_1()
            .border_color(theme.border)
            .overflow_y_scroll()
            .children(sides.iter().enumerate().map(|(index, side)| {
                let is_selected = self.selected == Some(index);
                let tooltip = side.tooltip(&zone);
                h_flex()
                    .id(("history-rev", index))
                    .gap_2()
                    .items_center()
                    .px_2()
                    .py_1()
                    .rounded(theme.radius)
                    .text_sm()
                    .cursor_pointer()
                    .when(is_selected, |row| row.bg(active))
                    .hover(move |style| style.bg(hover))
                    .on_click(cx.listener(move |history, _, _, cx| history.select(index, cx)))
                    .when_some(tooltip, |row, text| {
                        row.tooltip(move |window, cx| Tooltip::new(text.clone()).build(window, cx))
                    })
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .truncate()
                                    .font_family(theme.mono_font_family.clone())
                                    .child(side.title()),
                            )
                            .children(side.change_cause.as_ref().map(|cause| {
                                div()
                                    .truncate()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .child(cause.clone())
                            }))
                            .child(div().text_xs().text_color(theme.muted_foreground).child(
                                match side.created_at {
                                    Some(_) => {
                                        format!("{} ago", format_age(side.created_at, now))
                                    }
                                    None => "—".to_owned(),
                                },
                            )),
                    )
                    .children(self.roll_back.as_ref().zip(side.roll_back_target()).map(
                        |(offer, target)| {
                            offer
                                .button(("history-roll-back", index), "Roll back…", target)
                                .xsmall()
                                .ghost()
                        },
                    ))
                    .children(side.is_current.then(|| {
                        div()
                            .flex_shrink_0()
                            .px_1()
                            .rounded(theme.radius)
                            .text_xs()
                            .bg(theme.success.opacity(PILL_TINT))
                            .text_color(theme.success)
                            .child("current")
                    }))
            }))
            .into_any_element()
    }

    fn render_detail(&self, sides: &[RevisionSide], cx: &Context<Self>) -> AnyElement {
        let muted = cx.theme().muted_foreground;
        match (
            detail_note(sides, self.selected, self.diff.is_some()),
            &self.diff,
        ) {
            (Some(note), _) => muted_center(note, muted),
            (None, Some(diff)) => div()
                .size_full()
                .p_2()
                .child(diff.clone())
                .into_any_element(),
            (None, None) => div().into_any_element(),
        }
    }
}

/// What the detail pane says when there is no diff to draw, `None` when a diff is shown. Pure.
fn detail_note(
    sides: &[RevisionSide],
    selected: Option<usize>,
    has_diff: bool,
) -> Option<&'static str> {
    if sides.is_empty() {
        return Some("No revisions found");
    }
    if selected
        .and_then(|index| sides.get(index))
        .is_some_and(|side| side.is_current)
    {
        return Some(CURRENT_NOTE);
    }
    if has_diff {
        return None;
    }
    // Without a revision number there is no order, so "earlier" and the history limit mean nothing.
    if sides.iter().all(|side| side.revision.is_none()) {
        return Some(UNNUMBERED_NOTE);
    }
    Some(SINGLE_NOTE)
}

impl Render for RevisionHistory {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        match &self.state {
            HistoryState::Loading { .. } => busy("Loading revisions…", cx),
            HistoryState::Denied => muted_center(DENIED_NOTE, muted),
            HistoryState::Failed(reason) => muted_center(
                format!("Could not load revisions: {reason}"),
                cx.theme().danger,
            ),
            HistoryState::Ready(sides) => h_flex()
                .size_full()
                .child(self.render_list(sides, cx))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .child(self.render_detail(sides, cx)),
                )
                .into_any_element(),
        }
    }
}

#[cfg(feature = "screenshot")]
impl RevisionHistory {
    /// `--screen edit-yaml-history`: a loaded list over a fixed diff; no request is ever made.
    pub(crate) fn fixture(
        deployment: ResourceKey,
        sides: Vec<RevisionSide>,
        selected: usize,
        diff: Entity<RevisionDiffView>,
    ) -> Self {
        Self {
            deployment,
            connection: None,
            state: HistoryState::Ready(sides),
            selected: Some(selected),
            diff: Some(diff),
            roll_back: None,
        }
    }
}

#[cfg(test)]
impl RevisionHistory {
    pub(crate) fn is_ready(&self) -> bool {
        matches!(self.state, HistoryState::Ready(_))
    }
}

#[cfg(test)]
#[path = "revision_history_tests.rs"]
mod revision_history_tests;
