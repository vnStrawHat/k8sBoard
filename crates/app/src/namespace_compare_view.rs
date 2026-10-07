//! The namespace comparison dialog (spec 0057): pick the second namespace, read both through one
//! cluster call, then show per kind what only one side has and what differs. Read-only; the
//! cluster crate reduces Secret values to equality tokens and masks env literals before anything
//! reaches this view, and nothing here logs or keeps more than the lines it draws.

use cluster::{ClusterConnection, EnvValues, NamespaceComparison, ObjectKind};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Sizable as _, StyledExt as _, h_flex, v_flex,
};
use gpui_kit::{
    AnyElement, AppContext as _, Context, Div, Entity, FocusHandle, Focusable as _,
    InteractiveElement as _, IntoElement, KeyDownEvent, ParentElement as _, Point, Render,
    ScrollHandle, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Task,
    Window, div, point, prelude::FluentBuilder as _, px,
};

use crate::cluster_runtime::ClusterRuntime;
use crate::drawer::{DrawerScroll, scrolled_offset};
use crate::keymap::{NAMESPACE_COMPARE, PickNamespace};
use crate::namespace_compare_rows::{
    CompareLine, OpenDiffs, compare_lines, hidden_env_note, summary_text,
};
use crate::namespace_picker::matching_namespaces;
use crate::yaml_edit::yaml_edit_panels::diff_row_element;

/// The dialog is a share of the window high, so the lines have room to scroll.
const HEIGHT_SHARE: f32 = 0.7;
/// How many frames the filter is given the focus, see `focus_tries`.
const FOCUS_TRIES: u8 = 6;
const INDENT_NAME: f32 = 24.;
const INDENT_CHANGE: f32 = 40.;

enum CompareState {
    Pick,
    /// Dropping the task abandons the call; the lists are never kept.
    Loading {
        right: String,
        _task: Task<()>,
    },
    Failed(SharedString),
    Ready {
        comparison: NamespaceComparison,
        lines: Vec<CompareLine>,
    },
}

pub(crate) struct NamespaceCompareView {
    left: String,
    /// The namespaces that can be the second side, sorted, without `left`.
    candidates: Vec<String>,
    connection: ClusterConnection,
    filter: Entity<InputState>,
    /// Takes the focus when the lines show, so the page keys scroll them.
    focus_handle: FocusHandle,
    scroll: ScrollHandle,
    /// Frames left to put the focus in the filter (Pick) or the lines (Ready): the menu a dialog was
    /// opened from can hand the focus back to the table after the first render, so one attempt is
    /// not enough.
    focus_tries: u8,
    env: EnvValues,
    open_diffs: OpenDiffs,
    state: CompareState,
    _filter_events: Subscription,
}

