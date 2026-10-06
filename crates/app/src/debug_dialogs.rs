//! The options dialogs of the debug starts (spec 0037): Debug container… (a target container and an
//! image) and Open node shell (a namespace and an image). They only collect a valid choice; the
//! start itself goes through the guarded flow of the pod's or node's own cluster, which asks for
//! its confirm tier (and, for a node shell, always for the node name).
//!
//! Both forms take Enter themselves and submit on a fresh press only, so the Enter that opened a
//! menu item never starts anything.

use std::rc::Rc;

use cluster::is_valid_debug_image;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::select::{Select, SelectState};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, IndexPath, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement,
    KeyDownEvent, ParentElement as _, Render, SharedString, StatefulInteractiveElement as _,
    Styled as _, Subscription, WeakEntity, Window, div, px,
};

use crate::app_shell::AppShell;
use crate::cell_truncation::middle_truncate;
use crate::fresh_enter::{confirms, is_enter};
use crate::keymap::FORWARD_FORM;
use crate::port_forwards::is_dns_subdomain;

const DIALOG_WIDTH: f32 = 460.;
/// Shown in the warning color on the Debug container dialog.
pub(crate) const DEBUG_WARNING: &str = "Ephemeral containers cannot be removed. It stays in the pod spec until the pod is deleted. Each Reconnect adds another container.";
/// Shown muted on both dialogs.
pub(crate) const CLOSING_NOTE: &str =
    "Closing the tab ends the shell and everything started from it.";
pub(crate) const NODE_SHELL_NOTE: &str = "Closing the tab ends the shell and everything started from it. The pod is deleted when the shell ends.";
const NODE_SHELL_IMAGE_HINT: &str = "The image must provide nsenter and sh.";
const IMAGE_ERROR: &str = "Enter an image without spaces, up to 255 characters";

/// One running container of the pod the Debug container dialog can share a namespace with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DebugChoice {
    pub(crate) name: String,
    /// `MAIN` or `SIDECAR`.
    pub(crate) tag: &'static str,
}

/// What the Debug container dialog asks about.
pub(crate) struct DebugForm {
    pub(crate) pod: String,
    pub(crate) choices: Vec<DebugChoice>,
    /// The container to select first: the one an exec found no shell in, else the default.
    pub(crate) preselected: Option<String>,
    pub(crate) image: String,
}

/// What the Debug container dialog collected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DebugChosen {
    pub(crate) target_container: String,
    pub(crate) image: String,
}

/// What the Open node shell dialog asks about.
pub(crate) struct NodeShellForm {
    pub(crate) node: String,
    pub(crate) namespace: String,
    pub(crate) image: String,
}

/// What the Open node shell dialog collected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NodeShellChosen {
    pub(crate) namespace: String,
    pub(crate) image: String,
}

/// One inline error per field; a field without one is valid.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct FormErrors {
    pub(crate) target: Option<&'static str>,
    pub(crate) namespace: Option<&'static str>,
    pub(crate) image: Option<&'static str>,
}

fn check_image(image: &str, errors: &mut FormErrors) -> String {
    let image = image.trim();
    if !is_valid_debug_image(image) {
        errors.image = Some(IMAGE_ERROR);
    }
    image.to_owned()
}

/// The choice the Debug container fields describe, or an error under each field that is wrong.
pub(crate) fn validate_debug(
    target: Option<&str>,
    image: &str,
    choices: &[DebugChoice],
) -> Result<DebugChosen, FormErrors> {
    let mut errors = FormErrors::default();
    let target = match target.filter(|name| choices.iter().any(|choice| choice.name == *name)) {
        Some(name) => name.to_owned(),
        None => {
            errors.target = Some("Pick a running container");
            String::new()
        }
    };
    let image = check_image(image, &mut errors);
    if errors == FormErrors::default() {
        Ok(DebugChosen {
            target_container: target,
            image,
        })
    } else {
        Err(errors)
    }
}

/// The choice the node shell fields describe, or an error under each field that is wrong.
pub(crate) fn validate_node_shell(
    namespace: &str,
    image: &str,
) -> Result<NodeShellChosen, FormErrors> {
    let mut errors = FormErrors::default();
    let namespace = namespace.trim();
    if namespace.is_empty() {
        errors.namespace = Some("Enter a namespace");
    } else if !is_dns_subdomain(namespace) {
        errors.namespace = Some("Not a valid namespace name");
    }
    let image = check_image(image, &mut errors);
    if errors == FormErrors::default() {
        Ok(NodeShellChosen {
            namespace: namespace.to_owned(),
            image,
        })
    } else {
        Err(errors)
    }
}

/// The index of the container the Select shows first: the preselected one if it is listed, else
/// the first main container, else the first.
pub(crate) fn first_selected(choices: &[DebugChoice], preselected: Option<&str>) -> usize {
    preselected
        .and_then(|name| choices.iter().position(|choice| choice.name == name))
        .or_else(|| choices.iter().position(|choice| choice.tag == "MAIN"))
        .unwrap_or(0)
}

