//! The forms that build a Secret from fields (UX round 3): New docker-registry Secret (the WHY of a
//! pull that has no secret opens it), New TLS Secret, and Replace certificate of a TLS Secret. Like
//! the metadata editor it collects fields, then hands one intent to the guarded flow
//! (`start_write`), which shows the confirm dialog with the tier, the server dry-run, and the audit
//! line. Nothing here sends a change.
//!
//! A password and a private key are hidden by default: the password input is masked, and the key
//! shows `•••• N chars` until the user opens it. The screenshot build blocks Paste and Show of the
//! key like every other secret value.

use cluster::{
    ClusterConnection, KeyCheck, ObjectKind, ObjectRef, SecretDetails, ValuesBase, ValuesBaseError,
};
use gpui_kit::assets::IconName;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState, Textarea, TextareaState};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Icon, Sizable as _};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, Focusable as _, InteractiveElement as _,
    IntoElement, KeyDownEvent, ParentElement as _, Render, SharedString, Styled as _, Task,
    WeakEntity, Window, div, px,
};
use zeroize::Zeroizing;

use super::AppShell;
use super::node_editor::{DIALOG_WIDTH, ask_before_closing, input_cell, text_input};
use super::write_flow::{WriteIntent, notify};
use crate::cluster_registry::ClusterRef;
use crate::cluster_runtime::ClusterRuntime;
use crate::fresh_enter::{confirms, is_enter};
use crate::keymap::FORWARD_FORM;
use crate::kind_access::KindAccess;
use crate::kind_row::KindObject;
use crate::node_edits::NodeScope;
use crate::object_templates::template_namespace;
use crate::resource_actions::{
    ActionAvailability, ResourceAction, RowAction, action_availability, action_label,
    is_tls_secret, subject_action, unavailable_text, values_edit_block,
};
use crate::secret_forms::{
    DEFAULT_REGISTRY_SERVER, RegistryFields, TlsReport, registry_intent, tls_create_intent,
    tls_replace_intent, tls_report, tls_report_lines,
};
use crate::secret_values::ValueAccess;
use crate::table_selection::ClusterObject;
use crate::values_edit::copy_text;
use crate::yaml_view::object_ref;

const LABEL_WIDTH: f32 = 110.;
/// The namespace a form starts in when the cluster has none to offer.
const DEFAULT_NAMESPACE: &str = "default";
/// How long the form waits for the permissions of Secrets: up to `REVIEW_POLLS` looks, `REVIEW_POLL`
/// apart.
const REVIEW_POLL: std::time::Duration = std::time::Duration::from_millis(100);
const REVIEW_POLLS: usize = 100;
const BLOCKED_TOOLTIP: &str = "Disabled in screenshot runs";

/// Which Secret a form builds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SecretFormKind {
    DockerRegistry,
    Tls,
    /// The certificate of an existing TLS Secret.
    TlsReplace,
}

/// The cluster a form acts on: its own cluster and the name the dialogs call it.
pub(crate) struct FormCluster {
    pub(crate) cluster: ClusterRef,
    pub(crate) name: SharedString,
}

/// What a replace reads before it opens: the connection that reads the Secret, the Secret, and the
/// expiry it shows now.
struct ReplaceSource {
    connection: ClusterConnection,
    runtime: ClusterRuntime,
    object: ObjectRef,
    old_not_after: Option<jiff::Timestamp>,
}

/// What a form opens with: the namespace and the name it starts from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SecretFormStart {
    pub(crate) kind: SecretFormKind,
    pub(crate) namespace: String,
    pub(crate) name: String,
}

struct RegistryInputs {
    server: Entity<InputState>,
    username: Entity<InputState>,
    password: Entity<InputState>,
    email: Entity<InputState>,
}

struct TlsInputs {
    certificate: Entity<TextareaState>,
    key: Entity<TextareaState>,
    is_key_shown: bool,
}

enum Load {
    Ready,
    Loading { _task: Task<()> },
    Failed(SharedString),
}

