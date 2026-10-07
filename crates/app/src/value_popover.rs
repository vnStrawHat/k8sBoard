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
    ImageInput, ImageTarget, ReplicasInput, ScaleTarget, image_input, image_warnings,
    parse_replicas, replicas_input, scale_warnings, tag_range,
};

const POPOVER_WIDTH: f32 = 320.;
/// Wide enough for a registry host, a repository path, and a tag.
const IMAGE_POPOVER_WIDTH: f32 = 440.;
/// What the Set image popover says about the pods, whatever the container.
const IMAGE_STATE_TEXT: &str = "The pods are replaced with the new image";

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
    /// Set image: the container picked (an index into the target's containers), its image, and the
    /// change cause.
    Image {
        selected: usize,
        image: Entity<InputState>,
        cause: Entity<InputState>,
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
    /// The cursor workload that takes Set image, in its own cluster.
    ImageOne {
        object: ClusterObject,
        target: Box<ImageTarget>,
    },
}

pub(crate) struct ValuePopover {
    shell: WeakEntity<AppShell>,
    form: ValueForm,
    targets: ValueTargets,
    /// Whether the user changed a field since the popover opened: a form opens prefilled with the
    /// value the object has now, which no rule should complain about before it is touched.
    is_edited: bool,
    _subscriptions: Vec<Subscription>,
}

