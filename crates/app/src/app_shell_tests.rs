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
        shell.run_secret_action(key, SecretAction::RevealAll, cx);
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
        shell.change_selection(Some(key), cx);
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
    let key = secret_key_fixture();
    cx.new(|_| SecretValuesView::new(fetch, key, Vec::new(), ValueAccess::Enabled))
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
        shell.change_selection(Some(secret_key_fixture()), cx);
    });
    shell.read_with(cx, |shell, _| assert!(shell.drawer.secret_values.is_none()));
    // The same key again is a no-op, so a second view survives it.
    shell.update(cx, |shell, cx| {
        shell.drawer.secret_values = Some(counting_view(&calls, cx));
        shell.change_selection(Some(secret_key_fixture()), cx);
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
        shell.change_selection(Some(pod_key("api-0")), cx);
        shell.open_drawer_tab(pod_key("api-1"), DrawerTab::Yaml, cx);
    });
    cx.run_until_parked();
    shell.read_with(cx, |shell, _| {
        assert_eq!(shell.selected, Some(pod_key("api-1")));
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

fn last_used(cx: &mut TestAppContext) -> Option<ClusterRef> {
    cx.update(|cx| AppSettings::get(cx).registry.last_used.clone())
}

fn report(shell: &Entity<AppShell>, is_live: bool, cx: &mut TestAppContext) {
    cx.update(|cx| {
        shell.update(cx, |shell, cx| {
            shell.active = Some(context("ctx"));
            shell.record_last_used(is_live, cx);
        });
    });
}

#[gpui_kit::test]
fn open_drawer_tab_on_the_selection_needs_no_reveal(cx: &mut TestAppContext) {
    let (_, shell) = open_shell(cx);
    shell.update(cx, |shell, cx| {
        shell.change_selection(Some(pod_key("api-0")), cx);
        shell.open_drawer_tab(pod_key("api-0"), DrawerTab::Events, cx);
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
        shell.run_secret_action(secret_key(), SecretAction::RevealAll, cx);
    });
    cx.run_until_parked();
    // The pending action itself is consumed or dropped by the next frame (there is no session
    // to read the Secret from), so the tab and the selection are what stay.
    shell.read_with(cx, |shell, _| {
        assert_eq!(shell.selected, Some(secret_key()));
        assert_eq!(shell.drawer.tab, DrawerTab::Overview);
    });
}

#[gpui_kit::test]
fn helm_values_keep_their_revision_through_a_reveal(cx: &mut TestAppContext) {
    let (_, shell) = open_shell(cx);
    let key = ResourceKey::Kind {
        kind: ResourceKind::HelmReleases,
        namespace: Some("shop".to_owned()),
        name: "api".to_owned(),
    };
    shell.update(cx, |shell, cx| {
        shell.open_helm_values(key.clone(), 3, ValuesLayout::Diff, cx);
    });
    cx.run_until_parked();
    shell.read_with(cx, |shell, _| {
        assert_eq!(shell.selected, Some(key.clone()));
        assert_eq!(shell.drawer.tab, DrawerTab::Values);
        // Choosing the subject forgets a revision; the step sets it after.
        assert_eq!(shell.drawer.helm_revision, Some(3));
        assert!(matches!(
            &shell.drawer.pending_helm_layout,
            Some((pending, ValuesLayout::Diff)) if *pending == key
        ));
    });
}

#[gpui_kit::test]
fn last_used_is_written_on_live(cx: &mut TestAppContext) {
    let (_window, shell) = open_shell(cx);
    report(&shell, true, cx);
    let expected = ClusterRef::of(&context("ctx"));
    assert_eq!(last_used(cx), Some(expected));
}

#[gpui_kit::test]
fn last_used_is_not_written_on_failure(cx: &mut TestAppContext) {
    let (_window, shell) = open_shell(cx);
    report(&shell, false, cx);
    assert_eq!(last_used(cx), None);
}

#[gpui_kit::test]
fn last_used_is_written_once_per_session(cx: &mut TestAppContext) {
    let (_window, shell) = open_shell(cx);
    report(&shell, true, cx);
    cx.update(|cx| {
        shell.update(cx, |shell, cx| {
            shell.active = Some(context("other"));
            shell.record_last_used(true, cx);
        });
    });
    assert_eq!(last_used(cx), Some(ClusterRef::of(&context("ctx"))));
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
            shell.toggle_default_namespace("monitoring", cx);
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
    // Ctrl , is handled by the app (no window), and Ctrl O belongs to the Settings window.
    let elsewhere: [&dyn gpui_kit::Action; 2] = [
        &crate::settings_window::OpenSettings,
        &crate::settings_window::ImportKubeconfig,
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
        shell.change_selection(Some(pod_key("api-0")), cx);
        assert_eq!(shell.drawer_subject(), None);
        shell.drawer.is_open = true;
        assert_eq!(shell.drawer_subject(), Some(&pod_key("api-0")));
    });
}

#[gpui_kit::test]
fn closing_drawer_drops_pending_subjects(cx: &mut TestAppContext) {
    let (_, shell) = open_shell(cx);
    shell.update(cx, |shell, cx| {
        shell.change_selection(Some(pod_key("api-0")), cx);
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
        shell.change_selection(Some(pod_key("api-0")), cx);
        shell.set_drawer_open(true, cx);
        assert!(shell.drawer.is_open);
        shell.close_drawer(cx);
        assert!(!shell.drawer.is_open);
        assert_eq!(shell.selected, Some(pod_key("api-0")));
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
        shell.change_selection(Some(pod_key("api-0")), cx);
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
            shell.change_selection(Some(pod_key("api-0")), cx);
            assert!(!shell.drawer.is_open);
            shell.on_row_selected(Some(pod_key("api-0")), false, &table, window, cx);
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
            shell.on_row_selected(Some(pod_key("api-1")), true, &table, window, cx);
            assert_eq!(shell.selected, Some(pod_key("api-1")));
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
        shell.change_selection(Some(pod_key("api-0")), cx);
    });
    press(window, "enter", cx);
    shell.read_with(cx, |shell, _| {
        assert!(shell.drawer.is_open);
        assert_eq!(shell.selected, Some(pod_key("api-0")));
    });
}

#[gpui_kit::test]
fn escape_closes_the_drawer_then_clears_the_cursor(cx: &mut TestAppContext) {
    let (window, shell) = open_shell_with(&["--screen", "pods"], cx);
    render(window, cx);
    shell.update(cx, |shell, cx| {
        shell.change_selection(Some(pod_key("api-0")), cx);
        shell.set_drawer_open(true, cx);
    });
    press(window, "escape", cx);
    shell.read_with(cx, |shell, _| {
        assert!(!shell.drawer.is_open);
        assert_eq!(shell.selected, Some(pod_key("api-0")));
    });
    press(window, "escape", cx);
    shell.read_with(cx, |shell, _| assert_eq!(shell.selected, None));
}

#[gpui_kit::test]
fn copy_name_copies_the_cursor_row_name(cx: &mut TestAppContext) {
    let (window, shell) = open_shell(cx);
    render(window, cx);
    shell.update(cx, |shell, cx| {
        shell.change_selection(Some(pod_key("api-0")), cx);
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
        shell.change_selection(Some(pod_key("api-0")), cx);
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
        let dock = shell.log_dock.read(cx);
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
        shell.change_selection(Some(pod_key("api-0")), cx);
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
        shell.change_selection(Some(secret_key_fixture()), cx);
        shell.set_drawer_open(true, cx);
        shell.drawer.secret_values = Some(counting_view(&calls, cx));
        shell.drawer.pending_secret_action = Some((secret_key_fixture(), SecretAction::RevealAll));
        shell.close_drawer(cx);
        assert!(shell.drawer.secret_values.is_none());
        assert!(shell.drawer.pending_secret_action.is_none());
        // The row stays: only the drawer closed.
        assert_eq!(shell.selected, Some(secret_key_fixture()));
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
    let (_, shell) = open_shell(cx);
    let key = service_key("shop", "web");
    shell.update(cx, |shell, cx| {
        shell.select_on_topology(Some(key.clone()), cx);
        assert!(shell.drawer.is_open);
        assert_eq!(shell.drawer_subject(), Some(&key));
        // A click on the empty canvas closes it and drops the row.
        shell.select_on_topology(None, cx);
        assert!(!shell.drawer.is_open);
        assert_eq!(shell.selected, None);
    });
}

#[gpui_kit::test]
fn a_namespace_change_on_topology_clears_the_selection(cx: &mut TestAppContext) {
    let (window, shell) = open_shell(cx);
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
    let (window, shell) = open_shell_with(&["--screen", "topology"], cx);
    render(window, cx);
    shell.update(cx, |shell, cx| {
        shell.select_on_topology(Some(service_key("shop", "web")), cx);
        shell.close_drawer(cx);
    });
    // The graph is a canvas: J moves nothing and Enter opens nothing.
    press(window, "j", cx);
    press(window, "enter", cx);
    shell.read_with(cx, |shell, _| {
        assert_eq!(shell.selected, Some(service_key("shop", "web")));
        assert!(!shell.drawer.is_open);
    });
}
