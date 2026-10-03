//! What the Edit YAML view draws (wireframe W10): header, tabs, banner, the editor or the diff, the
//! side panel of changes and checks, and the footer. A child of `yaml_edit` because it reads the
//! view's state; it changes none of it except through the buttons' handlers.

use std::ops::Range;

use gpui_kit::component::alert::Alert;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::Editor;
use gpui_kit::component::kbd::Kbd;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Selectable as _, Sizable as _, StyledExt as _, h_flex,
    v_flex,
};
use gpui_kit::{
    AnyElement, App, Context, Div, InteractiveElement as _, IntoElement, KeyDownEvent,
    ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _, Window,
    div, px, uniform_list,
};

use super::{
    EditBanner, EditTab, LoadState, PassedPreview, PreviewFailure, PreviewState, YamlEditView,
    elide_middle, footer_text,
};
use crate::drawer::truncated_text_with_tooltip;
use crate::keymap::{ApplyEdit, YAML_EDIT};
use crate::yaml_diff::{DiffRow, DiffRowKind};

const SIDE_PANEL_WIDTH: f32 = 280.;
/// How many characters of a path fit the side panel in the mono font; a longer one is cut in the
/// middle (the tooltip has all of it), so the field that changed stays visible.
const PATH_CHARS: usize = 34;
const DIFF_ROW_HEIGHT: f32 = 20.;
/// The width of a line-number column of the diff.
const LINE_NUMBER_WIDTH: f32 = 44.;
/// How strongly a removed or added row is tinted by its theme token.
const ROW_TINT: f32 = 0.14;

/// The buttons that answer a banner.
enum BannerAnswers {
    /// Reload and keep my changes, or discard them.
    Reload,
    DiscardOnly,
    Dismiss,
    None,
}

impl YamlEditView {
    fn render_header(&self, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let name = match self.object.namespace() {
            Some(namespace) => format!("{namespace}/{}", self.object.name()),
            None => self.object.name().to_owned(),
        };
        let version = self
            .resource_version
            .as_ref()
            .map(|version| format!("resourceVersion {version} · {}", self.cluster_name))
            .unwrap_or_else(|| self.cluster_name.to_string());
        let is_shown = self.env == cluster::EnvValues::Shown;
        let env_tooltip = if self.is_dirty {
            "Discard your changes to show env values"
        } else if is_shown {
            "Hide env values"
        } else {
            "Show the env values this editor hides"
        };
        h_flex()
            .flex_shrink_0()
            .gap_3()
            .px_4()
            .py_2()
            .items_baseline()
            .border_b_1()
            .border_color(theme.border)
            .child(
                div()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(self.object.kind_name().to_owned()),
            )
            .child(
                div()
                    .text_lg()
                    .font_semibold()
                    .font_family(theme.mono_font_family.clone())
                    .child(name),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(version),
            )
            .child(div().ml_auto())
            .child(
                Button::new("edit-env")
                    .label("Env values")
                    .small()
                    .outline()
                    .selected(is_shown)
                    .disabled(self.is_dirty || self.base.is_none() || self.is_running())
                    .tooltip(env_tooltip)
                    .on_click(cx.listener(|view, _, window, cx| {
                        view.toggle_env_values(window, cx);
                    })),
            )
            .child(
                Button::new("edit-format")
                    .label("Format")
                    .small()
                    .outline()
                    .disabled(self.base.is_none())
                    .tooltip("Sort the keys and tidy the text")
                    .on_click(cx.listener(|view, _, window, cx| view.format(window, cx))),
            )
            .into_any_element()
    }

    fn render_tabs(&self, cx: &Context<Self>) -> AnyElement {
        let diff_label = match &self.preview {
            PreviewState::Passed(passed) => {
                format!("Diff vs cluster · {}", passed.changes.len())
            }
            _ => "Diff vs cluster".to_owned(),
        };
        let selected = match self.tab {
            EditTab::Editor => 0,
            EditTab::Diff => 1,
        };
        TabBar::new("edit-tabs")
            .underline()
            .selected_index(selected)
            .on_click(cx.listener(|view, index: &usize, _, cx| {
                let tab = if *index == 0 {
                    EditTab::Editor
                } else {
                    EditTab::Diff
                };
                view.show_tab(tab, cx);
            }))
            .prefix(div().w_4())
            .child(Tab::new().label("Editor"))
            .child(Tab::new().label(diff_label))
            .into_any_element()
    }

