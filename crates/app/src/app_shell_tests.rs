//! The `/` shortcut in a headless window. The kubeconfig is missing, so nothing touches the
//! network: the shell stays without a session, and the filter bar is not drawn. The quick
//! filter input is therefore out of the element tree, which is exactly what a focused input
//! that disappears looks like to the window.

use gpui_kit::base::Root;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AppContext as _, Bounds, Entity, Point, TestAppContext, WindowBounds, WindowHandle,
    WindowOptions, px, size,
};

use std::sync::atomic::{AtomicUsize, Ordering};

use super::app_shell_switch_tests::{open_switch_fixture, open_switch_fixture_with};
use super::*;
use crate::cluster_catalog::CatalogHandle;
use crate::cluster_registry::ClusterRef;
use crate::launch_options::{LaunchRequest, kubeconfig_chain, parse_launch_options};
use crate::settings::{AppSettings, Settings};
use crate::settings_store::{LoadedSettings, WriteMode};

/// Which of the shell's two focus targets holds the focus: the root, and the quick filter.
type Focus = (bool, bool);

const ROOT: Focus = (true, false);

fn open_shell(cx: &mut TestAppContext) -> (WindowHandle<Root>, Entity<AppShell>) {
    open_shell_with(&[], cx)
}

fn open_shell_with(
    extra: &[&str],
    cx: &mut TestAppContext,
) -> (WindowHandle<Root>, Entity<AppShell>) {
    open_shell_on("does-not-exist/kubeconfig.yml", extra, cx)
}

pub(super) fn open_shell_on(
    kubeconfig: &str,
    extra: &[&str],
    cx: &mut TestAppContext,
) -> (WindowHandle<Root>, Entity<AppShell>) {
    let leading = ["--kubeconfig", kubeconfig];
    let args = leading.iter().chain(extra).map(|arg| (*arg).to_owned());
    let Ok(LaunchRequest::Run(options)) = parse_launch_options(args) else {
        panic!("the launch flags are valid");
    };
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::keymap::bind_keys(cx);
        crate::cluster_switcher::bind_keys(cx);
        // Writes stay off: a shell test never saves settings.
        AppSettings::install(
            LoadedSettings {
                settings: Settings::default(),
                writes: WriteMode::Disabled,
                notice: None,
            },
            cx,
        );
        let chain = kubeconfig_chain(options.kubeconfig.clone(), None, None);
        CatalogHandle::install(chain, cx);
        let bounds = Bounds {
            origin: Point::default(),
            size: size(px(1320.), px(900.)),
        };
        let (window, shell) = gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            cx,
            |window, cx| cx.new(|cx| AppShell::new(*options, window, cx)),
        )
        .expect("open the test window");
        (window.downcast::<Root>().expect("a Root window"), shell)
    })
}

pub(super) fn render(window: WindowHandle<Root>, cx: &mut TestAppContext) {
    cx.update_window(window.into(), |_, window, cx| window.render_frame(cx))
        .expect("the window is open");
    cx.run_until_parked();
}

fn focus_in(shell: &Entity<AppShell>, window: &Window, cx: &App) -> Focus {
    let shell = shell.read(cx);
    (
        shell.focus_handle.is_focused(window),
        shell
            .quick_filter
            .read(cx)
            .focus_handle(cx)
            .is_focused(window),
    )
}

fn focus_of(
    window: WindowHandle<Root>,
    shell: &Entity<AppShell>,
    cx: &mut TestAppContext,
) -> Focus {
    cx.update_window(window.into(), |_, window, cx| focus_in(shell, window, cx))
        .expect("the window is open")
}

fn press_slash(window: WindowHandle<Root>, cx: &mut TestAppContext) {
    cx.update_window(window.into(), |_, window, cx| window.press("/", cx))
        .expect("the window is open");
    cx.run_until_parked();
}

/// Whether the `/` action has a handler where the keyboard is, which is what a key binding
/// needs to fire.
fn is_slash_available(window: WindowHandle<Root>, cx: &mut TestAppContext) -> bool {
    cx.update_window(window.into(), |_, window, cx| {
        window.is_action_available(&FocusQuickFilter, cx)
    })
    .expect("the window is open")
}

#[gpui_kit::test]
fn slash_is_available_before_any_click(cx: &mut TestAppContext) {
    let (window, shell) = open_shell(cx);
    render(window, cx);
    assert_eq!(focus_of(window, &shell, cx), ROOT);
    assert!(is_slash_available(window, cx));
}

#[gpui_kit::test]
fn focus_returns_to_the_shell_when_the_focused_input_leaves_the_tree(cx: &mut TestAppContext) {
    let (window, shell) = open_shell(cx);
    render(window, cx);
    // `/` moves the focus to the quick filter. There is no session, so the filter bar is not
    // drawn and that input is not in the tree: the window reports the focus as lost, exactly
    // as when the editor of a closed drawer disappears. Without the shell's restore handler
    // nothing would be focused and `/` would stop matching.
    for _ in 0..2 {
        press_slash(window, cx);
        render(window, cx);
        assert_eq!(focus_of(window, &shell, cx), ROOT);
        assert!(is_slash_available(window, cx));
    }
}

// ---- Secret values and the clipboard clear ----

/// Marks of fixture text only: no test here reads or writes the system clipboard.
fn fixture_mark(text: &str) -> ClipboardMark {
    ClipboardMark::of(text)
}

#[gpui_kit::test]
fn secret_value_access_follows_the_launch_options(cx: &mut TestAppContext) {
    let (_, shell) = open_shell(cx);
    assert_eq!(
        shell.read_with(cx, |shell, _| shell.secret_value_access()),
        ValueAccess::Enabled
    );
    let (_, screenshot_shell) = open_shell_with(&["--screenshot", "out.png"], cx);
    assert_eq!(
        screenshot_shell.read_with(cx, |shell, _| shell.secret_value_access()),
        ValueAccess::Blocked
    );
}

#[gpui_kit::test]
fn blocked_shell_ignores_a_secret_action(cx: &mut TestAppContext) {
    let (_, shell) = open_shell_with(&["--screenshot", "out.png"], cx);
    let key = ResourceKey::Kind {
        kind: ResourceKind::Secrets,
        namespace: Some("shop".to_owned()),
        name: "credentials".to_owned(),
    };
    shell.update(cx, |shell, cx| {
        shell.run_secret_action(object(key), SecretAction::RevealAll, cx);
    });
    shell.read_with(cx, |shell, _| {
        assert!(shell.drawer.pending_secret_action.is_none());
        assert!(shell.selected.is_none());
    });
}

#[gpui_kit::test]
fn copy_arms_clipboard_clear_and_survives_drawer_close(cx: &mut TestAppContext) {
    let (_, shell) = open_shell(cx);
    shell.update(cx, |shell, cx| {
        shell.arm_clipboard_clear(fixture_mark("fixture-one"), cx);
    });
    shell.update(cx, |shell, cx| shell.close_drawer(cx));
    shell.read_with(cx, |shell, _| {
        let armed = shell.clipboard_clear.as_ref().expect("a clear stays armed");
        assert!(armed.mark.matches("fixture-one"));
    });
}

#[gpui_kit::test]
fn new_copy_replaces_armed_clear(cx: &mut TestAppContext) {
    let (_, shell) = open_shell(cx);
    shell.update(cx, |shell, cx| {
        shell.arm_clipboard_clear(fixture_mark("fixture-one"), cx);
        shell.arm_clipboard_clear(fixture_mark("fixture-two"), cx);
    });
    shell.read_with(cx, |shell, _| {
        let armed = shell.clipboard_clear.as_ref().expect("a clear is armed");
        assert!(armed.mark.matches("fixture-two"));
        assert!(!armed.mark.matches("fixture-one"));
    });
}

#[gpui_kit::test]
fn changing_the_selection_drops_the_values_view_and_its_pending_action(cx: &mut TestAppContext) {
    let (_, shell) = open_shell(cx);
    let key = ResourceKey::Kind {
        kind: ResourceKind::Secrets,
        namespace: Some("shop".to_owned()),
        name: "credentials".to_owned(),
    };
    shell.update(cx, |shell, cx| {
        shell.drawer.pending_secret_action = Some((key.clone(), SecretAction::RevealAll));
        shell.change_selection(Some(object(key)), cx);
    });
    shell.read_with(cx, |shell, _| {
        assert!(shell.drawer.pending_secret_action.is_none());
        assert!(shell.drawer.secret_values.is_none());
    });
}

