//! The Environments page of the Settings window (spec 0053): the four built-in environments and
//! the custom ones the user adds, each with a name, a color, and the built-in tier whose rules it
//! follows. The page owns the view state only; the rules live in `environment_form.rs`.

use gpui_kit::component::button::{Button, ButtonVariant, ButtonVariants as _};
use gpui_kit::component::dialog::DialogButtonProps;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{
    ActiveTheme as _, Sizable as _, StyledExt as _, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, ElementId, Entity, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, SharedString, StatefulInteractiveElement as _,
    Styled as _, Subscription, Window, div, px,
};

use crate::cluster_form::FieldError;
use crate::environment::{
    CustomEnvironment, Environment, EnvironmentColor, EnvironmentTier, environment_badge,
    is_usable, palette_color,
};
use crate::environment_form::{
    add_environment, clusters_using, delete_dialog_text, delete_environment, edit_environment,
    is_weaker, rename_environment, validate_environment_name, weaken_dialog_text,
};
use crate::settings::AppSettings;
use crate::settings_window::tier_cell;
use crate::write_guard::{ActionRisk, ConfirmMode};

const BADGE_WIDTH: f32 = 90.;
const BUILT_IN_NAME_WIDTH: f32 = 140.;
const NAME_WIDTH: f32 = 160.;
const SWATCH_SIZE: f32 = 18.;
/// Wide enough for the longest tier name, so the Delete column lines up.
const TIER_WIDTH: f32 = 210.;

/// The name input of one custom environment, and the message under it.
struct NameRow {
    input: Entity<InputState>,
    error: Option<FieldError>,
}

pub(crate) struct EnvironmentsPage {
    /// One per `registry.environments` item, in the same order. Rebuilt only by this page's own
    /// add and delete, never on a settings change: a rebuild would reset the caret while typing.
    rows: Vec<NameRow>,
    new_name: Entity<InputState>,
    new_error: Option<FieldError>,
    _observer: Subscription,
    _new_name_subscription: Subscription,
    _row_subscriptions: Vec<Subscription>,
}

