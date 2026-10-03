//! What the Edit values view draws (wireframe W7): header, warnings, banner, one row per key, the
//! Add key line, and the footer. A child of `values_edit` because it reads the view's state; it
//! changes none of it except through the buttons' handlers.
//!
//! A masked Secret field is drawn as a `•••• N chars` placeholder, never as the textarea.

use gpui_kit::assets::IconName;
use gpui_kit::component::alert::Alert;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{
    Copy as CopyText, Cut as CutText, Input, Textarea, TextareaState,
};
use gpui_kit::component::kbd::Kbd;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, Sizable as _, StyledExt as _, h_flex, v_flex,
};
use gpui_kit::{
    AnyElement, Context, Entity, InteractiveElement as _, IntoElement, KeyDownEvent,
    ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _, Window,
    div, prelude::FluentBuilder as _, px,
};

use super::{
    FieldDisplay, FieldKind, LoadState, Reveal, RowState, ValueRow, ValuesBanner, ValuesEditView,
    field_display,
};
use crate::keymap::{ApplyEdit, VALUES_EDIT};
use crate::secret_values::ValueAccess;

const NAME_WIDTH: f32 = 220.;
const STATE_WIDTH: f32 = 70.;
const BUTTONS_WIDTH: f32 = 170.;
/// How strongly a banner is tinted by its theme token.
const BANNER_TINT: f32 = 0.14;
const BLOCKED_TOOLTIP: &str = "Disabled in screenshot runs";

