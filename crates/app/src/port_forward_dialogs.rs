//! The dialogs of the Port Forwarding page (spec 0035): New forward, Change local port…, and
//! Remove preset…. New forward and Change local port… only collect a valid spec; the start itself
//! goes through the guarded flow of the target's own cluster, which asks for its confirm tier.
//! Remove preset… is a click-only confirm: a held Enter never removes anything.

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement,
    KeyDownEvent, ParentElement as _, Render, SharedString, Styled as _, WeakEntity, Window, div,
    px,
};

use super::AppShell;
use crate::cluster_registry::ClusterRef;
use crate::environment::{Environment, environment_badge};
use crate::fresh_enter::{FreshEnter, confirms, is_enter};
use crate::keymap::FORWARD_FORM;
use crate::port_forwards::{
    ForwardId, ForwardSpec, LOCAL_PORT_FIELD_ERROR, TargetSpec, is_dns_subdomain,
    parse_local_port_field, parse_port, parse_target,
};

const DIALOG_WIDTH: f32 = 420.;

/// What New forward opens with.
pub(crate) struct NewForwardPrefill {
    pub(crate) cluster: ClusterRef,
    pub(crate) namespace: String,
    pub(crate) target: Option<TargetSpec>,
    pub(crate) remote_port: Option<u16>,
}

/// The texts of the fields as typed.
pub(crate) struct NewForwardInput<'a> {
    pub(crate) namespace: &'a str,
    pub(crate) target: &'a str,
    pub(crate) remote_port: &'a str,
    pub(crate) local_port: &'a str,
}

/// One inline error per field; a field without one is valid.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct FormErrors {
    pub(crate) namespace: Option<&'static str>,
    pub(crate) target: Option<&'static str>,
    pub(crate) remote_port: Option<&'static str>,
    pub(crate) local_port: Option<&'static str>,
}

impl FormErrors {
    fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

const PORT_ERROR: &str = "Enter a port from 1 to 65535";

/// The spec the fields describe, or an error under each field that is wrong. An empty local port
/// is automatic; a typed one is exact.
pub(crate) fn validate_new_forward(input: &NewForwardInput<'_>) -> Result<ForwardSpec, FormErrors> {
    let mut errors = FormErrors::default();
    let namespace = input.namespace.trim();
    if namespace.is_empty() {
        errors.namespace = Some("Enter a namespace");
    } else if !is_dns_subdomain(namespace) {
        errors.namespace = Some("Not a valid namespace name");
    }
    let target = parse_target(input.target);
    if let Err(error) = &target {
        errors.target = Some(error.text());
    }
    let remote_port = parse_port(input.remote_port);
    if remote_port.is_none() {
        errors.remote_port = Some(PORT_ERROR);
    }
    let local_port = parse_local_port_field(input.local_port);
    if local_port.is_none() {
        errors.local_port = Some(LOCAL_PORT_FIELD_ERROR);
    }
    match (target, remote_port, local_port) {
        (Ok(target), Some(remote_port), Some(local_port)) if errors.is_empty() => Ok(ForwardSpec {
            namespace: namespace.to_owned(),
            target,
            remote_port,
            local_port,
        }),
        _ => Err(errors),
    }
}

/// A port for Change local port…: 1 to 65535.
pub(crate) fn validate_local_port(text: &str) -> Result<u16, &'static str> {
    parse_port(text).ok_or(PORT_ERROR)
}

/// The cluster a form starts in: the open one, shown as a badge and a label.
struct FormCluster {
    cluster: ClusterRef,
    label: String,
    environment: Environment,
}

/// The body of the New forward dialog.
struct NewForwardForm {
    shell: WeakEntity<AppShell>,
    cluster: FormCluster,
    namespace: Entity<InputState>,
    target: Entity<InputState>,
    remote_port: Entity<InputState>,
    local_port: Entity<InputState>,
    errors: FormErrors,
}