/// How many characters of an image the Image field shows before its text runs out of room.
const IMAGE_FIELD_CHARS: usize = 54;

/// The image cut in the middle, so the registry and the end (a tag or a digest) show, when it is
/// longer than its field. The field itself keeps the start only.
fn image_summary(image: &str) -> Option<String> {
    let image = image.trim();
    (image.chars().count() > IMAGE_FIELD_CHARS)
        .then(|| middle_truncate(image, IMAGE_FIELD_CHARS).into_owned())
}

/// The line under an Image field that shows the end of a long image; its tooltip has all of it.
fn image_summary_line(image: &str, cx: &gpui_kit::App) -> Option<AnyElement> {
    let summary = image_summary(image)?;
    let theme = cx.theme();
    let full = SharedString::from(image.trim().to_owned());
    Some(
        div()
            .id("image-summary")
            .text_xs()
            .font_family(theme.mono_font_family.clone())
            .text_color(theme.muted_foreground)
            .child(summary)
            .tooltip(move |window, cx| Tooltip::new(full.clone()).build(window, cx))
            .into_any_element(),
    )
}

fn field(
    label: &'static str,
    body: impl IntoElement,
    error: Option<&'static str>,
    cx: &gpui_kit::App,
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
        .child(body)
        .children(error.map(|text| div().text_xs().text_color(theme.danger).child(text)))
        .into_any_element()
}

fn buttons(
    id: &'static str,
    continue_danger: bool,
    on_continue: impl Fn(&mut Window, &mut gpui_kit::App) + 'static,
) -> AnyElement {
    let primary = Button::new(id)
        .label("Continue")
        .small()
        .on_click(move |_, window, cx| on_continue(window, cx));
    let primary = if continue_danger {
        primary.danger()
    } else {
        primary.primary()
    };
    h_flex()
        .w_full()
        .gap_2()
        .justify_end()
        .child(
            Button::new("debug-cancel")
                .label("Cancel")
                .small()
                .outline()
                .on_click(|_, window, cx| window.close_dialog(cx)),
        )
        .child(primary)
        .into_any_element()
}

/// What Continue of the Debug container dialog runs with the validated choice.
type DebugStart = dyn Fn(&mut AppShell, DebugChosen, &mut Window, &mut Context<AppShell>);

/// What Continue of the Open node shell dialog runs with the validated choice.
type NodeShellStart = dyn Fn(&mut AppShell, NodeShellChosen, &mut Window, &mut Context<AppShell>);

/// The body of the Debug container dialog.
struct DebugBody {
    shell: WeakEntity<AppShell>,
    start: Rc<DebugStart>,
    choices: Vec<DebugChoice>,
    target: Entity<SelectState<Vec<String>>>,
    image: Entity<InputState>,
    errors: FormErrors,
    /// Draws the image summary again as the image is typed.
    _image_observer: Subscription,
}