    /// The condition of the object over the editor, with the buttons that answer it.
    fn render_banner(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let banner = self.banner.as_ref()?;
        let theme = cx.theme();
        let (title, lines, answers) = match banner {
            EditBanner::Conflict => (
                "The object changed since you opened it.".to_owned(),
                Vec::new(),
                BannerAnswers::Reload,
            ),
            EditBanner::Rebased {
                unreachable,
                overwritten,
            } => (
                "Your changes were moved onto the newest version.".to_owned(),
                unreachable.iter().chain(overwritten).cloned().collect(),
                BannerAnswers::Dismiss,
            ),
            EditBanner::Deleted => (
                "The object was deleted. Your text is kept but cannot be applied.".to_owned(),
                Vec::new(),
                BannerAnswers::None,
            ),
            EditBanner::Recreated => (
                "The object was deleted and created again. Your text was for the old one."
                    .to_owned(),
                Vec::new(),
                BannerAnswers::DiscardOnly,
            ),
            EditBanner::OutcomeUnknown => (
                "The last apply got no answer, so the change may have been applied.".to_owned(),
                Vec::new(),
                BannerAnswers::Reload,
            ),
        };
        let discard = Button::new("edit-discard")
            .label("Discard my changes")
            .small()
            .outline()
            .disabled(self.is_running())
            .on_click(cx.listener(|view, _, window, cx| {
                view.discard_my_changes(window, cx);
            }));
        let buttons = match answers {
            BannerAnswers::Reload => h_flex()
                .gap_2()
                .child(
                    Button::new("edit-keep")
                        .label("Reload and keep my changes")
                        .small()
                        .primary()
                        .disabled(self.is_running())
                        .on_click(cx.listener(|view, _, window, cx| {
                            view.keep_my_changes(window, cx);
                        })),
                )
                .child(discard),
            BannerAnswers::DiscardOnly => h_flex().gap_2().child(discard),
            BannerAnswers::Dismiss => h_flex().gap_2().child(
                Button::new("edit-dismiss")
                    .label("Dismiss")
                    .small()
                    .outline()
                    .on_click(cx.listener(|view, _, _, cx| view.dismiss_banner(cx))),
            ),
            BannerAnswers::None => h_flex(),
        };
        Some(
            v_flex()
                .flex_shrink_0()
                .gap_1()
                .px_4()
                .py_2()
                .border_b_1()
                .border_color(theme.border)
                .bg(theme.warning.opacity(ROW_TINT))
                .child(
                    h_flex()
                        .gap_3()
                        .items_center()
                        .child(div().text_sm().child(title))
                        .child(div().ml_auto())
                        .child(buttons),
                )
                .children(lines.into_iter().map(|line| {
                    div()
                        .text_xs()
                        .font_family(theme.mono_font_family.clone())
                        .child(line)
                }))
                .into_any_element(),
        )
    }

    fn render_body(&self, cx: &mut Context<Self>) -> AnyElement {
        match &self.load {
            LoadState::Loading { .. } => return busy("Reading the object…", cx),
            LoadState::Failed(message) => {
                let alert = Alert::error("edit-load-error", message.clone())
                    .title("Cannot read the object");
                return div().px_3().py_2().child(alert).into_any_element();
            }
            LoadState::Ready => {}
        }
        match self.tab {
            EditTab::Editor => Editor::new(&self.editor)
                .bordered(false)
                .text_xs()
                .size_full()
                .into_any_element(),
            EditTab::Diff => self.render_diff(cx),
        }
    }

    fn render_diff(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        match &self.preview {
            PreviewState::Running { .. } => busy("Server dry-run…", cx),
            PreviewState::Passed(passed) => {
                let count = passed.rows.len();
                uniform_list(
                    "edit-diff",
                    count,
                    cx.processor(|view, range: Range<usize>, _, cx| view.diff_rows_in(range, cx)),
                )
                .track_scroll(&self.diff_scroll)
                .size_full()
                .into_any_element()
            }
            PreviewState::NotChecked => muted_center(
                "Press Ctrl S to check the change with the server",
                theme.muted_foreground,
            ),
            PreviewState::Failed(failure) => {
                let text = match failure {
                    PreviewFailure::Local(error) => error.to_string(),
                    PreviewFailure::Invalid { message, .. } => message.to_string(),
                    PreviewFailure::Server(text) => text.to_string(),
                };
                muted_center(text, theme.danger)
            }
        }
    }

    /// The rows `range` of the passed preview, drawn on demand by the list.
    fn diff_rows_in(&self, range: Range<usize>, cx: &App) -> Vec<Div> {
        let PreviewState::Passed(passed) = &self.preview else {
            return Vec::new();
        };
        passed
            .rows
            .get(range)
            .unwrap_or_default()
            .iter()
            .map(|row| diff_row_element(row, cx))
            .collect()
    }

