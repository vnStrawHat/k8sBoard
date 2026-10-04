//! Fixtures of the log tab and dock window tests: a headless window over a live session whose pod
//! list the test sets, and a fake API server that answers the log reads. No test reaches a cluster.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use cluster::fake_api::FakeApi;
use cluster::{
    ClusterConnection, ContainerKind, ContainerProbes, ContainerState, ContainerSummary,
    Kubeconfig, NamespaceScope, PodStatus, PodSummary, ReadyCount, StatusReason, Termination,
    WritePolicy,
};
use gpui_kit::base::Root;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyView, App, AppContext as _, Bounds, Context, Entity, IntoElement, ParentElement as _, Point,
    Render, Styled as _, TestAppContext, Window, WindowBounds, WindowHandle, WindowOptions, div,
    px, size,
};

use crate::cluster_registry::ClusterRef;
use crate::cluster_runtime::ClusterRuntime;
use crate::cluster_session::ClusterSession;
use crate::custom_kind::CustomKindCache;
use crate::dock::LogOrigin;
use crate::log_tab::LogTab;
use crate::log_target::LogTarget;
use crate::settings::{AppSettings, Settings};
use crate::settings_store::{LoadedSettings, WriteMode};

/// One context whose server is a closed local port: the session goes live through
/// `go_live_for_test`, and its own watches fail fast.
const FIXTURE_YAML: &str = "apiVersion: v1\nkind: Config\nclusters:\n  - name: c\n    cluster: { server: 'https://127.0.0.1:1' }\ncontexts:\n  - name: prod-a\n    context: { cluster: c }\n";

/// The views a test puts in the window.
#[derive(Default)]
pub(crate) struct LogHost {
    children: Vec<AnyView>,
}

impl Render for LogHost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().children(self.children.clone())
    }
}

pub(crate) struct LogFixture {
    /// Alive for the whole test: the session and the log streams run on it.
    pub(crate) _runtime: tokio::runtime::Runtime,
    pub(crate) window: WindowHandle<Root>,
    pub(crate) host: Entity<LogHost>,
    pub(crate) session: Entity<ClusterSession>,
    /// The connection the tabs read through; its recorder shows what they asked.
    pub(crate) connection: ClusterConnection,
    pub(crate) api: FakeApi,
    pub(crate) cluster: ClusterRef,
}

/// A window and a live session that lists `pods`. Every pod log read is answered with `log_body`.
pub(crate) fn open_log_fixture(
    pods: Vec<PodSummary>,
    log_body: &'static str,
    cx: &mut TestAppContext,
) -> LogFixture {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .expect("a tokio runtime");
    // The tokio threads wake gpui tasks, which the deterministic scheduler forbids by default.
    cx.executor().allow_parking();
    let handle = runtime.handle().clone();
    let path = PathBuf::from("log-fixture.yaml");
    let kubeconfig = Kubeconfig::parse(FIXTURE_YAML, &path).expect("the fixture parses");
    let summary = kubeconfig.contexts()[0].clone();
    let own_connection = runtime
        .block_on(ClusterConnection::open(
            &kubeconfig,
            "prod-a",
            &cluster::ProxyChoice::Kubeconfig,
        ))
        .expect("a client builds without a round trip");
    let (connection, api) = {
        let _guard = runtime.enter();
        FakeApi::connection(WritePolicy::Blocked, move |request| {
            if request.path.ends_with("/log") {
                (200, log_body.to_owned())
            } else {
                (404, "{}".to_owned())
            }
        })
    };
    let (window, host, session) = cx.update(|cx| {
        cx.set_global(ClusterRuntime::new(handle));
        gpui_kit::init(cx);
        crate::keymap::bind_keys(cx);
        AppSettings::install(
            LoadedSettings {
                settings: Settings::default(),
                writes: WriteMode::Disabled,
                notice: None,
            },
            cx,
        );
        let session = cx.new(|cx| {
            ClusterSession::new(
                Arc::new(kubeconfig),
                &summary,
                None,
                None,
                CustomKindCache::default(),
                cx,
            )
        });
        let bounds = Bounds {
            origin: Point::default(),
            size: size(px(1320.), px(900.)),
        };
        let (window, host) = gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            cx,
            |_, cx| cx.new(|_| LogHost::default()),
        )
        .expect("open the test window");
        (
            window.downcast::<Root>().expect("a Root window"),
            host,
            session,
        )
    });
    session.update(cx, |session, cx| {
        session.go_live_for_test(own_connection, NamespaceScope::All, cx);
        session.set_pods_for_test(pods, cx);
    });
    cx.run_until_parked();
    LogFixture {
        _runtime: runtime,
        window,
        host,
        session,
        connection,
        api,
        cluster: ClusterRef {
            kubeconfig: path,
            context: "prod-a".to_owned(),
        },
    }
}

impl LogFixture {
    pub(crate) fn origin(&self) -> LogOrigin {
        LogOrigin {
            cluster: self.cluster.clone(),
            session: self.session.downgrade(),
            connection: self.connection.clone(),
        }
    }