/// The body of the form dialog.
pub(crate) struct SecretForm {
    shell: WeakEntity<AppShell>,
    cluster: ClusterRef,
    cluster_name: SharedString,
    kind: SecretFormKind,
    access: ValueAccess,
    name: Entity<InputState>,
    namespace: Entity<InputState>,
    registry: Option<RegistryInputs>,
    tls: Option<TlsInputs>,
    /// The Secret as the server has it, for a replace.
    base: Option<ValuesBase>,
    /// The expiry the Secret shows now, for the `old → new` line of a replace.
    old_not_after: Option<jiff::Timestamp>,
    load: Load,
    /// Review… was pressed: the problem line shows now, not while the user is still typing.
    has_tried_review: bool,
    /// What the server said when it refused the change the last Review… sent: the form stayed open
    /// under the confirm dialog, so the fields can be fixed and reviewed again.
    refusal: Option<SharedString>,
}

fn hidden_input(
    placeholder: &'static str,
    window: &mut Window,
    cx: &mut App,
) -> Entity<InputState> {
    cx.new(|cx| {
        InputState::new(window, cx)
            .placeholder(placeholder)
            .masked(true)
    })
}

fn pem_field(
    placeholder: &'static str,
    window: &mut Window,
    cx: &mut App,
) -> Entity<TextareaState> {
    cx.new(|cx| {
        TextareaState::new(window, cx)
            .auto_grow(3, 6)
            .placeholder(placeholder)
    })
}

impl SecretForm {
    fn new(
        shell: WeakEntity<AppShell>,
        target: FormCluster,
        start: &SecretFormStart,
        access: ValueAccess,
        replace: Option<ReplaceSource>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let registry = (start.kind == SecretFormKind::DockerRegistry).then(|| RegistryInputs {
            server: text_input(DEFAULT_REGISTRY_SERVER, "registry server", window, cx),
            username: text_input("", "user name", window, cx),
            password: hidden_input("password or access token", window, cx),
            email: text_input("", "email (optional)", window, cx),
        });
        let tls = (start.kind != SecretFormKind::DockerRegistry).then(|| TlsInputs {
            certificate: pem_field(
                "Paste the PEM certificate, the chain may follow",
                window,
                cx,
            ),
            key: pem_field("Paste the PEM private key", window, cx),
            is_key_shown: false,
        });
        let mut old_not_after = None;
        let load = match replace {
            Some(ReplaceSource {
                connection,
                runtime,
                object,
                old_not_after: expiry,
            }) => {
                old_not_after = expiry;
                let task = cx.spawn_in(window, async move |this, cx| {
                    let read = runtime
                        .spawn(async move { connection.values_base(&object).await })
                        .await;
                    let _ = this.update_in(cx, |form, _, cx| form.loaded(read, cx));
                });
                Load::Loading { _task: task }
            }
            None => Load::Ready,
        };
        let FormCluster { cluster, name } = target;
        Self {
            shell,
            cluster,
            cluster_name: name,
            kind: start.kind,
            access,
            name: text_input(&start.name, "name", window, cx),
            namespace: text_input(&start.namespace, "namespace", window, cx),
            registry,
            tls,
            base: None,
            old_not_after,
            load,
            has_tried_review: false,
            refusal: None,
        }
    }

    fn loaded(
        &mut self,
        read: Result<Result<ValuesBase, ValuesBaseError>, tokio::task::JoinError>,
        cx: &mut Context<Self>,
    ) {
        match read {
            Ok(Ok(base)) => {
                self.base = Some(base);
                self.load = Load::Ready;
            }
            Ok(Err(error)) => {
                self.load = Load::Failed(format!("Cannot replace: {error}").into());
            }
            Err(_) => self.load = Load::Failed("The request task stopped".into()),
        }
        cx.notify();
    }

    fn is_replace(&self) -> bool {
        self.kind == SecretFormKind::TlsReplace
    }

    fn text_of(input: &Entity<InputState>, cx: &App) -> String {
        input.read(cx).value().to_string()
    }

    /// The two PEM texts, each copied once and wiped on drop.
    fn pem_texts(&self, cx: &App) -> Option<(Zeroizing<String>, Zeroizing<String>)> {
        let tls = self.tls.as_ref()?;
        Some((copy_text(&tls.certificate, cx), copy_text(&tls.key, cx)))
    }