    fn render_side(&self, text: &str, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let heading = |text: String| {
            div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(text)
        };
        let mono = theme.mono_font_family.clone();
        let mut side = v_flex()
            .id("edit-side")
            .w(px(SIDE_PANEL_WIDTH))
            .flex_shrink_0()
            .h_full()
            .gap_2()
            .p_3()
            .border_l_1()
            .border_color(theme.border)
            .overflow_y_scroll();
        match &self.preview {
            PreviewState::Passed(passed) => {
                side = side.child(heading(changes_heading(passed)));
                for (index, change) in passed.changes.iter().enumerate() {
                    let shown =
                        |value: &Option<SharedString>| value.clone().unwrap_or_else(|| "—".into());
                    side =
                        side.child(
                            v_flex()
                                .gap_1()
                                .child(
                                    truncated_text_with_tooltip(
                                        ("edit-change", index),
                                        elide_middle(&change.path, PATH_CHARS),
                                        change.path.clone(),
                                    )
                                    .text_xs()
                                    .font_family(mono.clone()),
                                )
                                .child(div().text_xs().text_color(theme.muted_foreground).child(
                                    format!("{} → {}", shown(&change.old), shown(&change.new)),
                                )),
                        );
                }
                if passed.more_changes > 0 {
                    side = side.child(heading(format!("and {} more", passed.more_changes)));
                }
            }
            _ => side = side.child(heading("No changes checked yet".to_owned())),
        }
        side = side.child(heading("Checks".to_owned()));
        let (status, tone) = dry_run_line(&self.preview, text, cx);
        side = side.child(div().text_xs().text_color(tone).child(status));
        for line in &self.overwritten {
            side = side.child(
                div()
                    .text_xs()
                    .text_color(theme.warning)
                    .child(line.clone()),
            );
        }
        if let PreviewState::Passed(passed) = &self.preview {
            for check in &passed.checks {
                side = side.child(
                    div()
                        .text_xs()
                        .text_color(theme.warning)
                        .child(check.clone()),
                );
            }
        }
        if !self.server_changed.is_empty() {
            side = side.child(heading(
                "Changed on the server since you opened it".to_owned(),
            ));
            for (index, path) in self.server_changed.iter().enumerate() {
                side = side.child(
                    truncated_text_with_tooltip(
                        ("edit-server-change", index),
                        elide_middle(path, PATH_CHARS),
                        path.clone(),
                    )
                    .text_xs()
                    .font_family(mono.clone()),
                );
            }
        }
        if let PreviewState::Failed(PreviewFailure::Invalid { message, fields }) = &self.preview {
            side = side.child(
                div()
                    .text_xs()
                    .font_semibold()
                    .text_color(theme.danger)
                    .child("The change is invalid"),
            );
            side = side.child(
                div()
                    .text_xs()
                    .text_color(theme.danger)
                    .child(message.clone()),
            );
            for field in fields {
                side = side.child(
                    div()
                        .text_xs()
                        .font_family(mono.clone())
                        .text_color(theme.danger)
                        .child(field.clone()),
                );
            }
        }
        side.into_any_element()
    }

    fn render_footer(&self, text: &str, window: &Window, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (_, tone) = dry_run_line(&self.preview, text, cx);
        let status = footer_text(&self.preview, text);
        let apply_reason = if self.is_running() {
            Some("Waiting for the dry-run…")
        } else if !self.is_dirty {
            Some("No changes")
        } else {
            None
        };
        let apply = Button::new("edit-apply")
            .label("Apply…")
            .small()
            .primary()
            .disabled(apply_reason.is_some() || self.base.is_none())
            .tooltip(apply_reason.unwrap_or("Check the change with the server, then apply it"))
            .on_click(cx.listener(|view, _, window, cx| view.apply(window, cx)));
        let key = Kbd::binding_for_action(&ApplyEdit, Some(YAML_EDIT), window);
        h_flex()
            .flex_shrink_0()
            .gap_3()
            .px_4()
            .py_2()
            .items_center()
            .border_t_1()
            .border_color(theme.border)
            .child(
                div()
                    .id("edit-status")
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_sm()
                    .text_color(tone)
                    .child(status),
            )
            .child(
                Button::new("edit-cancel")
                    .label("Cancel")
                    .small()
                    .outline()
                    .on_click(cx.listener(|view, _, _, cx| view.cancel(cx))),
            )
            .child(h_flex().gap_1().items_center().child(apply).children(key))
            .into_any_element()
    }
}