impl ValuesEditView {
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
                    .child(format!("Edit values · {}", self.object.kind_name())),
            )
            .child(
                div()
                    .text_lg()
                    .font_semibold()
                    .font_family(theme.mono_font_family.clone())
                    .child(name),
            )
            .children(
                self.secret_type
                    .clone()
                    .map(|secret_type| chip(secret_type, theme.muted_foreground, theme.border)),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(version),
            )
            .into_any_element()
    }

    /// The non-blocking lines of decision 14, as the kit's warning alerts.
    fn render_warnings(&self) -> Option<AnyElement> {
        if self.warnings.is_empty() || !matches!(self.load, LoadState::Ready) {
            return None;
        }
        Some(
            v_flex()
                .flex_shrink_0()
                .gap_1()
                .px_4()
                .pt_2()
                .children(
                    self.warnings.iter().enumerate().map(|(index, line)| {
                        Alert::warning(("values-warning", index), line.clone())
                    }),
                )
                .into_any_element(),
        )
    }

    /// The condition of the object over the rows, with the buttons that answer it.
    fn render_banner(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let banner = self.banner.as_ref()?;
        let theme = cx.theme();
        let (title, lines, reload, dismiss) = match banner {
            ValuesBanner::Conflict => (
                "The object changed since you opened it.".to_owned(),
                Vec::new(),
                true,
                false,
            ),
            ValuesBanner::Rebased { dropped } => (
                "Your changes were moved onto the newest version.".to_owned(),
                dropped.clone(),
                false,
                true,
            ),
            ValuesBanner::Deleted => (
                "The object was deleted. Your changes cannot be applied.".to_owned(),
                Vec::new(),
                false,
                false,
            ),
            ValuesBanner::OutcomeUnknown => (
                "The last apply got no answer, so the change may have been applied.".to_owned(),
                Vec::new(),
                true,
                false,
            ),
        };
        let buttons = h_flex()
            .gap_2()
            .when(reload, |buttons| {
                buttons
                    .child(
                        Button::new("values-keep")
                            .label("Reload and keep my changes")
                            .small()
                            .primary()
                            .disabled(self.is_running())
                            .on_click(cx.listener(|view, _, window, cx| {
                                view.keep_my_changes(window, cx);
                            })),
                    )
                    .child(
                        Button::new("values-discard")
                            .label("Discard")
                            .small()
                            .outline()
                            .disabled(self.is_running())
                            .on_click(cx.listener(|view, _, window, cx| {
                                view.discard_my_changes(window, cx);
                            })),
                    )
            })
            .when(dismiss, |buttons| {
                buttons.child(
                    Button::new("values-dismiss")
                        .label("Dismiss")
                        .small()
                        .outline()
                        .on_click(cx.listener(|view, _, _, cx| view.dismiss_banner(cx))),
                )
            });
        Some(
            v_flex()
                .flex_shrink_0()
                .gap_1()
                .px_4()
                .py_2()
                .border_b_1()
                .border_color(theme.border)
                .bg(theme.warning.opacity(BANNER_TINT))
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

    fn render_body(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        match &self.load {
            LoadState::Loading { .. } => centered(
                v_flex()
                    .items_center()
                    .gap_3()
                    .child(Spinner::new())
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child("Reading the object…"),
                    )
                    .into_any_element(),
            ),
            LoadState::Failed(message) => div()
                .px_4()
                .py_2()
                .child(
                    Alert::error("values-load-error", message.clone())
                        .title("These values cannot be edited"),
                )
                .into_any_element(),
            LoadState::Ready => self.render_rows(window, cx),
        }
    }

    fn render_rows(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let rows = self
            .rows
            .iter()
            .enumerate()
            .map(|(index, row)| self.render_row(index, row, window, cx));
        div()
            .id("values-rows")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .child(
                v_flex()
                    .child(
                        h_flex()
                            .gap_3()
                            .px_4()
                            .py_1()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .border_b_1()
                            .border_color(theme.border)
                            .child(div().w(px(NAME_WIDTH)).child("Key"))
                            .child(div().w(px(STATE_WIDTH)).child("State"))
                            .child(div().flex_1().child("Value"))
                            .child(div().w(px(BUTTONS_WIDTH))),
                    )
                    .children(rows)
                    .child(self.render_add(cx)),
            )
            .into_any_element()
    }

    fn render_row(
        &self,
        index: usize,
        row: &ValueRow,
        window: &Window,
        cx: &Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let state = row.state().map(|state| {
            let (text, color) = match state {
                RowState::Added => ("added", theme.success),
                RowState::Changed => ("changed", theme.warning),
                RowState::Removed => ("removed", theme.danger),
            };
            chip(text.into(), color, color)
        });
        let error = self
            .row_errors
            .iter()
            .find(|(key, _)| key == &row.name)
            .map(|(_, text)| div().text_xs().text_color(theme.danger).child(text.clone()));
        h_flex()
            .gap_3()
            .px_4()
            .py_2()
            .items_start()
            .border_b_1()
            .border_color(theme.border)
            .child(
                div()
                    .id(("values-name", index))
                    .w(px(NAME_WIDTH))
                    .flex_shrink_0()
                    .truncate()
                    .text_sm()
                    .font_family(theme.mono_font_family.clone())
                    .child(row.name.clone()),
            )
            .child(div().w(px(STATE_WIDTH)).flex_shrink_0().children(state))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_1()
                    .child(self.render_value(index, row, window, cx))
                    .children(error),
            )
            .child(self.render_buttons(index, row, cx))
            .into_any_element()
    }

    /// The value cell: the text of a ConfigMap, a masked or shown Secret field, or what a key that
    /// can only be removed reads as.
    fn render_value(
        &self,
        index: usize,
        row: &ValueRow,
        window: &Window,
        cx: &Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let muted = |text: SharedString| {
            div()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child(text)
                .into_any_element()
        };
        if row.is_removed() {
            return muted("will be removed".into());
        }
        match &row.field {
            FieldKind::Binary { size_bytes } => muted(format!("binary, {size_bytes} bytes").into()),
            FieldKind::Large { size_bytes } => muted(
                format!(
                    "text, {} KiB · edit with Edit YAML",
                    size_bytes.div_ceil(1024)
                )
                .into(),
            ),
            FieldKind::Text { field, .. } => text_field(field, window),
            FieldKind::Secret { field, reveal } => {
                let char_count = field.read(cx).text().chars().count();
                match field_display(*reveal, char_count) {
                    FieldDisplay::Masked(text) => div()
                        .id(("values-mask", index))
                        .px_2()
                        .py_1()
                        .rounded(theme.radius)
                        .border_1()
                        .border_color(theme.border)
                        .text_sm()
                        .font_family(theme.mono_font_family.clone())
                        .text_color(theme.muted_foreground)
                        .child(text)
                        .into_any_element(),
                    FieldDisplay::Editor => secret_field(field),
                }
            }
        }
    }

    fn render_buttons(&self, index: usize, row: &ValueRow, cx: &Context<Self>) -> AnyElement {
        let blocked = self.access == ValueAccess::Blocked;
        let mut buttons = h_flex()
            .w(px(BUTTONS_WIDTH))
            .flex_shrink_0()
            .gap_1()
            .justify_end();
        if let FieldKind::Secret { reveal, .. } = &row.field
            && !row.is_removed()
        {
            let is_shown = matches!(reveal, Reveal::Shown { .. });
            let name = row.name.clone();
            let tooltip = if blocked {
                BLOCKED_TOOLTIP
            } else if is_shown {
                "Hide this value"
            } else {
                "Show this field for 30 seconds"
            };
            buttons = buttons.child(
                Button::new(("values-eye", index))
                    .icon(Icon::new(if is_shown {
                        IconName::EyeOff
                    } else {
                        IconName::Eye
                    }))
                    .ghost()
                    .xsmall()
                    .disabled(blocked)
                    .tooltip(tooltip)
                    .on_click(cx.listener(move |view, _, window, cx| {
                        view.toggle_reveal(&name, window, cx);
                    })),
            );
            if !is_shown {
                let name = row.name.clone();
                buttons = buttons.child(
                    Button::new(("values-paste", index))
                        .label("Paste")
                        .ghost()
                        .xsmall()
                        .tooltip("Paste the clipboard text into this field without showing it")
                        .on_click(cx.listener(move |view, _, window, cx| {
                            view.paste_into(&name, window, cx);
                        })),
                );
            }
        }
        let name = row.name.clone();
        let label = if row.is_removed() { "Undo" } else { "Remove" };
        buttons
            .child(
                Button::new(("values-remove", index))
                    .label(label)
                    .ghost()
                    .xsmall()
                    .on_click(cx.listener(move |view, _, _, cx| view.toggle_remove(&name, cx))),
            )
            .into_any_element()
    }

    /// `+ Add key`: the name is checked here; the value is typed in the new row.
    fn render_add(&self, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let can_add = self.base.is_some() && !self.is_running();
        v_flex()
            .gap_1()
            .px_4()
            .py_3()
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .w(px(NAME_WIDTH))
                            .child(Input::new(&self.add_name).small()),
                    )
                    .child(
                        Button::new("values-add")
                            .icon(Icon::new(IconName::Plus))
                            .label("Add key")
                            .small()
                            .outline()
                            .disabled(!can_add)
                            .on_click(cx.listener(|view, _, window, cx| view.add_key(window, cx))),
                    ),
            )
            .children(
                self.add_error
                    .clone()
                    .map(|text| div().text_xs().text_color(theme.danger).child(text)),
            )
            .into_any_element()
    }

    fn render_footer(&self, window: &Window, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (status, tone) = match &self.footer_error {
            Some(text) => (text.to_string(), theme.danger),
            None => {
                let count = self.change_count();
                let text = match count {
                    0 => "No changes".to_owned(),
                    1 => "1 change".to_owned(),
                    count => format!("{count} changes"),
                };
                (text, theme.muted_foreground)
            }
        };
        let is_deleted = matches!(self.banner, Some(ValuesBanner::Deleted));
        let apply_reason = if self.is_running() {
            Some("Reading the object…")
        } else if !self.is_dirty() {
            Some("No changes")
        } else if is_deleted {
            Some("The object was deleted")
        } else {
            None
        };
        let apply = Button::new("values-apply")
            .label("Apply…")
            .small()
            .primary()
            .disabled(apply_reason.is_some() || self.base.is_none())
            .tooltip(apply_reason.unwrap_or("Check the change with the server, then apply it"))
            .on_click(cx.listener(|view, _, window, cx| view.apply(window, cx)));
        let key = Kbd::binding_for_action(&ApplyEdit, Some(VALUES_EDIT), window);
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
                    .id("values-status")
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_sm()
                    .text_color(tone)
                    .child(status),
            )
            .child(
                Button::new("values-cancel")
                    .label(if matches!(self.load, LoadState::Failed(_)) {
                        "Close"
                    } else {
                        "Cancel"
                    })
                    .small()
                    .outline()
                    .on_click(cx.listener(|view, _, _, cx| view.cancel(cx))),
            )
            .child(h_flex().gap_1().items_center().child(apply).children(key))
            .into_any_element()
    }
}

