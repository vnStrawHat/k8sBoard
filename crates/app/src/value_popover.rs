//! The value popover (spec 0032): the one small form that asks for a number before a guarded
//! change. Today it asks for replicas, for the menu `Scale…`, the ⇧S key, and a palette Enter on
//! Scale; later specs add forms. It floats over the bottom of the workspace where the selection
//! bar sits, and it never sends anything itself: Scale hands the number to the shell, which builds
//! the intent and opens the confirm dialog.

use cluster::ObjectKind;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{InputEvent, InputState, NumberInput};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Sizable as _, StyledExt as _, h_flex, v_flex,
};
use gpui_kit::{
    App, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, SharedString, Styled as _, Subscription, WeakEntity, Window, div,
    px,
};

use crate::app_shell::AppShell;
use crate::keymap::CancelValuePopover;
use crate::table_selection::ClusterObject;
use crate::workload_actions::{
    ReplicasInput, ScaleTarget, parse_replicas, replicas_input, scale_warnings,
};

const POPOVER_WIDTH: f32 = 320.;

/// What the popover asks for. One form today; the others join it with their specs.
pub(crate) enum ValueForm {
    Replicas { input: Entity<InputState> },
}

/// What the typed value is for.
pub(crate) enum ValueTargets {
    /// The cursor row, in its own cluster.
    One {
        object: ClusterObject,
        target: Box<ScaleTarget>,
    },
    /// The ticked rows of one kind and one cluster; the shell reads them again when the count is
    /// submitted, so the batch names what is ticked then.
    Ticked { kind: ObjectKind, count: usize },
}

pub(crate) struct ValuePopover {
    shell: WeakEntity<AppShell>,
    form: ValueForm,
    targets: ValueTargets,
    _subscription: Subscription,
}

impl ValuePopover {
    /// A Scale popover for one row, prefilled with the replicas it has now.
    pub(crate) fn scale_one(
        shell: WeakEntity<AppShell>,
        object: ClusterObject,
        target: ScaleTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let initial = target.desired.to_string();
        Self::open(
            shell,
            ValueTargets::One {
                object,
                target: Box::new(target),
            },
            &initial,
            window,
            cx,
        )
    }

    /// A Scale popover for `count` ticked rows of `kind`, empty: one count for all of them.
    pub(crate) fn scale_ticked(
        shell: WeakEntity<AppShell>,
        kind: ObjectKind,
        count: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::open(shell, ValueTargets::Ticked { kind, count }, "", window, cx)
    }

