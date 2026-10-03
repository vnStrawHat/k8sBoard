//! Switching clusters and the switcher popover in a headless window. The fixture kubeconfig
//! points at a closed local port, so every connect fails fast and nothing leaves the machine.
//! Sessions connect on a real tokio runtime that the fixture keeps alive for the whole test.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use gpui_kit::base::Root;
use gpui_kit::component::kbd::Kbd;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{TestAppContext, WindowHandle};

use super::app_shell_tests::{open_shell_on, render};
use super::*;
use crate::cluster_session::SessionPhase;
use crate::cluster_switcher::SwitcherConfirm;
use cluster::ClusterConnection;

use crate::cluster_runtime::ClusterRuntime;
use crate::settings_window::{ManageClusters, OpenSettings};

const FIXTURE_YAML: &str = "apiVersion: v1\nkind: Config\nclusters:\n  - name: c\n    cluster: { server: 'https://127.0.0.1:1' }\ncontexts:\n  - name: prod-a\n    context: { cluster: c }\n  - name: stg-b\n    context: { cluster: c }\n  - name: dev-c\n    context: { cluster: c }\n";

struct SwitchFixture {
    /// Alive for the whole test: the sessions connect on it.
    runtime: tokio::runtime::Runtime,
    window: WindowHandle<Root>,
    shell: Entity<AppShell>,
    path: PathBuf,
}