    /// The change the fields describe now, or why there is none.
    fn intent(&self, cx: &App) -> Result<WriteIntent, SharedString> {
        let scope = NodeScope {
            cluster: &self.cluster,
            cluster_name: &self.cluster_name,
        };
        if let Some(registry) = &self.registry {
            let (name, namespace) = (
                Self::text_of(&self.name, cx),
                Self::text_of(&self.namespace, cx),
            );
            let password = Zeroizing::new(Self::text_of(&registry.password, cx));
            return registry_intent(
                &scope,
                &RegistryFields {
                    name: &name,
                    namespace: &namespace,
                    server: &Self::text_of(&registry.server, cx),
                    username: &Self::text_of(&registry.username, cx),
                    password: &password,
                    email: &Self::text_of(&registry.email, cx),
                },
            );
        }
        let Some((certificate, key)) = self.pem_texts(cx) else {
            return Err("Nothing to create".into());
        };
        if self.is_replace() {
            let Some(base) = &self.base else {
                return Err("Loading the secret…".into());
            };
            return tls_replace_intent(&scope, base, &certificate, &key, self.old_not_after);
        }
        tls_create_intent(
            &scope,
            &Self::text_of(&self.name, cx),
            &Self::text_of(&self.namespace, cx),
            &certificate,
            &key,
        )
    }

    /// Review…: starts the guarded flow, whose dialog opens over the form. The form stays open: a
    /// refusal comes back to it (`show_refusal`), and a commit that went through closes it. Nothing
    /// is sent from here.
    fn review(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !matches!(self.load, Load::Ready) {
            return;
        }
        let intent = match self.intent(cx) {
            Ok(intent) => intent,
            Err(_) => {
                self.has_tried_review = true;
                cx.notify();
                return;
            }
        };
        self.refusal = None;
        cx.notify();
        let shell = self.shell.clone();
        window.defer(cx, move |window, cx| {
            let _ = shell.update(cx, |shell, cx| shell.start_write(intent, window, cx));
        });
    }

    /// The server refused the change: its words show under the fields until the next Review….
    fn show_refusal(&mut self, text: SharedString, cx: &mut Context<Self>) {
        self.refusal = Some(text);
        cx.notify();
    }

    /// Whether any field has text: closing now would lose it.
    fn has_unsaved_fields(&self, cx: &App) -> bool {
        let typed = |input: &Entity<InputState>| !input.read(cx).value().is_empty();
        let registry = self.registry.as_ref().is_some_and(|inputs| {
            typed(&inputs.username) || typed(&inputs.password) || typed(&inputs.email)
        });
        let tls = self.tls.as_ref().is_some_and(|inputs| {
            inputs.certificate.read(cx).text().len() > 0 || inputs.key.read(cx).text().len() > 0
        });
        registry || tls
    }