impl NamespaceCompareView {
    pub(crate) fn new(
        left: String,
        mut candidates: Vec<String>,
        connection: ClusterConnection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        candidates.retain(|name| *name != left);
        candidates.sort();
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder("Filter namespaces…"));
        let events = cx.subscribe_in(&filter, window, Self::on_filter_event);
        Self {
            left,
            candidates,
            connection,
            filter,
            focus_handle: cx.focus_handle(),
            scroll: ScrollHandle::new(),
            focus_tries: FOCUS_TRIES,
            env: EnvValues::Hidden,
            open_diffs: OpenDiffs::new(),
            state: CompareState::Pick,
            _filter_events: events,
        }
    }

    fn on_filter_event(
        &mut self,
        _: &Entity<InputState>,
        event: &InputEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(event, InputEvent::Change) {
            cx.notify();
        }
    }

    /// Enter in the filter: the first namespace that matches, or the typed name when the list is
    /// empty (it failed to load) and the name is not the left side.
    fn choose_first_match(&mut self, cx: &mut Context<Self>) {
        if !matches!(self.state, CompareState::Pick) {
            return;
        }
        let text = self.filter.read(cx).value().trim().to_owned();
        let picked = matching_namespaces(&self.candidates, &text)
            .first()
            .map(|name| (*name).clone())
            .or_else(|| (self.candidates.is_empty() && !text.is_empty()).then_some(text));
        if let Some(right) = picked.filter(|right| *right != self.left) {
            self.choose(right, cx);
        }
    }

    fn choose(&mut self, right: String, cx: &mut Context<Self>) {
        self.open_diffs.clear();
        self.load(right, cx);
        cx.notify();
    }

    /// Reads both namespaces on the cluster runtime; the main thread only awaits the answer.
    fn load(&mut self, right: String, cx: &mut Context<Self>) {
        let (connection, left, env) = (self.connection.clone(), self.left.clone(), self.env);
        let runtime = cx.global::<ClusterRuntime>().clone();
        let read_right = right.clone();
        let task = cx.spawn(async move |this, cx| {
            let read = runtime
                .spawn(async move { connection.compare_namespaces(&left, &read_right, env).await })
                .await;
            let _ = this.update(cx, |view, cx| {
                view.state = match read {
                    Ok(comparison) => {
                        let lines = compare_lines(&comparison, &view.open_diffs);
                        view.focus_tries = FOCUS_TRIES;
                        view.scroll.set_offset(Point::default());
                        CompareState::Ready { comparison, lines }
                    }
                    Err(_) => CompareState::Failed("The request stopped before it finished".into()),
                };
                cx.notify();
            });
        });
        self.state = CompareState::Loading { right, _task: task };
    }

    fn toggle_env_values(&mut self, cx: &mut Context<Self>) {
        let CompareState::Ready { comparison, .. } = &self.state else {
            return;
        };
        let right = comparison.right.clone();
        self.env = match self.env {
            EnvValues::Hidden => EnvValues::Shown,
            EnvValues::Shown => EnvValues::Hidden,
        };
        self.load(right, cx);
        cx.notify();
    }

    fn change_namespace(&mut self, cx: &mut Context<Self>) {
        self.state = CompareState::Pick;
        self.focus_tries = FOCUS_TRIES;
        cx.notify();
    }

    /// Page Up, Page Down, Home and End scroll the lines like the drawer: nothing else in the dialog
    /// takes them while the comparison shows.
    fn scroll_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        if !matches!(self.state, CompareState::Ready { .. }) {
            return;
        }
        let step = match event.keystroke.key.as_str() {
            "pagedown" => DrawerScroll::PageDown,
            "pageup" => DrawerScroll::PageUp,
            "home" => DrawerScroll::Top,
            "end" => DrawerScroll::Bottom,
            _ => return,
        };
        let offset = self.scroll.offset();
        let y = scrolled_offset(
            offset.y.into(),
            self.scroll.bounds().size.height.into(),
            self.scroll.max_offset().y.into(),
            step,
        );
        self.scroll.set_offset(point(offset.x, px(y)));
        cx.notify();
    }

    /// Gives the focus to what the phase shows (the filter, or the lines) while the tries last.
    fn focus_phase(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let target = match self.state {
            CompareState::Pick => self.filter.read(cx).focus_handle(cx),
            CompareState::Ready { .. } => self.focus_handle.clone(),
            CompareState::Loading { .. } | CompareState::Failed(_) => return,
        };
        if self.focus_tries == 0 {
            return;
        }
        self.focus_tries -= 1;
        if target.is_focused(window) {
            self.focus_tries = 0;
            return;
        }
        window.focus(&target, cx);
        // The next frame checks that the focus stayed.
        cx.notify();
    }

    fn toggle_diff(&mut self, kind: ObjectKind, name: &str, cx: &mut Context<Self>) {
        let key = (kind, name.to_owned());
        if !self.open_diffs.remove(&key) {
            self.open_diffs.insert(key);
        }
        if let CompareState::Ready { comparison, lines } = &mut self.state {
            *lines = compare_lines(comparison, &self.open_diffs);
        }
        cx.notify();
    }

    /// The lines of the Ready state, for the tests that drive the dialog.
    #[cfg(test)]
    pub(crate) fn ready_lines(&self) -> Option<&[CompareLine]> {
        match &self.state {
            CompareState::Ready { lines, .. } => Some(lines),
            _ => None,
        }
    }

    /// Picks `right` as a click on its row would, for the tests that drive the dialog.
    #[cfg(test)]
    pub(crate) fn choose_for_test(&mut self, right: &str, cx: &mut Context<Self>) {
        self.choose(right.to_owned(), cx);
    }

    /// Opens or closes the diff of one object as its button would, for the tests.
    #[cfg(test)]
    pub(crate) fn toggle_diff_for_test(
        &mut self,
        kind: ObjectKind,
        name: &str,
        cx: &mut Context<Self>,
    ) {
        self.toggle_diff(kind, name, cx);
    }

    fn render_toolbar(&self, cx: &Context<Self>) -> AnyElement {
        let (text, buttons) = match &self.state {
            CompareState::Pick => (format!("Compare {} with", self.left), false),
            CompareState::Loading { right, .. } => (format!("{} ↔ {right}", self.left), false),
            CompareState::Failed(_) => (format!("{} ↔ …", self.left), true),
            CompareState::Ready { comparison, .. } => (summary_text(comparison), true),
        };
        let is_loading = matches!(self.state, CompareState::Loading { .. });
        let has_env_toggle = match &self.state {
            CompareState::Ready { comparison, .. } => {
                comparison.hidden_env_values > 0 || self.env == EnvValues::Shown
            }
            _ => false,
        };
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
                    .text_sm()
                    .child(SharedString::from(text)),
            )
            .when(has_env_toggle, |bar| {
                bar.child(
                    Button::new("compare-env")
                        .label(if self.env == EnvValues::Shown {
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
            .when(buttons, |bar| {
                bar.child(
                    Button::new("compare-change")
                        .label("Change namespace")
                        .small()
                        .outline()
                        .on_click(cx.listener(|view, _, _, cx| view.change_namespace(cx))),
                )
            })
            .into_any_element()
    }

    fn render_pick(&self, cx: &Context<Self>) -> AnyElement {
        let text = self.filter.read(cx).value().to_string();
        let shown = matching_namespaces(&self.candidates, &text);
        let list = v_flex()
            .id("compare-candidates")
            .overflow_y_scroll()
            .gap_0p5();
        let list = if shown.is_empty() {
            list.child(
                div()
                    .px_2()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(if self.candidates.is_empty() {
                        "No namespaces are loaded; type a name and press Enter"
                    } else {
                        "No namespace matches"
                    }),
            )
        } else {
            let hover = cx.theme().list_hover;
            list.children(shown.into_iter().enumerate().map(|(index, name)| {
                let picked = name.clone();
                div()
                    .id(("compare-candidate", index))
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .cursor_pointer()
                    .text_sm()
                    .font_family(cx.theme().mono_font_family.clone())
                    .hover(|row| row.bg(hover))
                    .on_click(cx.listener(move |view, _, _, cx| view.choose(picked.clone(), cx)))
                    .child(SharedString::from(name.clone()))
            }))
        };
        v_flex()
            .size_full()
            .gap_2()
            .child(Input::new(&self.filter).small().cleanable(true))
            .child(div().flex_1().min_h_0().child(list))
            .into_any_element()
    }

    fn render_lines(&self, lines: &[CompareLine], cx: &Context<Self>) -> AnyElement {
        v_flex()
            .id("compare-lines")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .children(
                lines
                    .iter()
                    .enumerate()
                    .map(|(index, line)| self.render_line(index, line, cx)),
            )
            .into_any_element()
    }

    fn render_line(&self, index: usize, line: &CompareLine, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let muted = theme.muted_foreground;
        let mono = theme.mono_font_family.clone();
        match line {
            CompareLine::Kind { title, summary } => h_flex()
                .gap_2()
                .items_baseline()
                .pt_3()
                .pb_1()
                .border_b_1()
                .border_color(theme.border)
                .child(div().text_sm().font_semibold().child(*title))
                .child(
                    div()
                        .text_xs()
                        .text_color(muted)
                        .child(SharedString::from(summary.clone())),
                )
                .into_any_element(),
            CompareLine::Group(label) => div()
                .pt_2()
                .pl_3()
                .text_xs()
                .text_color(muted)
                .child(SharedString::from(label.clone()))
                .into_any_element(),
            CompareLine::Name(name) => indented(INDENT_NAME)
                .font_family(mono)
                .text_sm()
                .child(SharedString::from(name.clone()))
                .into_any_element(),
            CompareLine::Object {
                kind,
                name,
                is_open,
            } => {
                let (kind, shown) = (*kind, name.clone());
                indented(INDENT_NAME)
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .font_family(mono)
                                    .text_sm()
                                    .child(SharedString::from(name.clone())),
                            )
                            .child(
                                Button::new(("compare-diff", index))
                                    .ghost()
                                    .small()
                                    .label(if *is_open { "Hide diff" } else { "Open diff" })
                                    .on_click(cx.listener(move |view, _, _, cx| {
                                        view.toggle_diff(kind, &shown, cx);
                                    })),
                            ),
                    )
                    .into_any_element()
            }
            CompareLine::Change(text) => indented(INDENT_CHANGE)
                .font_family(mono)
                .text_xs()
                .child(SharedString::from(text.clone()))
                .into_any_element(),
            CompareLine::Diff(row) => indented(INDENT_NAME)
                .child(diff_row_element(row, cx))
                .into_any_element(),
            CompareLine::Note(text) => indented(INDENT_NAME)
                .text_xs()
                .text_color(muted)
                .child(SharedString::from(text.clone()))
                .into_any_element(),
        }
    }

    fn render_body(&self, cx: &Context<Self>) -> AnyElement {
        let muted = cx.theme().muted_foreground;
        match &self.state {
            CompareState::Pick => self.render_pick(cx),
            CompareState::Loading { right, .. } => centered(
                v_flex().items_center().gap_3().child(Spinner::new()).child(
                    div()
                        .text_sm()
                        .text_color(muted)
                        .child(format!("Reading {} and {right}…", self.left)),
                ),
            ),
            CompareState::Failed(message) => centered(
                div()
                    .text_sm()
                    .text_color(cx.theme().danger)
                    .child(message.clone()),
            ),
            CompareState::Ready { comparison, lines } => v_flex()
                .size_full()
                .children(hidden_env_note(comparison).map(|note| {
                    div()
                        .flex_shrink_0()
                        .pb_1()
                        .text_xs()
                        .text_color(muted)
                        .child(note)
                }))
                .child(div().flex_1().min_h_0().child(self.render_lines(lines, cx)))
                .into_any_element(),
        }
    }
}

fn indented(left: f32) -> Div {
    div().w_full().pl(px(left)).py_0p5()
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

impl Render for NamespaceCompareView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.focus_phase(window, cx);
        v_flex()
            .key_context(NAMESPACE_COMPARE)
            .track_focus(&self.focus_handle)
            .on_key_down(
                cx.listener(|view, event: &KeyDownEvent, _, cx| view.scroll_key(event, cx)),
            )
            .on_action(cx.listener(|view, _: &PickNamespace, _, cx| view.choose_first_match(cx)))
            .size_full()
            .child(self.render_toolbar(cx))
            .child(div().flex_1().min_h_0().child(self.render_body(cx)))
    }
}

/// The view in a dialog: it fills its parent, so the dialog gives it a share of the window high.
pub(crate) fn dialog_body(view: &Entity<NamespaceCompareView>, window: &Window) -> Div {
    div()
        .w_full()
        .h(window.viewport_size().height * HEIGHT_SHARE)
        .child(view.clone())
}

#[cfg(test)]
#[path = "namespace_compare_view_tests.rs"]
mod namespace_compare_view_tests;