/// A values view over a fetcher that counts its calls: no test here reads a real Secret.
fn counting_view(calls: &Arc<AtomicUsize>, cx: &mut Context<AppShell>) -> Entity<SecretValuesView> {
    let calls = Arc::clone(calls);
    let fetch: crate::secret_values::FetchValues = Arc::new(move || {
        calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(Vec::new()) })
    });
    let object = object(secret_key_fixture());
    cx.new(|_| SecretValuesView::new(fetch, object, Vec::new(), ValueAccess::Enabled))
}

fn secret_key_fixture() -> ResourceKey {
    ResourceKey::Kind {
        kind: ResourceKind::Secrets,
        namespace: Some("shop".to_owned()),
        name: "credentials".to_owned(),
    }
}

#[gpui_kit::test]
fn leaving_the_overview_tab_drops_an_existing_values_view(cx: &mut TestAppContext) {
    let (_, shell) = open_shell(cx);
    let calls = Arc::new(AtomicUsize::new(0));
    shell.update(cx, |shell, cx| {
        shell.drawer.secret_values = Some(counting_view(&calls, cx));
        // Staying on Overview keeps it.
        shell.set_drawer_tab(DrawerTab::Overview, cx);
        assert!(shell.drawer.secret_values.is_some());
        shell.set_drawer_tab(DrawerTab::Yaml, cx);
    });
    shell.read_with(cx, |shell, _| assert!(shell.drawer.secret_values.is_none()));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[gpui_kit::test]
fn changing_the_selection_drops_an_existing_values_view(cx: &mut TestAppContext) {
    let (_, shell) = open_shell(cx);
    let calls = Arc::new(AtomicUsize::new(0));
    shell.update(cx, |shell, cx| {
        shell.drawer.secret_values = Some(counting_view(&calls, cx));
        shell.change_selection(Some(object(secret_key_fixture())), cx);
    });
    shell.read_with(cx, |shell, _| assert!(shell.drawer.secret_values.is_none()));
    // The same key again is a no-op, so a second view survives it.
    shell.update(cx, |shell, cx| {
        shell.drawer.secret_values = Some(counting_view(&calls, cx));
        shell.change_selection(Some(object(secret_key_fixture())), cx);
    });
    shell.read_with(cx, |shell, _| assert!(shell.drawer.secret_values.is_some()));
    shell.update(cx, |shell, cx| shell.close_drawer(cx));
    shell.read_with(cx, |shell, _| assert!(shell.drawer.secret_values.is_none()));
}

fn served_kind(crd_name: &str, extra_column: bool) -> CustomKind {
    let (plural, group) = crd_name.split_once('.').expect("a CRD name has a group");
    let mut columns = Vec::new();
    if extra_column {
        columns.push(cluster::PrinterColumn::new(
            "Extra",
            cluster::ColumnType::String,
            ".spec.extra",
            false,
        ));
    }
    let crd = cluster::CrdSummary {
        name: crd_name.to_owned(),
        group: group.to_owned(),
        kind: "Widget".to_owned(),
        plural: plural.to_owned(),
        singular: "widget".to_owned(),
        scope: cluster::ResourceScope::Namespaced,
        versions: vec![cluster::CrdVersion {
            name: "v1".to_owned(),
            is_served: true,
            is_storage: true,
            is_deprecated: false,
            deprecation_warning: None,
            printer_columns: columns,
            schema: cluster::SchemaOutline::default(),
        }],
        state: cluster::CrdState::Established,
        created_at: None,
    };
    crate::custom_kind::custom_kinds(&[crd], &mut crate::custom_kind::CustomKindCache::default())[0]
}

#[test]
fn custom_screen_remaps_to_the_changed_kind() {
    let shown = Screen::Kind(ResourceKind::Custom(served_kind("widgets.x.io", false)));
    let changed = served_kind("widgets.x.io", true);
    assert_eq!(
        remapped_screen(shown, &[changed]),
        Some(Screen::Kind(ResourceKind::Custom(changed)))
    );
}

#[test]
fn unchanged_custom_screen_stays() {
    let kind = served_kind("widgets.x.io", false);
    let shown = Screen::Kind(ResourceKind::Custom(kind));
    // Equal by source, even when the kind was built again.
    let rebuilt = served_kind("widgets.x.io", false);
    assert_eq!(remapped_screen(shown, &[rebuilt]), None);
}

#[test]
fn removed_custom_kind_returns_to_crds() {
    let shown = Screen::Kind(ResourceKind::Custom(served_kind("widgets.x.io", false)));
    let other = served_kind("gadgets.x.io", false);
    assert_eq!(
        remapped_screen(shown, &[other]),
        Some(Screen::Kind(ResourceKind::Crds))
    );
    assert_eq!(
        remapped_screen(shown, &[]),
        Some(Screen::Kind(ResourceKind::Crds))
    );
}

#[test]
fn other_screens_never_remap() {
    let kinds = [served_kind("widgets.x.io", false)];
    for screen in [
        Screen::Pods,
        Screen::Nodes,
        Screen::Kind(ResourceKind::Crds),
        Screen::Kind(ResourceKind::Deployments),
    ] {
        assert_eq!(remapped_screen(screen, &kinds), None);
    }
}

#[test]
fn custom_launch_resolves_an_established_crd() {
    let kind = served_kind("widgets.x.io", false);
    assert_eq!(resolve_custom_launch(&[kind], "widgets.x.io"), Ok(kind));
}

#[test]
fn custom_launch_without_an_established_crd_fails_with_its_name() {
    let kinds = [served_kind("gadgets.x.io", false)];
    assert_eq!(
        resolve_custom_launch(&kinds, "widgets.x.io"),
        Err("no Established CRD named widgets.x.io".to_owned())
    );
    // A denied or empty CRD list finds nothing either.
    assert_eq!(
        resolve_custom_launch(&[], "widgets.x.io"),
        Err("no Established CRD named widgets.x.io".to_owned())
    );
}

// ---- Reveal and the steps that build on the selection ----

fn pod_key(name: &str) -> ResourceKey {
    ResourceKey::Pod {
        namespace: "shop".to_owned(),
        name: name.to_owned(),
    }
}

/// `key` in a cluster of its own: these shells have no session, so the cluster is only a name.
fn object(key: ResourceKey) -> ClusterObject {
    ClusterObject::new(ClusterRef::of(&context("ctx")), key)
}

fn secret_key() -> ResourceKey {
    ResourceKey::Kind {
        kind: ResourceKind::Secrets,
        namespace: Some("shop".to_owned()),
        name: "credentials".to_owned(),
    }
}

#[gpui_kit::test]
fn open_drawer_tab_on_another_row_ends_on_that_tab(cx: &mut TestAppContext) {
    let (_, shell) = open_shell(cx);
    shell.update(cx, |shell, cx| {
        shell.change_selection(Some(object(pod_key("api-0"))), cx);
        shell.open_drawer_tab(object(pod_key("api-1")), DrawerTab::Yaml, cx);
    });
    cx.run_until_parked();
    shell.read_with(cx, |shell, _| {
        assert_eq!(shell.selected, Some(object(pod_key("api-1"))));
        assert_eq!(shell.drawer.tab, DrawerTab::Yaml);
    });
}

fn context(name: &str) -> ContextSummary {
    ContextSummary {
        name: name.to_owned(),
        cluster: "cluster".to_owned(),
        user: None,
        namespace: None,
        source: PathBuf::from("test.yaml"),
    }
}

#[gpui_kit::test]
fn open_drawer_tab_on_the_selection_needs_no_reveal(cx: &mut TestAppContext) {
    let (_, shell) = open_shell(cx);
    shell.update(cx, |shell, cx| {
        shell.change_selection(Some(object(pod_key("api-0"))), cx);
        shell.open_drawer_tab(object(pod_key("api-0")), DrawerTab::Events, cx);
        // At once, before anything is deferred.
        assert_eq!(shell.drawer.tab, DrawerTab::Events);
    });
}

#[gpui_kit::test]
fn secret_action_on_another_row_waits_for_the_selection(cx: &mut TestAppContext) {
    let (_, shell) = open_shell(cx);
    shell.update(cx, |shell, cx| {
        // Another tab shows, so only the action's step can bring the Overview back.
        shell.drawer.tab = DrawerTab::Yaml;
        shell.run_secret_action(object(secret_key()), SecretAction::RevealAll, cx);
    });
    cx.run_until_parked();
    // The pending action itself is consumed or dropped by the next frame (there is no session
    // to read the Secret from), so the tab and the selection are what stay.
    shell.read_with(cx, |shell, _| {
        assert_eq!(shell.selected, Some(object(secret_key())));
        assert_eq!(shell.drawer.tab, DrawerTab::Overview);
    });
}

#[gpui_kit::test]
fn helm_values_keep_their_revision_through_a_reveal(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture("helm-reveal", cx);
    let shell = fixture.shell.clone();
    // A release link inside an open drawer: the drawer's cluster is the open one.
    let cluster = fixture.cluster("prod-a", cx);
    let object = |key: ResourceKey| ClusterObject::new(cluster.clone(), key);
    let key = ResourceKey::Kind {
        kind: ResourceKind::HelmReleases,
        namespace: Some("shop".to_owned()),
        name: "api".to_owned(),
    };
    shell.update(cx, |shell, cx| {
        shell.change_selection(Some(object(pod_key("api-0"))), cx);
        shell.set_drawer_open(true, cx);
        shell.open_helm_values(key.clone(), 3, ValuesLayout::Diff, cx);
    });
    cx.run_until_parked();
    shell.read_with(cx, |shell, _| {
        assert_eq!(shell.selected, Some(object(key.clone())));
        assert_eq!(shell.drawer.tab, DrawerTab::Values);
        // Choosing the subject forgets a revision; the step sets it after.
        assert_eq!(shell.drawer.helm_revision, Some(3));
        assert!(matches!(
            &shell.drawer.pending_helm_layout,
            Some((pending, ValuesLayout::Diff)) if *pending == key
        ));
    });
}

fn saved_pods(cx: &mut TestAppContext) -> Option<crate::settings::TablePrefs> {
    cx.update(|cx| AppSettings::get(cx).tables.get("pods").cloned())
}

#[gpui_kit::test]
fn cycle_sort_persists_the_sort_by_column_name(cx: &mut TestAppContext) {
    let (_window, shell) = open_shell(cx);
    assert_eq!(saved_pods(cx), None);
    // The shell starts on Overview, which has no table.
    cx.update(|cx| shell.update(cx, |shell, cx| shell.show_screen(Screen::Pods, cx)));
    // Logical column 3 of the Pods table is Restarts.
    cx.update(|cx| shell.update(cx, |shell, cx| shell.cycle_sort(3, cx)));
    let sort = saved_pods(cx).and_then(|prefs| prefs.sort);
    assert_eq!(
        sort,
        Some(crate::settings::SavedSort {
            column: "Restarts".to_owned(),
            direction: crate::table_sort::SortDirection::Ascending,
        })
    );
}

#[gpui_kit::test]
fn toggle_column_persists_hidden_columns_by_name(cx: &mut TestAppContext) {
    let (_window, shell) = open_shell(cx);
    cx.update(|cx| shell.update(cx, |shell, cx| shell.show_screen(Screen::Pods, cx)));
    // Logical column 6 of the Pods table is Node; CPU is hidden by default.
    cx.update(|cx| shell.update(cx, |shell, cx| shell.toggle_column(6, cx)));
    let hidden = saved_pods(cx).map(|prefs| prefs.hidden);
    assert_eq!(hidden, Some(vec!["CPU".to_owned(), "Node".to_owned()]));
}

fn toggle_default(shell: &Entity<AppShell>, cx: &mut TestAppContext) {
    cx.update(|cx| {
        shell.update(cx, |shell, cx| {
            shell.active = Some(context("ctx"));
            shell.toggle_default_namespace(&ClusterRef::of(&context("ctx")), "monitoring", cx);
        });
    });
}

fn saved_default(cx: &mut TestAppContext) -> Option<String> {
    cx.update(|cx| {
        AppSettings::get(cx)
            .registry
            .profile(&context("ctx"))
            .default_namespace
    })
}

#[gpui_kit::test]
fn toggle_default_namespace_stores_the_namespace(cx: &mut TestAppContext) {
    let (_window, shell) = open_shell(cx);
    toggle_default(&shell, cx);
    assert_eq!(saved_default(cx).as_deref(), Some("monitoring"));
}

#[gpui_kit::test]
fn toggle_default_namespace_clears_it_when_already_set(cx: &mut TestAppContext) {
    let (_window, shell) = open_shell(cx);
    toggle_default(&shell, cx);
    toggle_default(&shell, cx);
    assert_eq!(saved_default(cx), None);
}

#[gpui_kit::test]
fn the_shell_starts_on_overview_without_screen_flag(cx: &mut TestAppContext) {
    let (_window, shell) = open_shell(cx);
    shell.read_with(cx, |shell, _| assert_eq!(shell.screen, Screen::Overview));
}

// ---- Settings edits reach the main window ----

/// A shell over a real kubeconfig with one context, `ctx`. The requested context does not
/// exist, so no session starts (nothing touches the network); `active` is set by hand as the
/// first load would.
fn open_shell_over_fixture(
    name: &str,
    cx: &mut TestAppContext,
) -> (WindowHandle<Root>, Entity<AppShell>, ContextSummary) {
    let dir =
        std::env::temp_dir().join(format!("k8sboard-0025-shell-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let path = dir.join("kubeconfig.yaml");
    std::fs::write(
        &path,
        "apiVersion: v1\nkind: Config\nclusters:\n  - name: c\n    cluster: { server: 'https://127.0.0.1:1' }\ncontexts:\n  - name: ctx\n    context: { cluster: c }\n",
    )
    .expect("write fixture");
    let (window, shell) = open_shell_on(&path.to_string_lossy(), &["--context", "nope"], cx);
    cx.run_until_parked();
    let summary = ContextSummary {
        name: "ctx".to_owned(),
        cluster: "c".to_owned(),
        user: None,
        namespace: None,
        source: path,
    };
    shell.update(cx, |shell, _| shell.active = Some(summary.clone()));
    (window, shell, summary)
}

fn remove_fixture(summary: &ContextSummary) {
    if let Some(dir) = summary.source.parent() {
        let _ = std::fs::remove_dir_all(dir);
    }
}

fn switcher_labels(shell: &Entity<AppShell>, cx: &mut TestAppContext) -> Vec<String> {
    cx.update(|cx| {
        shell
            .read(cx)
            .all_switcher_sections(cx)
            .into_iter()
            .flat_map(|section| section.rows)
            .map(|row| row.label)
            .collect()
    })
}

fn active_display_name(shell: &Entity<AppShell>, cx: &mut TestAppContext) -> Option<String> {
    cx.update(|cx| {
        shell
            .read(cx)
            .active_profile(cx)
            .map(|profile| profile.display_name)
    })
}

#[gpui_kit::test]
fn editing_display_name_updates_title_bar(cx: &mut TestAppContext) {
    let (window, shell, summary) = open_shell_over_fixture("rename", cx);
    render(window, cx);
    assert_eq!(active_display_name(&shell, cx).as_deref(), Some("ctx"));
    assert_eq!(switcher_labels(&shell, cx), ["ctx"]);
    // What the Settings window does when the display name field changes.
    let cluster = ClusterRef::of(&summary);
    cx.update(|cx| {
        AppSettings::update(cx, |settings| {
            crate::cluster_form::edit_entry(&mut settings.registry, &cluster, |entry| {
                entry.display_name = Some("uat-monitor".to_owned());
            });
        });
    });
    cx.run_until_parked();
    // The title bar reads the profile and the switcher labels at render time.
    render(window, cx);
    assert_eq!(
        active_display_name(&shell, cx).as_deref(),
        Some("uat-monitor")
    );
    assert_eq!(switcher_labels(&shell, cx), ["uat-monitor"]);
    remove_fixture(&summary);
}

#[gpui_kit::test]
fn read_only_switch_leaves_title_bar_badge(cx: &mut TestAppContext) {
    let (window, shell, summary) = open_shell_over_fixture("read-only", cx);
    render(window, cx);
    let cluster = ClusterRef::of(&summary);
    let profile = |cx: &mut TestAppContext| {
        cx.update(|cx| shell.read(cx).active_profile(cx))
            .expect("an active cluster")
    };
    let before = profile(cx);
    assert!(!before.read_only, "ctx is not guessed as Production");
    cx.update(|cx| {
        AppSettings::update(cx, |settings| {
            crate::cluster_form::edit_entry(&mut settings.registry, &cluster, |entry| {
                entry.read_only = Some(true);
            });
        });
    });
    cx.run_until_parked();
    // The switch is stored and read by the profile; the title bar still renders, with its
    // fixed "Read-only" badge (`read_only_badge` takes no profile), the name, and the
    // environment unchanged.
    render(window, cx);
    let after = profile(cx);
    assert!(after.read_only);
    assert_eq!(after.display_name, before.display_name);
    assert_eq!(after.environment, before.environment);
    remove_fixture(&summary);
}

// ---- Topology ----

fn service_key(namespace: &str, name: &str) -> ResourceKey {
    ResourceKey::Kind {
        kind: ResourceKind::Services,
        namespace: Some(namespace.to_owned()),
        name: name.to_owned(),
    }
}

#[gpui_kit::test]
fn show_in_topology_draws_the_namespace_of_the_object(cx: &mut TestAppContext) {
    let (window, shell) = open_shell(cx);
    // The shell calls the view inside its own update, so the view must not update the shell back
    // in the same turn.
    shell.update(cx, |shell, cx| {
        shell.show_in_topology(&service_key("shop", "web"), cx);
    });
    render(window, cx);
    shell.read_with(cx, |shell, cx| {
        assert_eq!(shell.screen, Screen::Topology);
        let count = shell.topology.read(cx).header_count();
        assert_eq!(count.as_deref(), Some("ns: shop \u{b7} loading\u{2026}"));
    });
}

#[gpui_kit::test]
fn show_in_topology_ignores_other_kinds(cx: &mut TestAppContext) {
    let (_, shell) = open_shell(cx);
    let key = ResourceKey::Kind {
        kind: ResourceKind::Deployments,
        namespace: Some("shop".to_owned()),
        name: "api".to_owned(),
    };
    shell.update(cx, |shell, cx| shell.show_in_topology(&key, cx));
    shell.read_with(cx, |shell, _| assert_ne!(shell.screen, Screen::Topology));
}

#[gpui_kit::test]
fn topology_can_be_shown_and_left(cx: &mut TestAppContext) {
    let (window, shell) = open_shell(cx);
    for screen in [Screen::Topology, Screen::Pods, Screen::Topology] {
        shell.update(cx, |shell, cx| shell.show_screen(screen, cx));
        render(window, cx);
        shell.read_with(cx, |shell, _| assert_eq!(shell.screen, screen));
    }
}

#[gpui_kit::test]
fn topology_launch_screen_starts_on_topology(cx: &mut TestAppContext) {
    let (window, shell) = open_shell_with(&["--screen", "topology"], cx);
    render(window, cx);
    shell.read_with(cx, |shell, _| assert_eq!(shell.screen, Screen::Topology));
}

// ---- The key map ----

fn press(window: WindowHandle<Root>, key: &str, cx: &mut TestAppContext) {
    cx.update_window(window.into(), |_, window, cx| window.press(key, cx))
        .expect("the window is open");
    cx.run_until_parked();
}

fn has_dialog(window: WindowHandle<Root>, cx: &mut TestAppContext) -> bool {
    cx.update_window(window.into(), |_, window, cx| window.has_active_dialog(cx))
        .expect("the window is open")
}

#[gpui_kit::test]
fn question_mark_opens_the_shortcut_sheet(cx: &mut TestAppContext) {
    let (window, _) = open_shell(cx);
    cx.update(|cx| cx.set_reduce_motion(true));
    render(window, cx);
    assert!(!has_dialog(window, cx));
    press(window, "?", cx);
    assert!(has_dialog(window, cx));
}

#[gpui_kit::test]
fn escape_closes_the_shortcut_sheet(cx: &mut TestAppContext) {
    let (window, shell) = open_shell(cx);
    cx.update(|cx| cx.set_reduce_motion(true));
    render(window, cx);
    press(window, "?", cx);
    press(window, "escape", cx);
    render(window, cx);
    assert!(!has_dialog(window, cx));
    assert_eq!(focus_of(window, &shell, cx), ROOT);
}

#[gpui_kit::test]
fn the_shell_handles_every_key_action_of_its_tree(cx: &mut TestAppContext) {
    let (window, _) = open_shell(cx);
    render(window, cx);
    // Ctrl , is handled by the app (no window), Ctrl O and the Alt arrows belong to the Settings
    // window, and the terminal chords belong to a focused shell tab (`shell_tab_tests` dispatches each of them), and
    // Ctrl S belongs to the Edit YAML view (`app_shell_edit_tests` presses it there).
    let elsewhere: [&dyn gpui_kit::Action; 8] = [
        &crate::settings_window::OpenSettings,
        &crate::settings_window::ImportKubeconfig,
        &crate::settings_window::MoveClusterUp,
        &crate::settings_window::MoveClusterDown,
        &crate::keymap::TerminalCopy,
        &crate::keymap::TerminalPaste,
        &crate::keymap::TerminalFind,
        &crate::keymap::ApplyEdit,
    ];
    for row in crate::keymap::shortcut_rows() {
        if elsewhere.iter().any(|other| other.partial_eq(&*row.action)) {
            continue;
        }
        let is_available = cx
            .update_window(window.into(), |_, window, cx| {
                window.is_action_available(&*row.action, cx)
            })
            .expect("the window is open");
        assert!(is_available, "{} has no handler", row.label);
    }
}

#[gpui_kit::test]
fn closed_drawer_has_no_subject(cx: &mut TestAppContext) {
    let (_, shell) = open_shell(cx);
    shell.update(cx, |shell, cx| {
        shell.change_selection(Some(object(pod_key("api-0"))), cx);
        assert_eq!(shell.drawer_subject(), None);
        shell.drawer.is_open = true;
        assert_eq!(shell.drawer_subject(), Some(&object(pod_key("api-0"))));
    });
}

#[gpui_kit::test]
fn closing_drawer_drops_pending_subjects(cx: &mut TestAppContext) {
    let (_, shell) = open_shell(cx);
    shell.update(cx, |shell, cx| {
        shell.change_selection(Some(object(pod_key("api-0"))), cx);
        shell.drawer.is_open = true;
        shell.follow_drawer_subjects(cx);
        assert!(shell.pending_subjects.is_some());
        shell.close_drawer(cx);
        assert!(shell.pending_subjects.is_none());
    });
}

#[gpui_kit::test]
fn closing_the_drawer_keeps_the_row(cx: &mut TestAppContext) {
    let (_, shell) = open_shell(cx);
    shell.update(cx, |shell, cx| {
        shell.change_selection(Some(object(pod_key("api-0"))), cx);
        shell.set_drawer_open(true, cx);
        assert!(shell.drawer.is_open);
        shell.close_drawer(cx);
        assert!(!shell.drawer.is_open);
        assert_eq!(shell.selected, Some(object(pod_key("api-0"))));
    });
}

#[gpui_kit::test]
fn the_drawer_does_not_open_without_a_row(cx: &mut TestAppContext) {
    let (_, shell) = open_shell(cx);
    shell.update(cx, |shell, cx| {
        shell.set_drawer_open(true, cx);
        assert!(!shell.drawer.is_open);
    });
}

#[gpui_kit::test]
fn clearing_the_selection_closes_the_drawer(cx: &mut TestAppContext) {
    let (_, shell) = open_shell(cx);
    shell.update(cx, |shell, cx| {
        shell.change_selection(Some(object(pod_key("api-0"))), cx);
        shell.drawer.is_open = true;
        shell.change_selection(None, cx);
        assert!(!shell.drawer.is_open);
    });
}

#[gpui_kit::test]
fn a_click_opens_the_drawer_even_on_the_selected_row(cx: &mut TestAppContext) {
    let (window, shell) = open_shell_with(&["--screen", "pods"], cx);
    render(window, cx);
    cx.update_window(window.into(), |_, window, cx| {
        shell.update(cx, |shell, cx| {
            let table = shell.pod_table.clone();
            shell.change_selection(Some(object(pod_key("api-0"))), cx);
            assert!(!shell.drawer.is_open);
            shell.on_row_selected(Some(object(pod_key("api-0"))), false, &table, window, cx);
            assert!(shell.drawer.is_open);
        });
    })
    .expect("the window is open");
}

#[gpui_kit::test]
fn the_echo_of_a_shell_move_never_opens_the_drawer(cx: &mut TestAppContext) {
    let (window, shell) = open_shell_with(&["--screen", "pods"], cx);
    render(window, cx);
    cx.update_window(window.into(), |_, window, cx| {
        shell.update(cx, |shell, cx| {
            let table = shell.pod_table.clone();
            shell.on_row_selected(Some(object(pod_key("api-1"))), true, &table, window, cx);
            assert_eq!(shell.selected, Some(object(pod_key("api-1"))));
            assert!(!shell.drawer.is_open);
        });
    })
    .expect("the window is open");
}

#[gpui_kit::test]
fn a_shell_move_leaves_no_echo_behind(cx: &mut TestAppContext) {
    let (window, shell) = open_shell_with(&["--screen", "pods"], cx);
    render(window, cx);
    shell.update(cx, |shell, cx| {
        let table = shell.pod_table.clone();
        shell.select_table_row(&table, 0, cx);
        assert_eq!(shell.row_echo, Some(0));
    });
    cx.run_until_parked();
    shell.read_with(cx, |shell, _| {
        assert_eq!(shell.row_echo, None);
        assert!(!shell.drawer.is_open);
    });
}

#[gpui_kit::test]
fn enter_opens_the_drawer_on_the_cursor_row(cx: &mut TestAppContext) {
    let (window, shell) = open_shell_with(&["--screen", "pods"], cx);
    render(window, cx);
    shell.update(cx, |shell, cx| {
        shell.change_selection(Some(object(pod_key("api-0"))), cx);
    });
    press(window, "enter", cx);
    shell.read_with(cx, |shell, _| {
        assert!(shell.drawer.is_open);
        assert_eq!(shell.selected, Some(object(pod_key("api-0"))));
    });
}

#[gpui_kit::test]
fn escape_closes_the_drawer_then_clears_the_cursor(cx: &mut TestAppContext) {
    let (window, shell) = open_shell_with(&["--screen", "pods"], cx);
    render(window, cx);
    shell.update(cx, |shell, cx| {
        shell.change_selection(Some(object(pod_key("api-0"))), cx);
        shell.set_drawer_open(true, cx);
    });
    press(window, "escape", cx);
    shell.read_with(cx, |shell, _| {
        assert!(!shell.drawer.is_open);
        assert_eq!(shell.selected, Some(object(pod_key("api-0"))));
    });
    press(window, "escape", cx);
    shell.read_with(cx, |shell, _| assert_eq!(shell.selected, None));
}

#[gpui_kit::test]
fn copy_name_copies_the_cursor_row_name(cx: &mut TestAppContext) {
    let (window, shell) = open_shell(cx);
    render(window, cx);
    shell.update(cx, |shell, cx| {
        shell.change_selection(Some(object(pod_key("api-0"))), cx);
    });
    // The test platform has its own clipboard: the system clipboard is untouched.
    press(window, "secondary-c", cx);
    let copied = cx.read_from_clipboard().and_then(|item| item.text());
    assert_eq!(copied.as_deref(), Some("api-0"));
}

#[gpui_kit::test]
fn row_keys_do_nothing_without_a_live_cluster(cx: &mut TestAppContext) {
    let (window, shell) = open_shell(cx);
    render(window, cx);
    shell.update(cx, |shell, cx| {
        shell.change_selection(Some(object(pod_key("api-0"))), cx);
    });
    for key in ["l", "y", "s", "e", "delete", "shift-s"] {
        press(window, key, cx);
    }
    shell.read_with(cx, |shell, _| assert!(!shell.drawer.is_open));
    assert_eq!(
        cx.update_window(window.into(), |_, window, cx| window
            .notifications(cx)
            .len())
            .expect("the window is open"),
        0
    );
}

#[gpui_kit::test]
fn dock_keys_do_nothing_without_tabs(cx: &mut TestAppContext) {
    let (window, shell) = open_shell(cx);
    render(window, cx);
    for key in [
        "ctrl-`",
        "ctrl-tab",
        "ctrl-shift-tab",
        "secondary-w",
        "secondary-shift-m",
    ] {
        press(window, key, cx);
    }
    shell.read_with(cx, |shell, cx| {
        let dock = shell.dock.read(cx);
        assert!(!dock.has_tabs());
        assert_eq!(dock.mode(), DockMode::Normal);
    });
}

#[gpui_kit::test]
fn enter_in_the_quick_filter_hands_the_keyboard_back(cx: &mut TestAppContext) {
    let (window, shell) = open_shell(cx);
    render(window, cx);
    press_slash(window, cx);
    render(window, cx);
    // No session draws no filter bar, so the input leaves the tree and the root restores focus;
    // what matters is that Enter on plain text does not stay in a dead field.
    shell.update(cx, |shell, cx| {
        let input = shell.quick_filter.clone();
        input.update(cx, |_, cx| {
            cx.emit(InputEvent::PressEnter {
                secondary: false,
                shift: false,
            })
        });
    });
    cx.run_until_parked();
    render(window, cx);
    assert_eq!(focus_of(window, &shell, cx), ROOT);
}

#[gpui_kit::test]
fn enter_on_a_focused_button_stays_with_the_button(cx: &mut TestAppContext) {
    let (window, shell) = open_shell_with(&["--screen", "pods"], cx);
    render(window, cx);
    shell.update(cx, |shell, cx| {
        shell.change_selection(Some(object(pod_key("api-0"))), cx);
    });
    // Tab moves the focus from the shell root to a title-bar button.
    press(window, "tab", cx);
    assert_ne!(focus_of(window, &shell, cx), ROOT);
    press(window, "enter", cx);
    shell.read_with(cx, |shell, _| assert!(!shell.drawer.is_open));
}

#[gpui_kit::test]
fn closing_the_drawer_drops_revealed_secret_values(cx: &mut TestAppContext) {
    let (_, shell) = open_shell(cx);
    let calls = Arc::new(AtomicUsize::new(0));
    shell.update(cx, |shell, cx| {
        shell.change_selection(Some(object(secret_key_fixture())), cx);
        shell.set_drawer_open(true, cx);
        shell.drawer.secret_values = Some(counting_view(&calls, cx));
        shell.drawer.pending_secret_action = Some((secret_key_fixture(), SecretAction::RevealAll));
        shell.close_drawer(cx);
        assert!(shell.drawer.secret_values.is_none());
        assert!(shell.drawer.pending_secret_action.is_none());
        // The row stays: only the drawer closed.
        assert_eq!(shell.selected, Some(object(secret_key_fixture())));
    });
}

#[gpui_kit::test]
fn enter_on_issues_without_a_cursor_does_nothing(cx: &mut TestAppContext) {
    let (window, shell) = open_shell_with(&["--screen", "issues"], cx);
    render(window, cx);
    press(window, "enter", cx);
    shell.read_with(cx, |shell, _| {
        assert_eq!(shell.selected, None);
        assert!(!shell.drawer.is_open);
    });
}

// ---- Topology and the drawer split ----

#[gpui_kit::test]
fn a_topology_click_opens_the_drawer(cx: &mut TestAppContext) {
    // The graph draws the primary cluster, so the click needs a shell that has one.
    let fixture = open_switch_fixture("topology-click", cx);
    let shell = fixture.shell.clone();
    let key = service_key("shop", "web");
    shell.update(cx, |shell, cx| {
        shell.select_on_topology(Some(key.clone()), cx);
        assert!(shell.drawer.is_open);
        assert_eq!(shell.drawer_subject().map(|object| &object.key), Some(&key));
        // A click on the empty canvas closes it and drops the row.
        shell.select_on_topology(None, cx);
        assert!(!shell.drawer.is_open);
        assert_eq!(shell.selected, None);
    });
}

#[gpui_kit::test]
fn a_topology_click_on_a_cluster_scoped_node_keeps_the_screen(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture_with("topology-cluster", &["--screen", "topology"], cx);
    let shell = fixture.shell.clone();
    render(fixture.window, cx);
    for kind in [
        ResourceKind::ClusterRoles,
        ResourceKind::ClusterRoleBindings,
    ] {
        let key = ResourceKey::Kind {
            kind,
            namespace: None,
            name: "cluster-admin".to_owned(),
        };
        shell.update(cx, |shell, cx| {
            shell.select_on_topology(Some(key.clone()), cx);
            assert_eq!(shell.screen, Screen::Topology);
            assert!(shell.drawer.is_open);
            assert_eq!(shell.drawer_subject().map(|object| &object.key), Some(&key));
        });
    }
}

#[gpui_kit::test]
fn a_namespace_change_on_topology_clears_the_selection(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture("topology-namespace", cx);
    let (window, shell) = (fixture.window, fixture.shell.clone());
    let (namespace, id) = topology_target(&service_key("blog", "web")).expect("a Service");
    shell.update(cx, |shell, cx| {
        shell.select_on_topology(Some(service_key("shop", "web")), cx);
        assert!(shell.drawer.is_open);
        // The view draws another namespace, so the object of the old one cannot stay.
        shell
            .topology
            .update(cx, |view, cx| view.show_object(&namespace, id, cx));
    });
    cx.run_until_parked();
    render(window, cx);
    shell.read_with(cx, |shell, _| {
        assert_eq!(shell.selected, None);
        assert!(!shell.drawer.is_open);
        assert_eq!(shell.drawer_subject(), None);
    });
}

#[gpui_kit::test]
fn row_keys_do_nothing_on_topology(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture_with("topology-keys", &["--screen", "topology"], cx);
    let (window, shell) = (fixture.window, fixture.shell.clone());
    render(window, cx);
    shell.update(cx, |shell, cx| {
        shell.select_on_topology(Some(service_key("shop", "web")), cx);
        shell.close_drawer(cx);
    });
    // The graph is a canvas: J moves nothing and Enter opens nothing.
    press(window, "j", cx);
    press(window, "enter", cx);
    shell.read_with(cx, |shell, _| {
        assert_eq!(
            shell.selected.as_ref().map(|object| &object.key),
            Some(&service_key("shop", "web"))
        );
        assert!(!shell.drawer.is_open);
    });
}

// ---- Command palette ----

/// The text of the focused input, or `None` when no input holds the focus.
fn focused_text(window: WindowHandle<Root>, cx: &mut TestAppContext) -> Option<String> {
    cx.update_window(window.into(), |_, window, cx| {
        window
            .focused_input(cx)
            .map(|input| input.value(cx).to_string())
    })
    .expect("the window is open")
}

fn type_text(window: WindowHandle<Root>, text: &str, cx: &mut TestAppContext) {
    cx.update_window(window.into(), |_, window, cx| window.input(text, cx))
        .expect("the window is open");
    cx.run_until_parked();
}

#[gpui_kit::test]
fn ctrl_k_opens_the_palette_dialog(cx: &mut TestAppContext) {
    let (window, shell) = open_shell(cx);
    render(window, cx);
    assert!(!has_dialog(window, cx));
    press(window, "secondary-k", cx);
    assert!(has_dialog(window, cx));
    // The query input holds the focus, and it is empty.
    assert_eq!(focused_text(window, cx).as_deref(), Some(""));
    // The shell behind the scrim still has its own focus handle, which commands dispatch on.
    shell.read_with(cx, |shell, _| assert_eq!(shell.screen, Screen::Overview));
}

#[gpui_kit::test]
fn colon_opens_the_palette_seeded_with_a_colon(cx: &mut TestAppContext) {
    let (window, _) = open_shell(cx);
    render(window, cx);
    press(window, ":", cx);
    assert!(has_dialog(window, cx));
    assert_eq!(focused_text(window, cx).as_deref(), Some(":"));
}

#[gpui_kit::test]
fn escape_on_the_seeded_colon_closes(cx: &mut TestAppContext) {
    let (window, _) = open_shell(cx);
    render(window, cx);
    press(window, ":", cx);
    press(window, "escape", cx);
    assert!(!has_dialog(window, cx));
    // After typing, the kit rule stands: Esc clears the query first.
    press(window, ":", cx);
    type_text(window, "p", cx);
    assert_eq!(focused_text(window, cx).as_deref(), Some(":p"));
    press(window, "escape", cx);
    assert!(has_dialog(window, cx));
    assert_eq!(focused_text(window, cx).as_deref(), Some(""));
}

#[gpui_kit::test]
fn escape_clears_the_query_then_closes(cx: &mut TestAppContext) {
    let (window, shell) = open_shell(cx);
    render(window, cx);
    press(window, "secondary-k", cx);
    type_text(window, "x", cx);
    assert_eq!(focused_text(window, cx).as_deref(), Some("x"));
    press(window, "escape", cx);
    assert!(has_dialog(window, cx));
    assert_eq!(focused_text(window, cx).as_deref(), Some(""));
    press(window, "escape", cx);
    assert!(!has_dialog(window, cx));
    // The focus returns to where it was: the shell root.
    assert_eq!(focus_of(window, &shell, cx), ROOT);
}

#[gpui_kit::test]
fn the_palette_survives_renders_with_its_query_and_focus(cx: &mut TestAppContext) {
    let (window, _) = open_shell(cx);
    render(window, cx);
    press(window, "secondary-k", cx);
    type_text(window, "x", cx);
    // A new entity per render would lose the typed text and the focus.
    for _ in 0..3 {
        render(window, cx);
    }
    assert_eq!(focused_text(window, cx).as_deref(), Some("x"));
}

#[gpui_kit::test]
fn single_keys_do_not_reach_the_shell_behind_the_palette(cx: &mut TestAppContext) {
    let (window, shell) = open_shell(cx);
    render(window, cx);
    press(window, "secondary-k", cx);
    // `j` is typed into the query; it must not move a cursor, and `?` must not open the sheet.
    type_text(window, "j", cx);
    assert_eq!(focused_text(window, cx).as_deref(), Some("j"));
    shell.read_with(cx, |shell, _| assert_eq!(shell.selected, None));
}

#[gpui_kit::test]
fn the_palette_snapshot_of_a_shell_without_a_session_lists_commands_and_screens(
    cx: &mut TestAppContext,
) {
    let (window, shell) = open_shell(cx);
    render(window, cx);
    let snapshot = shell.read_with(cx, |shell, cx| {
        shell.palette_snapshot(&parse_query("pod"), cx)
    });
    assert!(!snapshot.context.has_session);
    assert!(snapshot.context.scope_label.is_none());
    assert!(snapshot.entries.iter().all(|entry| matches!(
        entry.target,
        crate::palette_search::PaletteTarget::Command(_)
            | crate::palette_search::PaletteTarget::Screen(_)
    )));
}

#[gpui_kit::test]
fn tab_previews_nothing_without_a_session(cx: &mut TestAppContext) {
    let (window, shell) = open_shell(cx);
    render(window, cx);
    let key = object(ResourceKey::Pod {
        namespace: "shop".to_owned(),
        name: "api-0".to_owned(),
    });
    shell.update(cx, |shell, cx| {
        assert!(!shell.can_preview_row(&key, cx));
        shell.preview_resource(&key, cx);
        assert_eq!(shell.selected, None);
        assert!(!shell.drawer.is_open);
    });
}

#[gpui_kit::test]
fn confirming_a_kind_opens_its_screen_and_closes_the_palette(cx: &mut TestAppContext) {
    let (window, shell) = open_shell(cx);
    cx.update(|cx| cx.set_reduce_motion(true));
    render(window, cx);
    press(window, ":", cx);
    type_text(window, "po", cx);
    press(window, "enter", cx);
    assert!(!has_dialog(window, cx));
    shell.read_with(cx, |shell, _| assert_eq!(shell.screen, Screen::Pods));
}

#[gpui_kit::test]
fn confirming_a_command_runs_the_handler_of_its_key(cx: &mut TestAppContext) {
    let (window, shell) = open_shell(cx);
    cx.update(|cx| cx.set_reduce_motion(true));
    render(window, cx);
    press(window, "secondary-k", cx);
    type_text(window, "cluster switcher", cx);
    press(window, "enter", cx);
    assert!(!has_dialog(window, cx));
    // The same handler as Ctrl Shift C: the switcher popover opens.
    shell.read_with(cx, |shell, _| assert!(shell.switcher().is_open()));
}

#[gpui_kit::test]
fn a_disabled_command_never_runs(cx: &mut TestAppContext) {
    let (window, shell) = open_shell(cx);
    cx.update(|cx| cx.set_reduce_motion(true));
    render(window, cx);
    let mode_before = shell.read_with(cx, |shell, cx| shell.dock.read(cx).mode());
    press(window, "secondary-k", cx);
    // The dock has no tab, so this command is disabled with its reason.
    type_text(window, "> toggle the dock", cx);
    assert_eq!(
        focused_text(window, cx).as_deref(),
        Some("> toggle the dock")
    );
    press(window, "enter", cx);
    // The palette stays open and the dock handler never ran.
    assert!(has_dialog(window, cx));
    shell.read_with(cx, |shell, cx| {
        assert!(!shell.dock.read(cx).has_tabs());
        assert_eq!(shell.dock.read(cx).mode(), mode_before);
    });
}

#[gpui_kit::test]
fn the_title_bar_search_box_opens_the_palette(cx: &mut TestAppContext) {
    let (window, _) = open_shell(cx);
    cx.update(|cx| cx.set_reduce_motion(true));
    render(window, cx);
    assert!(!has_dialog(window, cx));
    cx.update_window(window.into(), |_, window, cx| {
        window.click("palette-search", cx)
    })
    .expect("the window is open");
    cx.run_until_parked();
    assert!(has_dialog(window, cx));
    assert_eq!(focused_text(window, cx).as_deref(), Some(""));
}

// ---- Spec 0039 step 1: log tabs opened from the pod menus ----

pub(super) fn logs_pod() -> cluster::PodSummary {
    let container = |name: &str, kind: cluster::ContainerKind| cluster::ContainerSummary {
        terminal: cluster::ContainerTerminal::None,
        name: name.to_owned(),
        image: "img".to_owned(),
        kind,
        state: cluster::ContainerState::Running { started_at: None },
        is_ready: true,
        restart_count: 0,
        last_termination: None,
        image_digest: None,
        pull_policy: None,
        is_started: None,
        ports: Vec::new(),
        resources: Vec::new(),
        probes: cluster::ContainerProbes::default(),
        env: Vec::new(),
        env_from: Vec::new(),
        mounts: Vec::new(),
    };
    cluster::PodSummary {
        is_finished: false,
        namespace: "shop".to_owned(),
        name: "api-0".to_owned(),
        status: cluster::PodStatus::Reason(cluster::StatusReason::Running),
        ready: cluster::ReadyCount { ready: 2, total: 2 },
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
        containers: vec![
            container("app", cluster::ContainerKind::Main),
            container("proxy", cluster::ContainerKind::Sidecar),
        ],
    }
}

/// Opens the dock tab that `target_of` names, as a menu entry does, and returns the tab labels.
fn log_tab_labels_after(
    name: &str,
    target_of: impl Fn(&cluster::PodSummary, &crate::cluster_session::AccessState) -> Option<LogTarget>,
    cx: &mut TestAppContext,
) -> Vec<String> {
    let fixture = open_log_tab_fixture(name, target_of, cx);
    fixture
        .shell
        .read_with(cx, |shell, cx| shell.dock.read(cx).log_tab_labels(cx))
}

/// A live shell whose dock holds the log tab that `target_of` names.
pub(super) fn open_log_tab_fixture(
    name: &str,
    target_of: impl Fn(&cluster::PodSummary, &crate::cluster_session::AccessState) -> Option<LogTarget>,
    cx: &mut TestAppContext,
) -> super::app_shell_switch_tests::SwitchFixture {
    let fixture = open_live_logs_fixture(name, cx);
    open_log_tab(&fixture, target_of, cx);
    fixture
}

/// A live shell with every access allowed and one pod (`logs_pod`), and an empty dock.
pub(super) fn open_live_logs_fixture(
    name: &str,
    cx: &mut TestAppContext,
) -> super::app_shell_switch_tests::SwitchFixture {
    let fixture = open_switch_fixture(name, cx);
    fixture.go_live(NamespaceScope::All, cx);
    let session = fixture.session(cx);
    session.update(cx, |session, cx| {
        let reviews = cluster::AccessCheck::ALL
            .into_iter()
            .map(|check| cluster::AccessReview {
                check,
                decision: cluster::AccessDecision::Allowed,
            })
            .collect();
        session.set_access_for_test(
            crate::cluster_session::AccessState::Known(cluster::AccessReport { reviews }),
            cx,
        );
        session.set_pods_for_test(vec![logs_pod()], cx);
    });
    cx.run_until_parked();
    fixture
}

/// Opens the dock tab that `target_of` names, as a menu entry does.
pub(super) fn open_log_tab(
    fixture: &super::app_shell_switch_tests::SwitchFixture,
    target_of: impl Fn(&cluster::PodSummary, &crate::cluster_session::AccessState) -> Option<LogTarget>,
    cx: &mut TestAppContext,
) {
    let cluster = fixture.cluster("prod-a", cx);
    fixture.with_window(cx, |window, cx| {
        let shell = fixture.shell.read(cx);
        let row = shell
            .row_context_of(&cluster, cx)
            .expect("the cluster is viewed");
        let live = shell.live_of(&cluster, cx).expect("a live slot");
        let pod = live.pods.items().first().expect("a pod").clone();
        let target = target_of(&pod, &live.access).expect("a log target");
        let connection = live.connection().clone();
        let dock = shell.dock.clone();
        dock.update(cx, |dock, cx| {
            dock.open(
                crate::dock::LogOrigin::new(&row, connection),
                target,
                window,
                cx,
            )
        });
    });
}

#[gpui_kit::test]
fn container_menu_opens_logs_of_that_container(cx: &mut TestAppContext) {
    // The container menu asks `logs_launch` with the shown container, as its View logs item does.
    let labels = log_tab_labels_after(
        "container-menu-logs",
        |pod, access| crate::resource_actions::logs_launch(pod, Some("proxy"), access).ok(),
        cx,
    );
    assert_eq!(labels, ["api-0/proxy"]);
}

#[gpui_kit::test]
fn logs_submenu_opens_tab_on_the_picked_container(cx: &mut TestAppContext) {
    // A submenu entry opens `LogTarget::of_container` for its container.
    let labels = log_tab_labels_after(
        "logs-submenu",
        |pod, _| LogTarget::of_container(pod, "proxy"),
        cx,
    );
    assert_eq!(labels, ["api-0/proxy"]);
}

// ---- Spec 0039 step 3: the revision diff dialog ----

fn revision_request() -> crate::revision_diff::RevisionDiffRequest {
    let side =
        |replica_set: &str, revision: u64, is_current: bool| crate::revision_diff::RevisionSide {
            replica_set: replica_set.to_owned(),
            revision: Some(revision),
            tag: None,
            is_current,
            created_at: None,
        };
    crate::revision_diff::diff_request(
        ResourceKey::Kind {
            kind: crate::resource_kind::ResourceKind::Deployments,
            namespace: Some("shop".to_owned()),
            name: "api".to_owned(),
        },
        side("api-old", 37, false),
        side("api-new", 38, true),
    )
}

const REPLICA_SET_JSON: &str = r#"{"apiVersion":"apps/v1","kind":"ReplicaSet","metadata":{"name":"x","namespace":"shop"},"spec":{"template":{"spec":{"containers":[{"name":"api","image":"api:1"}]}}}}"#;

#[gpui_kit::test]
fn revision_diff_uses_the_session_connection(cx: &mut TestAppContext) {
    // Without a session there is no connection to read through, so nothing opens.
    let (window, shell) = open_shell(cx);
    render(window, cx);
    cx.update_window(window.into(), |_, window, cx| {
        shell.update(cx, |shell, cx| {
            shell.open_revision_diff(revision_request(), window, cx)
        });
    })
    .expect("the window is open");
    cx.run_until_parked();
    assert!(!has_dialog(window, cx));
}

#[gpui_kit::test]
fn revision_diff_reads_both_templates_through_the_drawer_cluster(cx: &mut TestAppContext) {
    let fixture = open_switch_fixture("revision-diff-reads", cx);
    let (connection, api) = {
        let _guard = fixture.runtime.enter();
        cluster::fake_api::FakeApi::connection(cluster::WritePolicy::Blocked, |_| {
            (200, REPLICA_SET_JSON.to_owned())
        })
    };
    let session = fixture.session(cx);
    session.update(cx, |session, cx| {
        session.go_live_for_test(connection, NamespaceScope::All, cx);
    });
    let cluster = fixture.cluster("prod-a", cx);
    let request = revision_request();
    fixture.shell.update(cx, |shell, cx| {
        let object = ClusterObject::new(cluster, request.deployment.clone());
        shell.change_selection(Some(object), cx);
        shell.set_drawer_open(true, cx);
    });
    cx.run_until_parked();
    fixture.with_window(cx, |window, cx| {
        fixture.shell.update(cx, |shell, cx| {
            shell.open_revision_diff(request, window, cx)
        });
    });
    assert!(fixture.with_window(cx, |window, cx| window.has_active_dialog(cx)));
    // Both templates are read at once; the session's own lists and checks are not the dialog's.
    let template_requests = || -> Vec<(String, String)> {
        let mut found: Vec<(String, String)> = api
            .requests()
            .into_iter()
            .filter(|request| request.path.contains("/replicasets/"))
            .map(|request| (request.method, request.path))
            .collect();
        found.sort();
        found
    };
    for _ in 0..1_500 {
        cx.run_until_parked();
        if template_requests().len() >= 2 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let paths = template_requests();
    assert_eq!(
        paths,
        [
            (
                "GET".to_owned(),
                "/apis/apps/v1/namespaces/shop/replicasets/api-new".to_owned()
            ),
            (
                "GET".to_owned(),
                "/apis/apps/v1/namespaces/shop/replicasets/api-old".to_owned()
            ),
        ]
    );
}

// ---- Spec 0044 step 1: the remembered dock height ----

fn dock_height_of(
    fixture: &super::app_shell_switch_tests::SwitchFixture,
    cx: &mut TestAppContext,
) -> Option<f32> {
    fixture.shell.read_with(cx, |shell, cx| {
        shell
            .dock_split
            .read(cx)
            .sizes()
            .get(1)
            .copied()
            .map(f32::from)
    })
}

fn saved_height(cx: &mut TestAppContext) -> Option<f32> {
    cx.update(|cx| AppSettings::get(cx).dock.height)
}

fn resize_dock(
    fixture: &super::app_shell_switch_tests::SwitchFixture,
    height: f32,
    cx: &mut TestAppContext,
) {
    let split = fixture
        .shell
        .read_with(cx, |shell, _| shell.dock_split.clone());
    fixture.with_window(cx, |window, cx| {
        split.update(cx, |state, cx| {
            state.resize_panel(1, px(height), window, cx);
        });
    });
}

#[gpui_kit::test]
fn resize_end_saves_the_dock_height(cx: &mut TestAppContext) {
    let fixture = open_log_tab_fixture(
        "dock-height-save",
        |pod, _| LogTarget::of_container(pod, "proxy"),
        cx,
    );
    fixture.draw_twice(cx);
    resize_dock(&fixture, 400., cx);
    assert_eq!(saved_height(cx), Some(400.));
}

#[gpui_kit::test]
fn double_click_on_the_handle_resets_the_dock(cx: &mut TestAppContext) {
    let fixture = open_log_tab_fixture(
        "dock-height-reset",
        |pod, _| LogTarget::of_container(pod, "proxy"),
        cx,
    );
    fixture.draw_twice(cx);
    resize_dock(&fixture, 400., cx);
    fixture.draw_twice(cx);
    assert_eq!(dock_height_of(&fixture, cx), Some(400.));
    let mut visual = gpui_kit::VisualTestContext::from_window(fixture.window.into(), cx);
    let area = visual
        .debug_bounds("dock-handle-reset")
        .expect("the handle has its double-click area");
    visual.simulate_event(gpui_kit::MouseDownEvent {
        button: gpui_kit::MouseButton::Left,
        position: area.center(),
        modifiers: gpui_kit::Modifiers::default(),
        click_count: 2,
        first_mouse: false,
    });
    assert_eq!(dock_height_of(&fixture, cx), Some(280.));
    assert_eq!(saved_height(cx), None);
}

#[gpui_kit::test]
fn dock_opens_at_the_saved_height(cx: &mut TestAppContext) {
    let fixture = open_live_logs_fixture("dock-height-restore", cx);
    cx.update(|cx| AppSettings::update(cx, |settings| settings.dock.height = Some(400.)));
    open_log_tab(&fixture, |pod, _| LogTarget::of_container(pod, "proxy"), cx);
    fixture.draw_twice(cx);
    assert_eq!(dock_height_of(&fixture, cx), Some(400.));
}

// ---- Spec 0044 step 4: Pop out ----

#[gpui_kit::test]
fn leaving_work_ignores_popped_log_tabs(cx: &mut TestAppContext) {
    let fixture = open_log_tab_fixture(
        "pop-out-leaving-work",
        |pod, _| LogTarget::of_container(pod, "proxy"),
        cx,
    );
    let tab = fixture.shell.read_with(cx, |shell, cx| {
        shell.dock.read(cx).log_tab_entities().remove(0)
    });
    tab.update(cx, |_, cx| cx.emit(crate::log_tab::LogTabEvent::PopOut));
    cx.run_until_parked();
    let cluster = fixture.cluster("prod-a", cx);
    fixture.shell.read_with(cx, |shell, cx| {
        assert!(!shell.dock.read(cx).has_tabs());
        assert!(shell.leaving_work(&[cluster], cx).is_empty());
    });
}

#[gpui_kit::test]
fn closing_the_main_window_closes_the_pop_outs(cx: &mut TestAppContext) {
    let fixture = open_log_tab_fixture(
        "pop-out-main-close",
        |pod, _| LogTarget::of_container(pod, "proxy"),
        cx,
    );
    let tab = fixture.shell.read_with(cx, |shell, cx| {
        shell.dock.read(cx).log_tab_entities().remove(0)
    });
    tab.update(cx, |_, cx| cx.emit(crate::log_tab::LogTabEvent::PopOut));
    cx.run_until_parked();
    assert_eq!(cx.update(|cx| cx.windows().len()), 2);
    fixture.with_window(cx, |window, _| window.remove_window());
    cx.run_until_parked();
    assert_eq!(cx.update(|cx| cx.windows().len()), 0);
}

#[gpui_kit::test]
fn open_drawer_section_reveals_the_row_on_its_overview(cx: &mut TestAppContext) {
    let (_, shell) = open_shell(cx);
    shell.update(cx, |shell, cx| {
        // Another tab shows, so only the section request can bring the Overview back.
        shell.drawer.tab = DrawerTab::Yaml;
        shell.open_drawer_section(object(secret_key()), "Selected pods", cx);
    });
    cx.run_until_parked();
    // The drawer paints the request away only when a session shows the row; there is none here.
    shell.read_with(cx, |shell, _| {
        assert_eq!(shell.selected, Some(object(secret_key())));
        assert_eq!(shell.drawer.tab, DrawerTab::Overview);
        assert_eq!(shell.drawer.reveal_section.get(), Some("Selected pods"));
    });
}

#[gpui_kit::test]
fn open_drawer_section_on_the_selection_needs_no_reveal(cx: &mut TestAppContext) {
    let (_, shell) = open_shell(cx);
    shell.update(cx, |shell, cx| {
        shell.change_selection(Some(object(secret_key())), cx);
        shell.open_drawer_section(object(secret_key()), "Remaining resources", cx);
        // At once, before anything is deferred.
        assert_eq!(
            shell.drawer.reveal_section.get(),
            Some("Remaining resources")
        );
    });
}