    fn cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !ask_before_closing(self.has_unsaved_fields(cx), window, cx) {
            window.close_dialog(cx);
        }
    }

    /// Enter in a single-line field presses Review…, a fresh press only.
    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(registry) = &self.registry else {
            return;
        };
        let is_focused =
            |input: &Entity<InputState>| input.read(cx).focus_handle(cx).is_focused(window);
        let in_field = [
            &self.name,
            &self.namespace,
            &registry.server,
            &registry.username,
            &registry.password,
            &registry.email,
        ]
        .into_iter()
        .any(is_focused);
        if !is_enter(event) || !in_field {
            return;
        }
        window.prevent_default();
        cx.stop_propagation();
        if confirms(event) {
            self.review(window, cx);
        }
    }

    /// Paste of the key field: the clipboard text replaces the text, and nothing else sees it.
    fn paste_key(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.access == ValueAccess::Blocked {
            return;
        }
        let Some(tls) = &self.tls else {
            return;
        };
        let Some(text) = cx
            .read_from_clipboard()
            .and_then(|item| item.text())
            .map(Zeroizing::new)
        else {
            return;
        };
        tls.key.update(cx, |state, cx| {
            state.set_value(text.as_str().to_owned(), window, cx);
        });
        cx.notify();
    }

    fn toggle_key(&mut self, cx: &mut Context<Self>) {
        if self.access == ValueAccess::Blocked {
            return;
        }
        if let Some(tls) = &mut self.tls {
            tls.is_key_shown = !tls.is_key_shown;
        }
        cx.notify();
    }

    fn labeled(&self, label: &'static str, field: AnyElement, cx: &App) -> AnyElement {
        h_flex()
            .gap_2()
            .items_start()
            .child(
                div()
                    .w(px(LABEL_WIDTH))
                    .flex_shrink_0()
                    .pt_1()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(label),
            )
            .child(div().flex_1().min_w_0().child(field))
            .into_any_element()
    }

    fn input(&self, input: &Entity<InputState>, cx: &App) -> AnyElement {
        input_cell(Input::new(input).small(), false, cx.theme().danger).into_any_element()
    }

    /// The name and namespace of a new Secret, or the fixed ones of a replace.
    fn target_rows(&self, cx: &App) -> Vec<AnyElement> {
        if self.is_replace() {
            return Vec::new();
        }
        vec![
            self.labeled("Name", self.input(&self.name, cx), cx),
            self.labeled("Namespace", self.input(&self.namespace, cx), cx),
        ]
    }

    fn registry_rows(&self, registry: &RegistryInputs, cx: &App) -> Vec<AnyElement> {
        vec![
            self.labeled("Server", self.input(&registry.server, cx), cx),
            self.labeled("User name", self.input(&registry.username, cx), cx),
            self.labeled("Password", self.input(&registry.password, cx), cx),
            self.labeled("Email", self.input(&registry.email, cx), cx),
        ]
    }

    fn key_field(&self, tls: &TlsInputs, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (muted, border, radius) = (theme.muted_foreground, theme.border, theme.radius);
        let mono = theme.mono_font_family.clone();
        let is_blocked = self.access == ValueAccess::Blocked;
        let count = tls.key.read(cx).text().len();
        let body: AnyElement = if tls.is_key_shown {
            div()
                .flex_1()
                .min_w_0()
                .child(Textarea::new(&tls.key).w_full())
                .into_any_element()
        } else {
            let text = match count {
                0 => "empty".to_owned(),
                1 => "•••• 1 char".to_owned(),
                count => format!("•••• {count} chars"),
            };
            div()
                .flex_1()
                .px_2()
                .py_1()
                .rounded(radius)
                .border_1()
                .border_color(border)
                .text_sm()
                .font_family(mono)
                .text_color(muted)
                .child(text)
                .into_any_element()
        };
        let tooltip = |text: &'static str| if is_blocked { BLOCKED_TOOLTIP } else { text };
        v_flex()
            .gap_1()
            .child(body)
            .child(
                h_flex()
                    .gap_1()
                    .child(
                        Button::new("secret-form-key-paste")
                            .label("Paste")
                            .small()
                            .outline()
                            .disabled(is_blocked)
                            .tooltip(tooltip("Replace the key with the clipboard text"))
                            .on_click(cx.listener(|form, _, window, cx| {
                                form.paste_key(window, cx);
                            })),
                    )
                    .child(
                        Button::new("secret-form-key-show")
                            .icon(Icon::new(if tls.is_key_shown {
                                IconName::EyeOff
                            } else {
                                IconName::Eye
                            }))
                            .label(if tls.is_key_shown { "Hide" } else { "Show" })
                            .small()
                            .outline()
                            .disabled(is_blocked)
                            .tooltip(tooltip("Show the key text"))
                            .on_click(cx.listener(|form, _, _, cx| form.toggle_key(cx))),
                    ),
            )
            .into_any_element()
    }

    /// Subject, Not after, and the key check of the pair as typed now.
    fn report_rows(&self, cx: &App) -> Vec<AnyElement> {
        let Some((certificate, key)) = self.pem_texts(cx) else {
            return Vec::new();
        };
        let theme = cx.theme();
        let line = |text: String, color| {
            div()
                .text_sm()
                .text_color(color)
                .child(text)
                .into_any_element()
        };
        match tls_report(&certificate, &key) {
            TlsReport::Empty => Vec::new(),
            TlsReport::Problem(text) => vec![line(text.to_string(), theme.danger)],
            TlsReport::Ready(info) => {
                let color = if info.key == KeyCheck::Differs {
                    theme.danger
                } else {
                    theme.foreground
                };
                tls_report_lines(&info, jiff::Timestamp::now())
                    .into_iter()
                    .map(|text| line(text, color))
                    .collect()
            }
        }
    }

    fn render_fields(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut rows = self.target_rows(cx);
        if let Some(registry) = &self.registry {
            rows.extend(self.registry_rows(registry, cx));
        }
        if let Some(tls) = &self.tls {
            rows.push(self.labeled(
                "Certificate",
                Textarea::new(&tls.certificate).w_full().into_any_element(),
                cx,
            ));
            rows.push(self.labeled("Private key", self.key_field(tls, cx), cx));
            rows.extend(self.report_rows(cx));
        }
        v_flex().gap_2().children(rows).into_any_element()
    }

    fn render_footer(&self, cx: &mut Context<Self>) -> AnyElement {
        let is_ready = matches!(self.load, Load::Ready);
        h_flex()
            .w_full()
            .gap_2()
            .justify_end()
            .child(
                Button::new("secret-form-cancel")
                    .label("Cancel")
                    .small()
                    .outline()
                    .on_click(cx.listener(|form, _, window, cx| form.cancel(window, cx))),
            )
            .child(
                Button::new("secret-form-review")
                    .label("Review…")
                    .small()
                    .primary()
                    .disabled(!is_ready)
                    .on_click(cx.listener(|form, _, window, cx| form.review(window, cx))),
            )
            .into_any_element()
    }
}