/// A ConfigMap text field: the kit textarea, copying as usual.
fn text_field(field: &Entity<TextareaState>, _window: &Window) -> AnyElement {
    Textarea::new(field).w_full().into_any_element()
}

/// A shown Secret field. Copy and Cut are dropped before the textarea sees them, and its context
/// menu has no items: the text of a Secret never reaches the clipboard from here.
fn secret_field(field: &Entity<TextareaState>) -> AnyElement {
    div()
        .capture_action(|_: &CopyText, _, cx| cx.stop_propagation())
        .capture_action(|_: &CutText, _, cx| cx.stop_propagation())
        .child(
            Textarea::new(field)
                .w_full()
                .context_menu(|menu, _, _| menu),
        )
        .into_any_element()
}

/// A small outlined label: the state of a row, the type of a Secret.
fn chip(text: SharedString, text_color: gpui_kit::Hsla, border: gpui_kit::Hsla) -> AnyElement {
    div()
        .flex_none()
        .px_1()
        .rounded_sm()
        .border_1()
        .border_color(border)
        .text_xs()
        .text_color(text_color)
        .child(text)
        .into_any_element()
}

fn centered(content: AnyElement) -> AnyElement {
    v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .child(content)
        .into_any_element()
}

impl Render for ValuesEditView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = self.render_body(window, cx);
        v_flex()
            .key_context(VALUES_EDIT)
            .track_focus(&self.focus_handle)
            .capture_key_down(
                cx.listener(|view, event: &KeyDownEvent, _, _| view.note_key_down(event)),
            )
            .on_action(cx.listener(|view, _: &ApplyEdit, window, cx| {
                view.apply_from_key(window, cx);
                // A handled action ends the key event before the key-down listeners, which tell a
                // held key from a fresh one: let the event go on.
                cx.propagate();
            }))
            .size_full()
            .min_h_0()
            .child(self.render_header(cx))
            .children(self.render_warnings())
            .children(self.render_banner(cx))
            .child(body)
            .child(self.render_footer(window, cx))
    }
}
