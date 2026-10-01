//! The YAML tab of an open drawer: one masked GET of the selected object, shown in the kit's
//! read-only code editor. The cluster crate masks secrets before the text reaches this module,
//! and nothing here logs or writes it.

use cluster::{ClusterConnection, EnvValues, ObjectRef};
use gpui_kit::assets::IconName;
use gpui_kit::component::alert::Alert;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Editor, EditorState};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, Selectable as _, Sizable as _,
};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::{
    AnyElement, AppContext as _, ClipboardItem, Context, Entity, IntoElement, ParentElement as _,
    Render, SharedString, Styled as _, Task, Window, div, prelude::FluentBuilder as _,
};

use crate::age::format_age;
use crate::cluster_runtime::ClusterRuntime;
use crate::cluster_session::error_text;
use crate::drawer::{DRAWER_SUBJECT_DELAY, DrawerTab, drawer_tabs, shown_tab};
use crate::table_selection::ResourceKey;

/// The YAML tab of the open drawer. Dropping it aborts the request and frees the text.
pub(crate) struct YamlView {
    connection: ClusterConnection,
    object: ObjectRef,
    /// What the editor shows. A new subject starts at `Hidden`.
    env: EnvValues,
    editor: Entity<EditorState>,
    /// `None` until the first success.
    fetched_at: Option<jiff::Timestamp>,
    hidden_env_values: usize,
    request: YamlRequest,
}

enum YamlRequest {
    /// Dropping the task aborts both a pending delay and a running request.
    Running {
        _task: Task<()>,
    },
    Idle,
    Failed {
        message: String,
    },
}

/// Whether a fetch first waits `DRAWER_SUBJECT_DELAY`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FetchStart {
    Debounced,
    Immediate,
}

impl YamlView {
    /// Never notifies itself or the shell: it runs inside `AppShell::render`, and a notify
    /// there would re-render forever. The first notify comes from the fetch task.
    pub(crate) fn new(
        connection: ClusterConnection,
        object: ObjectRef,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let editor = cx.new(|cx| {
            EditorState::new(window, cx)
                .language("yaml")
                .line_number(true)
        });
        let mut view = Self {
            connection,
            object,
            env: EnvValues::Hidden,
            editor,
            fetched_at: None,
            hidden_env_values: 0,
            request: YamlRequest::Idle,
        };
        view.fetch(EnvValues::Hidden, FetchStart::Debounced, window, cx);
        view
    }

    pub(crate) fn is_for(&self, object: &ObjectRef) -> bool {
        self.object == *object
    }

    /// The first fetch is still in flight, so there is nothing to show yet.
    #[cfg(feature = "screenshot")]
    pub(crate) fn is_loading(&self) -> bool {
        self.fetched_at.is_none() && matches!(self.request, YamlRequest::Running { .. })
    }

    fn is_running(&self) -> bool {
        matches!(self.request, YamlRequest::Running { .. })
    }

    /// Replaces any running request. Only the first fetch of a view is debounced; the
    /// toolbar buttons are explicit and start at once.
    fn fetch(
        &mut self,
        env: EnvValues,
        start: FetchStart,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let connection = self.connection.clone();
        let object = self.object.clone();
        let runtime = cx.global::<ClusterRuntime>().clone();
        let task = cx.spawn_in(window, async move |this, cx| {
            if start == FetchStart::Debounced {
                cx.background_executor().timer(DRAWER_SUBJECT_DELAY).await;
            }
            let fetched = runtime
                .spawn(async move { connection.object_yaml(&object, env).await })
                .await;
            let _ = this.update_in(cx, |view, window, cx| match fetched {
                Ok(Ok(yaml)) => view.finish(env, yaml.text, yaml.hidden_env_values, window, cx),
                Ok(Err(error)) => view.fail(error_text(&error), cx),
                Err(_) => view.fail("The request stopped before it finished".to_owned(), cx),
            });
        });
        self.request = YamlRequest::Running { _task: task };
    }

    fn finish(
        &mut self,
        env: EnvValues,
        text: String,
        hidden_env_values: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.editor
            .update(cx, |editor, cx| editor.set_value(text, window, cx));
        self.env = env;
        self.fetched_at = Some(jiff::Timestamp::now());
        self.hidden_env_values = hidden_env_values;
        self.request = YamlRequest::Idle;
        cx.notify();
    }

    /// The editor, `env`, and `fetched_at` keep their last values, so a failed toggle does not
    /// flip the button.
    fn fail(&mut self, message: String, cx: &mut Context<Self>) {
        self.request = YamlRequest::Failed { message };
        cx.notify();
    }

    fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.fetch(self.env, FetchStart::Immediate, window, cx);
        cx.notify();
    }

    fn toggle_env_values(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let flipped = match self.env {
            EnvValues::Hidden => EnvValues::Shown,
            EnvValues::Shown => EnvValues::Hidden,
        };
        self.fetch(flipped, FetchStart::Immediate, window, cx);
        cx.notify();
    }

    /// Copies what the editor shows, so masked values stay masked.
    fn copy(&self, cx: &mut Context<Self>) {
        let text = self.editor.read(cx).text().to_string();
        cx.write_to_clipboard(ClipboardItem::new_string(text));
    }

    fn render_toolbar(&self, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let is_running = self.is_running();
        let status = fetch_status(self.fetched_at, &self.request, jiff::Timestamp::now());
        let is_shown = self.env == EnvValues::Shown;
        let env_tooltip = if is_shown {
            "Hide env values"
        } else {
            "Show the env values this view hides"
        };
        h_flex()
            .flex_shrink_0()
            .gap_2()
            .px_4()
            .py_2()
            .items_center()
            .border_b_1()
            .border_color(theme.border)
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(status),
            )
            .child(div().ml_auto())
            .when(shows_env_toggle(self.env, self.hidden_env_values), |bar| {
                bar.child(
                    Button::new("yaml-env")
                        .label("Env values")
                        .small()
                        .outline()
                        .selected(is_shown)
                        .disabled(is_running)
                        .tooltip(env_tooltip)
                        .on_click(
                            cx.listener(|view, _, window, cx| view.toggle_env_values(window, cx)),
                        ),
                )
            })
            .child(
                Button::new("yaml-copy")
                    .ghost()
                    .small()
                    .icon(Icon::new(IconName::Copy))
                    .disabled(self.fetched_at.is_none())
                    .tooltip("Copy YAML")
                    .on_click(cx.listener(|view, _, _, cx| view.copy(cx))),
            )
            .child(
                Button::new("yaml-refresh")
                    .ghost()
                    .small()
                    .icon(Icon::new(IconName::RefreshCw))
                    .disabled(is_running)
                    .tooltip("Fetch again")
                    .on_click(cx.listener(|view, _, window, cx| view.refresh(window, cx))),
            )
            .into_any_element()
    }

    fn render_alert(&self) -> Option<AnyElement> {
        let YamlRequest::Failed { message } = &self.request else {
            return None;
        };
        let alert = Alert::error("yaml-error", message.clone()).title("Cannot read the YAML");
        Some(
            div()
                .flex_shrink_0()
                .px_3()
                .py_2()
                .child(alert)
                .into_any_element(),
        )
    }

    fn render_body(&self, cx: &Context<Self>) -> AnyElement {
        let is_first_fetch_pending = self.fetched_at.is_none();
        if is_first_fetch_pending && self.is_running() {
            return v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap_3()
                .child(Spinner::new())
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child("Reading the YAML…"),
                )
                .into_any_element();
        }
        if is_first_fetch_pending {
            return div().into_any_element();
        }
        Editor::new(&self.editor)
            .readonly(true)
            .bordered(false)
            .text_xs()
            .size_full()
            .into_any_element()
    }
}

impl Render for YamlView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .child(self.render_toolbar(cx))
            .children(self.render_alert())
            .child(div().flex_1().min_h_0().child(self.render_body(cx)))
    }
}

/// `None` for a key the cluster crate cannot address (cannot happen for today's keys).
pub(crate) fn object_ref(key: &ResourceKey) -> Option<ObjectRef> {
    match key {
        ResourceKey::Pod { namespace, name } => ObjectRef::new(
            cluster::ObjectKind::Pod,
            Some(namespace.clone()),
            name.clone(),
        ),
        ResourceKey::Node { name } => ObjectRef::new(cluster::ObjectKind::Node, None, name.clone()),
        ResourceKey::Kind {
            kind,
            namespace,
            name,
        } => ObjectRef::new(kind.object(), namespace.clone(), name.clone()),
    }
}

/// The object the YAML tab should show, or `None` when the tab is not shown.
pub(crate) fn yaml_subject(selected: Option<&ResourceKey>, tab: DrawerTab) -> Option<ObjectRef> {
    let key = selected?;
    if shown_tab(drawer_tabs(key), tab) != DrawerTab::Yaml {
        return None;
    }
    object_ref(key)
}