impl EnvironmentsPage {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let new_name = cx.new(|cx| InputState::new(window, cx).placeholder("New environment name"));
        let mut page = Self {
            rows: Vec::new(),
            new_error: None,
            _observer: cx.observe_global::<AppSettings>(|_, cx| cx.notify()),
            _new_name_subscription: cx.subscribe_in(&new_name, window, Self::on_new_name_event),
            new_name,
            _row_subscriptions: Vec::new(),
        };
        page.rebuild_rows(window, cx);
        page
    }

    fn rebuild_rows(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let names: Vec<String> = AppSettings::get(cx)
            .registry
            .environments
            .iter()
            .map(|environment| environment.name.clone())
            .collect();
        self.rows.clear();
        self._row_subscriptions.clear();
        for name in names {
            let input = cx.new(|cx| {
                let mut state = InputState::new(window, cx);
                state.set_value(name, window, cx);
                state
            });
            self._row_subscriptions
                .push(cx.subscribe_in(&input, window, Self::on_name_event));
            self.rows.push(NameRow { input, error: None });
        }
    }

    fn on_name_event(
        &mut self,
        input: &Entity<InputState>,
        event: &InputEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !matches!(event, InputEvent::Change) {
            return;
        }
        let Some(at) = self.rows.iter().position(|row| row.input == *input) else {
            return;
        };
        let text = input.read(cx).value();
        let custom = AppSettings::get(cx).registry.environments.clone();
        let error = match validate_environment_name(&text, Some(at), &custom) {
            Ok(name) => {
                let is_changed = custom.get(at).is_some_and(|stored| stored.name != name);
                if is_changed {
                    AppSettings::update(cx, |settings| {
                        rename_environment(&mut settings.registry, at, name);
                    });
                }
                None
            }
            Err(error) => Some(error),
        };
        self.rows[at].error = error;
        cx.notify();
    }

    fn on_new_name_event(
        &mut self,
        _: &Entity<InputState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Change => {
                self.new_error = None;
                cx.notify();
            }
            InputEvent::PressEnter { .. } => self.add(window, cx),
            _ => {}
        }
    }

    fn add(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.new_name.read(cx).value();
        let custom = AppSettings::get(cx).registry.environments.clone();
        match validate_environment_name(&text, None, &custom) {
            Ok(name) => {
                AppSettings::update(cx, |settings| add_environment(&mut settings.registry, name));
                self.new_name
                    .update(cx, |state, cx| state.set_value(String::new(), window, cx));
                self.new_error = None;
                self.rebuild_rows(window, cx);
            }
            Err(error) => self.new_error = Some(error),
        }
        cx.notify();
    }

    fn confirm_delete(&self, at: usize, window: &mut Window, cx: &mut Context<Self>) {
        let registry = &AppSettings::get(cx).registry;
        let Some(environment) = registry.environments.get(at) else {
            return;
        };
        // A skipped row (reserved or repeated name) owns no references: delete moves none.
        let using = if is_usable(&registry.environments, at) {
            clusters_using(registry, &environment.name)
        } else {
            0
        };
        let (title, body) = delete_dialog_text(environment, using);
        let page = cx.entity();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let page = page.clone();
            alert
                .title(title.clone())
                .description(body.clone())
                .confirm()
                .button_props(
                    DialogButtonProps::default()
                        .ok_text("Delete")
                        .ok_variant(ButtonVariant::Danger)
                        .show_cancel(true),
                )
                // `at` stays valid: the alert is modal, so nothing else edits the list meanwhile.
                .on_ok(move |_, window, cx| {
                    AppSettings::update(cx, |settings| {
                        delete_environment(&mut settings.registry, at);
                    });
                    page.update(cx, |page, cx| page.rebuild_rows(window, cx));
                    true
                })
        });
    }

    /// A weaker tier on an environment that clusters use asks first; anything else saves at once.
    fn pick_tier(
        &self,
        at: usize,
        tier: EnvironmentTier,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let registry = &AppSettings::get(cx).registry;
        let Some(environment) = registry.environments.get(at) else {
            return;
        };
        if environment.tier == tier {
            return;
        }
        let using = if is_usable(&registry.environments, at) {
            clusters_using(registry, &environment.name)
        } else {
            0
        };
        if !(is_weaker(environment.tier, tier) && using > 0) {
            set_tier(at, tier, cx);
            return;
        }
        let (title, body) = weaken_dialog_text(environment, tier, using);
        window.open_alert_dialog(cx, move |alert, _, _| {
            alert
                .title(title.clone())
                .description(body.clone())
                .confirm()
                .button_props(
                    DialogButtonProps::default()
                        .ok_text("Change")
                        .ok_variant(ButtonVariant::Danger)
                        .show_cancel(true),
                )
                // `at` stays valid: the alert is modal, so nothing else edits the list meanwhile.
                .on_ok(move |_, _, cx| {
                    set_tier(at, tier, cx);
                    true
                })
        });
    }

    /// The message under the name input: what the user typed wrong, or why a hand-edited row is
    /// ignored (a reserved or repeated name).
    fn row_error(&self, at: usize, custom: &[CustomEnvironment]) -> Option<FieldError> {
        if let Some(error) = self.rows.get(at).and_then(|row| row.error.clone()) {
            return Some(error);
        }
        if is_usable(custom, at) {
            return None;
        }
        validate_environment_name(&custom.get(at)?.name, Some(at), custom).err()
    }

    fn render_custom_row(
        &self,
        at: usize,
        custom: &[CustomEnvironment],
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let environment = custom.get(at)?;
        let input = &self.rows.get(at)?.input;
        let theme = cx.theme();
        let (danger, muted) = (theme.danger, theme.muted_foreground);
        let ring = theme.foreground;
        let page = cx.entity();
        let current = environment.tier;
        let swatches = EnvironmentColor::ALL.into_iter().enumerate().fold(
            h_flex().gap_2().items_center(),
            |swatches, (index, color)| {
                let is_current = environment.color == color;
                swatches.child(
                    div()
                        .id(ElementId::from(("environment-color", at * 10 + index)))
                        .size(px(SWATCH_SIZE))
                        .flex_none()
                        .rounded_full()
                        .bg(palette_color(color, cx))
                        .cursor_pointer()
                        .when(is_current, |swatch| swatch.border_2().border_color(ring))
                        .tooltip(move |window, cx| Tooltip::new(color.name()).build(window, cx))
                        .on_click(move |_, _, cx| set_color(at, color, cx)),
                )
            },
        );
        let tier_menu = Button::new(("environment-tier", at))
            .small()
            .outline()
            .w(px(TIER_WIDTH))
            .label(format!("Behaves like {}", current.name()))
            .dropdown_caret(true)
            .dropdown_menu(move |menu, _, _| {
                EnvironmentTier::ALL.into_iter().fold(menu, |menu, tier| {
                    let page = page.clone();
                    menu.item(
                        PopupMenuItem::new(tier.name())
                            .checked(tier == current)
                            .on_click(move |_, window, cx| {
                                page.update(cx, |page, cx| page.pick_tier(at, tier, window, cx));
                            }),
                    )
                })
            });
        let delete = Button::new(("environment-delete", at))
            .danger()
            .outline()
            .small()
            .label("Delete")
            .on_click(cx.listener(move |page, _, window, cx| {
                page.confirm_delete(at, window, cx);
            }));
        let under = match self.row_error(at, custom) {
            Some(error) => div().text_xs().text_color(danger).child(error.0),
            None => {
                let using = clusters_using(&AppSettings::get(cx).registry, &environment.name);
                let text = match using {
                    0 => "Not used".to_owned(),
                    1 => "Used by 1 cluster".to_owned(),
                    count => format!("Used by {count} clusters"),
                };
                div().text_xs().text_color(muted).child(text)
            }
        };
        Some(
            v_flex()
                .w_full()
                .gap_1()
                .child(
                    h_flex()
                        .w_full()
                        .gap_3()
                        .items_center()
                        .child(
                            div()
                                .w(px(BADGE_WIDTH))
                                .flex_none()
                                .child(environment_badge(
                                    &Environment::Custom(environment.clone()),
                                    cx,
                                )),
                        )
                        .child(Input::new(input).w(px(NAME_WIDTH)))
                        .child(swatches)
                        .child(tier_menu)
                        .child(delete),
                )
                .child(div().pl(px(BADGE_WIDTH + 12.)).child(under))
                .into_any_element(),
        )
    }
}