impl NewForwardForm {
    /// Forward: validates, then closes and starts through the guarded flow of the cluster.
    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let input = |state: &Entity<InputState>| state.read(cx).value().to_string();
        let (namespace, target) = (input(&self.namespace), input(&self.target));
        let (remote_port, local_port) = (input(&self.remote_port), input(&self.local_port));
        let spec = validate_new_forward(&NewForwardInput {
            namespace: &namespace,
            target: &target,
            remote_port: &remote_port,
            local_port: &local_port,
        });
        let chosen = self.cluster.cluster.clone();
        match spec {
            Err(errors) => {
                self.errors = errors;
                cx.notify();
            }
            Ok(spec) => {
                let shell = self.shell.clone();
                window.close_dialog(cx);
                // After the close: starting opens the confirm dialog, which the close must not pop.
                window.defer(cx, move |window, cx| {
                    let _ = shell.update(cx, |shell, cx| {
                        shell.start_forward(&chosen, spec, None, window, cx);
                    });
                });
            }
        }
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !is_enter(event) {
            return;
        }
        window.prevent_default();
        cx.stop_propagation();
        if confirms(event) {
            self.submit(window, cx);
        }
    }

    fn field(
        &self,
        label: &'static str,
        input: &Entity<InputState>,
        error: Option<&'static str>,
        cx: &Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        v_flex()
            .gap_1()
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(label),
            )
            .child(Input::new(input).small())
            .children(error.map(|text| div().text_xs().text_color(theme.danger).child(text)))
            .into_any_element()
    }
}

impl Render for NewForwardForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .key_context(FORWARD_FORM)
            .on_key_down(cx.listener(Self::on_key_down))
            .w_full()
            .gap_3()
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("Cluster"),
                    )
                    .child(
                        h_flex()
                            .id("forward-cluster")
                            .gap_2()
                            .items_center()
                            .child(environment_badge(&self.cluster.environment, cx))
                            .child(
                                div()
                                    .min_w_0()
                                    .font_family(cx.theme().mono_font_family.clone())
                                    .child(self.cluster.label.clone()),
                            ),
                    ),
            )
            .child(self.field("Namespace", &self.namespace, self.errors.namespace, cx))
            .child(self.field(
                "Target (pod/NAME, svc/NAME, deploy/NAME, sts/NAME)",
                &self.target,
                self.errors.target,
                cx,
            ))
            .child(self.field(
                "Remote port",
                &self.remote_port,
                self.errors.remote_port,
                cx,
            ))
            .child(self.field(
                "Local port (empty = automatic)",
                &self.local_port,
                self.errors.local_port,
                cx,
            ))
            .child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .justify_end()
                    .child(
                        Button::new("forward-cancel")
                            .label("Cancel")
                            .small()
                            .outline()
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("forward-submit")
                            .label("Forward")
                            .small()
                            .primary()
                            .on_click(cx.listener(|form, _, window, cx| form.submit(window, cx))),
                    ),
            )
    }
}

/// The body of the Change local port… dialog: one number field.
struct LocalPortForm {
    shell: WeakEntity<AppShell>,
    id: ForwardId,
    port: Entity<InputState>,
    error: Option<&'static str>,
}

impl LocalPortForm {
    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.port.read(cx).value().to_string();
        match validate_local_port(&text) {
            Err(error) => {
                self.error = Some(error);
                cx.notify();
            }
            Ok(port) => {
                let (shell, id) = (self.shell.clone(), self.id);
                window.close_dialog(cx);
                window.defer(cx, move |window, cx| {
                    let _ = shell.update(cx, |shell, cx| {
                        shell.change_local_port(id, port, window, cx);
                    });
                });
            }
        }
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !is_enter(event) {
            return;
        }
        window.prevent_default();
        cx.stop_propagation();
        if confirms(event) {
            self.submit(window, cx);
        }
    }
}

impl Render for LocalPortForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        v_flex()
            .key_context(FORWARD_FORM)
            .on_key_down(cx.listener(Self::on_key_down))
            .w_full()
            .gap_3()
            .child(
                v_flex()
                    .gap_1()
                    .child(Input::new(&self.port).small())
                    .children(
                        self.error
                            .map(|text| div().text_xs().text_color(theme.danger).child(text)),
                    ),
            )
            .child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .justify_end()
                    .child(
                        Button::new("port-cancel")
                            .label("Cancel")
                            .small()
                            .outline()
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("port-apply")
                            .label("Apply")
                            .small()
                            .primary()
                            .on_click(cx.listener(|form, _, window, cx| form.submit(window, cx))),
                    ),
            )
    }
}

impl AppShell {
    /// New forward, in the open cluster; the rest is typed. A prefill of any other cluster opens
    /// nothing: only the open one can start a forward.
    pub(crate) fn open_new_forward(
        &mut self,
        prefill: NewForwardPrefill,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(open) = self
            .active_session()
            .filter(|open| open.cluster == prefill.cluster)
        else {
            return;
        };
        let cluster = FormCluster {
            cluster: open.cluster.clone(),
            label: open.label.clone(),
            environment: open.profile.environment.clone(),
        };
        open_form(cluster, prefill, window, cx);
    }

