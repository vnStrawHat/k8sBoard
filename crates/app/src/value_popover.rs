//! The value popover (specs 0032 and 0032b): the one small form that asks for a number before a
//! guarded change. Each form owns its inputs: the replicas of Scale, the min / max of an HPA, and the
//! size of a PVC. It
//! floats over the bottom of the workspace where the selection bar sits, and it never sends anything
//! itself: a submit hands the values to the shell, which builds the intent and opens the confirm
//! dialog.

use cluster::{HorizontalPodAutoscalerSummary, ObjectKind, PersistentVolumeClaimSummary};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState, NumberInput};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Sizable as _, StyledExt as _, h_flex, v_flex,
};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, SharedString, Styled as _, Subscription, WeakEntity, Window, div,
    px,
};

use crate::app_shell::AppShell;
use crate::keymap::CancelValuePopover;
use crate::resource_edits::{
    RangeInput, StorageInput, claim_floor, claim_state_text, expand_warnings, hpa_range_warnings,
    hpa_state_text, range_input, storage_input,
};
use crate::status_tone::{StatusTone, tone_color};
use crate::table_selection::ClusterObject;
use crate::workload_actions::{
    ReplicasInput, ScaleTarget, parse_replicas, replicas_input, scale_warnings,
};

const POPOVER_WIDTH: f32 = 320.;

/// What the popover asks for, with the inputs it owns.
pub(crate) enum ValueForm {
    Replicas {
        input: Entity<InputState>,
    },
    ReplicaRange {
        min: Entity<InputState>,
        max: Entity<InputState>,
    },
    Storage {
        input: Entity<InputState>,
    },
}

/// What the typed values are for.
pub(crate) enum ValueTargets {
    /// The cursor row, in its own cluster.
    One {
        object: ClusterObject,
        target: Box<ScaleTarget>,
    },
    /// The ticked rows of one kind and one cluster; the shell reads them again when the count is
    /// submitted, so the batch names what is ticked then.
    Ticked { kind: ObjectKind, count: usize },
    /// The cursor HPA, in its own cluster.
    HpaOne {
        object: ClusterObject,
        hpa: Box<HorizontalPodAutoscalerSummary>,
    },
    /// The ticked HPAs.
    HpaTicked { count: usize },
    /// The cursor claim, in its own cluster.
    ClaimOne {
        object: ClusterObject,
        claim: Box<PersistentVolumeClaimSummary>,
    },
    /// The ticked claims.
    ClaimTicked { count: usize },
}

pub(crate) struct ValuePopover {
    shell: WeakEntity<AppShell>,
    form: ValueForm,
    targets: ValueTargets,
    _subscriptions: Vec<Subscription>,
}