impl Render for SecretForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, danger) = (theme.muted_foreground, theme.danger);
        let body: AnyElement = match &self.load {
            Load::Loading { .. } => h_flex()
                .gap_2()
                .items_center()
                .child(Spinner::new())
                .child(div().text_sm().text_color(muted).child("Loading secret…"))
                .into_any_element(),
            Load::Failed(text) => div()
                .text_sm()
                .text_color(danger)
                .child(text.clone())
                .into_any_element(),
            Load::Ready => {
                let problem = self
                    .has_tried_review
                    .then(|| self.intent(cx).err())
                    .flatten();
                v_flex()
                    .gap_3()
                    .child(self.render_fields(cx))
                    .children(problem.map(|text| div().text_sm().text_color(danger).child(text)))
                    .children(
                        self.refusal
                            .clone()
                            .map(|text| div().text_sm().text_color(danger).child(text)),
                    )
                    .into_any_element()
            }
        };
        v_flex()
            .key_context(FORWARD_FORM)
            .on_key_down(cx.listener(Self::on_key_down))
            .w_full()
            .gap_3()
            .child(body)
            .child(self.render_footer(cx))
    }
}

/// Opens the form as the window's modal. An outside click does not close it, and Escape asks first
/// when a field has text.
fn show_secret_form(form: Entity<SecretForm>, title: String, window: &mut Window, cx: &mut App) {
    window.open_dialog(cx, move |dialog, _, _| {
        let asked = form.clone();
        dialog
            .title(title.clone())
            .w(px(DIALOG_WIDTH))
            .child(form.clone())
            .overlay_closable(false)
            .on_cancel(move |_, window, cx| {
                !ask_before_closing(asked.read(cx).has_unsaved_fields(cx), window, cx)
            })
    });
}

impl AppShell {
    /// Opens New docker-registry Secret or New TLS Secret on the active cluster. The header menu and
    /// the WHY of a missing pull secret end here, after the gate said yes; the gate is read again
    /// because the button may be a moment old.
    pub(crate) fn open_secret_form(
        &mut self,
        start: SecretFormStart,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(cluster) = self.active_cluster() else {
            return;
        };
        // The create right of Secrets is reviewed when the Secrets screen shows. From a pod's
        // diagnosis it was never asked: ask now, and open the form once the answer is in.
        if self.is_secret_access_pending(&cluster, cx) {
            self.review_secret_access_then_open(start, cluster, window, cx);
            return;
        }
        self.open_secret_form_now(start, cluster, window, cx);
    }