    fn open(
        shell: WeakEntity<AppShell>,
        targets: ValueTargets,
        initial: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Replicas")
                // Digits only: a sign or a fraction cannot be typed, and the stepper stops at 0.
                .validate(|text, _| text.bytes().all(|byte| byte.is_ascii_digit()))
                .min(0.)
                .step(1.)
                .default_value(initial.to_owned())
        });
        let subscription =
            cx.subscribe_in(
                &input,
                window,
                |popover, _, event, window, cx| match event {
                    InputEvent::PressEnter { .. } => popover.submit(window, cx),
                    InputEvent::Change => cx.notify(),
                    _ => {}
                },
            );
        input.update(cx, |input, cx| input.focus(window, cx));
        Self {
            shell,
            form: ValueForm::Replicas { input },
            targets,
            _subscription: subscription,
        }
    }

    fn typed(&self, cx: &App) -> SharedString {
        match &self.form {
            ValueForm::Replicas { input } => input.read(cx).value(),
        }
    }

    /// The row of a one-row popover as it is now, so the unchanged check and the "Now" line follow
    /// a rollout or an HPA that changed the count while the form was open. The row the form opened
    /// on stands in when it is no longer listed; the submit refuses that case with a notice.
    fn current_target(&self, cx: &App) -> Option<ScaleTarget> {
        let ValueTargets::One { object, target } = &self.targets else {
            return None;
        };
        let current = self
            .shell
            .upgrade()
            .and_then(|shell| shell.read(cx).scale_target_of(object, cx));
        Some(current.unwrap_or_else(|| (**target).clone()))
    }

    /// What the Scale button would do with the text now. Several rows have no common count to
    /// compare with, so only an empty or invalid text turns the button off there.
    fn choice(&self, cx: &App) -> ReplicasInput {
        let text = self.typed(cx);
        match self.current_target(cx) {
            Some(target) => replicas_input(&text, target.desired),
            None => match parse_replicas(&text) {
                Some(replicas) => ReplicasInput::Set(replicas),
                None => ReplicasInput::Invalid,
            },
        }
    }

    /// `Scale deployment/api`, or `Scale 3 deployments`.
    fn title(&self) -> String {
        match &self.targets {
            ValueTargets::One { target, .. } => format!("Scale {}", target.subject_text()),
            ValueTargets::Ticked { kind, count } => {
                format!("Scale {count} {}s", kind.name().to_ascii_lowercase())
            }
        }
    }

    /// The muted line under the field.
    fn state_text(&self, cx: &App) -> String {
        match &self.targets {
            ValueTargets::One { .. } => self
                .current_target(cx)
                .map(|target| target.state_text())
                .unwrap_or_default(),
            ValueTargets::Ticked { count, .. } => format!("One count for the {count} ticked rows"),
        }
    }

    /// Scale, or Enter: hands a new whole number to the shell. Nothing happens for an empty,
    /// invalid, or unchanged value.
    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ReplicasInput::Set(replicas) = self.choice(cx) else {
            return;
        };
        let _ = self.shell.update(cx, |shell, cx| match &self.targets {
            ValueTargets::One { object, target } => {
                shell.submit_scale(object, target, replicas, window, cx);
            }
            ValueTargets::Ticked { kind, .. } => {
                shell.submit_bulk_scale(*kind, replicas, window, cx);
            }
        });
    }

    fn cancel(&mut self, cx: &mut Context<Self>) {
        let _ = self
            .shell
            .update(cx, |shell, cx| shell.close_value_popover(cx));
    }

    /// The warnings of the typed number, shown while it is typed. A batch shows its own in the
    /// dialog, which knows each row.
    fn warnings(&self, cx: &App) -> Vec<SharedString> {
        match (self.current_target(cx), self.choice(cx)) {
            (Some(target), ReplicasInput::Set(replicas)) => scale_warnings(&target, replicas),
            _ => Vec::new(),
        }
    }
}

impl Render for ValuePopover {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, warning) = (theme.muted_foreground, theme.warning);
        let title = self.title();
        let state = self.state_text(cx);
        let can_submit = matches!(self.choice(cx), ReplicasInput::Set(_));
        let ValueForm::Replicas { input } = &self.form;
        v_flex()
            .key_context("ValuePopover")
            .on_action(cx.listener(|popover, _: &CancelValuePopover, _, cx| popover.cancel(cx)))
            .w(px(POPOVER_WIDTH))
            .gap_2()
            .p_3()
            .rounded_lg()
            .border_1()
            .border_color(theme.border)
            .bg(theme.popover)
            .shadow_md()
            .child(div().text_sm().font_semibold().truncate().child(title))
            .child(NumberInput::new(input))
            .child(div().text_xs().text_color(muted).child(state))
            .children(
                self.warnings(cx)
                    .into_iter()
                    .map(|line| div().text_xs().text_color(warning).child(line)),
            )
            .child(
                h_flex()
                    .gap_2()
                    .justify_end()
                    .child(
                        Button::new("value-cancel")
                            .label("Cancel")
                            .small()
                            .outline()
                            .on_click(cx.listener(|popover, _, _, cx| popover.cancel(cx))),
                    )
                    .child(
                        Button::new("value-submit")
                            .label("Scale")
                            .small()
                            .primary()
                            .disabled(!can_submit)
                            .on_click(
                                cx.listener(|popover, _, window, cx| popover.submit(window, cx)),
                            ),
                    ),
            )
    }
}

/// What the shell tests read from, and do to, an open popover.
#[cfg(test)]
impl ValuePopover {
    pub(crate) fn type_text(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        let ValueForm::Replicas { input } = &self.form;
        input.update(cx, |input, cx| input.set_value(text.to_owned(), window, cx));
    }

    pub(crate) fn typed_text(&self, cx: &App) -> String {
        self.typed(cx).to_string()
    }

    /// Whether the Scale button is enabled.
    pub(crate) fn can_submit(&self, cx: &App) -> bool {
        matches!(self.choice(cx), ReplicasInput::Set(_))
    }

    pub(crate) fn press_submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.submit(window, cx);
    }

    pub(crate) fn state_line(&self, cx: &App) -> String {
        self.state_text(cx)
    }

    pub(crate) fn warning_lines(&self, cx: &App) -> Vec<SharedString> {
        self.warnings(cx)
    }
}