impl Render for YamlEditView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let text = self.text(cx);
        let body = self.render_body(cx);
        v_flex()
            .key_context(YAML_EDIT)
            .track_focus(&self.focus_handle)
            .capture_key_down(
                cx.listener(|view, event: &KeyDownEvent, _, _| view.note_key_down(event)),
            )
            .on_action(cx.listener(|view, _: &ApplyEdit, window, cx| {
                view.apply_from_key(window, cx);
                // A handled action ends the key event before the key-down listeners, which tell a held
                // key from a fresh one: let the event go on.
                cx.propagate();
            }))
            .size_full()
            .min_h_0()
            .child(self.render_header(cx))
            .child(self.render_tabs(cx))
            .children(self.render_banner(cx))
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .child(div().flex_1().min_w_0().h_full().child(body))
                    .child(self.render_side(&text, cx)),
            )
            .child(self.render_footer(&text, window, cx))
    }
}

fn busy(text: &str, cx: &App) -> AnyElement {
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

fn muted_center(text: impl Into<SharedString>, color: gpui_kit::Hsla) -> AnyElement {
    v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .px_4()
        .child(div().text_sm().text_color(color).child(text.into()))
        .into_any_element()
}

/// One row of a line diff: line numbers, sign, and a tint from the theme's danger and success
/// tokens. Shared by the Edit YAML Diff tab and the revision diff dialog.
pub(crate) fn diff_row_element(row: &DiffRow, cx: &App) -> Div {
    let theme = cx.theme();
    let (danger, success, muted) = (theme.danger, theme.success, theme.muted_foreground);
    let base = h_flex()
        .w_full()
        .h(px(DIFF_ROW_HEIGHT))
        .items_center()
        .font_family(theme.mono_font_family.clone())
        .text_xs();
    let number = |line: Option<usize>| {
        div()
            .w(px(LINE_NUMBER_WIDTH))
            .flex_shrink_0()
            .pr_2()
            .text_right()
            .text_color(muted)
            .child(line.map(|line| line.to_string()).unwrap_or_default())
    };
    match row.kind {
        DiffRowKind::Folded { lines } => base
            .text_color(muted)
            .pl(px(LINE_NUMBER_WIDTH * 2.))
            .child(format!("··· {lines} unchanged lines")),
        DiffRowKind::Same => base
            .child(number(row.old_line))
            .child(number(row.new_line))
            .child(sign("", muted))
            .child(line_text(row)),
        DiffRowKind::Removed => base
            .bg(danger.opacity(ROW_TINT))
            .child(number(row.old_line))
            .child(number(row.new_line))
            .child(sign("−", danger))
            .child(line_text(row)),
        DiffRowKind::Added => base
            .bg(success.opacity(ROW_TINT))
            .child(number(row.old_line))
            .child(number(row.new_line))
            .child(sign("+", success))
            .child(line_text(row)),
    }
}

fn sign(text: &'static str, color: gpui_kit::Hsla) -> Div {
    div()
        .w(px(16.))
        .flex_shrink_0()
        .text_color(color)
        .child(text)
}

fn line_text(row: &DiffRow) -> Div {
    div()
        .flex_1()
        .min_w_0()
        .whitespace_nowrap()
        .overflow_hidden()
        .child(row.text.clone())
}

/// `2 changes`, or `1 change`.
fn changes_heading(passed: &PassedPreview) -> String {
    let count = passed.changes.len() + passed.more_changes;
    match count {
        0 => "No changes".to_owned(),
        1 => "1 change".to_owned(),
        count => format!("{count} changes"),
    }
}

/// The dry-run line of the Checks section and its tone.
fn dry_run_line(preview: &PreviewState, current: &str, cx: &App) -> (String, gpui_kit::Hsla) {
    let theme = cx.theme();
    match preview {
        PreviewState::Passed(passed) if passed.for_text == current => {
            ("Server dry-run passed".to_owned(), theme.success)
        }
        PreviewState::Passed(_) => (
            "Changed since the last check".to_owned(),
            theme.muted_foreground,
        ),
        PreviewState::Failed(PreviewFailure::Local(_) | PreviewFailure::Server(_)) => {
            (footer_text(preview, current), theme.danger)
        }
        PreviewState::Failed(PreviewFailure::Invalid { .. }) => {
            ("Server dry-run refused the change".to_owned(), theme.danger)
        }
        PreviewState::Running { .. } | PreviewState::NotChecked => {
            (footer_text(preview, current), theme.muted_foreground)
        }
    }
}