fn set_color(at: usize, color: EnvironmentColor, cx: &mut App) {
    AppSettings::update(cx, |settings| {
        edit_environment(&mut settings.registry, at, |environment| {
            environment.color = color;
        });
    });
}

fn set_tier(at: usize, tier: EnvironmentTier, cx: &mut App) {
    AppSettings::update(cx, |settings| {
        edit_environment(&mut settings.registry, at, |environment| {
            environment.tier = tier;
        });
    });
}

fn section_title(title: &'static str, cx: &App) -> impl IntoElement {
    div()
        .text_sm()
        .font_semibold()
        .pb_1()
        .border_b_1()
        .border_color(cx.theme().border)
        .child(title)
}

fn muted_text(text: impl Into<SharedString>, cx: &App) -> gpui_kit::Div {
    div()
        .text_sm()
        .text_color(cx.theme().muted_foreground)
        .child(text.into())
}

/// What a built-in environment does: Production also opens read-only (`profile.read_only`), and its
/// typed name is the object's for one object and the cluster's for several (spec 0030).
fn tier_description(tier: EnvironmentTier) -> String {
    match tier {
        EnvironmentTier::Production => "Opens read-only. Confirms by typing the object name (one object) or the cluster name (several)".to_owned(),
        EnvironmentTier::Staging | EnvironmentTier::Development | EnvironmentTier::Local => {
            tier_cell(ConfirmMode::for_tier(tier), ActionRisk::Change)
        }
    }
}

/// The four built-ins, read-only: badge, name, and how they open and how a change is confirmed.
fn built_in_rows(cx: &App) -> Vec<AnyElement> {
    EnvironmentTier::ALL
        .into_iter()
        .map(|tier| {
            h_flex()
                .w_full()
                .gap_3()
                .items_center()
                .child(
                    div()
                        .w(px(BADGE_WIDTH))
                        .flex_none()
                        .child(environment_badge(&Environment::BuiltIn(tier), cx)),
                )
                .child(
                    div()
                        .w(px(BUILT_IN_NAME_WIDTH))
                        .text_sm()
                        .child(tier.name()),
                )
                .child(muted_text(tier_description(tier), cx).flex_1().min_w_0())
                .into_any_element()
        })
        .collect()
}

impl Render for EnvironmentsPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let custom = AppSettings::get(cx).registry.environments.clone();
        let danger = cx.theme().danger;
        let rows: Vec<AnyElement> = (0..custom.len())
            .filter_map(|at| self.render_custom_row(at, &custom, cx))
            .collect();
        let add = v_flex()
            .w_full()
            .gap_1()
            .child(
                h_flex()
                    .gap_3()
                    .items_center()
                    .child(Input::new(&self.new_name).w(px(NAME_WIDTH + BADGE_WIDTH + 12.)))
                    .child(
                        Button::new("add-environment")
                            .small()
                            .label("Add")
                            .on_click(cx.listener(|page, _, window, cx| page.add(window, cx))),
                    ),
            )
            .children(
                self.new_error
                    .as_ref()
                    .map(|error| div().text_xs().text_color(danger).child(error.0.clone())),
            );
        v_flex()
            .w_full()
            .gap_4()
            .child(
                v_flex()
                    .w_full()
                    .gap_2()
                    .child(section_title("Built-in", cx))
                    .children(built_in_rows(cx)),
            )
            .child(
                v_flex()
                    .w_full()
                    .gap_2()
                    .child(section_title("Custom", cx))
                    .when(rows.is_empty(), |list| {
                        list.child(muted_text("No custom environments yet.", cx))
                    })
                    .children(rows)
                    .child(add),
            )
    }
}

#[cfg(test)]
#[path = "environments_page_tests.rs"]
mod environments_page_tests;