/// Focuses a number field with its prefilled `text` selected, so typing replaces the value
/// instead of extending it (`0` over `1` gives `0`, not `01`).
fn focus_selected(
    input: &Entity<InputState>,
    text: &str,
    window: &mut Window,
    cx: &mut Context<ValuePopover>,
) {
    input.update(cx, |input, cx| {
        input.focus(window, cx);
        input.set_selected_range(0..text.len(), cx);
    });
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

    /// A Set image popover for one workload: the first container, its image with the tag selected.
    pub(crate) fn image_one(
        shell: WeakEntity<AppShell>,
        object: ClusterObject,
        target: ImageTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let initial = target.containers[0].image.clone();
        let image = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("registry/name:tag")
                .default_value(initial.clone())
        });
        let cause = cx.new(|cx| InputState::new(window, cx).placeholder("Change cause (optional)"));
        let subscriptions = Self::listen(&[&image, &cause], window, cx);
        let mut popover = Self {
            shell,
            form: ValueForm::Image {
                selected: 0,
                image,
                cause,
            },
            targets: ValueTargets::ImageOne {
                object,
                target: Box::new(target),
            },
            is_edited: false,
            _subscriptions: subscriptions,
        };
        popover.show_image(&initial, window, cx);
        popover
    }

    /// Puts `text` in the image field with its tag selected, and focuses the field.
    fn show_image(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        let ValueForm::Image { image, .. } = &self.form else {
            return;
        };
        image.update(cx, |input, cx| {
            input.set_value(text.to_owned(), window, cx);
            input.set_selected_range(tag_range(text), cx);
            input.focus(window, cx);
        });
    }

    /// The container buttons: another container shows its own image.
    fn pick_container(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let ValueTargets::ImageOne { target, .. } = &self.targets else {
            return;
        };
        let Some(container) = target.containers.get(index) else {
            return;
        };
        let text = container.image.clone();
        if let ValueForm::Image { selected, .. } = &mut self.form {
            *selected = index;
        }
        self.show_image(&text, window, cx);
        cx.notify();
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
        focus_selected(&input, initial, window, cx);
        Self {
            shell,
            form: ValueForm::Replicas { input },
            targets,
            is_edited: false,
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
        let min_text = min;
        let min = number_input("Min", min_text, 1., window, cx);
        let max = number_input("Max", max, 1., window, cx);
        let subscriptions = Self::listen(&[&min, &max], window, cx);
        focus_selected(&min, min_text, window, cx);
        Self {
            shell,
            form: ValueForm::ReplicaRange { min, max },
            targets,
            is_edited: false,
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
            is_edited: false,
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
                    |popover, input, event, window, cx| match event {
                        InputEvent::PressEnter { .. } => popover.submit(window, cx),
                        InputEvent::Change => {
                            popover.is_edited = true;
                            cx.notify();
                        }
                        // Tab into a number field selects its value, like the field that opens focused.
                        InputEvent::Focus
                            if matches!(
                                popover.form,
                                ValueForm::Replicas { .. } | ValueForm::ReplicaRange { .. }
                            ) =>
                        {
                            input.update(cx, |input, cx| {
                                let length = input.value().len();
                                input.set_selected_range(0..length, cx);
                            });
                        }
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
            ValueForm::Image { image, .. } => image.read(cx).value(),
        }
    }

    /// The workload of a Set image popover as it is now; the row the form opened on stands in when
    /// it is no longer listed, and the submit refuses that case with a notice.
    fn current_image_target(&self, cx: &App) -> Option<ImageTarget> {
        let ValueTargets::ImageOne { object, target } = &self.targets else {
            return None;
        };
        let current = self
            .shell
            .upgrade()
            .and_then(|shell| shell.read(cx).image_target_of(object, cx));
        Some(current.unwrap_or_else(|| (**target).clone()))
    }

    /// The container the Set image popover has picked, with the image it has now.
    fn picked_container(&self, cx: &App) -> Option<(String, String)> {
        let ValueForm::Image { selected, .. } = &self.form else {
            return None;
        };
        let target = self.current_image_target(cx)?;
        let container = target.containers.get(*selected)?;
        Some((container.name.clone(), container.image.clone()))
    }

    /// What the Set image button would do with the text now.
    fn image_choice(&self, cx: &App) -> ImageInput {
        match self.picked_container(cx) {
            Some((_, current)) => image_input(&self.typed(cx), &current),
            None => ImageInput::Invalid,
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
            ValueForm::Image { .. } => matches!(self.image_choice(cx), ImageInput::Set(_)),
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
            ValueTargets::ImageOne { target, .. } => {
                format!("Set image of {}", target.subject_text())
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
            ValueTargets::ImageOne { .. } => IMAGE_STATE_TEXT.to_owned(),
        }
    }

    /// The label of the submit button.
    fn submit_label(&self) -> &'static str {
        match &self.form {
            ValueForm::Replicas { .. } => "Scale",
            ValueForm::ReplicaRange { .. } => "Set limits",
            ValueForm::Storage { .. } => "Expand",
            ValueForm::Image { .. } => "Set image",
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
                    | ValueTargets::ClaimTicked { .. }
                    | ValueTargets::ImageOne { .. } => {}
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
                    | ValueTargets::ClaimTicked { .. }
                    | ValueTargets::ImageOne { .. } => {}
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
                    | ValueTargets::HpaTicked { .. }
                    | ValueTargets::ImageOne { .. } => {}
                });
            }
            ValueForm::Image { cause, .. } => {
                let ImageInput::Set(image) = self.image_choice(cx) else {
                    return;
                };
                let Some((container, _)) = self.picked_container(cx) else {
                    return;
                };
                let cause = cause.read(cx).value();
                let _ = self.shell.update(cx, |shell, cx| {
                    if let ValueTargets::ImageOne { object, .. } = &self.targets {
                        shell.submit_set_image(object, &container, &image, &cause, window, cx);
                    }
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
            ValueForm::Image { .. } => self
                .current_image_target(cx)
                .map(|target| image_warnings(&target))
                .unwrap_or_default(),
        }
    }

    /// The reason the typed values are refused, under the field; `None` while they are fine or
    /// still incomplete.
    fn field_error(&self, cx: &App) -> Option<String> {
        match &self.form {
            ValueForm::Replicas { .. } => None,
            ValueForm::Image { .. } => match self.image_choice(cx) {
                ImageInput::Invalid if !self.typed(cx).trim().is_empty() => {
                    Some("An image has no spaces".to_owned())
                }
                ImageInput::Invalid | ImageInput::Unchanged | ImageInput::Set(_) => None,
            },
            ValueForm::ReplicaRange { .. } => match self.range_choice(cx) {
                RangeInput::Refused(reason) => Some(reason.to_owned()),
                RangeInput::Incomplete | RangeInput::Unchanged | RangeInput::Set { .. } => None,
            },
            ValueForm::Storage { .. } => match self.storage_choice(cx) {
                StorageInput::Refused(reason) if self.is_edited => Some(reason),
                StorageInput::Refused(_) => None,
                StorageInput::Incomplete | StorageInput::Set(_) => None,
            },
        }
    }

    /// The container buttons of a Set image popover: one per container of the template, shown only
    /// when there is a choice.
    fn render_container_picker(
        &self,
        selected: usize,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let ValueTargets::ImageOne { target, .. } = &self.targets else {
            return None;
        };
        if target.containers.len() < 2 {
            return None;
        }
        let buttons = target
            .containers
            .iter()
            .enumerate()
            .map(|(index, container)| {
                let button = Button::new(("image-container", index))
                    .label(container.name.clone())
                    .small()
                    .on_click(cx.listener(move |popover, _, window, cx| {
                        popover.pick_container(index, window, cx);
                    }));
                if index == selected {
                    button.primary()
                } else {
                    button.outline()
                }
            });
        Some(
            v_flex()
                .gap_1()
                .child(div().text_xs().child("Container"))
                .child(h_flex().gap_1().flex_wrap().children(buttons))
                .into_any_element(),
        )
    }

    fn render_fields(&self, cx: &mut Context<Self>) -> AnyElement {
        match &self.form {
            ValueForm::Replicas { input } => NumberInput::new(input).into_any_element(),
            ValueForm::Image {
                selected,
                image,
                cause,
            } => v_flex()
                .gap_2()
                .children(self.render_container_picker(*selected, cx))
                .child(
                    v_flex()
                        .gap_1()
                        .child(div().text_xs().child("Image"))
                        .child(Input::new(image)),
                )
                .child(
                    v_flex()
                        .gap_1()
                        .child(div().text_xs().child("Change cause"))
                        .child(Input::new(cause)),
                )
                .into_any_element(),
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
        let (muted, border, popover) = (theme.muted_foreground, theme.border, theme.popover);
        let (warning, danger) = (
            tone_color(StatusTone::Warn, cx),
            tone_color(StatusTone::Bad, cx),
        );
        let title = self.title();
        let state = self.state_text(cx);
        let can_submit = self.is_submittable(cx);
        let error = self.field_error(cx);
        let fields = self.render_fields(cx);
        v_flex()
            .key_context("ValuePopover")
            .on_action(cx.listener(|popover, _: &CancelValuePopover, _, cx| popover.cancel(cx)))
            .w(px(match self.form {
                ValueForm::Image { .. } => IMAGE_POPOVER_WIDTH,
                _ => POPOVER_WIDTH,
            }))
            .gap_2()
            .p_3()
            .rounded_lg()
            .border_1()
            .border_color(border)
            .bg(popover)
            .shadow_md()
            .child(div().text_sm().font_semibold().truncate().child(title))
            .child(fields)
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

    /// Types into the image field of a Set image popover.
    pub(crate) fn type_image(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        let ValueForm::Image { image, .. } = &self.form else {
            return;
        };
        image.update(cx, |input, cx| input.set_value(text.to_owned(), window, cx));
    }

    /// Types into the change cause field of a Set image popover.
    pub(crate) fn type_cause(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        let ValueForm::Image { cause, .. } = &self.form else {
            return;
        };
        cause.update(cx, |input, cx| input.set_value(text.to_owned(), window, cx));
    }

    /// The container button at `index` of a Set image popover.
    pub(crate) fn press_container(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.pick_container(index, window, cx);
    }

    /// The selected bytes of the image field: the tag a Set image popover opens with.
    pub(crate) fn selected_image_text(&self, cx: &App) -> Option<String> {
        let ValueForm::Image { image, .. } = &self.form else {
            return None;
        };
        let input = image.read(cx);
        Some(input.value()[input.selected_range()].to_string())
    }

    /// Types into the size field of an Expand popover.
    pub(crate) fn type_storage(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        let ValueForm::Storage { input } = &self.form else {
            return;
        };
        // `set_value` is silent; a user typing sends `Change`, which sets this.
        self.is_edited = true;
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