impl DebugBody {
    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let selected = self.target.read(cx).selected_index(cx).map(|ix| ix.row);
        let target = selected.and_then(|row| self.choices.get(row).map(|c| c.name.clone()));
        let image = self.image.read(cx).value().to_string();
        match validate_debug(target.as_deref(), &image, &self.choices) {
            Err(errors) => {
                self.errors = errors;
                cx.notify();
            }
            Ok(chosen) => {
                let shell = self.shell.clone();
                window.close_dialog(cx);
                // After the close: the start opens the confirm dialog, which the close must not
                // pop.
                let start = Rc::clone(&self.start);
                window.defer(cx, move |window, cx| {
                    let _ = shell.update(cx, |shell, cx| start(shell, chosen, window, cx));
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

impl Render for DebugBody {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (warning, muted) = (theme.warning, theme.muted_foreground);
        let weak = cx.weak_entity();
        v_flex()
            .key_context(FORWARD_FORM)
            .on_key_down(cx.listener(Self::on_key_down))
            .w_full()
            .gap_3()
            .child(field(
                "Target container",
                Select::new(&self.target).small(),
                self.errors.target,
                cx,
            ))
            .child(field(
                "Image",
                v_flex()
                    .gap_1()
                    .child(Input::new(&self.image).small())
                    .children(image_summary_line(&self.image.read(cx).value(), cx)),
                self.errors.image,
                cx,
            ))
            .child(div().text_sm().text_color(warning).child(DEBUG_WARNING))
            .child(div().text_xs().text_color(muted).child(CLOSING_NOTE))
            .child(buttons("debug-continue", false, move |window, cx| {
                let _ = weak.update(cx, |body, cx| body.submit(window, cx));
            }))
    }
}

/// The body of the Open node shell dialog.
struct NodeShellBody {
    shell: WeakEntity<AppShell>,
    node: String,
    start: Rc<NodeShellStart>,
    namespace: Entity<InputState>,
    image: Entity<InputState>,
    errors: FormErrors,
    /// Draws the image summary again as the image is typed.
    _image_observer: Subscription,
}

impl NodeShellBody {
    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let namespace = self.namespace.read(cx).value().to_string();
        let image = self.image.read(cx).value().to_string();
        match validate_node_shell(&namespace, &image) {
            Err(errors) => {
                self.errors = errors;
                cx.notify();
            }
            Ok(chosen) => {
                let shell = self.shell.clone();
                window.close_dialog(cx);
                let start = Rc::clone(&self.start);
                window.defer(cx, move |window, cx| {
                    let _ = shell.update(cx, |shell, cx| start(shell, chosen, window, cx));
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

impl Render for NodeShellBody {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (danger, muted) = (theme.danger, theme.muted_foreground);
        let weak = cx.weak_entity();
        v_flex()
            .key_context(FORWARD_FORM)
            .on_key_down(cx.listener(Self::on_key_down))
            .w_full()
            .gap_3()
            .child(
                div()
                    .text_sm()
                    .text_color(danger)
                    .child(node_shell_warning(&self.node)),
            )
            .child(field(
                "Namespace",
                Input::new(&self.namespace).small(),
                self.errors.namespace,
                cx,
            ))
            .child(field(
                "Image",
                v_flex()
                    .gap_1()
                    .child(Input::new(&self.image).small())
                    .children(image_summary_line(&self.image.read(cx).value(), cx))
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .child(NODE_SHELL_IMAGE_HINT),
                    ),
                self.errors.image,
                cx,
            ))
            .child(div().text_xs().text_color(muted).child(NODE_SHELL_NOTE))
            .child(buttons("node-shell-continue", true, move |window, cx| {
                let _ = weak.update(cx, |body, cx| body.submit(window, cx));
            }))
    }
}

/// The danger line of the Open node shell dialog.
pub(crate) fn node_shell_warning(node: &str) -> SharedString {
    format!(
        "Creates a privileged pod with host PID access on {node}. Anything you run affects the node."
    )
    .into()
}

impl AppShell {
    /// Opens the Debug container dialog. `start` runs after Continue with the validated choice.
    pub(crate) fn show_debug_form(
        &mut self,
        form: DebugForm,
        start: impl Fn(&mut AppShell, DebugChosen, &mut Window, &mut Context<AppShell>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let shell = cx.weak_entity();
        let DebugForm {
            pod,
            choices,
            preselected,
            image,
        } = form;
        let selected = first_selected(&choices, preselected.as_deref());
        let body = cx.new(|cx| {
            let labels: Vec<String> = choices
                .iter()
                .map(|choice| format!("{} · {}", choice.name, choice.tag))
                .collect();
            let target = cx.new(|cx| {
                SelectState::new(labels, Some(IndexPath::default().row(selected)), window, cx)
            });
            let image = cx.new(|cx| {
                let mut input = InputState::new(window, cx).placeholder("busybox:1.36");
                input.set_value(image, window, cx);
                input
            });
            let _image_observer = cx.observe(&image, |_, _, cx| cx.notify());
            DebugBody {
                shell,
                start: Rc::new(start),
                choices,
                target,
                image,
                errors: FormErrors::default(),
                _image_observer,
            }
        });
        window.open_dialog(cx, move |dialog, _, _| {
            dialog
                .title(format!("Debug container on {pod}?"))
                .w(px(DIALOG_WIDTH))
                .child(body.clone())
        });
    }

    /// Opens the Open node shell dialog. `start` runs after Continue with the validated choice.
    pub(crate) fn show_node_shell_form(
        &mut self,
        form: NodeShellForm,
        start: impl Fn(&mut AppShell, NodeShellChosen, &mut Window, &mut Context<AppShell>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let shell = cx.weak_entity();
        let NodeShellForm {
            node,
            namespace,
            image,
        } = form;
        let title = format!("Open a privileged shell on node {node}?");
        let body = cx.new(|cx| {
            let mut text_input =
                |value: String, placeholder: &'static str, cx: &mut Context<NodeShellBody>| {
                    cx.new(|cx| {
                        let mut input = InputState::new(window, cx).placeholder(placeholder);
                        input.set_value(value, window, cx);
                        input
                    })
                };
            let image = text_input(image, "busybox:1.36", cx);
            let _image_observer = cx.observe(&image, |_, _, cx| cx.notify());
            NodeShellBody {
                shell,
                node,
                start: Rc::new(start),
                namespace: text_input(namespace, "kube-system", cx),
                image,
                errors: FormErrors::default(),
                _image_observer,
            }
        });
        window.open_dialog(cx, move |dialog, _, _| {
            dialog
                .title(title.clone())
                .w(px(DIALOG_WIDTH))
                .child(body.clone())
        });
    }
}

#[cfg(test)]
#[path = "debug_dialogs_tests.rs"]
mod debug_dialogs_tests;