    /// `open_secret_form` once the permissions have been asked: the gate decides, whatever it knows.
    fn open_secret_form_now(
        &mut self,
        start: SecretFormStart,
        cluster: ClusterRef,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let label = action_label(ResourceAction::CreateObject(ObjectKind::Secret));
        let (cluster_name, namespace) = {
            let Some(guard) = self.guard_for(&cluster, cx) else {
                notify(
                    window,
                    cx,
                    unavailable_text(label, "the cluster is not open"),
                );
                return;
            };
            if let ActionAvailability::Disabled { reason } =
                action_availability(ResourceAction::CreateObject(ObjectKind::Secret), &guard)
            {
                notify(window, cx, unavailable_text(label, &reason));
                return;
            }
            let namespace = self.live_of(&cluster, cx).map_or_else(
                || DEFAULT_NAMESPACE.to_owned(),
                |live| template_namespace(&live.scope).to_owned(),
            );
            (
                SharedString::from(guard.display_name().to_owned()),
                namespace,
            )
        };
        let start = SecretFormStart {
            namespace: if start.namespace.is_empty() {
                namespace
            } else {
                start.namespace
            },
            ..start
        };
        let target = FormCluster {
            cluster,
            name: cluster_name,
        };
        self.show_secret_form(&start, target, None, window, cx);
    }

    /// Whether the permissions of Secrets are not known yet: never asked, or still being asked.
    fn is_secret_access_pending(&self, cluster: &ClusterRef, cx: &App) -> bool {
        self.guard_for(cluster, cx).is_some_and(|guard| {
            !matches!(
                guard.kind_access.get(ObjectKind::Secret),
                Some(KindAccess::Known(_) | KindAccess::Unknown)
            )
        })
    }

    /// Asks for the permissions of Secrets without changing the kind the shown screen reviews, then
    /// opens the form when the answer comes (or after a few seconds, when the gate says why not).
    fn review_secret_access_then_open(
        &mut self,
        start: SecretFormStart,
        cluster: ClusterRef,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let screen_kind = self.screen.access_kind();
        if let Some(session) = self.session_of(&cluster).cloned() {
            session.update(cx, |session, cx| {
                session.request_kind_access(Some(ObjectKind::Secret), cx);
                session.request_kind_access(screen_kind, cx);
            });
        }
        cx.spawn_in(window, async move |this, cx| {
            for _ in 0..REVIEW_POLLS {
                cx.background_executor().timer(REVIEW_POLL).await;
                let is_pending = this
                    .read_with(cx, |shell, cx| shell.is_secret_access_pending(&cluster, cx))
                    .unwrap_or(false);
                if !is_pending {
                    break;
                }
            }
            // After the last poll the gate speaks for itself: `Checking permissions…` or the answer.
            let _ = this.update_in(cx, |shell, window, cx| {
                shell.open_secret_form_now(start, cluster, window, cx);
            });
        })
        .detach();
    }

    /// Opens Replace certificate on `subject`, a TLS Secret, in its own cluster.
    pub(crate) fn open_certificate_replace(
        &mut self,
        subject: ClusterObject,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let label = action_label(ResourceAction::ReplaceCertificate);
        let (Some(ResourceAction::ReplaceCertificate), Some(object)) = (
            subject_action(RowAction::ReplaceCertificate, &subject.key),
            object_ref(&subject.key),
        ) else {
            notify(
                window,
                cx,
                unavailable_text(label, "this object has no certificate to replace"),
            );
            return;
        };
        let (cluster_name, connection, expiry) = {
            let (Some(guard), Some(live)) = (
                self.guard_for(&subject.cluster, cx),
                self.live_of(&subject.cluster, cx),
            ) else {
                notify(
                    window,
                    cx,
                    unavailable_text(label, "the cluster is not open"),
                );
                return;
            };
            if let ActionAvailability::Disabled { reason } =
                action_availability(ResourceAction::ReplaceCertificate, &guard)
            {
                notify(window, cx, unavailable_text(label, &reason));
                return;
            }
            let row = live.row_of(&subject.key);
            let object_row = row.map(|row| &row.object);
            if object_row.is_some_and(|object| !is_tls_secret(object)) {
                notify(
                    window,
                    cx,
                    unavailable_text(label, "only a kubernetes.io/tls Secret has a certificate"),
                );
                return;
            }
            if let Some(reason) = object_row.and_then(values_edit_block) {
                notify(window, cx, unavailable_text(label, &reason));
                return;
            }
            let expiry = row.and_then(|row| match &row.object {
                KindObject::Secret(secret) => match &secret.details {
                    SecretDetails::Certificate { chain } => {
                        chain.first().map(|leaf| leaf.not_after)
                    }
                    _ => None,
                },
                _ => None,
            });
            (
                SharedString::from(guard.display_name().to_owned()),
                live.connection().clone(),
                expiry,
            )
        };
        let start = SecretFormStart {
            kind: SecretFormKind::TlsReplace,
            namespace: object.namespace().unwrap_or_default().to_owned(),
            name: object.name().to_owned(),
        };
        let runtime = cx.global::<ClusterRuntime>().clone();
        let target = FormCluster {
            cluster: subject.cluster,
            name: cluster_name,
        };
        let source = ReplaceSource {
            connection,
            runtime,
            object,
            old_not_after: expiry,
        };
        self.show_secret_form(&start, target, Some(source), window, cx);
    }