/// A number field that takes digits only: a sign or a fraction cannot be typed, and the stepper
/// stops at `min`.
fn number_input(
    placeholder: &'static str,
    initial: &str,
    min: f64,
    window: &mut Window,
    cx: &mut Context<ValuePopover>,
) -> Entity<InputState> {
    cx.new(|cx| {
        InputState::new(window, cx)
            .placeholder(placeholder)
            .validate(|text, _| text.bytes().all(|byte| byte.is_ascii_digit()))
            .min(min)
            .step(1.)
            .default_value(initial.to_owned())
    })
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
        let targets = ValueTargets::One {
            object,
            target: Box::new(target),
        };
        Self::open_replicas(shell, targets, &initial, window, cx)
    }

    /// A Scale popover for `count` ticked rows of `kind`, empty: one count for all of them.
    pub(crate) fn scale_ticked(
        shell: WeakEntity<AppShell>,
        kind: ObjectKind,
        count: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let targets = ValueTargets::Ticked { kind, count };
        Self::open_replicas(shell, targets, "", window, cx)
    }

    /// An Edit min / max popover for one HPA, prefilled with the range it has now.
    pub(crate) fn hpa_range_one(
        shell: WeakEntity<AppShell>,
        object: ClusterObject,
        hpa: HorizontalPodAutoscalerSummary,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let (min, max) = (hpa.min_replicas.to_string(), hpa.max_replicas.to_string());
        let targets = ValueTargets::HpaOne {
            object,
            hpa: Box::new(hpa),
        };
        Self::open_range(shell, targets, &min, &max, window, cx)
    }

    /// An Edit min / max popover for `count` ticked HPAs, empty: one range for all of them.
    pub(crate) fn hpa_range_ticked(
        shell: WeakEntity<AppShell>,
        count: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let targets = ValueTargets::HpaTicked { count };
        Self::open_range(shell, targets, "", "", window, cx)
    }

    /// An Expand popover for one claim, prefilled with the size it has now.
    pub(crate) fn expand_one(
        shell: WeakEntity<AppShell>,
        object: ClusterObject,
        claim: PersistentVolumeClaimSummary,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let initial = claim_floor(&claim).unwrap_or_default().to_owned();
        let targets = ValueTargets::ClaimOne {
            object,
            claim: Box::new(claim),
        };
        Self::open_storage(shell, targets, &initial, window, cx)
    }

    /// An Expand popover for `count` ticked claims, empty: one size for all of them.
    pub(crate) fn expand_ticked(
        shell: WeakEntity<AppShell>,
        count: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let targets = ValueTargets::ClaimTicked { count };
        Self::open_storage(shell, targets, "", window, cx)
    }

    fn open_replicas(
        shell: WeakEntity<AppShell>,
        targets: ValueTargets,
        initial: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = number_input("Replicas", initial, 0., window, cx);
        let subscriptions = Self::listen(&[&input], window, cx);
        input.update(cx, |input, cx| input.focus(window, cx));
        Self {
            shell,
            form: ValueForm::Replicas { input },
            targets,
            _subscriptions: subscriptions,
        }
    }

    fn open_range(
        shell: WeakEntity<AppShell>,
        targets: ValueTargets,
        min: &str,
        max: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        // Both fields start at 1: the API refuses a minimum of 0.
        let min = number_input("Min", min, 1., window, cx);
        let max = number_input("Max", max, 1., window, cx);
        let subscriptions = Self::listen(&[&min, &max], window, cx);
        min.update(cx, |input, cx| input.focus(window, cx));
        Self {
            shell,
            form: ValueForm::ReplicaRange { min, max },
            targets,
            _subscriptions: subscriptions,
        }
    }

    fn open_storage(
        shell: WeakEntity<AppShell>,
        targets: ValueTargets,
        initial: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("150Gi")
                .default_value(initial.to_owned())
        });
        let subscriptions = Self::listen(&[&input], window, cx);
        input.update(cx, |input, cx| input.focus(window, cx));
        Self {
            shell,
            form: ValueForm::Storage { input },
            targets,
            _subscriptions: subscriptions,
        }
    }

    /// Enter in a field submits, and a change redraws the button and the warnings.
    fn listen(
        inputs: &[&Entity<InputState>],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<Subscription> {
        inputs
            .iter()
            .map(|input| {
                cx.subscribe_in(
                    *input,
                    window,
                    |popover, _, event, window, cx| match event {
                        InputEvent::PressEnter { .. } => popover.submit(window, cx),
                        InputEvent::Change => cx.notify(),
                        _ => {}
                    },
                )
            })
            .collect()
    }

    fn typed(&self, cx: &App) -> SharedString {
        match &self.form {
            ValueForm::Replicas { input } => input.read(cx).value(),
            ValueForm::ReplicaRange { min, .. } => min.read(cx).value(),
            ValueForm::Storage { input } => input.read(cx).value(),
        }
    }

    /// The row of a one-row Scale popover as it is now, so the unchanged check and the "Now" line
    /// follow a rollout or an HPA that changed the count while the form was open. The row the form
    /// opened on stands in when it is no longer listed; the submit refuses that case with a notice.
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

    /// The HPA of a one-row range popover as it is now; the row the form opened on stands in when
    /// it is no longer listed.
    fn current_hpa(&self, cx: &App) -> Option<HorizontalPodAutoscalerSummary> {
        let ValueTargets::HpaOne { object, hpa } = &self.targets else {
            return None;
        };
        let current = self
            .shell
            .upgrade()
            .and_then(|shell| shell.read(cx).hpa_of(object, cx));
        Some(current.unwrap_or_else(|| (**hpa).clone()))
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

    /// What the Set limits button would do with the two fields now. Several rows have no common
    /// range to compare with.
    fn range_choice(&self, cx: &App) -> RangeInput {
        let ValueForm::ReplicaRange { min, max } = &self.form else {
            return RangeInput::Incomplete;
        };
        let current = self
            .current_hpa(cx)
            .map(|hpa| (hpa.min_replicas, hpa.max_replicas));
        range_input(&min.read(cx).value(), &max.read(cx).value(), current)
    }

    /// The claim of a one-row Expand popover as it is now; the row the form opened on stands in
    /// when it is no longer listed.
    fn current_claim(&self, cx: &App) -> Option<PersistentVolumeClaimSummary> {
        let ValueTargets::ClaimOne { object, claim } = &self.targets else {
            return None;
        };
        let current = self
            .shell
            .upgrade()
            .and_then(|shell| shell.read(cx).claim_of(object, cx));
        Some(current.unwrap_or_else(|| (**claim).clone()))
    }

    /// What the Expand button would do with the text now. Several rows share no size to compare
    /// with, so any positive size passes there and the batch skips what it does not grow.
    fn storage_choice(&self, cx: &App) -> StorageInput {
        let ValueForm::Storage { input } = &self.form else {
            return StorageInput::Incomplete;
        };
        let claim = self.current_claim(cx);
        let floor = claim.as_ref().and_then(claim_floor);
        storage_input(&input.read(cx).value(), floor)
    }

    fn is_submittable(&self, cx: &App) -> bool {
        match &self.form {
            ValueForm::Replicas { .. } => matches!(self.choice(cx), ReplicasInput::Set(_)),
            ValueForm::ReplicaRange { .. } => {
                matches!(self.range_choice(cx), RangeInput::Set { .. })
            }
            ValueForm::Storage { .. } => matches!(self.storage_choice(cx), StorageInput::Set(_)),
        }
    }

    /// `Scale deployment/api`, or `Scale 3 deployments`.
    fn title(&self) -> String {
        match &self.targets {
            ValueTargets::One { target, .. } => format!("Scale {}", target.subject_text()),
            ValueTargets::Ticked { kind, count } => {
                format!("Scale {count} {}s", kind.name().to_ascii_lowercase())
            }
            ValueTargets::HpaOne { hpa, .. } => format!("Edit min / max of hpa/{}", hpa.name),
            ValueTargets::HpaTicked { count } => format!("Edit min / max of {count} hpas"),
            ValueTargets::ClaimOne { claim, .. } => format!("Expand claim {}", claim.name),
            ValueTargets::ClaimTicked { count } => format!("Expand {count} claims"),
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
            ValueTargets::HpaOne { .. } => self
                .current_hpa(cx)
                .map(|hpa| hpa_state_text(&hpa))
                .unwrap_or_default(),
            ValueTargets::HpaTicked { count } => format!("One range for the {count} ticked rows"),
            ValueTargets::ClaimOne { .. } => self
                .current_claim(cx)
                .map(|claim| claim_state_text(&claim))
                .unwrap_or_default(),
            ValueTargets::ClaimTicked { count } => format!("One size for the {count} ticked rows"),
        }
    }

    /// The label of the submit button.
    fn submit_label(&self) -> &'static str {
        match &self.form {
            ValueForm::Replicas { .. } => "Scale",
            ValueForm::ReplicaRange { .. } => "Set limits",
            ValueForm::Storage { .. } => "Expand",
        }
    }

    /// Scale, Set limits, or Enter: hands the new values to the shell. Nothing happens for an
    /// empty, invalid, or unchanged value.
    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match &self.form {
            ValueForm::Replicas { .. } => {
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
                    ValueTargets::HpaOne { .. }
                    | ValueTargets::HpaTicked { .. }
                    | ValueTargets::ClaimOne { .. }
                    | ValueTargets::ClaimTicked { .. } => {}
                });
            }
            ValueForm::ReplicaRange { .. } => {
                let RangeInput::Set { min, max } = self.range_choice(cx) else {
                    return;
                };
                let _ = self.shell.update(cx, |shell, cx| match &self.targets {
                    ValueTargets::HpaOne { object, .. } => {
                        shell.submit_hpa_range(object, min, max, window, cx);
                    }
                    ValueTargets::HpaTicked { .. } => {
                        shell.submit_bulk_hpa_range(min, max, window, cx);
                    }
                    ValueTargets::One { .. }
                    | ValueTargets::Ticked { .. }
                    | ValueTargets::ClaimOne { .. }
                    | ValueTargets::ClaimTicked { .. } => {}
                });
            }
            ValueForm::Storage { .. } => {
                let StorageInput::Set(storage) = self.storage_choice(cx) else {
                    return;
                };
                let _ = self.shell.update(cx, |shell, cx| match &self.targets {
                    ValueTargets::ClaimOne { object, .. } => {
                        shell.submit_expand(object, &storage, window, cx);
                    }
                    ValueTargets::ClaimTicked { .. } => {
                        shell.submit_bulk_expand(storage, window, cx);
                    }
                    ValueTargets::One { .. }
                    | ValueTargets::Ticked { .. }
                    | ValueTargets::HpaOne { .. }
                    | ValueTargets::HpaTicked { .. } => {}
                });
            }
        }
    }

    fn cancel(&mut self, cx: &mut Context<Self>) {
        let _ = self
            .shell
            .update(cx, |shell, cx| shell.close_value_popover(cx));
    }

    /// The warnings of the typed values, shown while they are typed. A batch shows its own in the
    /// dialog, which knows each row.
    fn warnings(&self, cx: &App) -> Vec<SharedString> {
        match &self.form {
            ValueForm::Replicas { .. } => match (self.current_target(cx), self.choice(cx)) {
                (Some(target), ReplicasInput::Set(replicas)) => scale_warnings(&target, replicas),
                _ => Vec::new(),
            },
            ValueForm::ReplicaRange { .. } => match (self.current_hpa(cx), self.range_choice(cx)) {
                (Some(hpa), RangeInput::Set { min, max }) => hpa_range_warnings(&hpa, min, max),
                _ => Vec::new(),
            },
            ValueForm::Storage { .. } => match (self.current_claim(cx), self.storage_choice(cx)) {
                (Some(claim), StorageInput::Set(_)) => expand_warnings(&claim),
                _ => Vec::new(),
            },
        }
    }

    /// The reason the typed values are refused, under the field; `None` while they are fine or
    /// still incomplete.
    fn field_error(&self, cx: &App) -> Option<String> {
        match &self.form {
            ValueForm::Replicas { .. } => None,
            ValueForm::ReplicaRange { .. } => match self.range_choice(cx) {
                RangeInput::Refused(reason) => Some(reason.to_owned()),
                RangeInput::Incomplete | RangeInput::Unchanged | RangeInput::Set { .. } => None,
            },
            ValueForm::Storage { .. } => match self.storage_choice(cx) {
                StorageInput::Refused(reason) => Some(reason),
                StorageInput::Incomplete | StorageInput::Set(_) => None,
            },
        }
    }

    fn render_fields(&self) -> AnyElement {
        match &self.form {
            ValueForm::Replicas { input } => NumberInput::new(input).into_any_element(),
            ValueForm::Storage { input } => Input::new(input).into_any_element(),
            ValueForm::ReplicaRange { min, max } => h_flex()
                .gap_2()
                .child(
                    v_flex()
                        .flex_1()
                        .gap_1()
                        .child(div().text_xs().child("Min"))
                        .child(NumberInput::new(min)),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .gap_1()
                        .child(div().text_xs().child("Max"))
                        .child(NumberInput::new(max)),
                )
                .into_any_element(),
        }
    }
}