    pub(crate) fn with_window<R>(
        &self,
        cx: &mut TestAppContext,
        run: impl FnOnce(&mut Window, &mut App) -> R,
    ) -> R {
        let result = cx
            .update_window(self.window.into(), |_, window, cx| run(window, cx))
            .expect("the window is open");
        cx.run_until_parked();
        result
    }

    /// Draws a frame, so the views have their bounds.
    pub(crate) fn draw(&self, cx: &mut TestAppContext) {
        self.with_window(cx, |window, cx| window.render_frame(cx));
    }

    /// Puts `view` in the window.
    pub(crate) fn show(&self, view: impl Into<AnyView>, cx: &mut TestAppContext) {
        let view = view.into();
        self.host.update(cx, |host, cx| {
            host.children.push(view);
            cx.notify();
        });
        self.draw(cx);
    }

    /// A log tab of `container` in the first pod of the list, shown in the window.
    pub(crate) fn open_tab(
        &self,
        pod: &PodSummary,
        container: &str,
        cx: &mut TestAppContext,
    ) -> Entity<LogTab> {
        let target = LogTarget::of_container(pod, container).expect("the pod has that container");
        let tab = self.with_window(cx, |window, cx| {
            let session = self.session.clone();
            cx.new(|cx| LogTab::new(self.origin(), target, &session, window, cx))
        });
        self.show(tab.clone(), cx);
        tab
    }

    /// Runs the executor until `done` holds.
    pub(crate) fn wait_until(
        &self,
        what: &str,
        cx: &mut TestAppContext,
        done: impl Fn(&mut TestAppContext) -> bool,
    ) {
        for _ in 0..500 {
            cx.run_until_parked();
            if done(cx) {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("timed out waiting for {what}");
    }
}

pub(crate) fn fixture_container(
    name: &str,
    restart_count: u32,
    last_termination: Option<Termination>,
) -> ContainerSummary {
    ContainerSummary {
        terminal: cluster::ContainerTerminal::None,
        name: name.to_owned(),
        image: "img".to_owned(),
        kind: ContainerKind::Main,
        state: ContainerState::Running { started_at: None },
        is_ready: true,
        restart_count,
        last_termination,
        image_digest: None,
        pull_policy: None,
        is_started: None,
        ports: Vec::new(),
        resources: Vec::new(),
        probes: ContainerProbes::default(),
        env: Vec::new(),
        env_from: Vec::new(),
        mounts: Vec::new(),
    }
}

/// The termination an OOM kill leaves behind.
pub(crate) fn oom_killed(finished_at: &str) -> Termination {
    Termination {
        reason: Some(StatusReason::OomKilled),
        exit_code: 137,
        signal: None,
        started_at: None,
        finished_at: finished_at.parse().ok(),
    }
}

pub(crate) fn fixture_pod(name: &str, containers: Vec<ContainerSummary>) -> PodSummary {
    PodSummary {
        is_finished: false,
        namespace: "shop".to_owned(),
        name: name.to_owned(),
        status: PodStatus::Reason(StatusReason::Running),
        ready: ReadyCount { ready: 1, total: 1 },
        restarts: 0,
        node_name: None,
        created_at: None,
        pod_ip: None,
        qos_class: None,
        service_account: None,
        controller: None,
        conditions: Vec::new(),
        status_message: None,
        labels: Vec::new(),
        host_network: false,
        image_pull_secrets: Vec::new(),
        containers,
    }
}

impl LogFixture {
    /// How many pod log reads the tabs sent so far.
    pub(crate) fn log_reads(&self) -> usize {
        self.api
            .requests()
            .iter()
            .filter(|request| request.path.ends_with("/log"))
            .count()
    }
}

impl LogFixture {
    /// An empty dock in the window, over this fixture's cluster.
    pub(crate) fn open_dock(&self, cx: &mut TestAppContext) -> Entity<crate::dock::Dock> {
        let dock = self.with_window(cx, |_, cx| {
            cx.new(|_| crate::dock::Dock::new(gpui_kit::WeakEntity::new_invalid()))
        });
        self.show(dock.clone(), cx);
        dock
    }

    /// Opens a dock tab of `container` in `pod`, as View logs does.
    pub(crate) fn open_in_dock(
        &self,
        dock: &Entity<crate::dock::Dock>,
        pod: &PodSummary,
        container: &str,
        cx: &mut TestAppContext,
    ) {
        let target = LogTarget::of_container(pod, container).expect("the pod has that container");
        self.with_window(cx, |window, cx| {
            dock.update(cx, |dock, cx| dock.open(self.origin(), target, window, cx));
        });
    }
}

impl LogFixture {
    /// Takes every view out of the window, as a tab that moves to another window leaves it.
    pub(crate) fn clear(&self, cx: &mut TestAppContext) {
        self.host.update(cx, |host, cx| {
            host.children.clear();
            cx.notify();
        });
        self.draw(cx);
    }
}