    fn show_secret_form(
        &mut self,
        start: &SecretFormStart,
        target: FormCluster,
        replace: Option<ReplaceSource>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let title = match start.kind {
            SecretFormKind::DockerRegistry if start.name.is_empty() => {
                format!("New docker-registry Secret in {}", start.namespace)
            }
            SecretFormKind::DockerRegistry => format!(
                "New docker-registry Secret {} in {}",
                start.name, start.namespace
            ),
            SecretFormKind::Tls => format!("New TLS Secret in {}", start.namespace),
            SecretFormKind::TlsReplace => format!(
                "Replace certificate of secret {} in {}",
                start.name, start.namespace
            ),
        };
        let access = self.secret_value_access();
        let shell = cx.weak_entity();
        let form = cx.new(|cx| SecretForm::new(shell, target, start, access, replace, window, cx));
        self.secret_form = Some(form.downgrade());
        show_secret_form(form, title, window, cx);
    }

    /// Hands the server's refusal of a Secret form's change back to the form, which is still open
    /// under the confirm dialog. `false` when no form is open (a Secret made from YAML), so the
    /// dialog keeps the text itself.
    pub(crate) fn secret_form_refused(
        &mut self,
        text: SharedString,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(form) = self.secret_form.as_ref().and_then(WeakEntity::upgrade) else {
            return false;
        };
        form.update(cx, |form, cx| form.show_refusal(text, cx));
        true
    }

    /// The commit of a Secret form's change went through: the form closes. The confirm dialog
    /// above it closed first, so the form is the top dialog.
    pub(crate) fn secret_form_done(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .secret_form
            .take()
            .and_then(|form| form.upgrade())
            .is_some()
        {
            window.close_dialog(cx);
        }
    }
}

/// What the shell tests read from an open form and how they type into it.
#[cfg(test)]
impl SecretForm {
    pub(crate) fn is_loaded(&self) -> bool {
        matches!(self.load, Load::Ready)
    }

    pub(crate) fn failure(&self) -> Option<SharedString> {
        match &self.load {
            Load::Failed(text) => Some(text.clone()),
            _ => None,
        }
    }

    pub(crate) fn kind(&self) -> SecretFormKind {
        self.kind
    }

    /// What the server said when it refused the last Review….
    pub(crate) fn refusal(&self) -> Option<SharedString> {
        self.refusal.clone()
    }

    pub(crate) fn name_text(&self, cx: &App) -> String {
        Self::text_of(&self.name, cx)
    }

    pub(crate) fn namespace_text(&self, cx: &App) -> String {
        Self::text_of(&self.namespace, cx)
    }

    pub(crate) fn fill_registry(
        &mut self,
        username: &str,
        password: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(registry) = &self.registry else {
            return;
        };
        registry.username.update(cx, |input, cx| {
            input.set_value(username.to_owned(), window, cx)
        });
        registry.password.update(cx, |input, cx| {
            input.set_value(password.to_owned(), window, cx)
        });
    }

    pub(crate) fn fill_pair(
        &mut self,
        certificate: &str,
        key: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tls) = &self.tls else {
            return;
        };
        tls.certificate.update(cx, |state, cx| {
            state.set_value(certificate.to_owned(), window, cx)
        });
        tls.key
            .update(cx, |state, cx| state.set_value(key.to_owned(), window, cx));
    }

    pub(crate) fn press_review(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.review(window, cx);
    }

    pub(crate) fn current_intent(&self, cx: &App) -> Result<WriteIntent, SharedString> {
        self.intent(cx)
    }

    /// Whether the key text is on screen.
    pub(crate) fn is_key_shown(&self) -> bool {
        self.tls.as_ref().is_some_and(|tls| tls.is_key_shown)
    }
}
