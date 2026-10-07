//! Switching clusters and the switcher popover in a headless window. The fixture kubeconfig
//! points at a closed local port, so every connect fails fast and nothing leaves the machine.
//! Sessions connect on a real tokio runtime that the fixture keeps alive for the whole test.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use gpui_kit::base::Root;
use gpui_kit::component::dialog::Confirm;
use gpui_kit::component::kbd::Kbd;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{TestAppContext, WindowHandle};

use super::app_shell_tests::{open_shell_on, render};
use super::*;
use crate::cluster_session::SessionPhase;
use crate::cluster_switcher::SwitcherConfirm;
use cluster::ProxyChoice;

use crate::cluster_registry::open_cluster;

use crate::cluster_runtime::ClusterRuntime;
use crate::settings_window::{ManageClusters, OpenSettings};

static FIXTURE_SERIAL: AtomicU64 = AtomicU64::new(0);

pub(super) const FIXTURE_YAML: &str = "apiVersion: v1\nkind: Config\nclusters:\n  - name: c\n    cluster: { server: 'https://127.0.0.1:1' }\ncontexts:\n  - name: prod-a\n    context: { cluster: c }\n  - name: stg-b\n    context: { cluster: c }\n  - name: dev-c\n    context: { cluster: c }\n";

pub(super) struct SwitchFixture {
    /// Alive for the whole test: the sessions connect on it.
    pub(super) runtime: tokio::runtime::Runtime,
    pub(super) window: WindowHandle<Root>,
    pub(super) shell: Entity<AppShell>,
    pub(super) path: PathBuf,
}