fn fetch_status(
    fetched_at: Option<jiff::Timestamp>,
    request: &YamlRequest,
    now: jiff::Timestamp,
) -> SharedString {
    let text = match (fetched_at, request) {
        (None, YamlRequest::Running { .. }) => "Loading…".to_owned(),
        (Some(_), YamlRequest::Running { .. }) => "Refreshing…".to_owned(),
        (Some(fetched_at), _) => format!("Fetched {} ago", format_age(Some(fetched_at), now)),
        (None, _) => String::new(),
    };
    text.into()
}

/// The toggle appears when it has something to do: env values are hidden, or shown.
fn shows_env_toggle(env: EnvValues, hidden_env_values: usize) -> bool {
    env == EnvValues::Shown || hidden_env_values > 0
}

#[cfg(test)]
mod tests {
    use cluster::ObjectKind;

    use super::*;
    use crate::resource_kind::ResourceKind;

    fn at(seconds: i64) -> jiff::Timestamp {
        jiff::Timestamp::from_second(seconds).expect("valid timestamp")
    }

    fn running() -> YamlRequest {
        YamlRequest::Running {
            _task: Task::ready(()),
        }
    }

    fn failed() -> YamlRequest {
        YamlRequest::Failed {
            message: "boom".to_owned(),
        }
    }

    fn pod_key() -> ResourceKey {
        ResourceKey::Pod {
            namespace: "shop".to_owned(),
            name: "api-0".to_owned(),
        }
    }

    #[test]
    fn object_ref_maps_pods_nodes_and_kinds() {
        assert_eq!(
            object_ref(&pod_key()),
            ObjectRef::new(ObjectKind::Pod, Some("shop".to_owned()), "api-0".to_owned())
        );
        let node = ResourceKey::Node {
            name: "node-1".to_owned(),
        };
        assert_eq!(
            object_ref(&node),
            ObjectRef::new(ObjectKind::Node, None, "node-1".to_owned())
        );
        let namespace = ResourceKey::Kind {
            kind: ResourceKind::Namespaces,
            namespace: None,
            name: "shop".to_owned(),
        };
        assert_eq!(
            object_ref(&namespace),
            ObjectRef::new(ObjectKind::Namespace, None, "shop".to_owned())
        );
        let deployment = ResourceKey::Kind {
            kind: ResourceKind::Deployments,
            namespace: Some("shop".to_owned()),
            name: "api".to_owned(),
        };
        assert_eq!(
            object_ref(&deployment),
            ObjectRef::new(
                ObjectKind::Deployment,
                Some("shop".to_owned()),
                "api".to_owned()
            )
        );
        let event = ResourceKey::Kind {
            kind: ResourceKind::Events,
            namespace: Some("shop".to_owned()),
            name: "api-0.1".to_owned(),
        };
        assert_eq!(
            object_ref(&event),
            ObjectRef::new(
                ObjectKind::Event,
                Some("shop".to_owned()),
                "api-0.1".to_owned()
            )
        );
    }

    #[test]
    fn yaml_subject_only_when_the_yaml_tab_is_shown() {
        let key = pod_key();
        assert_eq!(yaml_subject(None, DrawerTab::Yaml), None);
        assert_eq!(yaml_subject(Some(&key), DrawerTab::Overview), None);
        assert_eq!(yaml_subject(Some(&key), DrawerTab::Events), None);
        assert_eq!(yaml_subject(Some(&key), DrawerTab::Yaml), object_ref(&key));
        let event = ResourceKey::Kind {
            kind: ResourceKind::Events,
            namespace: Some("shop".to_owned()),
            name: "api-0.1".to_owned(),
        };
        assert_eq!(
            yaml_subject(Some(&event), DrawerTab::Yaml),
            object_ref(&event)
        );
        // An event drawer has no Containers tab, so it falls back to Overview.
        assert_eq!(yaml_subject(Some(&event), DrawerTab::Containers), None);
    }

    #[test]
    fn fetch_status_by_state() {
        let now = at(100);
        let status = |fetched_at, request| fetch_status(fetched_at, &request, now).to_string();
        assert_eq!(status(None, running()), "Loading…");
        assert_eq!(status(Some(at(0)), running()), "Refreshing…");
        assert_eq!(status(Some(at(88)), YamlRequest::Idle), "Fetched 12s ago");
        assert_eq!(status(Some(at(88)), failed()), "Fetched 12s ago");
        assert_eq!(status(None, failed()), "");
        assert_eq!(status(None, YamlRequest::Idle), "");
    }

    #[test]
    fn env_toggle_shows_when_values_are_hidden_or_shown() {
        assert!(shows_env_toggle(EnvValues::Hidden, 2));
        assert!(shows_env_toggle(EnvValues::Shown, 0));
        assert!(!shows_env_toggle(EnvValues::Hidden, 0));
    }
}