impl Drop for SwitchFixture {
    fn drop(&mut self) {
        if let Some(dir) = self.path.parent() {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

/// A shell on `prod-a` of a kubeconfig with three contexts (`prod-a`, `stg-b`, `dev-c`).
fn open_switch_fixture(name: &str, cx: &mut TestAppContext) -> SwitchFixture {
    open_switch_fixture_with(name, &[], cx)
}

/// The same, with more launch flags.
fn open_switch_fixture_with(name: &str, extra: &[&str], cx: &mut TestAppContext) -> SwitchFixture {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .expect("a tokio runtime");
    let handle = runtime.handle().clone();
    // The tokio threads wake gpui tasks, which the deterministic scheduler forbids by default.
    cx.executor().allow_parking();
    cx.update(|cx| cx.set_global(ClusterRuntime::new(handle)));
    let dir = std::env::temp_dir().join(format!("k8sboard-0026-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create the temp dir");
    let path = dir.join("kubeconfig.yaml");
    std::fs::write(&path, FIXTURE_YAML).expect("write the fixture");
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
    fn cluster(&self, context: &str, cx: &mut TestAppContext) -> ClusterRef {
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

    fn session(&self, cx: &mut TestAppContext) -> Entity<ClusterSession> {
        self.shell
            .read_with(cx, |shell, _| shell.session.clone())
            .expect("a session")
    }

    fn active_context(&self, cx: &mut TestAppContext) -> Option<String> {
        self.shell.read_with(cx, |shell, cx| {
            shell
                .session
                .as_ref()
                .map(|session| session.read(cx).context().to_owned())
        })
    }

    fn switch(&self, context: &str, cx: &mut TestAppContext) {
        let target = self.cluster(context, cx);
        self.shell
            .update(cx, |shell, cx| shell.switch_cluster(&target, cx));
    }

    /// Makes the active session live over a client that never connects, as if the connect had
    /// answered; its scope is `scope`.
    fn go_live(&self, scope: NamespaceScope, cx: &mut TestAppContext) {
        let context = self.active_context(cx).expect("a session");
        let kubeconfig = Kubeconfig::parse(FIXTURE_YAML, &self.path).expect("the fixture parses");
        let connection = self
            .runtime
            .block_on(ClusterConnection::open(&kubeconfig, &context))
            .expect("a client builds without a round trip");
        let session = self.session(cx);
        session.update(cx, |session, cx| {
            session.go_live_for_test(connection, scope, cx)
        });
        cx.run_until_parked();
    }

    /// Whether the element with `id` is in the last drawn frame (kit controls register themselves).
    fn is_drawn(&self, id: &'static str, cx: &mut TestAppContext) -> bool {
        self.with_window(cx, |window, _| window.try_find(id).is_some())
    }

    fn draw_twice(&self, cx: &mut TestAppContext) {
        render(self.window, cx);
        render(self.window, cx);
    }

    fn wait_until(
        &self,
        what: &str,
        cx: &mut TestAppContext,
        done: impl Fn(&AppShell, &App) -> bool,
    ) {
        for _ in 0..500 {
            cx.run_until_parked();
            if self.shell.read_with(cx, |shell, cx| done(shell, cx)) {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("timed out waiting for {what}");
    }

    fn wait_until_failed(&self, cx: &mut TestAppContext) {
        self.wait_until("the connect to fail", cx, |shell, cx| {
            shell.session.as_ref().is_some_and(|session| {
                matches!(session.read(cx).phase(), SessionPhase::Failed { .. })
            })
        });
    }

    fn with_window<R>(
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

    fn press(&self, keys: &str, cx: &mut TestAppContext) {
        self.with_window(cx, |window, cx| window.press(keys, cx));
    }

    fn open_switcher(&self, cx: &mut TestAppContext) {
        self.with_window(cx, |window, cx| {
            self.shell
                .update(cx, |shell, cx| shell.open_cluster_switcher(window, cx));
        });
        render(self.window, cx);
    }

    fn is_switcher_open(&self, cx: &mut TestAppContext) -> bool {
        self.shell
            .read_with(cx, |shell, _| shell.switcher.is_open())
    }

    fn highlight(&self, cx: &mut TestAppContext) -> Option<String> {
        self.shell.read_with(cx, |shell, _| {
            shell
                .switcher
                .highlight()
                .map(|cluster| cluster.context.clone())
        })
    }
}

/// `Ctrl` (`Cmd` on macOS) plus `keys`: the `secondary` modifier the bindings use.
fn chord(keys: &str) -> String {
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
        assert!(shell.topology.read(cx).header_count().is_some());
    });
    fixture.switch("stg-b", cx);
    cx.run_until_parked();
    fixture.draw_twice(cx);
    // The view held the session and a namespace of the old cluster: both are gone.
    assert!(old.upgrade().is_none(), "the old session is still held");
    fixture.shell.read_with(cx, |shell, cx| {
        assert_eq!(shell.topology.read(cx).header_count(), None);
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
    fixture.shell.update(cx, |shell, cx| {
        shell.change_selection(Some(key), cx);
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
        .block_on(ClusterConnection::open(&kubeconfig, "prod-a"))
        .expect("a client builds without a round trip");
    let target = LogTarget::of_workload(PodOwner::Deployment {
        namespace: "shop".to_owned(),
        name: "web".to_owned(),
    })
    .expect("a deployment has a log target");
    fixture.with_window(cx, |window, cx| {
        let dock = fixture.shell.read(cx).log_dock.clone();
        dock.update(cx, |dock, cx| dock.open(connection, target, window, cx));
    });
    fixture
        .shell
        .read_with(cx, |shell, cx| assert!(shell.log_dock.read(cx).has_tabs()));
    fixture.switch("stg-b", cx);
    fixture
        .shell
        .read_with(cx, |shell, cx| assert!(!shell.log_dock.read(cx).has_tabs()));
}

#[gpui_kit::test]
fn switch_drops_the_launch_requests_of_the_old_cluster(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture("launch-requests", cx);
    fixture.shell.update(cx, |shell, _| {
        shell.pending_custom_launch = Some(CustomLaunch {
            crd_name: "widgets.example.com",
            tab: None,
        });
        shell.pending_dialog_launch = Some(LaunchScreen::WhoCan);
        shell.pending_reveal = Some(ResourceKey::Pod {
            namespace: "shop".to_owned(),
            name: "web".to_owned(),
        });
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
fn space_stays_unbound(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture("space", cx);
    let bound_to_switcher = cx.update(|cx| {
        let space = [gpui_kit::Keystroke::parse("space").expect("a valid keystroke")];
        cx.all_bindings_for_input(&space)
            .iter()
            .any(|binding| binding.action().partial_eq(&SwitcherConfirm))
    });
    assert!(!bound_to_switcher);
    drop(fixture);
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
    cx.update(crate::settings_window::bind_keys);
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
        shell.session.as_ref().is_some_and(|session| {
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