impl Drop for SwitchFixture {
    fn drop(&mut self) {
        if let Some(dir) = self.path.parent() {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

/// A shell on `prod-a` of a kubeconfig with three contexts (`prod-a`, `stg-b`, `dev-c`).
pub(super) fn open_switch_fixture(name: &str, cx: &mut TestAppContext) -> SwitchFixture {
    open_switch_fixture_with(name, &[], cx)
}

/// The same, with more launch flags.
pub(super) fn open_switch_fixture_with(
    name: &str,
    extra: &[&str],
    cx: &mut TestAppContext,
) -> SwitchFixture {
    open_switch_fixture_over(name, FIXTURE_YAML, extra, cx)
}

/// The same over another kubeconfig text; a `--context` in `extra` overrides `prod-a`.
pub(super) fn open_switch_fixture_over(
    name: &str,
    yaml: &str,
    extra: &[&str],
    cx: &mut TestAppContext,
) -> SwitchFixture {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .expect("a tokio runtime");
    let handle = runtime.handle().clone();
    // The tokio threads wake gpui tasks, which the deterministic scheduler forbids by default.
    cx.executor().allow_parking();
    cx.update(|cx| cx.set_global(ClusterRuntime::new(handle)));
    // Tests of different files share names ("gate", "release"), and tests run in parallel in one
    // process, so the folder is unique per fixture: a shared one lets a test delete another's file.
    let serial = FIXTURE_SERIAL.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "k8sboard-0026-{name}-{}-{serial}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create the temp dir");
    let path = dir.join("kubeconfig.yaml");
    std::fs::write(&path, yaml).expect("write the fixture");
    let flags: Vec<&str> = ["--context", "prod-a"]
        .into_iter()
        .chain(extra.iter().copied())
        .collect();
    let (window, shell) = open_shell_on(&path.to_string_lossy(), &flags, cx);
    cx.run_until_parked();
    SwitchFixture {
        runtime,
        window,
        shell,
        path,
    }
}

impl SwitchFixture {
    pub(super) fn cluster(&self, context: &str, cx: &mut TestAppContext) -> ClusterRef {
        cx.update(|cx| {
            self.shell
                .read(cx)
                .all_switcher_sections(cx)
                .into_iter()
                .flat_map(|section| section.rows)
                .find(|row| row.cluster.context == context)
                .map(|row| row.cluster)
        })
        .expect("the fixture has that context")
    }

    pub(super) fn session(&self, cx: &mut TestAppContext) -> Entity<ClusterSession> {
        self.shell
            .read_with(cx, |shell, _| shell.session().cloned())
            .expect("a session")
    }

    pub(super) fn active_context(&self, cx: &mut TestAppContext) -> Option<String> {
        self.shell.read_with(cx, |shell, cx| {
            shell
                .session()
                .map(|session| session.read(cx).context().to_owned())
        })
    }

    pub(super) fn switch(&self, context: &str, cx: &mut TestAppContext) {
        let target = self.cluster(context, cx);
        self.shell
            .update(cx, |shell, cx| shell.switch_cluster(&target, cx));
    }

    /// Makes the active session live over a client that never connects, as if the connect had
    /// answered; its scope is `scope`.
    pub(super) fn go_live(&self, scope: NamespaceScope, cx: &mut TestAppContext) {
        let context = self.active_context(cx).expect("a session");
        let kubeconfig = Kubeconfig::parse(FIXTURE_YAML, &self.path).expect("the fixture parses");
        let connection = self
            .runtime
            .block_on(open_cluster(
                &kubeconfig,
                &context,
                &Ok(ProxyChoice::Kubeconfig),
            ))
            .expect("a client builds without a round trip");
        let session = self.session(cx);
        session.update(cx, |session, cx| {
            session.go_live_for_test(connection, scope, cx)
        });
        cx.run_until_parked();
    }

    /// Whether the element with `id` is in the last drawn frame (kit controls register themselves).
    pub(super) fn is_drawn(&self, id: &'static str, cx: &mut TestAppContext) -> bool {
        self.with_window(cx, |window, _| window.try_find(id).is_some())
    }

    pub(super) fn draw_twice(&self, cx: &mut TestAppContext) {
        render(self.window, cx);
        render(self.window, cx);
    }

    pub(super) fn wait_until(
        &self,
        what: &str,
        cx: &mut TestAppContext,
        done: impl Fn(&AppShell, &App) -> bool,
    ) {
        for _ in 0..1_500 {
            cx.run_until_parked();
            if self.shell.read_with(cx, |shell, cx| done(shell, cx)) {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("timed out waiting for {what}");
    }

    pub(super) fn wait_until_failed(&self, cx: &mut TestAppContext) {
        self.wait_until("the connect to fail", cx, |shell, cx| {
            shell.session().is_some_and(|session| {
                matches!(session.read(cx).phase(), SessionPhase::Failed { .. })
            })
        });
    }

    pub(super) fn with_window<R>(
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

    pub(super) fn press(&self, keys: &str, cx: &mut TestAppContext) {
        self.with_window(cx, |window, cx| window.press(keys, cx));
    }

    pub(super) fn open_switcher(&self, cx: &mut TestAppContext) {
        self.with_window(cx, |window, cx| {
            self.shell
                .update(cx, |shell, cx| shell.open_cluster_switcher(window, cx));
        });
        render(self.window, cx);
    }

    pub(super) fn is_switcher_open(&self, cx: &mut TestAppContext) -> bool {
        self.shell
            .read_with(cx, |shell, _| shell.switcher.is_open())
    }

    pub(super) fn highlight(&self, cx: &mut TestAppContext) -> Option<String> {
        self.shell.read_with(cx, |shell, _| {
            shell
                .switcher
                .highlight()
                .map(|cluster| cluster.context.clone())
        })
    }
}

/// `Ctrl` (`Cmd` on macOS) plus `keys`: the `secondary` modifier the bindings use.
pub(super) fn chord(keys: &str) -> String {
    let modifier = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    };
    format!("{modifier}-{keys}")
}

// ---- Step 1: the switch lifecycle ----

#[gpui_kit::test]
fn switch_releases_the_old_session(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture("release", cx);
    let old = fixture.session(cx).downgrade();
    fixture.switch("stg-b", cx);
    cx.run_until_parked();
    fixture.draw_twice(cx);
    assert!(old.upgrade().is_none(), "the old session is still held");
    assert_eq!(fixture.active_context(cx).as_deref(), Some("stg-b"));
}

#[gpui_kit::test]
fn old_session_is_gone_before_the_new_connect(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture("break-before-make", cx);
    fixture.switch("stg-b", cx);
    cx.run_until_parked();
    let gone = fixture
        .shell
        .read_with(cx, |shell, _| shell.old_session_gone_at_connect.clone());
    assert_eq!(gone, [true]);
}

#[gpui_kit::test]
fn switch_leaves_no_topology_of_the_old_cluster(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture_with("topology", &["--screen", "topology"], cx);
    let old = fixture.session(cx).downgrade();
    let key = ResourceKey::Kind {
        kind: ResourceKind::Services,
        namespace: Some("shop".to_owned()),
        name: "web".to_owned(),
    };
    fixture
        .shell
        .update(cx, |shell, cx| shell.show_in_topology(&key, cx));
    cx.run_until_parked();
    fixture.shell.read_with(cx, |shell, cx| {
        assert!(shell.topology.read(cx).header_count(cx).is_some());
    });
    fixture.switch("stg-b", cx);
    cx.run_until_parked();
    fixture.draw_twice(cx);
    // The view held the session and a namespace of the old cluster: both are gone.
    assert!(old.upgrade().is_none(), "the old session is still held");
    fixture.shell.read_with(cx, |shell, cx| {
        assert_eq!(shell.topology.read(cx).header_count(cx), None);
    });
}

#[gpui_kit::test]
fn switch_closes_the_drawer(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture("drawer", cx);
    let key = ResourceKey::Kind {
        kind: ResourceKind::Namespaces,
        namespace: None,
        name: "default".to_owned(),
    };
    let cluster = fixture.cluster("prod-a", cx);
    fixture.shell.update(cx, |shell, cx| {
        shell.change_selection(Some(ClusterObject::new(cluster, key)), cx);
        assert!(shell.selected.is_some());
    });
    fixture.switch("stg-b", cx);
    fixture
        .shell
        .read_with(cx, |shell, _| assert!(shell.selected.is_none()));
}

#[gpui_kit::test]
fn switch_closes_log_tabs(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture("log-tabs", cx);
    let kubeconfig = Kubeconfig::parse(FIXTURE_YAML, &fixture.path).expect("the fixture parses");
    let connection = fixture
        .runtime
        .block_on(open_cluster(
            &kubeconfig,
            "prod-a",
            &Ok(ProxyChoice::Kubeconfig),
        ))
        .expect("a client builds without a round trip");
    let target = LogTarget::of_workload(PodOwner::Deployment {
        namespace: "shop".to_owned(),
        name: "web".to_owned(),
    })
    .expect("a deployment has a log target");
    let cluster = fixture.cluster("prod-a", cx);
    fixture.with_window(cx, |window, cx| {
        let shell = fixture.shell.read(cx);
        let row = shell
            .row_context_of(&cluster, cx)
            .expect("the cluster is viewed");
        let dock = shell.dock.clone();
        dock.update(cx, |dock, cx| {
            dock.open(LogOrigin::new(&row, connection), target, window, cx)
        });
    });
    fixture
        .shell
        .read_with(cx, |shell, cx| assert!(shell.dock.read(cx).has_tabs()));
    fixture.switch("stg-b", cx);
    fixture
        .shell
        .read_with(cx, |shell, cx| assert!(!shell.dock.read(cx).has_tabs()));
}

#[gpui_kit::test]
fn switch_drops_the_launch_requests_of_the_old_cluster(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture("launch-requests", cx);
    let cluster = fixture.cluster("prod-a", cx);
    fixture.shell.update(cx, |shell, _| {
        shell.pending_custom_launch = Some(CustomLaunch {
            crd_name: "widgets.example.com",
            tab: None,
        });
        shell.pending_dialog_launch = Some(LaunchScreen::WhoCan);
        shell.pending_reveal = Some(ClusterObject::new(
            cluster,
            ResourceKey::Pod {
                namespace: "shop".to_owned(),
                name: "web".to_owned(),
            },
        ));
    });
    fixture.switch("stg-b", cx);
    fixture.shell.read_with(cx, |shell, _| {
        assert!(shell.pending_custom_launch.is_none());
        assert!(shell.pending_dialog_launch.is_none());
        assert!(shell.pending_reveal.is_none());
    });
}

#[gpui_kit::test]
fn switch_keeps_screen_and_resets_filters(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture("filters", cx);
    let screen = Screen::Kind(ResourceKind::Deployments);
    let filter_text = |cx: &mut TestAppContext| {
        fixture.shell.read_with(cx, |shell, cx| {
            shell.toolkit_state(cx).map(|state| state.text.to_string())
        })
    };
    fixture.shell.update(cx, |shell, cx| {
        shell.show_screen(screen, cx);
        shell.update_view(cx, |view| view.filter.text = "web".into());
    });
    assert_eq!(filter_text(cx).as_deref(), Some("web"));
    fixture.switch("stg-b", cx);
    cx.run_until_parked();
    fixture
        .shell
        .read_with(cx, |shell, _| assert_eq!(shell.screen, screen));
    assert_eq!(filter_text(cx).as_deref(), Some(""));
}

#[gpui_kit::test]
fn switch_to_active_target_is_noop(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture("noop", cx);
    let before = fixture.session(cx).entity_id();
    fixture.switch("prod-a", cx);
    cx.run_until_parked();
    assert_eq!(fixture.session(cx).entity_id(), before);
}

#[gpui_kit::test]
fn second_switch_wins_over_pending_connect(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture("second-wins", cx);
    let first = fixture.session(cx).downgrade();
    // One update, so the deferred connect of `stg-b` has not run when `dev-c` is chosen.
    let (second, third) = (fixture.cluster("stg-b", cx), fixture.cluster("dev-c", cx));
    fixture.shell.update(cx, |shell, cx| {
        shell.switch_cluster(&second, cx);
        shell.switch_cluster(&third, cx);
    });
    cx.run_until_parked();
    fixture.draw_twice(cx);
    assert!(first.upgrade().is_none());
    assert_eq!(fixture.active_context(cx).as_deref(), Some("dev-c"));
    let gone = fixture
        .shell
        .read_with(cx, |shell, _| shell.old_session_gone_at_connect.clone());
    assert!(!gone.contains(&false), "{gone:?}");
}

#[gpui_kit::test]
fn failed_connect_shows_back_to_previous(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture("back-shown", cx);
    fixture.switch("stg-b", cx);
    fixture.wait_until_failed(cx);
    render(fixture.window, cx);
    fixture.shell.read_with(cx, |shell, cx| {
        assert_eq!(shell.active_label(cx).as_deref(), Some("stg-b"));
        assert_eq!(shell.previous_label(cx).as_deref(), Some("prod-a"));
    });
    assert!(fixture.is_drawn("retry", cx));
    assert!(fixture.is_drawn("back", cx));
}

#[gpui_kit::test]
fn back_to_previous_returns_and_becomes_previous_in_turn(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture("back-returns", cx);
    fixture.switch("stg-b", cx);
    fixture.wait_until_failed(cx);
    fixture
        .shell
        .update(cx, |shell, cx| shell.back_to_previous(cx));
    cx.run_until_parked();
    assert_eq!(fixture.active_context(cx).as_deref(), Some("prod-a"));
    fixture.shell.read_with(cx, |shell, cx| {
        assert_eq!(shell.previous_label(cx).as_deref(), Some("stg-b"));
    });
}

#[gpui_kit::test]
fn back_is_hidden_when_previous_is_gone(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture("back-gone", cx);
    fixture.shell.update(cx, |shell, _| {
        shell.previous = Some(ClusterRef {
            kubeconfig: PathBuf::from("gone.yaml"),
            context: "gone".to_owned(),
        });
    });
    fixture.shell.read_with(cx, |shell, cx| {
        assert_eq!(shell.previous_label(cx), None);
    });
    fixture.wait_until_failed(cx);
    render(fixture.window, cx);
    // The failed view is drawn, without the Back button.
    assert!(fixture.is_drawn("retry", cx));
    assert!(!fixture.is_drawn("back", cx));
}

#[gpui_kit::test]
fn missing_target_shows_notice(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture("missing", cx);
    let before = fixture.session(cx).entity_id();
    let ghost = ClusterRef {
        kubeconfig: fixture.path.clone(),
        context: "ghost".to_owned(),
    };
    fixture
        .shell
        .update(cx, |shell, cx| shell.switch_cluster(&ghost, cx));
    cx.run_until_parked();
    assert_eq!(fixture.session(cx).entity_id(), before);
    let notices = cx.update(|cx| fixture.shell.read(cx).notices(cx));
    assert_eq!(notices, ["'ghost' is no longer in its kubeconfig"]);
}

#[gpui_kit::test]
fn a_switch_away_from_a_non_live_cluster_remembers_nothing(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture("scope-none", cx);
    // The fixture never goes live, so there is no scope to remember for `prod-a`.
    fixture.switch("stg-b", cx);
    fixture
        .shell
        .read_with(cx, |shell, _| assert!(shell.scope_memory.is_empty()));
}

#[gpui_kit::test]
fn a_switch_remembers_the_scope_of_a_live_cluster(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture("scope-live", cx);
    fixture.go_live(NamespaceScope::Named("kube-system".to_owned()), cx);
    let prod = fixture.cluster("prod-a", cx);
    fixture.switch("stg-b", cx);
    fixture.shell.read_with(cx, |shell, _| {
        assert_eq!(
            shell.scope_memory.get(&prod),
            Some(&NamespaceScope::Named("kube-system".to_owned()))
        );
    });
}

#[gpui_kit::test]
fn switch_starts_in_the_remembered_scope(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture("scope-start", cx);
    let remembered = NamespaceScope::Named("kube-system".to_owned());
    let stg = fixture.cluster("stg-b", cx);
    fixture.shell.update(cx, |shell, _| {
        shell.scope_memory.insert(stg, remembered.clone());
    });
    fixture.switch("stg-b", cx);
    cx.run_until_parked();
    let scopes = fixture
        .shell
        .read_with(cx, |shell, _| shell.connected_scopes.clone());
    // The first start had no scope; the switch carries the remembered one.
    assert_eq!(scopes, [None, Some(remembered)]);
}

#[gpui_kit::test]
fn leaving_and_coming_back_restores_the_scope(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture("scope-round-trip", cx);
    let chosen = NamespaceScope::Named("payments".to_owned());
    fixture.go_live(chosen.clone(), cx);
    fixture.switch("stg-b", cx);
    cx.run_until_parked();
    fixture.switch("prod-a", cx);
    cx.run_until_parked();
    let scopes = fixture
        .shell
        .read_with(cx, |shell, _| shell.connected_scopes.clone());
    assert_eq!(scopes.last(), Some(&Some(chosen)));
}

#[gpui_kit::test]
fn palette_switch_starts_the_target_in_the_carried_scope(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture("scope-carried", cx);
    let carried = NamespaceScope::Named("payments".to_owned());
    let stg = fixture.cluster("stg-b", cx);
    // The target remembers another namespace: the carried scope wins for this switch.
    fixture.shell.update(cx, |shell, _| {
        shell
            .scope_memory
            .insert(stg.clone(), NamespaceScope::Named("default".to_owned()));
    });
    fixture.shell.update(cx, |shell, cx| {
        shell.switch_cluster_in_scope(&stg, Some(carried.clone()), cx);
    });
    cx.run_until_parked();
    let scopes = fixture
        .shell
        .read_with(cx, |shell, _| shell.connected_scopes.clone());
    assert_eq!(scopes, [None, Some(carried)]);
}

#[gpui_kit::test]
fn palette_switch_with_open_work_asks_then_carries_the_scope(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture("scope-carried-asks", cx);
    let carried = NamespaceScope::Named("payments".to_owned());
    let (prod, stg) = (fixture.cluster("prod-a", cx), fixture.cluster("stg-b", cx));
    // A running batch is the cheapest open work the leaving dialog asks about.
    fixture.shell.update(cx, |shell, cx| {
        shell.running_batches.insert(prod);
        shell.switch_cluster_in_scope(&stg, Some(carried.clone()), cx);
    });
    cx.run_until_parked();
    let asked = fixture
        .shell
        .read_with(cx, |shell, _| shell.last_leaving.clone());
    assert!(asked.is_some(), "the dialog asked before leaving");
    // Nothing started yet: the scope waits for Continue.
    let before = fixture
        .shell
        .read_with(cx, |shell, _| shell.connected_scopes.clone());
    assert_eq!(before, [None]);
    fixture.with_window(cx, |window, cx| {
        window.dispatch_action(Box::new(Confirm { secondary: false }), cx);
    });
    cx.run_until_parked();
    let scopes = fixture
        .shell
        .read_with(cx, |shell, _| shell.connected_scopes.clone());
    assert_eq!(scopes, [None, Some(carried)]);
}

#[gpui_kit::test]
fn the_namespace_flag_sets_the_first_scope(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture_with("scope-flag", &["--namespace", "web"], cx);
    let scopes = fixture
        .shell
        .read_with(cx, |shell, _| shell.connected_scopes.clone());
    assert_eq!(scopes, [Some(NamespaceScope::Named("web".to_owned()))]);
}

#[gpui_kit::test]
fn a_leaving_live_session_is_recorded_as_reachable(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture("leave-live", cx);
    fixture.go_live(NamespaceScope::All, cx);
    let prod = fixture.cluster("prod-a", cx);
    fixture.switch("stg-b", cx);
    let health = fixture
        .shell
        .read_with(cx, |shell, _| shell.switcher.health().row_health(&prod));
    assert_eq!(
        health,
        RowHealth::Reachable(std::time::Duration::from_millis(7))
    );
}

#[gpui_kit::test]
fn a_leaving_failed_session_is_recorded_as_unreachable(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture("leave-failed", cx);
    fixture.wait_until_failed(cx);
    let prod = fixture.cluster("prod-a", cx);
    fixture.switch("stg-b", cx);
    let health = fixture
        .shell
        .read_with(cx, |shell, _| shell.switcher.health().row_health(&prod));
    assert_eq!(health, RowHealth::Unreachable);
}

// ---- Step 2: the popover ----

#[gpui_kit::test]
fn open_focuses_the_filter(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture("focus-open", cx);
    render(fixture.window, cx);
    fixture.open_switcher(cx);
    let is_focused = fixture.with_window(cx, |window, cx| {
        let filter = fixture.shell.read(cx).switcher.filter().clone();
        filter.read(cx).focus_handle(cx).is_focused(window)
    });
    assert!(is_focused);
}

#[gpui_kit::test]
fn close_restores_previous_focus(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture("focus-close", cx);
    render(fixture.window, cx);
    let root_is_focused = |cx: &mut TestAppContext| {
        fixture.with_window(cx, |window, cx| {
            fixture.shell.read(cx).focus_handle.is_focused(window)
        })
    };
    assert!(root_is_focused(cx));
    fixture.open_switcher(cx);
    assert!(!root_is_focused(cx));
    fixture.press("escape", cx);
    render(fixture.window, cx);
    assert!(!fixture.is_switcher_open(cx));
    assert!(root_is_focused(cx));
}

#[gpui_kit::test]
fn row_click_switches_and_closes(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture("row-click", cx);
    fixture.open_switcher(cx);
    // Rows are numbered in display order: Production, Staging, Development.
    fixture.with_window(cx, |window, cx| window.click(("switcher-row", 2usize), cx));
    assert!(!fixture.is_switcher_open(cx));
    assert_eq!(fixture.active_context(cx).as_deref(), Some("stg-b"));
}

#[gpui_kit::test]
fn footer_dispatches_manage_clusters(cx: &mut TestAppContext) {
    static OPENED: AtomicBool = AtomicBool::new(false);
    let fixture = open_switch_fixture("footer", cx);
    cx.update(|cx| {
        cx.on_action(|_: &ManageClusters, _| OPENED.store(true, Ordering::SeqCst));
    });
    fixture.open_switcher(cx);
    fixture.with_window(cx, |window, cx| window.click("switcher-manage", cx));
    assert!(OPENED.load(Ordering::SeqCst));
    assert!(!fixture.is_switcher_open(cx));
}

#[gpui_kit::test]
fn segment_click_filters_to_connected(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture("segment", cx);
    fixture.open_switcher(cx);
    fixture.with_window(cx, |window, cx| window.click("switcher-connected", cx));
    let segment = fixture
        .shell
        .read_with(cx, |shell, _| shell.switcher.segment());
    assert_eq!(segment, SwitcherSegment::Connected);
    // Nothing is Live or Reachable yet, so no row passes.
    assert_eq!(fixture.highlight(cx), None);
}

// ---- Step 3: keys ----

#[gpui_kit::test]
fn ctrl_shift_c_toggles(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture("toggle", cx);
    render(fixture.window, cx);
    fixture.press(&chord("shift-c"), cx);
    assert!(fixture.is_switcher_open(cx));
    render(fixture.window, cx);
    // The same chord closes it from inside the filter.
    fixture.press(&chord("shift-c"), cx);
    assert!(!fixture.is_switcher_open(cx));
}

#[gpui_kit::test]
fn ctrl_digit_switches_from_inside_the_switcher_filter(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture("digit", cx);
    fixture.open_switcher(cx);
    // Focus is in the text field; the chord still works there.
    fixture.press(&chord("3"), cx);
    assert!(!fixture.is_switcher_open(cx));
    assert_eq!(fixture.active_context(cx).as_deref(), Some("dev-c"));
}

#[gpui_kit::test]
fn ctrl_digit_switches_from_inside_quick_filter(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture("digit-quick", cx);
    // The filter bar is drawn for a live session only.
    fixture.go_live(NamespaceScope::All, cx);
    fixture
        .shell
        .update(cx, |shell, cx| shell.show_screen(Screen::Pods, cx));
    render(fixture.window, cx);
    fixture.with_window(cx, |window, cx| {
        window.dispatch_action(Box::new(FocusQuickFilter), cx);
    });
    render(fixture.window, cx);
    let quick_filter_is_focused = fixture.with_window(cx, |window, cx| {
        let quick_filter = fixture.shell.read(cx).quick_filter.clone();
        quick_filter.read(cx).focus_handle(cx).is_focused(window)
    });
    assert!(quick_filter_is_focused);
    fixture.press(&chord("2"), cx);
    assert_eq!(fixture.active_context(cx).as_deref(), Some("stg-b"));
}

#[gpui_kit::test]
fn ctrl_digit_works_from_the_main_window(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture("digit-main", cx);
    render(fixture.window, cx);
    fixture.press(&chord("2"), cx);
    assert_eq!(fixture.active_context(cx).as_deref(), Some("stg-b"));
}

#[gpui_kit::test]
fn ctrl_digit_beyond_rows_does_nothing(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture("digit-none", cx);
    render(fixture.window, cx);
    let before = fixture.session(cx).entity_id();
    fixture.press(&chord("9"), cx);
    assert_eq!(fixture.session(cx).entity_id(), before);
}

#[gpui_kit::test]
fn enter_in_filter_switches_to_highlight(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture("enter-filter", cx);
    fixture.open_switcher(cx);
    fixture.with_window(cx, |window, cx| window.input("stg", cx));
    assert_eq!(fixture.highlight(cx).as_deref(), Some("stg-b"));
    fixture.press("enter", cx);
    assert!(!fixture.is_switcher_open(cx));
    assert_eq!(fixture.active_context(cx).as_deref(), Some("stg-b"));
}

#[gpui_kit::test]
fn enter_on_row_switches(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture("enter-row", cx);
    fixture.open_switcher(cx);
    // Tab leaves the filter for the next control in the popover, which is no input.
    fixture.press("down", cx);
    fixture.press("tab", cx);
    render(fixture.window, cx);
    let filter_is_focused = fixture.with_window(cx, |window, cx| {
        let filter = fixture.shell.read(cx).switcher.filter().clone();
        filter.read(cx).focus_handle(cx).is_focused(window)
    });
    assert!(!filter_is_focused, "tab did not leave the filter");
    fixture.press("enter", cx);
    assert!(!fixture.is_switcher_open(cx));
    assert_eq!(fixture.active_context(cx).as_deref(), Some("stg-b"));
}

#[gpui_kit::test]
fn escape_in_switcher_filter_closes(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture("escape", cx);
    fixture.open_switcher(cx);
    fixture.press("escape", cx);
    assert!(!fixture.is_switcher_open(cx));
}

#[gpui_kit::test]
fn arrows_move_highlight_in_filter(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture("arrows", cx);
    fixture.open_switcher(cx);
    assert_eq!(fixture.highlight(cx).as_deref(), Some("prod-a"));
    fixture.press("down", cx);
    assert_eq!(fixture.highlight(cx).as_deref(), Some("stg-b"));
    fixture.press("down", cx);
    fixture.press("down", cx);
    // Wraps past the last row.
    assert_eq!(fixture.highlight(cx).as_deref(), Some("prod-a"));
    fixture.press("up", cx);
    assert_eq!(fixture.highlight(cx).as_deref(), Some("dev-c"));
    assert!(fixture.is_switcher_open(cx));
}

#[gpui_kit::test]
fn space_never_confirms_in_the_switcher(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture("space", cx);
    let (is_ignored, confirms) = cx.update(|cx| {
        let space = [gpui_kit::Keystroke::parse("space").expect("a valid keystroke")];
        let bindings = cx.all_bindings_for_input(&space);
        (
            bindings
                .iter()
                .any(|binding| binding.action().partial_eq(&gpui_kit::NoAction)),
            bindings
                .iter()
                .any(|binding| binding.action().partial_eq(&SwitcherConfirm)),
        )
    });
    assert!(is_ignored);
    assert!(!confirms);
    drop(fixture);
}

#[gpui_kit::test]
fn space_keeps_the_switcher_open(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture("space-open", cx);
    fixture.open_switcher(cx);
    let before = fixture.session(cx).entity_id();
    // In the filter.
    fixture.press("space", cx);
    assert!(fixture.is_switcher_open(cx));
    assert_eq!(fixture.session(cx).entity_id(), before);
    // On a row: Tab leaves the filter for the controls of the popover.
    fixture.press("down", cx);
    fixture.press("tab", cx);
    render(fixture.window, cx);
    fixture.press("space", cx);
    assert!(fixture.is_switcher_open(cx));
    assert_eq!(fixture.active_context(cx).as_deref(), Some("prod-a"));
    assert_eq!(fixture.session(cx).entity_id(), before);
}

#[gpui_kit::test]
fn enter_always_switches_to_the_highlight(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture("enter-highlight", cx);
    fixture.open_switcher(cx);
    assert_eq!(fixture.highlight(cx).as_deref(), Some("prod-a"));
    fixture.press("down", cx);
    assert_eq!(fixture.highlight(cx).as_deref(), Some("stg-b"));
    fixture.press("enter", cx);
    assert!(!fixture.is_switcher_open(cx));
    assert_eq!(fixture.active_context(cx).as_deref(), Some("stg-b"));
}

#[gpui_kit::test]
fn kbd_hint_of_a_row_resolves_in_app_shell_context(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture("kbd-row", cx);
    fixture.open_switcher(cx);
    let has_hint = fixture.with_window(cx, |window, _| {
        Kbd::binding_for_action(&SwitchToCluster1, Some("AppShell"), window).is_some()
    });
    assert!(has_hint);
}

#[gpui_kit::test]
fn kbd_hint_of_the_footer_resolves_in_app_shell_context(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture("kbd-footer", cx);
    fixture.open_switcher(cx);
    let has_hint = fixture.with_window(cx, |window, _| {
        Kbd::binding_for_action(&OpenSettings, Some("AppShell"), window).is_some()
    });
    assert!(has_hint);
}

// ---- Step 4: probes ----

#[gpui_kit::test]
fn open_probes_only_in_process_clusters_and_close_drops_them(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture("probes", cx);
    fixture.open_switcher(cx);
    let (probes, running) = fixture.shell.read_with(cx, |shell, _| {
        (
            shell.switcher.probe_count(),
            shell.switcher.health().is_probing(),
        )
    });
    // The active cluster is never probed: one stream for the other two.
    assert_eq!(probes, 1);
    assert!(running);
    fixture.press("escape", cx);
    fixture.shell.read_with(cx, |shell, _| {
        assert!(shell.switcher.probe_count() == 0);
        assert!(!shell.switcher.health().is_probing());
    });
}

#[gpui_kit::test]
fn unreachable_probe_marks_the_row(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture("probe-result", cx);
    fixture.open_switcher(cx);
    fixture.wait_until("both probes to answer", cx, |shell, _| {
        !shell.switcher.health().is_probing()
    });
    let stg = fixture.cluster("stg-b", cx);
    let health = fixture
        .shell
        .read_with(cx, |shell, _| shell.switcher.health().row_health(&stg));
    assert_eq!(health, RowHealth::Unreachable);
}

#[gpui_kit::test]
fn closing_switcher_drops_probes(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture("probes-drop", cx);
    fixture.open_switcher(cx);
    fixture
        .shell
        .update(cx, |shell, cx| shell.close_cluster_switcher(cx));
    fixture.shell.read_with(cx, |shell, _| {
        assert!(shell.switcher.probe_count() == 0);
        assert!(!shell.switcher.health().is_probing());
    });
}

#[gpui_kit::test]
fn retry_on_active_failed_row_retries_the_session(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture("retry-active", cx);
    fixture.wait_until_failed(cx);
    let active = fixture.cluster("prod-a", cx);
    fixture
        .shell
        .update(cx, |shell, cx| shell.probe_cluster(&active, cx));
    let is_connecting = fixture.shell.read_with(cx, |shell, cx| {
        shell.session().is_some_and(|session| {
            matches!(session.read(cx).phase(), SessionPhase::Connecting { .. })
        })
    });
    assert!(is_connecting);
    // No probe ran for the active cluster.
    fixture.shell.read_with(cx, |shell, _| {
        assert!(!shell.switcher.health().is_probing())
    });
}

#[gpui_kit::test]
fn retry_on_a_running_row_starts_no_second_probe(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture("retry-running", cx);
    fixture.open_switcher(cx);
    let stg = fixture.cluster("stg-b", cx);
    let before = fixture
        .shell
        .read_with(cx, |shell, _| shell.switcher.probe_count());
    fixture
        .shell
        .update(cx, |shell, cx| shell.probe_cluster(&stg, cx));
    let after = fixture
        .shell
        .read_with(cx, |shell, _| shell.switcher.probe_count());
    assert_eq!(before, after);
}

// ---- Command palette over a live list ----

pub(super) fn palette_pod(name: &str) -> cluster::PodSummary {
    cluster::PodSummary {
        is_finished: false,
        namespace: "shop".to_owned(),
        name: name.to_owned(),
        status: cluster::PodStatus::Reason(cluster::StatusReason::Running),
        ready: cluster::ReadyCount { ready: 1, total: 1 },
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
        node_selector: Vec::new(),
        node_affinity: Vec::new(),
        containers: Vec::new(),
    }
}

fn palette_pod_key(name: &str) -> ResourceKey {
    ResourceKey::Pod {
        namespace: "shop".to_owned(),
        name: name.to_owned(),
    }
}

/// The pod `name` of the fixture's primary cluster, as the palette and the cursor name it.
pub(super) fn palette_pod_object(
    fixture: &SwitchFixture,
    name: &str,
    cx: &mut TestAppContext,
) -> ClusterObject {
    let cluster = fixture
        .shell
        .read_with(cx, |shell, _| shell.active_cluster())
        .expect("the fixture has a primary cluster");
    ClusterObject::new(cluster, palette_pod_key(name))
}

/// A live fixture on Pods with two pods loaded.
fn pods_fixture(name: &str, cx: &mut TestAppContext) -> SwitchFixture {
    let fixture = open_switch_fixture(name, cx);
    fixture.go_live(NamespaceScope::All, cx);
    let session = fixture.session(cx);
    session.update(cx, |session, cx| {
        session.set_pods_for_test(vec![palette_pod("api-0"), palette_pod("web-0")], cx);
    });
    fixture
        .shell
        .update(cx, |shell, cx| shell.show_screen(Screen::Pods, cx));
    cx.run_until_parked();
    fixture.draw_twice(cx);
    fixture
}

#[gpui_kit::test]
fn palette_pair_copy_name_copies_the_hit(cx: &mut TestAppContext) {
    let fixture = pods_fixture("pair-copy-name", cx);
    let cursor = palette_pod_object(&fixture, "api-0", cx);
    let hit = palette_pod_object(&fixture, "web-0", cx);
    fixture.shell.update(cx, |shell, cx| {
        shell.change_selection(Some(cursor), cx);
    });
    fixture.with_window(cx, |window, cx| {
        fixture.shell.update(cx, |shell, cx| {
            shell.run_row_action_on(hit.clone(), RowAction::CopyName, window, cx);
        });
    });
    cx.run_until_parked();
    // The test platform has its own clipboard: the system clipboard is untouched.
    let copied = cx.read_from_clipboard().and_then(|item| item.text());
    assert_eq!(copied.as_deref(), Some("web-0"));
    fixture.shell.read_with(cx, |shell, _| {
        assert_eq!(shell.selected.as_ref(), Some(&hit))
    });
}

#[gpui_kit::test]
fn tab_moves_the_cursor_without_opening_the_drawer(cx: &mut TestAppContext) {
    let fixture = pods_fixture("tab-preview", cx);
    let key = palette_pod_object(&fixture, "web-0", cx);
    fixture.shell.update(cx, |shell, cx| {
        assert!(shell.can_preview_row(&key, cx));
        shell.preview_resource(&key, cx);
    });
    cx.run_until_parked();
    fixture.shell.read_with(cx, |shell, _| {
        assert_eq!(shell.selected.as_ref(), Some(&key));
        assert!(!shell.drawer.is_open);
        assert_eq!(shell.screen, Screen::Pods);
    });
}

#[gpui_kit::test]
fn tab_ignores_a_resource_of_another_screen(cx: &mut TestAppContext) {
    let fixture = pods_fixture("tab-other-screen", cx);
    let cluster = fixture.cluster("prod-a", cx);
    let node = ClusterObject::new(
        cluster,
        ResourceKey::Node {
            name: "node-1".to_owned(),
        },
    );
    fixture.shell.update(cx, |shell, cx| {
        assert!(!shell.can_preview_row(&node, cx));
        shell.preview_resource(&node, cx);
        assert_eq!(shell.selected, None);
        assert_eq!(shell.screen, Screen::Pods);
    });
}

#[gpui_kit::test]
fn tab_does_nothing_while_a_drawer_is_open(cx: &mut TestAppContext) {
    let fixture = pods_fixture("tab-drawer", cx);
    let open = palette_pod_object(&fixture, "api-0", cx);
    let other = palette_pod_object(&fixture, "web-0", cx);
    fixture.shell.update(cx, |shell, cx| {
        shell.change_selection(Some(open.clone()), cx);
        shell.set_drawer_open(true, cx);
        assert!(!shell.can_preview_row(&other, cx));
        shell.preview_resource(&other, cx);
        assert_eq!(shell.selected.as_ref(), Some(&open));
        assert!(shell.drawer.is_open);
    });
}

#[gpui_kit::test]
fn the_palette_lists_the_loaded_pods_and_the_row_actions_of_the_cursor(cx: &mut TestAppContext) {
    let fixture = pods_fixture("palette-live", cx);
    let key = palette_pod_object(&fixture, "api-0", cx);
    let web = palette_pod_object(&fixture, "web-0", cx);
    fixture.shell.update(cx, |shell, cx| {
        shell.change_selection(Some(key.clone()), cx);
    });
    let snapshot = fixture.shell.read_with(cx, |shell, cx| {
        shell.palette_snapshot(&parse_query("pod"), cx)
    });
    assert!(snapshot.context.has_session);
    let has_pod = |wanted: &ClusterObject| {
        snapshot.entries.iter().any(|entry| {
            matches!(&entry.target, crate::palette_search::PaletteTarget::Resource(found)
                if found == wanted)
        })
    };
    assert!(has_pod(&key) && has_pod(&web));
    assert!(snapshot.entries.iter().any(|entry| matches!(
        entry.target,
        crate::palette_search::PaletteTarget::RowAction(
            crate::resource_actions::RowAction::ViewLogs
        )
    )));
    // A query that cannot list resources builds none of them.
    let snapshot = fixture
        .shell
        .read_with(cx, |shell, cx| shell.palette_snapshot(&parse_query(""), cx));
    assert!(!snapshot.entries.iter().any(|entry| matches!(
        entry.target,
        crate::palette_search::PaletteTarget::Resource(_)
    )));
}

// ---- spec 0046: one cluster is open at a time ----

fn last_used(cx: &mut TestAppContext) -> Option<ClusterRef> {
    cx.update(|cx| AppSettings::get(cx).registry.last_used.clone())
}

fn select_pod_row(fixture: &SwitchFixture, row: usize, cx: &mut TestAppContext) {
    let table = fixture
        .shell
        .read_with(cx, |shell, _| shell.pod_table.clone());
    cx.update(|cx| table.update(cx, |table, cx| table.set_selected_row(row, cx)));
    cx.run_until_parked();
}

fn selected_name(fixture: &SwitchFixture, cx: &mut TestAppContext) -> Option<String> {
    fixture.shell.read_with(cx, |shell, _| {
        shell.selected.as_ref().map(|object| match &object.key {
            ResourceKey::Pod { name, .. } => name.clone(),
            other => panic!("not a pod: {other:?}"),
        })
    })
}

#[gpui_kit::test]
fn last_used_is_written_on_live(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture("last-used-live", cx);
    assert_eq!(last_used(cx), None);
    fixture.go_live(NamespaceScope::All, cx);
    assert_eq!(last_used(cx), Some(fixture.cluster("prod-a", cx)));
}

#[gpui_kit::test]
fn last_used_is_not_written_on_failure(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture("last-used-failed", cx);
    fixture.wait_until_failed(cx);
    assert_eq!(last_used(cx), None);
}

#[gpui_kit::test]
fn last_used_is_written_once_per_session(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture("last-used-once", cx);
    fixture.go_live(NamespaceScope::All, cx);
    // Another cluster is saved meanwhile; the same session going on does not write it back.
    let other = fixture.cluster("dev-c", cx);
    cx.update(|cx| {
        AppSettings::update(cx, |settings| {
            settings.registry.last_used = Some(other.clone())
        });
    });
    fixture
        .session(cx)
        .update(cx, |session, cx| session.set_pods_for_test(Vec::new(), cx));
    cx.run_until_parked();
    assert_eq!(last_used(cx), Some(other));
}

#[gpui_kit::test]
fn select_all_follows_the_ticks(cx: &mut TestAppContext) {
    let fixture = pods_fixture("select-all", cx);
    let is_all_checked = |cx: &mut TestAppContext| {
        fixture.shell.read_with(cx, |shell, cx| {
            shell.pod_table.read(cx).delegate().all_checked()
        })
    };
    assert!(!is_all_checked(cx));
    fixture
        .shell
        .update(cx, |shell, cx| shell.set_all_checked(true, cx));
    assert!(is_all_checked(cx));
    fixture
        .shell
        .update(cx, |shell, cx| shell.toggle_row_checked(0, cx));
    assert!(!is_all_checked(cx));
}

#[gpui_kit::test]
fn a_closed_drawer_keeps_its_cursor(cx: &mut TestAppContext) {
    let fixture = pods_fixture("closed-drawer-context", cx);
    select_pod_row(&fixture, 1, cx);
    assert_eq!(selected_name(&fixture, cx).as_deref(), Some("web-0"));
    fixture.shell.update(cx, |shell, cx| shell.close_drawer(cx));
    // The cursor stays on its row with the drawer closed.
    assert_eq!(selected_name(&fixture, cx).as_deref(), Some("web-0"));
    fixture
        .shell
        .read_with(cx, |shell, _| assert!(!shell.drawer.is_open));
    // A bare reveal moves it to the object it names, in the one open cluster.
    fixture
        .shell
        .update(cx, |shell, cx| shell.reveal(palette_pod_key("api-0"), cx));
    cx.run_until_parked();
    assert_eq!(selected_name(&fixture, cx).as_deref(), Some("api-0"));
}

#[gpui_kit::test]
fn palette_resource_entries_have_no_cluster_label(cx: &mut TestAppContext) {
    let fixture = pods_fixture("palette-single", cx);
    let snapshot = fixture.shell.read_with(cx, |shell, cx| {
        shell.palette_snapshot(&parse_query("pod"), cx)
    });
    let detail = snapshot.entries.iter().find_map(|entry| {
        matches!(
            entry.target,
            crate::palette_search::PaletteTarget::Resource(_)
        )
        .then(|| entry.detail.clone())
    });
    assert_eq!(detail.flatten().as_deref(), Some("shop/api-0"));
}

#[gpui_kit::test]
fn the_yaml_key_acts_on_the_cursor_row(cx: &mut TestAppContext) {
    let fixture = pods_fixture("yaml-key", cx);
    select_pod_row(&fixture, 1, cx);
    fixture.shell.update(cx, |shell, cx| shell.close_drawer(cx));
    fixture.draw_twice(cx);
    fixture.press("y", cx);
    cx.run_until_parked();
    fixture.shell.read_with(cx, |shell, _| {
        assert!(shell.drawer.is_open);
        assert_eq!(shell.drawer.tab, DrawerTab::Yaml);
        assert_eq!(
            shell.drawer_subject().map(|object| object.key.clone()),
            Some(palette_pod_key("web-0"))
        );
    });
}

#[gpui_kit::test]
fn right_click_menu_action_acts_on_the_clicked_row(cx: &mut TestAppContext) {
    let fixture = pods_fixture("right-click-menu", cx);
    // The cursor sits on the first row, with the drawer closed.
    select_pod_row(&fixture, 0, cx);
    fixture.shell.update(cx, |shell, cx| shell.close_drawer(cx));
    fixture.draw_twice(cx);
    assert_eq!(selected_name(&fixture, cx).as_deref(), Some("api-0"));
    // A right click on the second row opens its menu.
    fixture.with_window(cx, |window, cx| window.right_click(("row", 1usize), cx));
    cx.run_until_parked();
    fixture.draw_twice(cx);
    cx.run_until_parked();
    // The kit leaves the cursor alone, so the shell moves it: key actions run on the cursor.
    assert_eq!(selected_name(&fixture, cx).as_deref(), Some("web-0"));
    fixture
        .shell
        .read_with(cx, |shell, _| assert!(!shell.drawer.is_open));
    // The second clickable item of the menu is View YAML.
    for key in ["down", "down", "enter"] {
        fixture.press(key, cx);
        cx.run_until_parked();
    }
    fixture.shell.read_with(cx, |shell, _| {
        assert!(shell.drawer.is_open);
        assert_eq!(shell.drawer.tab, DrawerTab::Yaml);
        assert_eq!(
            shell.drawer_subject().map(|object| object.key.clone()),
            Some(palette_pod_key("web-0"))
        );
    });
}

#[gpui_kit::test]
fn a_disabled_menu_item_confirmed_by_key_acts_on_the_clicked_row(cx: &mut TestAppContext) {
    let fixture = pods_fixture("right-click-key", cx);
    select_pod_row(&fixture, 0, cx);
    fixture.shell.update(cx, |shell, cx| shell.close_drawer(cx));
    fixture.draw_twice(cx);
    fixture.with_window(cx, |window, cx| window.right_click(("row", 1usize), cx));
    cx.run_until_parked();
    fixture.draw_twice(cx);
    cx.run_until_parked();
    // The first item is confirmed without a handler of its own: the kit dispatches its action,
    // which runs on the cursor. It must be the clicked row.
    for key in ["down", "enter"] {
        fixture.press(key, cx);
        cx.run_until_parked();
    }
    assert_eq!(selected_name(&fixture, cx).as_deref(), Some("web-0"));
}

#[gpui_kit::test]
fn a_switch_clears_ticks_of_a_same_named_row(cx: &mut TestAppContext) {
    let fixture = pods_fixture("clears-ticks", cx);
    fixture.shell.update(cx, |shell, cx| {
        shell.check_rows(crate::table_view::RowCheck::Toggle(0), cx);
    });
    cx.run_until_parked();
    let ticked = |cx: &mut TestAppContext| {
        fixture.shell.read_with(cx, |shell, cx| {
            let count = shell
                .pod_table
                .read(cx)
                .delegate()
                .view()
                .map_or(0, crate::table_view::TableView::checked_count);
            (count, shell.checked_objects(cx).len())
        })
    };
    assert_eq!(ticked(cx), (1, 1), "api-0 of the first cluster is ticked");
    // The second cluster lists a pod of the same name and namespace.
    fixture.switch("stg-b", cx);
    cx.run_until_parked();
    fixture.go_live(NamespaceScope::All, cx);
    fixture.session(cx).update(cx, |session, cx| {
        session.set_pods_for_test(vec![palette_pod("api-0")], cx);
    });
    cx.run_until_parked();
    fixture.draw_twice(cx);
    assert_eq!(ticked(cx), (0, 0), "no tick follows the switch");
}

#[gpui_kit::test]
fn set_session_clears_ticks_and_anchor_on_any_change(cx: &mut TestAppContext) {
    let fixture = pods_fixture("set-session-clears", cx);
    let ticked = |cx: &mut TestAppContext| {
        fixture.shell.read_with(cx, |shell, cx| {
            shell
                .pod_table
                .read(cx)
                .delegate()
                .view()
                .map_or(0, crate::table_view::TableView::checked_count)
        })
    };
    let set_session = |session: Option<crate::row_context::TableSession>,
                       cx: &mut TestAppContext| {
        fixture.shell.update(cx, |shell, cx| {
            shell
                .pod_table
                .update(cx, |table, _| table.delegate_mut().set_session(session));
        });
    };
    let open = |cx: &mut TestAppContext| {
        fixture.shell.read_with(cx, |shell, _| {
            shell.active_session().map(ActiveSession::table_session)
        })
    };
    fixture.shell.update(cx, |shell, cx| {
        shell.check_rows(crate::table_view::RowCheck::Toggle(0), cx);
    });
    assert_eq!(ticked(cx), 1);
    // The same session again changes nothing.
    set_session(open(cx), cx);
    assert_eq!(ticked(cx), 1, "the same session keeps its ticks");
    // Going to none clears them, and coming back to the same session does not restore them.
    set_session(None, cx);
    assert_eq!(ticked(cx), 0, "none clears the ticks");
    set_session(open(cx), cx);
    assert_eq!(ticked(cx), 0, "the ticks are gone for good");
}

#[gpui_kit::test]
fn session_of_is_none_for_a_cluster_that_is_not_active(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture("slot-session", cx);
    let prod = fixture.cluster("prod-a", cx);
    let stg = fixture.cluster("stg-b", cx);
    let has_session = |cluster: &ClusterRef, cx: &mut TestAppContext| {
        fixture
            .shell
            .read_with(cx, |shell, _| shell.session_of(cluster).is_some())
    };
    assert!(has_session(&prod, cx));
    assert!(!has_session(&stg, cx), "a cluster that is not open");
    fixture.switch("stg-b", cx);
    cx.run_until_parked();
    assert!(has_session(&stg, cx));
    assert!(!has_session(&prod, cx), "the cluster just left");
}

fn ticked_pod_names(fixture: &SwitchFixture, cx: &mut TestAppContext) -> Vec<String> {
    fixture.shell.read_with(cx, |shell, cx| {
        shell
            .pod_table
            .read(cx)
            .delegate()
            .checked_objects(cx)
            .into_iter()
            .map(|object| match object.key {
                ResourceKey::Pod { name, .. } => name,
                other => panic!("not a pod: {other:?}"),
            })
            .collect()
    })
}

#[gpui_kit::test]
fn space_ticks_and_unticks_the_cursor_row(cx: &mut TestAppContext) {
    let fixture = pods_fixture("space-tick", cx);
    fixture.press("j", cx);
    fixture.press("space", cx);
    assert_eq!(ticked_pod_names(&fixture, cx), ["api-0"]);
    fixture.press("space", cx);
    assert!(ticked_pod_names(&fixture, cx).is_empty());
}

#[gpui_kit::test]
fn shift_j_moves_down_and_ticks_the_range(cx: &mut TestAppContext) {
    let fixture = pods_fixture("shift-j-tick", cx);
    fixture.press("j", cx);
    fixture.press("shift-j", cx);
    assert_eq!(ticked_pod_names(&fixture, cx), ["api-0", "web-0"]);
    assert_eq!(selected_name(&fixture, cx).as_deref(), Some("web-0"));
}

#[gpui_kit::test]
fn shift_k_moves_up_and_ticks_the_range(cx: &mut TestAppContext) {
    let fixture = pods_fixture("shift-k-tick", cx);
    select_pod_row(&fixture, 1, cx);
    fixture.draw_twice(cx);
    fixture.press("shift-k", cx);
    assert_eq!(ticked_pod_names(&fixture, cx), ["api-0", "web-0"]);
    assert_eq!(selected_name(&fixture, cx).as_deref(), Some("api-0"));
}

#[gpui_kit::test]
fn a_wrapping_shift_step_ticks_nothing(cx: &mut TestAppContext) {
    let fixture = pods_fixture("shift-wrap", cx);
    select_pod_row(&fixture, 1, cx);
    fixture.draw_twice(cx);
    // Shift J on the last row wraps the cursor to the top: it must not tick the whole table.
    fixture.press("shift-j", cx);
    assert!(ticked_pod_names(&fixture, cx).is_empty());
}

#[gpui_kit::test]
fn ctrl_a_ticks_every_shown_row_and_again_unticks(cx: &mut TestAppContext) {
    let fixture = pods_fixture("ctrl-a-tick", cx);
    fixture.press("j", cx);
    fixture.press(&chord("a"), cx);
    assert_eq!(ticked_pod_names(&fixture, cx), ["api-0", "web-0"]);
    fixture.press(&chord("a"), cx);
    assert!(ticked_pod_names(&fixture, cx).is_empty());
}

#[gpui_kit::test]
fn a_filter_that_hides_ticked_rows_says_how_many_it_unticked(cx: &mut TestAppContext) {
    let fixture = pods_fixture("unticked-notice", cx);
    let notice_count = |cx: &mut TestAppContext| {
        fixture.shell.read_with(cx, |shell, _| {
            shell.unticked_notice.as_ref().map(|n| n.count)
        })
    };
    fixture.shell.update(cx, |shell, cx| {
        shell.set_all_checked(true, cx);
        // Both test pods are Running, so the unhealthy chip hides them.
        shell.toggle_unhealthy(cx);
    });
    assert_eq!(notice_count(cx), Some(2));
    assert!(ticked_pod_names(&fixture, cx).is_empty());
    // Leaving the screen drops the notice.
    fixture
        .shell
        .update(cx, |shell, cx| shell.show_screen(Screen::Nodes, cx));
    assert_eq!(notice_count(cx), None);
}

#[gpui_kit::test]
fn clearing_the_ticks_yourself_is_not_announced(cx: &mut TestAppContext) {
    let fixture = pods_fixture("unticked-silent", cx);
    fixture.shell.update(cx, |shell, cx| {
        shell.set_all_checked(true, cx);
        shell.clear_checked(cx);
    });
    fixture
        .shell
        .read_with(cx, |shell, _| assert!(shell.unticked_notice.is_none()));
}
