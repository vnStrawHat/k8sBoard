//! The Deployment revision diff (spec 0039, wireframe W7 Deployments): a read-only dialog with the
//! line diff of the pod templates of two ReplicaSets. The cluster crate masks the templates like the
//! YAML tab, so env literals stay hidden until the toggle asks for them, and nothing here logs or
//! keeps the text beyond the diff rows it draws.

use std::rc::Rc;

use cluster::{ClusterConnection, EnvValues, ObjectKind, ObjectRef, ReplicaSetSummary};
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::Button;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, Context, Div, ElementId, Entity, IntoElement, ListAlignment, ListState,
    ParentElement as _, Render, SharedString, Styled as _, Task, WeakEntity, Window, div, list,
    prelude::FluentBuilder as _, px,
};

use crate::age::format_local_time;
use crate::app_shell::AppShell;
use crate::cluster_runtime::ClusterRuntime;
use crate::cluster_session::error_text;
use crate::table_selection::{ClusterObject, ResourceKey};
use crate::workload_actions::{RevisionTarget, image_tag};
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
    /// When the ReplicaSet was created; the history list shows its age.
    pub(crate) created_at: Option<jiff::Timestamp>,
    /// Its `kubernetes.io/change-cause`: why the revision was made.
    pub(crate) change_cause: Option<String>,
}

