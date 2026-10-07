//! The YAML tab of an open drawer: one masked GET of the selected object, shown in the kit's
//! read-only code editor. The cluster crate masks secrets before the text reaches this module,
//! and nothing here logs or writes it.

use cluster::{ClusterConnection, EnvValues, ObjectRef, clean_yaml};
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
use crate::cluster_registry::ClusterRef;
use crate::cluster_runtime::ClusterRuntime;
use crate::cluster_session::error_text;
use crate::drawer::{DRAWER_SUBJECT_DELAY, DrawerTab, drawer_tabs, shown_tab};
use crate::file_export::{ExportState, export_file_name, start_export};
use crate::resource_kind::ResourceKind;
use crate::table_selection::ResourceKey;

/// The YAML tab of the open drawer. Dropping it aborts the request and frees the text.
pub(crate) struct YamlView {
    connection: ClusterConnection,
    /// The cluster that holds the object: the same name exists in several clusters.
    cluster: ClusterRef,
    object: ObjectRef,
    /// What the editor shows. A new subject starts at `Hidden`.
    env: EnvValues,
    editor: Entity<EditorState>,
    /// `None` until the first success.
    fetched_at: Option<jiff::Timestamp>,
    hidden_env_values: usize,
    request: YamlRequest,
    /// What the last Copy or Save said, until the next fetch.
    notice: Option<String>,
    /// Save as…: the save dialog, the write, and what they ended in.
    export: ExportState,
    _export: Option<Task<()>>,
    /// The hidden values the saved manifest left out, said once the file is written.
    saved_left_out: String,
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
        cluster: ClusterRef,
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
            cluster,
            object,
            env: EnvValues::Hidden,
            editor,
            fetched_at: None,
            hidden_env_values: 0,
            request: YamlRequest::Idle,
            notice: None,
            export: ExportState::Idle,
            _export: None,
            saved_left_out: String::new(),
        };
        view.fetch(EnvValues::Hidden, FetchStart::Debounced, window, cx);
        view
    }

    pub(crate) fn is_for(&self, cluster: &ClusterRef, object: &ObjectRef) -> bool {
        self.cluster == *cluster && self.object == *object
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
        self.notice = None;
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

    /// The manifest for Git: what the editor shows without `status`, the server's metadata, and
    /// the `<hidden>` values, with how many of those went. `Err` is the line to show.
    fn clean_text(&self, cx: &Context<Self>) -> Result<(String, String), String> {
        let shown = self.editor.read(cx).text().to_string();
        let clean =
            clean_yaml(&shown).map_err(|error| format!("Could not clean the YAML: {error}"))?;
        let left_out = hidden_note(clean.hidden_dropped);
        Ok((clean.text, left_out))
    }

    /// Copy clean YAML: the manifest for Git on the clipboard.
    fn copy_clean(&mut self, cx: &mut Context<Self>) {
        self.notice = Some(match self.clean_text(cx) {
            Ok((text, left_out)) => {
                cx.write_to_clipboard(ClipboardItem::new_string(text));
                format!("Copied clean YAML{left_out}")
            }
            Err(message) => message,
        });
        cx.notify();
    }

    /// Save as…: the save dialog first, then the manifest for Git is written to the chosen path.
    fn save_clean(&mut self, cx: &mut Context<Self>) {
        if self.export.is_busy() || self.fetched_at.is_none() {
            return;
        }
        let label = format!(
            "{}-{}",
            self.object.kind_name().to_ascii_lowercase(),
            self.object.name()
        );
        let name = export_file_name(&label, "yaml", jiff::Timestamp::now());
        self.export = ExportState::Choosing;
        self._export = Some(start_export(
            name,
            "YAML",
            |view: &mut Self, cx| view.clean_text(cx),
            Self::set_export,
            |view, left_out| view.saved_left_out = left_out,
            cx,
        ));
        cx.notify();
    }

    fn set_export(&mut self, state: ExportState, cx: &mut Context<Self>) {
        if let ExportState::Saved { file_name } = &state {
            self.notice = Some(format!("Saved to {file_name}{}", self.saved_left_out));
        }
        self.export = state;
        cx.notify();
    }

    fn render_toolbar(&self, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let is_running = self.is_running();
        let status = self.status_text();
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
                Button::new("yaml-copy-clean")
                    .ghost()
                    .small()
                    .icon(Icon::new(IconName::ClipboardCheck))
                    .disabled(self.fetched_at.is_none())
                    .tooltip(CLEAN_TOOLTIP)
                    .on_click(cx.listener(|view, _, _, cx| view.copy_clean(cx))),
            )
            .child(
                Button::new("yaml-save")
                    .ghost()
                    .small()
                    .icon(Icon::new(IconName::Download))
                    .disabled(self.fetched_at.is_none() || self.export.is_busy())
                    .tooltip("Save as… (clean YAML)")
                    .on_click(cx.listener(|view, _, _, cx| view.save_clean(cx))),
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

    /// The line left of the buttons: what the last Copy or Save said, else how fresh the YAML is.
    fn status_text(&self) -> SharedString {
        match &self.notice {
            Some(notice) => notice.clone().into(),
            None => fetch_status(self.fetched_at, &self.request, jiff::Timestamp::now()),
        }
    }

    fn render_alert(&self) -> Option<AnyElement> {
        if let ExportState::Failed { message } = &self.export {
            let alert =
                Alert::error("yaml-save-error", message.clone()).title("Cannot save the YAML");
            return Some(
                div()
                    .flex_shrink_0()
                    .px_3()
                    .py_2()
                    .child(alert)
                    .into_any_element(),
            );
        }
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
        // The release Secrets hold values the YAML tab must never show.
        ResourceKey::Kind {
            kind: ResourceKind::HelmReleases,
            ..
        } => None,
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
        } => kind.object_ref(namespace.clone(), name.clone()),
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

const CLEAN_TOOLTIP: &str = "Copy clean YAML: no status, no server metadata, no hidden values";

/// What the hidden values said when they left the manifest: ` · 2 hidden values left out`.
fn hidden_note(count: usize) -> String {
    match count {
        0 => String::new(),
        1 => " \u{b7} 1 hidden value left out".to_owned(),
        count => format!(" \u{b7} {count} hidden values left out"),
    }
}

/// The toggle appears when it has something to do: env values are hidden, or shown.
pub(crate) fn shows_env_toggle(env: EnvValues, hidden_env_values: usize) -> bool {
    env == EnvValues::Shown || hidden_env_values > 0
}

#[cfg(test)]
mod tests {
    use cluster::ObjectKind;
    use gpui_kit::Entity;

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
    fn helm_release_has_no_object_ref() {
        let key = ResourceKey::Kind {
            kind: ResourceKind::HelmReleases,
            namespace: Some("shop".to_owned()),
            name: "api".to_owned(),
        };
        assert_eq!(object_ref(&key), None);
        assert_eq!(yaml_subject(Some(&key), DrawerTab::Yaml), None);
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
    fn the_clean_note_counts_the_hidden_values_that_left() {
        assert_eq!(hidden_note(0), "");
        assert_eq!(hidden_note(1), " \u{b7} 1 hidden value left out");
        assert_eq!(hidden_note(3), " \u{b7} 3 hidden values left out");
    }

    const SECRET_JSON: &str = r#"{"apiVersion":"v1","kind":"Secret","metadata":{"name":"db","namespace":"shop","uid":"u-1","resourceVersion":"7","creationTimestamp":"2026-10-01T08:00:00Z","labels":{"app":"db"}},"type":"Opaque","data":{"password":"c2VjcmV0LXZhbHVl"}}"#;

    /// A YAML view of the Secret `shop/db` over a fake server, with its first fetch done.
    fn fetched_view(
        runtime: &tokio::runtime::Runtime,
        cx: &mut gpui_kit::TestAppContext,
    ) -> Entity<YamlView> {
        use cluster::WritePolicy;
        use cluster::fake_api::FakeApi;
        cx.executor().allow_parking();
        cx.update(|cx| cx.set_global(ClusterRuntime::new(runtime.handle().clone())));
        let (connection, _api) = {
            let _guard = runtime.enter();
            FakeApi::connection(WritePolicy::Blocked, |_| (200, SECRET_JSON.to_owned()))
        };
        let cluster = ClusterRef {
            kubeconfig: std::path::PathBuf::from("test.yaml"),
            context: "stg-b".to_owned(),
        };
        let object = ObjectRef::new(
            cluster::ObjectKind::Secret,
            Some("shop".to_owned()),
            "db".to_owned(),
        )
        .expect("a secret has a namespace");
        let view = cx.update(|cx| {
            gpui_kit::init(cx);
            gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| YamlView::new(connection, cluster, object, window, cx))
            })
            .expect("open the test window")
            .1
        });
        cx.executor().advance_clock(DRAWER_SUBJECT_DELAY * 2);
        for _ in 0..200 {
            cx.run_until_parked();
            if view.read_with(cx, |view, _| view.fetched_at.is_some()) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        view
    }

    #[gpui_kit::test]
    fn copy_clean_puts_the_manifest_without_server_fields_or_hidden_values_on_the_clipboard(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .expect("a tokio runtime");
        let view = fetched_view(&runtime, cx);
        view.update(cx, |view, cx| view.copy_clean(cx));
        let copied = cx
            .read_from_clipboard()
            .and_then(|item| item.text())
            .expect("the manifest is on the clipboard");
        assert!(copied.contains("name: db"), "{copied}");
        assert!(copied.contains("app: db"), "{copied}");
        for left_out in [
            "uid",
            "resourceVersion",
            "creationTimestamp",
            "hidden",
            "password",
        ] {
            assert!(!copied.contains(left_out), "{left_out} in {copied}");
        }
        let notice = view.read_with(cx, |view, _| view.notice.clone());
        assert_eq!(
            notice.as_deref(),
            Some("Copied clean YAML \u{b7} 1 hidden value left out")
        );
    }

    #[gpui_kit::test]
    fn save_as_writes_the_clean_manifest_to_the_chosen_path_and_says_so(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .expect("a tokio runtime");
        let view = fetched_view(&runtime, cx);
        let dir = std::env::temp_dir().join(format!("k8sboard-yaml-save-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp folder");
        let target = dir.join("secret-db.yaml");
        view.update(cx, |view, cx| view.save_clean(cx));
        cx.run_until_parked();
        // Nothing is written before the dialog returns a path.
        assert!(!target.exists());
        let chosen = target.clone();
        cx.simulate_new_path_selection(move |_| Some(chosen.clone()));
        for _ in 0..200 {
            cx.run_until_parked();
            if target.exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let written = std::fs::read_to_string(&target).expect("the manifest is written");
        assert!(written.contains("name: db"), "{written}");
        assert!(!written.contains("uid"), "{written}");
        cx.run_until_parked();
        let notice = view.read_with(cx, |view, _| view.notice.clone());
        assert_eq!(
            notice.as_deref(),
            Some("Saved to secret-db.yaml \u{b7} 1 hidden value left out")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn env_toggle_shows_when_values_are_hidden_or_shown() {
        assert!(shows_env_toggle(EnvValues::Hidden, 2));
        assert!(shows_env_toggle(EnvValues::Shown, 0));
        assert!(!shows_env_toggle(EnvValues::Hidden, 0));
    }
}