impl Render for ValuePopover {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, warning, danger) = (
            theme.muted_foreground,
            tone_color(StatusTone::Warn, cx),
            tone_color(StatusTone::Bad, cx),
        );
        let title = self.title();
        let state = self.state_text(cx);
        let can_submit = self.is_submittable(cx);
        let error = self.field_error(cx);
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
            .child(self.render_fields())
            .children(error.map(|reason| div().text_xs().text_color(danger).child(reason)))
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
                            .label(self.submit_label())
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
        let ValueForm::Replicas { input } = &self.form else {
            return;
        };
        input.update(cx, |input, cx| input.set_value(text.to_owned(), window, cx));
    }

    /// Types into the size field of an Expand popover.
    pub(crate) fn type_storage(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        let ValueForm::Storage { input } = &self.form else {
            return;
        };
        input.update(cx, |input, cx| input.set_value(text.to_owned(), window, cx));
    }

    pub(crate) fn typed_text(&self, cx: &App) -> String {
        self.typed(cx).to_string()
    }

    /// The Min and Max texts of a range popover.
    pub(crate) fn typed_range(&self, cx: &App) -> Option<(String, String)> {
        let ValueForm::ReplicaRange { min, max } = &self.form else {
            return None;
        };
        Some((
            min.read(cx).value().to_string(),
            max.read(cx).value().to_string(),
        ))
    }

    /// Whether the submit button is enabled.
    pub(crate) fn can_submit(&self, cx: &App) -> bool {
        self.is_submittable(cx)
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

    /// The reason shown under the fields.
    pub(crate) fn error_line(&self, cx: &App) -> Option<String> {
        self.field_error(cx)
    }

    pub(crate) fn title_text(&self) -> String {
        self.title()
    }
}

/// Fills the fields without a keyboard: the tests, and the fixed picture of `--screen
/// hpa-range-popover`.
#[cfg(any(test, feature = "screenshot"))]
impl ValuePopover {
    /// Types into the Min and Max fields of a range popover.
    pub(crate) fn type_range(
        &mut self,
        min_text: &str,
        max_text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ValueForm::ReplicaRange { min, max } = &self.form else {
            return;
        };
        min.update(cx, |input, cx| {
            input.set_value(min_text.to_owned(), window, cx)
        });
        max.update(cx, |input, cx| {
            input.set_value(max_text.to_owned(), window, cx)
        });
    }
}