    /// `--screen port-forward-new-fixture`: the form over one fixed cluster, so it needs none.
    #[cfg(feature = "screenshot")]
    pub(super) fn open_new_forward_fixture(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (production, _) = crate::screenshot::forward_fixture_clusters();
        let cluster = FormCluster {
            cluster: production.0.clone(),
            label: production.1.to_owned(),
            environment: Environment::PRODUCTION,
        };
        let prefill = NewForwardPrefill {
            cluster: production.0,
            namespace: "payments".to_owned(),
            target: Some(TargetSpec::pod("postgres-0")),
            remote_port: Some(5432),
        };
        open_form(cluster, prefill, window, cx);
    }

    /// Change local port…: the current port, or the preset's, in one field.
    pub(crate) fn open_change_local_port(
        &mut self,
        id: ForwardId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(current) = self.port_forwards.read(cx).get(id).map(|forward| {
            forward
                .local
                .map_or_else(|| forward.spec.requested_local_port(), |local| local.port())
        }) else {
            return;
        };
        let shell = cx.weak_entity();
        let form = cx.new(|cx| LocalPortForm {
            shell,
            id,
            port: cx.new(|cx| {
                let mut input = InputState::new(window, cx).placeholder("Local port");
                input.set_value(current.to_string(), window, cx);
                input
            }),
            error: None,
        });
        window.open_dialog(cx, move |dialog, _, _| {
            dialog
                .title("Change local port")
                .w(px(DIALOG_WIDTH))
                .child(form.clone())
        });
    }

    /// Remove preset…: `Remove the preset {target}:{port}?`, answered by a click only.
    pub(crate) fn open_remove_preset(
        &mut self,
        id: ForwardId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(text) = self
            .port_forwards
            .read(cx)
            .get(id)
            .filter(|forward| forward.is_preset)
            .map(|forward| {
                SharedString::from(format!(
                    "Remove the preset {}:{}?",
                    forward.spec.target_text(),
                    forward.spec.remote_port
                ))
            })
        else {
            return;
        };
        let shell = cx.weak_entity();
        // Enter never removes: the kit binding is off and the fresh Enter does nothing. The buttons
        // are the ones of the confirm dialogs, so Remove reads as the red, enabled button it is.
        let body = cx.new(|cx| {
            FreshEnter::new(
                move |_| {
                    let shell = shell.clone();
                    v_flex()
                        .w_full()
                        .gap_3()
                        .child(text.clone())
                        .child(
                            h_flex()
                                .w_full()
                                .gap_2()
                                .justify_end()
                                .child(
                                    Button::new("preset-cancel")
                                        .label("Cancel")
                                        .small()
                                        .outline()
                                        .on_click(|_, window, cx| window.close_dialog(cx)),
                                )
                                .child(
                                    Button::new("preset-remove")
                                        .label("Remove")
                                        .small()
                                        .danger()
                                        .on_click(move |_, window, cx| {
                                            let _ = shell.update(cx, |shell, cx| {
                                                shell.remove_forward_preset(id, cx);
                                            });
                                            window.close_dialog(cx);
                                        }),
                                ),
                        )
                        .into_any_element()
                },
                |_, _| {},
                cx,
            )
        });
        window.open_dialog(cx, move |dialog, _, _| {
            dialog
                .title("Remove preset")
                .w(px(DIALOG_WIDTH))
                .child(body.clone())
        });
    }
}

/// Opens the New forward dialog in `cluster`.
fn open_form(
    cluster: FormCluster,
    prefill: NewForwardPrefill,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    let shell = cx.weak_entity();
    let form = cx.new(|cx| {
        let mut text_input =
            |placeholder: &'static str, value: Option<String>, cx: &mut Context<NewForwardForm>| {
                cx.new(|cx| {
                    let mut input = InputState::new(window, cx).placeholder(placeholder);
                    if let Some(value) = value {
                        input.set_value(value, window, cx);
                    }
                    input
                })
            };
        let namespace = text_input("payments", Some(prefill.namespace.clone()), cx);
        let target = text_input(
            "pod/NAME",
            prefill
                .target
                .as_ref()
                .map(|target| format!("{}/{}", target.kind.short(), target.name)),
            cx,
        );
        let remote_port = text_input("5432", prefill.remote_port.map(|port| port.to_string()), cx);
        let local_port = text_input("automatic", None, cx);
        NewForwardForm {
            shell,
            cluster,
            namespace,
            target,
            remote_port,
            local_port,
            errors: FormErrors::default(),
        }
    });
    window.open_dialog(cx, move |dialog, _, _| {
        dialog
            .title("New forward")
            .w(px(DIALOG_WIDTH))
            .child(form.clone())
    });
}

#[cfg(test)]
#[path = "port_forward_dialogs_tests.rs"]
mod port_forward_dialogs_tests;