impl RevisionSide {
    pub(crate) fn of(replica_set: &ReplicaSetSummary, is_current: bool) -> Self {
        Self {
            created_at: replica_set.created_at,
            change_cause: replica_set.change_cause.clone(),
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

    /// `rev 12 · v2.1`.
    pub(crate) fn title(&self) -> String {
        let revision = self
            .revision
            .map_or_else(|| "—".to_owned(), |number| number.to_string());
        let mut title = format!("rev {revision}");
        if let Some(tag) = &self.tag {
            title.push_str(&format!(" · {tag}"));
        }
        title
    }

    /// The hover text of the revision: the whole cause, then the absolute creation time.
    pub(crate) fn tooltip(&self, zone: &jiff::tz::TimeZone) -> Option<String> {
        revision_tooltip(self.created_at, self.change_cause.as_deref(), zone)
    }

    /// The title, with `(current)` after the side the Deployment runs.
    fn label(&self) -> String {
        let mut label = self.title();
        if self.is_current {
            label.push_str(" (current)");
        }
        label
    }

    /// What rolling back to this side would go to; `None` for the current revision and for a
    /// ReplicaSet without a revision number.
    pub(crate) fn roll_back_target(&self) -> Option<RevisionTarget> {
        let revision = self.revision.filter(|_| !self.is_current)?;
        Some(RevisionTarget {
            replica_set: self.replica_set.clone(),
            revision,
            tag: self.tag.clone(),
        })
    }

    fn name_in_error(&self) -> String {
        self.revision.map_or_else(
            || self.replica_set.clone(),
            |number| format!("rev {number}"),
        )
    }
}

/// The hover text of a revision row: its change cause (a row cuts a long one), then `Created` with
/// the absolute time in `zone`. `None` when the row knows neither. Pure.
pub(crate) fn revision_tooltip(
    created_at: Option<jiff::Timestamp>,
    change_cause: Option<&str>,
    zone: &jiff::tz::TimeZone,
) -> Option<String> {
    let created = created_at.map(|time| format!("Created {}", format_local_time(time, zone)));
    match (change_cause, created) {
        (Some(cause), Some(created)) => Some(format!(
            "{cause}
{created}"
        )),
        (Some(cause), None) => Some(cause.to_owned()),
        (None, created) => created,
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

/// The Deployment's ReplicaSets as sides, newest revision first (a missing number last, then by
/// name). The highest number is the current revision: the Deployment controller gives a rolled-back
/// ReplicaSet the next number. Pure.
pub(crate) fn revision_list(replica_sets: &[ReplicaSetSummary]) -> Vec<RevisionSide> {
    let mut sides: Vec<RevisionSide> = replica_sets
        .iter()
        .map(|replica_set| RevisionSide::of(replica_set, false))
        .collect();
    sides.sort_by(|a, b| {
        b.revision
            .is_some()
            .cmp(&a.revision.is_some())
            .then(b.revision.cmp(&a.revision))
            .then_with(|| a.replica_set.cmp(&b.replica_set))
    });
    if let Some(newest) = sides.first_mut() {
        newest.is_current = newest.revision.is_some();
    }
    sides
}

/// (newest, previous) of the list; `None` with fewer than two numbered revisions. Pure.
pub(crate) fn latest_pair(sides: &[RevisionSide]) -> Option<(RevisionSide, RevisionSide)> {
    match sides {
        [newest, previous, ..] if newest.revision.is_some() && previous.revision.is_some() => {
            Some((newest.clone(), previous.clone()))
        }
        _ => None,
    }
}

/// (newer, older) for a click on a rollout row: the ReplicaSet the event `named` against the
/// highest numbered side below it when both are listed, else the latest pair. Pure.
pub(crate) fn change_pair(
    sides: &[RevisionSide],
    named: Option<&str>,
) -> Option<(RevisionSide, RevisionSide)> {
    let pair = named.and_then(|name| {
        let (position, side) = sides
            .iter()
            .enumerate()
            .find(|(_, side)| side.replica_set == name)?;
        let number = side.revision?;
        let predecessor = sides[position + 1..]
            .iter()
            .find(|candidate| candidate.revision.is_some_and(|older| older < number))?;
        Some((side.clone(), predecessor.clone()))
    });
    pair.or_else(|| latest_pair(sides))
}

impl RevisionDiffRequest {
    /// The revision the dialog's Roll back goes to: the side that is not current. `None` unless
    /// exactly one side is current (a rollout pair in the middle of the history has none).
    pub(crate) fn roll_back_target(&self) -> Option<RevisionTarget> {
        match (self.older.is_current, self.newer.is_current) {
            (false, true) => self.older.roll_back_target(),
            (true, false) => self.newer.roll_back_target(),
            _ => None,
        }
    }

    /// `rev 12 · v2.1 → rev 14 · v2.3 (current)`.
    pub(crate) fn subtitle(&self) -> String {
        format!("{} → {}", self.older.label(), self.newer.label())
    }

    /// One line per side that has a change cause: `rev 12: release test`. Empty when neither has one.
    pub(crate) fn cause_lines(&self) -> Vec<String> {
        [&self.older, &self.newer]
            .into_iter()
            .filter_map(|side| {
                let number = side
                    .revision
                    .map_or_else(|| "—".to_owned(), |number| number.to_string());
                Some(format!("rev {number}: {}", side.change_cause.as_ref()?))
            })
            .collect()
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

/// What a Roll back button next to a revision does: starts the confirm flow of the shell, or says
/// why it cannot (permission, lock, paused rollout), as the drawer's buttons do.
#[derive(Clone)]
pub(crate) enum RollBackOffer {
    Enabled {
        shell: WeakEntity<AppShell>,
        subject: ClusterObject,
    },
    Disabled(SharedString),
}

impl RollBackOffer {
    /// The button for `target`. A click closes the diff dialog it may sit in, then opens the same
    /// confirm dialog as the drawer's button; the rest of the shell state is read at that moment.
    pub(crate) fn button(
        &self,
        id: impl Into<ElementId>,
        label: impl Into<SharedString>,
        target: RevisionTarget,
    ) -> Button {
        let button = Button::new(id).label(label);
        match self {
            Self::Disabled(reason) => button.disabled(true).tooltip(reason.clone()),
            Self::Enabled { shell, subject } => {
                let (shell, subject) = (shell.clone(), subject.clone());
                button.on_click(move |_, window, cx| {
                    // The row behind a history button selects its revision on a click.
                    cx.stop_propagation();
                    window.close_dialog(cx);
                    let _ = shell.update(cx, |shell, cx| {
                        shell.begin_roll_back(&subject, &target, window, cx);
                    });
                })
            }
        }
    }
}

/// The `Go to deployment` button of a dialog opened from the timeline: the row it reveals, and the
/// shell that reveals it.
struct GoTo {
    deployment: ResourceKey,
    shell: WeakEntity<AppShell>,
}

/// The dialog child: fetches the two templates when opened and shows their diff. It is dropped with
/// the dialog, and nothing is cached.
pub(crate) struct RevisionDiffView {
    request: RevisionDiffRequest,
    /// `None` for the screenshot fixture, which never fetches.
    connection: Option<ClusterConnection>,
    env: EnvValues,
    state: DiffState,
    /// Follows the count of the Ready rows; the rows wrap, so their heights differ.
    diff_list: ListState,
    /// The Deployment the footer button reveals; `None` where the dialog is already on it.
    go_to: Option<GoTo>,
    /// The footer Roll back to the other revision; `None` where the dialog is read-only (no shell).
    roll_back: Option<RollBackOffer>,
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
            diff_list: ListState::new(0, ListAlignment::Top, px(200.)),
            go_to: None,
            roll_back: None,
        };
        view.load(EnvValues::Hidden, cx);
        view
    }

    /// Adds the `Go to deployment` button for a dialog opened from the Overview timeline.
    pub(crate) fn with_go_to(
        mut self,
        deployment: ResourceKey,
        shell: WeakEntity<AppShell>,
    ) -> Self {
        self.go_to = Some(GoTo { deployment, shell });
        self
    }

    /// Adds the footer button that rolls the Deployment back to the revision being compared.
    pub(crate) fn with_roll_back(mut self, offer: RollBackOffer) -> Self {
        self.roll_back = Some(offer);
        self
    }

    /// Closes the dialog and reveals the Deployment's row.
    fn go_to_deployment(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(go_to) = &self.go_to else {
            return;
        };
        window.close_dialog(cx);
        let key = go_to.deployment.clone();
        let _ = go_to.shell.update(cx, |shell, cx| shell.reveal(key, cx));
    }

    /// What a click on the footer button does, for the tests that drive the dialog.
    #[cfg(test)]
    pub(crate) fn go_to_deployment_for_test(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.go_to_deployment(window, cx);
    }

    /// `--screen revision-diff`: a loaded diff of two fixed templates; no request is ever made.
    #[cfg(feature = "screenshot")]
    pub(crate) fn fixture(
        request: RevisionDiffRequest,
        older: &str,
        newer: &str,
        hidden_env_values: usize,
    ) -> Self {
        let rows: Rc<[DiffRow]> = diff_rows(older, newer).into();
        Self {
            request,
            connection: None,
            env: EnvValues::Hidden,
            diff_list: ListState::new(rows.len(), ListAlignment::Top, px(200.)),
            state: DiffState::Ready {
                rows,
                hidden_env_values,
            },
            go_to: None,
            roll_back: None,
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
                let count = match &state {
                    DiffState::Ready { rows, .. } => rows.len(),
                    DiffState::Loading { .. } | DiffState::Failed(_) => 0,
                };
                view.diff_list.reset(count);
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
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(div().truncate().child(self.request.subtitle()))
                    .children(
                        self.request
                            .cause_lines()
                            .into_iter()
                            .map(|line| div().truncate().child(line)),
                    ),
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

    /// The footer of the dialog: Roll back to the revision that is not current, and, for the dialog
    /// opened from the timeline, the button that goes to the Deployment.
    fn render_footer(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let roll_back = self
            .roll_back
            .as_ref()
            .zip(self.request.roll_back_target())
            .map(|(offer, target)| {
                let label = format!("Roll back to rev {}…", target.revision);
                offer
                    .button("revision-diff-roll-back", label, target)
                    .small()
                    .outline()
            });
        if roll_back.is_none() && self.go_to.is_none() {
            return None;
        }
        Some(
            h_flex()
                .flex_shrink_0()
                .justify_end()
                .gap_2()
                .pt_2()
                .children(roll_back)
                .children(self.go_to.as_ref().map(|_| {
                    Button::new("revision-diff-go-to")
                        .label("Go to deployment")
                        .small()
                        .outline()
                        .on_click(cx.listener(|view, _, window, cx| {
                            view.go_to_deployment(window, cx);
                        }))
                }))
                .into_any_element(),
        )
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
                    list(
                        self.diff_list.clone(),
                        move |ix, _: &mut Window, cx: &mut App| {
                            rows.get(ix)
                                .map_or_else(div, |row| diff_row_element(row, cx))
                                .into_any_element()
                        },
                    )
                    .size_full()
                    .into_any_element()
                }
            },
        }
    }
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
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .child(self.render_toolbar(cx))
            .child(div().flex_1().min_h_0().child(self.render_body(cx)))
            .children(self.render_footer(cx))
    }
}

/// The view in a dialog: the view fills its parent, so the dialog gives it a share of the window
/// high. The Revision history tab embeds the view without this wrapper.
pub(crate) fn dialog_body(view: &Entity<RevisionDiffView>, window: &Window) -> Div {
    div()
        .w_full()
        .h(window.viewport_size().height * HEIGHT_SHARE)
        .child(view.clone())
}

#[cfg(test)]
#[path = "revision_diff_tests.rs"]
mod revision_diff_tests;
