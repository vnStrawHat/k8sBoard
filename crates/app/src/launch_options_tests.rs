use super::*;
use crate::drawer::DrawerTab;
use crate::resource_kind::ResourceKind;
use crate::settings::ThemePreference;

fn parse(args: &[&str]) -> Result<LaunchRequest, String> {
    parse_launch_options(args.iter().map(|arg| (*arg).to_owned()))
}

fn run_options(args: &[&str]) -> LaunchOptions {
    match parse(args) {
        Ok(LaunchRequest::Run(options)) => *options,
        other => panic!("expected run options, got {other:?}"),
    }
}

#[test]
fn parses_all_flags() {
    let options = run_options(&[
        "--kubeconfig",
        "kube.yml",
        "--context",
        "ctx",
        "--namespace",
        "team-a",
        "--filter",
        "label:app=api",
        "--select",
        "team-a/api",
        "--theme",
        "dark",
        "--config-dir",
        "cfg",
        "--screen",
        "pod-containers",
        "--screenshot",
        "out.png",
    ]);
    assert_eq!(
        options,
        LaunchOptions {
            kubeconfig: Some(PathBuf::from("kube.yml")),
            context: Some("ctx".to_owned()),
            namespace: Some(NamespaceScope::Named("team-a".to_owned())),
            filter: Some("label:app=api".to_owned()),
            select: Some("team-a/api".to_owned()),
            theme: Some(ThemePreference::Dark),
            config_dir: Some(PathBuf::from("cfg")),
            screen: LaunchScreen::PodDrawer(DrawerTab::Containers),
            screenshot: Some(PathBuf::from("out.png")),
            window_width: None,
            palette: None,
        }
    );
}

#[test]
fn defaults_without_flags() {
    let options = run_options(&[]);
    assert_eq!(options.window_width, None);
    assert_eq!(options.theme, None);
    assert_eq!(options.screenshot, None);
    assert_eq!(options.kubeconfig, None);
    assert_eq!(options.context, None);
    assert_eq!(options.namespace, None);
}

#[test]
fn unknown_flag_is_error() {
    let error = parse(&["--bogus"]).expect_err("unknown flag must fail");
    assert!(error.contains("--bogus"), "{error}");
}

#[test]
fn missing_value_is_error() {
    let error = parse(&["--context"]).expect_err("missing value must fail");
    assert!(error.contains("--context"), "{error}");
}

#[test]
fn invalid_screen_or_theme_is_error() {
    assert!(parse(&["--screen", "traffic"]).is_err());
    assert!(parse(&["--theme", "sepia"]).is_err());
}

#[test]
fn help_flag_returns_help() {
    assert_eq!(
        parse(&["--context", "a", "--help"]),
        Ok(LaunchRequest::Help)
    );
}

fn join(entries: &[&str]) -> OsString {
    std::env::join_paths(entries).expect("joinable paths")
}

fn absolute(path: &str) -> PathBuf {
    std::path::absolute(path).expect("absolute path")
}

#[test]
fn kubeconfig_chain_flag_wins() {
    let chain = kubeconfig_chain(
        Some(PathBuf::from("flag.yml")),
        Some(join(&["env.yml"])),
        Some(PathBuf::from("home")),
    );
    assert_eq!(chain, [absolute("flag.yml")]);
}

#[test]
fn kubeconfig_chain_lists_every_env_entry() {
    let env = join(&["first.yml", "", "second.yml"]);
    let chain = kubeconfig_chain(None, Some(env), Some(PathBuf::from("home")));
    assert_eq!(chain, [absolute("first.yml"), absolute("second.yml")]);
}

#[test]
fn kubeconfig_chain_falls_back_to_home() {
    let home = PathBuf::from("home");
    let expected = [absolute("home/.kube/config")];
    assert_eq!(kubeconfig_chain(None, None, Some(home.clone())), expected);
    let from_empty_env = kubeconfig_chain(None, Some(OsString::new()), Some(home));
    assert_eq!(from_empty_env, expected);
}

#[test]
fn kubeconfig_chain_is_empty_when_nothing_is_available() {
    assert!(kubeconfig_chain(None, None, None).is_empty());
}

#[test]
fn standalone_files_skip_chain_members_and_duplicates() {
    let chain = [absolute("chain.yml")];
    let registered = [
        PathBuf::from("chain.yml"),
        PathBuf::from("extra.yml"),
        PathBuf::from("extra.yml"),
        PathBuf::from("other.yml"),
    ];
    assert_eq!(
        standalone_files(&registered, &chain),
        [absolute("extra.yml"), absolute("other.yml")]
    );
}

#[test]
fn parses_logs_screens() {
    assert_eq!(
        run_options(&["--screen", "logs-dock"]).screen,
        LaunchScreen::LogsDock
    );
    assert_eq!(
        run_options(&["--screen", "logs-zoomed"]).screen,
        LaunchScreen::LogsZoomed
    );
    assert_eq!(
        run_options(&["--screen", "logs-popout"]).screen,
        LaunchScreen::LogsPopout
    );
    assert_eq!(
        run_options(&["--screen", "logs-workload"]).screen,
        LaunchScreen::LogsWorkload
    );
    for screen in [
        LaunchScreen::LogsDock,
        LaunchScreen::LogsZoomed,
        LaunchScreen::LogsPopout,
        LaunchScreen::LogsWorkload,
    ] {
        assert_eq!(screen.screen(), Screen::Pods);
        assert!(screen.has_dock());
        assert!(!screen.has_drawer());
    }
}

#[test]
fn kind_screens_parse_from_plural_slugs() {
    for kind in ResourceKind::ALL {
        let list = run_options(&["--screen", kind.plural()]).screen;
        assert_eq!(list, LaunchScreen::Kind(kind));
        assert_eq!(list.screen(), Screen::Kind(kind));
        assert!(!list.has_drawer());

        let drawer = run_options(&["--screen", &format!("{}-drawer", kind.plural())]).screen;
        assert_eq!(drawer, LaunchScreen::KindDrawer(kind, DrawerTab::Overview));
        assert_eq!(drawer.screen(), Screen::Kind(kind));
        assert!(drawer.has_drawer());
    }
    assert!(parse(&["--screen", "pods-drawer"]).is_err());
}

#[test]
fn drawer_screens_parse_with_their_tab() {
    let cases = [
        (
            "pod-drawer",
            LaunchScreen::PodDrawer(DrawerTab::Overview),
            Screen::Pods,
        ),
        (
            "pod-containers",
            LaunchScreen::PodDrawer(DrawerTab::Containers),
            Screen::Pods,
        ),
        (
            "pod-events",
            LaunchScreen::PodDrawer(DrawerTab::Events),
            Screen::Pods,
        ),
        (
            "node-drawer",
            LaunchScreen::NodeDrawer(DrawerTab::Overview),
            Screen::Nodes,
        ),
        (
            "node-events",
            LaunchScreen::NodeDrawer(DrawerTab::Events),
            Screen::Nodes,
        ),
        (
            "deployments-events",
            LaunchScreen::KindDrawer(ResourceKind::Deployments, DrawerTab::Events),
            Screen::Kind(ResourceKind::Deployments),
        ),
    ];
    for (name, expected, screen) in cases {
        let launch = run_options(&["--screen", name]).screen;
        assert_eq!(launch, expected, "{name}");
        assert_eq!(launch.screen(), screen, "{name}");
        assert!(launch.has_drawer(), "{name}");
        assert!(!launch.has_dock(), "{name}");
    }
    assert_eq!(LaunchScreen::Pods.drawer_tab(), None);
    assert_eq!(
        LaunchScreen::NodeDrawer(DrawerTab::Events).drawer_tab(),
        Some(DrawerTab::Events)
    );
}

#[test]
fn every_kind_has_an_events_screen() {
    for kind in ResourceKind::ALL {
        let launch = run_options(&["--screen", &format!("{}-events", kind.plural())]).screen;
        assert_eq!(launch, LaunchScreen::KindDrawer(kind, DrawerTab::Events));
    }
    assert!(parse(&["--screen", "pods-events"]).is_err());
}

#[test]
fn yaml_screens_open_the_yaml_tab() {
    let cases = [
        ("pod-yaml", LaunchScreen::PodDrawer(DrawerTab::Yaml)),
        ("node-yaml", LaunchScreen::NodeDrawer(DrawerTab::Yaml)),
        (
            "deployments-yaml",
            LaunchScreen::KindDrawer(ResourceKind::Deployments, DrawerTab::Yaml),
        ),
    ];
    for (name, expected) in cases {
        let launch = run_options(&["--screen", name]).screen;
        assert_eq!(launch, expected, "{name}");
        assert!(launch.has_drawer(), "{name}");
        assert_eq!(launch.drawer_tab(), Some(DrawerTab::Yaml), "{name}");
    }
    assert!(parse(&["--screen", "pods-yaml"]).is_err());
}

#[test]
fn filter_flag_is_parsed() {
    assert_eq!(
        run_options(&["--filter", "argo"]).filter.as_deref(),
        Some("argo")
    );
    assert_eq!(run_options(&[]).filter, None);
    assert_eq!(
        parse(&["--filter"]),
        Err("missing value for --filter".to_owned())
    );
}

#[test]
fn namespace_flag_accepts_a_comma_list_up_to_five() {
    let scope = |text: &str| run_options(&["--namespace", text]).namespace;
    assert_eq!(scope("a"), Some(NamespaceScope::Named("a".to_owned())));
    assert_eq!(
        scope("b,a,a"),
        Some(NamespaceScope::of_namespaces([
            "a".to_owned(),
            "b".to_owned()
        ]))
    );
    assert!(matches!(scope("a,b,c,d,e"), Some(NamespaceScope::Several(names)) if names.len() == 5));
    assert_eq!(
        parse(&["--namespace", "a,b,c,d,e,f"]),
        Err("at most 5 namespaces for --namespace".to_owned())
    );
    assert!(parse(&["--namespace", ",,"]).is_err());
}

#[test]
fn selected_screens_tick_the_first_rows() {
    for (text, screen) in [
        ("pods-selected", LaunchScreen::PodsSelected),
        ("nodes-selected", LaunchScreen::NodesSelected),
    ] {
        let options = run_options(&["--screen", text]);
        assert_eq!(options.screen, screen);
        assert!(screen.checks_rows());
        assert!(!screen.has_drawer());
    }
    assert_eq!(LaunchScreen::PodsSelected.screen(), Screen::Pods);
    assert_eq!(LaunchScreen::NodesSelected.screen(), Screen::Nodes);
    assert!(!LaunchScreen::Pods.checks_rows());
}

#[test]
fn monitor_screens_parse() {
    let screen = |name: &str| run_options(&["--kubeconfig", "k", "--screen", name]).screen;
    assert_eq!(
        screen("pod-monitor"),
        LaunchScreen::PodDrawer(DrawerTab::Monitor)
    );
    assert_eq!(
        screen("node-monitor"),
        LaunchScreen::NodeDrawer(DrawerTab::Monitor)
    );
    assert_eq!(
        screen("deployments-monitor"),
        LaunchScreen::KindDrawer(ResourceKind::Deployments, DrawerTab::Monitor)
    );
    // They open expanded (W4c) and wait for two ticks of their feed.
    assert!(screen("pod-monitor").opens_expanded());
    assert_eq!(screen("pod-monitor").min_metrics_ticks(), 2);
    assert_eq!(screen("pods").min_metrics_ticks(), 1);
    assert!(!screen("pod-drawer").opens_expanded());
    // Only the workloads that own pods have a Monitor tab.
    for name in ["configmaps-monitor", "cronjobs-monitor", "nope-monitor"] {
        assert!(
            parse(&["--kubeconfig", "k", "--screen", name]).is_err(),
            "{name}"
        );
    }
}

#[test]
fn select_flag_is_parsed() {
    assert_eq!(
        run_options(&["--select", "kong-config"]).select.as_deref(),
        Some("kong-config")
    );
    assert_eq!(run_options(&[]).select, None);
    assert_eq!(
        parse(&["--select"]),
        Err("missing value for --select".to_owned())
    );
}

#[test]
fn pvc_screens_wait_for_kubelet_stats() {
    let pvcs = ResourceKind::PersistentVolumeClaims;
    assert!(LaunchScreen::Kind(pvcs).shows_kubelet_stats());
    assert!(LaunchScreen::KindDrawer(pvcs, DrawerTab::Overview).shows_kubelet_stats());
    assert!(!LaunchScreen::KindDrawer(pvcs, DrawerTab::Yaml).shows_kubelet_stats());
    assert!(!LaunchScreen::Kind(ResourceKind::Deployments).shows_kubelet_stats());
}

#[test]
fn releases_values_and_manifest_slugs_parse() {
    for (name, tab) in [
        ("releases-values", DrawerTab::Values),
        ("releases-manifest", DrawerTab::Manifest),
    ] {
        let launch = run_options(&["--screen", name]).screen;
        assert_eq!(
            launch,
            LaunchScreen::KindDrawer(ResourceKind::HelmReleases, tab)
        );
        assert!(launch.has_drawer(), "{name}");
    }
    let drawer = run_options(&["--screen", "releases-drawer"]).screen;
    assert_eq!(
        drawer,
        LaunchScreen::KindDrawer(ResourceKind::HelmReleases, DrawerTab::Overview)
    );
}

#[test]
fn values_slug_rejected_for_other_kinds() {
    assert!(parse(&["--screen", "secrets-values"]).is_err());
    assert!(parse(&["--screen", "deployments-manifest"]).is_err());
    assert!(parse(&["--screen", "pods-values"]).is_err());
}

#[test]
fn crds_slugs_parse_through_plural() {
    let screen = |name| run_options(&["--screen", name]).screen;
    assert_eq!(
        screen("customresourcedefinitions"),
        LaunchScreen::Kind(ResourceKind::Crds)
    );
    for (name, tab) in [
        ("customresourcedefinitions-drawer", DrawerTab::Overview),
        ("customresourcedefinitions-events", DrawerTab::Events),
        ("customresourcedefinitions-yaml", DrawerTab::Yaml),
    ] {
        assert_eq!(
            screen(name),
            LaunchScreen::KindDrawer(ResourceKind::Crds, tab),
            "{name}"
        );
    }
}

#[test]
fn parses_custom_screen_with_suffixes() {
    for (text, tab) in [
        ("custom:certificates.cert-manager.io", None),
        (
            "custom:certificates.cert-manager.io-drawer",
            Some(DrawerTab::Overview),
        ),
        (
            "custom:certificates.cert-manager.io-events",
            Some(DrawerTab::Events),
        ),
        (
            "custom:certificates.cert-manager.io-yaml",
            Some(DrawerTab::Yaml),
        ),
    ] {
        let options = run_options(&["--screen", text]);
        let expected = LaunchScreen::Custom {
            crd_name: "certificates.cert-manager.io",
            tab,
        };
        assert_eq!(options.screen, expected, "{text}");
        assert_eq!(options.screen.has_drawer(), tab.is_some(), "{text}");
        // Until the CRD list resolves it, the launch shows the CRDs screen.
        assert_eq!(options.screen.screen(), Screen::Kind(ResourceKind::Crds));
    }
}

#[test]
fn custom_names_with_dashes_keep_their_dashes() {
    let yaml = run_options(&["--screen", "custom:kafka-topics.kafka.strimzi.io-yaml"]);
    assert_eq!(
        yaml.screen,
        LaunchScreen::Custom {
            crd_name: "kafka-topics.kafka.strimzi.io",
            tab: Some(DrawerTab::Yaml)
        }
    );
    let plain = run_options(&["--screen", "custom:kafka-topics.kafka.strimzi.io"]);
    assert_eq!(
        plain.screen,
        LaunchScreen::Custom {
            crd_name: "kafka-topics.kafka.strimzi.io",
            tab: None
        }
    );
}

#[test]
fn parses_logs_workload_screen() {
    let screen = run_options(&["--screen", "logs-workload"]).screen;
    assert_eq!(screen, LaunchScreen::LogsWorkload);
    assert!(screen.has_dock());
    assert_eq!(screen.screen(), Screen::Pods);
}

#[test]
fn screen_topology_parses() {
    let list = run_options(&["--screen", "topology"]).screen;
    assert_eq!(list, LaunchScreen::Topology);
    assert_eq!(list.screen(), Screen::Topology);
    assert!(!list.has_drawer());
    assert!(list.shows_topology());
}

#[test]
fn screen_topology_rbac_parses() {
    let list = run_options(&["--screen", "topology-rbac"]).screen;
    assert_eq!(list, LaunchScreen::TopologyRbac);
    assert_eq!(list.screen(), Screen::Topology);
    assert!(list.shows_topology());
}

#[test]
fn screen_topology_problems_parses() {
    let list = run_options(&["--screen", "topology-problems"]).screen;
    assert_eq!(list, LaunchScreen::TopologyProblems);
    assert_eq!(list.screen(), Screen::Topology);
    assert!(list.shows_topology());
}

#[test]
fn screen_topology_selected_parses() {
    let list = run_options(&["--screen", "topology-selected"]).screen;
    assert_eq!(list, LaunchScreen::TopologySelected);
    assert_eq!(list.screen(), Screen::Topology);
    assert!(list.shows_topology());
}

#[test]
fn screen_issues_parses() {
    let list = run_options(&["--screen", "issues"]).screen;
    assert_eq!(list, LaunchScreen::Issues);
    assert_eq!(list.screen(), Screen::Issues);
    assert!(!list.has_drawer());
    // The drawer variant reveals the first issue, which opens its object's drawer.
    let drawer = run_options(&["--screen", "issues-drawer"]).screen;
    assert_eq!(drawer, LaunchScreen::IssuesDrawer);
    assert_eq!(drawer.screen(), Screen::Issues);
    assert!(drawer.has_drawer());
}

#[test]
fn parses_analysis_screens() {
    let screen = run_options(&["--screen", "who-can"]).screen;
    assert_eq!(screen, LaunchScreen::WhoCan);
    assert!(screen.opens_dialog());
    assert!(!screen.has_drawer() && !screen.has_dock());
    assert_eq!(screen.screen(), Screen::Kind(ResourceKind::ClusterRoles));
    assert!(!LaunchScreen::Pods.opens_dialog());
    let screen = run_options(&["--screen", "test-traffic"]).screen;
    assert_eq!(screen, LaunchScreen::TestTraffic);
    assert!(screen.opens_dialog());
    assert_eq!(screen.screen(), Screen::Kind(ResourceKind::NetworkPolicies));
    for (text, expected) in [
        ("check-permissions", LaunchScreen::CheckPermissions),
        ("account-permissions", LaunchScreen::AccountPermissions),
    ] {
        let screen = run_options(&["--screen", text]).screen;
        assert_eq!(screen, expected);
        assert!(screen.opens_dialog());
        assert_eq!(screen.screen(), Screen::Kind(ResourceKind::ServiceAccounts));
    }
}

#[test]
fn config_dir_flag_is_parsed() {
    let options = run_options(&["--config-dir", ".tmp/config"]);
    assert_eq!(options.config_dir, Some(PathBuf::from(".tmp/config")));
    assert_eq!(run_options(&[]).config_dir, None);
}

#[test]
fn config_dir_flag_needs_a_value() {
    assert!(parse(&["--config-dir"]).is_err());
}

#[test]
fn theme_accepts_system() {
    for (text, expected) in [
        ("system", ThemePreference::System),
        ("light", ThemePreference::Light),
        ("dark", ThemePreference::Dark),
    ] {
        assert_eq!(run_options(&["--theme", text]).theme, Some(expected));
    }
}

#[test]
fn screen_overview_parses() {
    let overview = run_options(&["--screen", "overview"]).screen;
    assert_eq!(overview, LaunchScreen::Overview);
    assert_eq!(overview.screen(), Screen::Overview);
    assert!(!overview.has_drawer());
}

#[test]
fn screen_switcher_parses_and_opens_overview() {
    let switcher = run_options(&["--screen", "switcher"]).screen;
    assert_eq!(switcher, LaunchScreen::Switcher);
    assert_eq!(switcher.screen(), Screen::Overview);
    assert!(!switcher.has_drawer());
    assert!(switcher.settings_screen().is_none());
}

#[test]
fn theme_rejects_unknown() {
    let error = parse(&["--theme", "sepia"]).expect_err("unknown theme");
    assert!(error.contains("--theme"), "{error}");
}

#[test]
fn screen_settings_parses() {
    let screen = run_options(&["--screen", "settings"]).screen;
    assert_eq!(
        screen,
        LaunchScreen::Settings(SettingsPage::Clusters, SettingsSize::Standard)
    );
    assert_eq!(
        screen.settings_screen(),
        Some((SettingsPage::Clusters, SettingsSize::Standard))
    );
    // The main window behind it is the default one.
    assert_eq!(screen.screen(), Screen::Overview);
    assert!(!screen.has_drawer());
}

#[test]
fn screen_settings_appearance_parses() {
    let screen = run_options(&["--screen", "settings-appearance"]).screen;
    assert_eq!(
        screen,
        LaunchScreen::Settings(SettingsPage::Appearance, SettingsSize::Standard)
    );
}

#[test]
fn screen_settings_tall_parses() {
    let screen = run_options(&["--screen", "settings-tall"]).screen;
    assert_eq!(
        screen,
        LaunchScreen::Settings(SettingsPage::Clusters, SettingsSize::Tall)
    );
}

#[test]
fn other_screens_have_no_settings_page() {
    assert_eq!(LaunchScreen::Pods.settings_screen(), None);
}

#[test]
fn default_screen_is_overview() {
    let options = run_options(&[]);
    assert_eq!(options.screen, LaunchScreen::Overview);
    assert_eq!(options.screen.screen(), Screen::Overview);
}

#[test]
fn screen_pods_still_parses() {
    let options = run_options(&["--screen", "pods"]);
    assert_eq!(options.screen, LaunchScreen::Pods);
    assert_eq!(options.screen.screen(), Screen::Pods);
}

#[test]
fn window_width_parses() {
    assert_eq!(
        run_options(&["--window-width", "1000"]).window_width,
        Some(1000)
    );
    assert_eq!(
        run_options(&["--window-width", "800"]).window_width,
        Some(800)
    );
    assert_eq!(
        run_options(&["--window-width", "3840"]).window_width,
        Some(3840)
    );
}

#[test]
fn window_width_out_of_range_is_an_error() {
    for text in ["799", "3841", "wide", "-1", ""] {
        let error = parse(&["--window-width", text]).expect_err("must fail");
        assert!(error.contains("--window-width"), "{error}");
    }
    assert!(parse(&["--window-width"]).is_err());
}

#[test]
fn parses_shortcuts_and_pods_cursor_screens() {
    let sheet = run_options(&["--screen", "shortcuts"]).screen;
    assert_eq!(sheet, LaunchScreen::Shortcuts);
    assert_eq!(sheet.screen(), Screen::Pods);
    // The sheet is a dialog over the list: no row is selected.
    assert!(sheet.opens_dialog());
    assert!(!sheet.selects_row());

    let cursor = run_options(&["--screen", "pods-cursor"]).screen;
    assert_eq!(cursor, LaunchScreen::PodsCursor);
    assert_eq!(cursor.screen(), Screen::Pods);
    // It selects a row but opens no drawer.
    assert!(cursor.selects_row());
    assert!(!cursor.has_drawer());
    assert!(!cursor.opens_dialog());
}

#[test]
fn the_usage_lists_the_key_map_screens() {
    assert!(USAGE.contains("shortcuts"));
    assert!(USAGE.contains("pods-cursor"));
}

#[test]
fn screen_settings_shortcuts_parses() {
    let screen = run_options(&["--screen", "settings-shortcuts"]).screen;
    assert_eq!(
        screen,
        LaunchScreen::Settings(SettingsPage::KeyboardShortcuts, SettingsSize::Tall)
    );
}

#[test]
fn palette_is_off_without_the_flag() {
    assert_eq!(run_options(&[]).palette, None);
}

#[test]
fn palette_flag_takes_the_query_text() {
    let options = run_options(&["--screen", "deployments-drawer", "--palette", "> rest pay"]);
    assert_eq!(options.palette.as_deref(), Some("> rest pay"));
}

#[test]
fn palette_flag_needs_a_value() {
    assert!(parse(&["--palette"]).is_err());
}

#[test]
fn the_usage_lists_the_palette_flag() {
    assert!(USAGE.contains("--palette"));
}

#[test]
fn the_write_dialog_screens_open_on_nodes() {
    for (name, screen) in [
        ("cordon-confirm", LaunchScreen::CordonConfirm),
        ("unlock-confirm", LaunchScreen::UnlockConfirm),
    ] {
        let parsed = run_options(&["--screen", name]).screen;
        assert_eq!(parsed, screen);
        assert!(parsed.opens_dialog());
        assert_eq!(parsed.screen(), Screen::Nodes);
        assert!(parsed.shows_node_usage());
    }
}

#[test]
fn the_shell_fixture_screens_open_pods_with_a_dock() {
    for (name, screen) in [
        ("shell-fixture", LaunchScreen::ShellFixture),
        ("shell-dock-fixture", LaunchScreen::ShellDockFixture),
        ("shell-paste-fixture", LaunchScreen::ShellPasteFixture),
        ("shell-picker-fixture", LaunchScreen::ShellPickerFixture),
        ("shell-confirm-fixture", LaunchScreen::ShellConfirmFixture),
        ("shell-find-fixture", LaunchScreen::ShellFindFixture),
    ] {
        let parsed = run_options(&["--screen", name]).screen;
        assert_eq!(parsed, screen);
        assert_eq!(parsed.screen(), Screen::Pods);
        assert_eq!(
            parsed.has_dock(),
            screen != LaunchScreen::ShellConfirmFixture,
            "{name}: the confirm screen is a dialog, not a dock"
        );
        assert_eq!(
            parsed.opens_dialog(),
            screen == LaunchScreen::ShellConfirmFixture
        );
    }
}

#[test]
fn the_scale_screens_open_on_the_first_deployment() {
    for (name, screen) in [
        ("scale-popover", LaunchScreen::ScalePopover),
        ("scale-confirm", LaunchScreen::ScaleConfirm),
    ] {
        let parsed = run_options(&["--screen", name]).screen;
        assert_eq!(parsed, screen);
        assert_eq!(parsed.screen(), Screen::Kind(ResourceKind::Deployments));
        // The cursor row is picked first, then the popover or the dialog opens on it.
        assert!(parsed.selects_row());
        assert!(parsed.opens_dialog());
        assert_eq!(parsed.row_kind(), Some(ResourceKind::Deployments));
        // No drawer: the screen shows the list with the cursor on a row.
        assert!(!parsed.has_drawer());
    }
    assert_eq!(
        LaunchScreen::KindDrawer(ResourceKind::Jobs, DrawerTab::Overview).row_kind(),
        Some(ResourceKind::Jobs)
    );
    assert_eq!(LaunchScreen::Nodes.row_kind(), None);
}

#[test]
fn restart_bulk_confirm_ticks_four_deployments_then_opens_the_dialog() {
    let screen = run_options(&["--screen", "restart-bulk-confirm"]).screen;
    assert_eq!(screen, LaunchScreen::RestartBulkConfirm);
    assert_eq!(screen.screen(), Screen::Kind(ResourceKind::Deployments));
    assert!(screen.checks_rows());
    assert_eq!(screen.checked_count(), 4);
    assert!(screen.opens_dialog());
    // The other tick screens keep their two rows.
    assert_eq!(LaunchScreen::PodsSelected.checked_count(), 2);
}

#[test]
fn a_menu_screen_opens_the_drawer_menu_of_the_first_row() {
    let screen = run_options(&["--screen", "cronjobs-menu"]).screen;
    assert_eq!(screen, LaunchScreen::KindMenu(ResourceKind::CronJobs));
    assert_eq!(screen.screen(), Screen::Kind(ResourceKind::CronJobs));
    assert!(screen.opens_menu());
    // A menu hangs off the drawer, so the first row is selected and its drawer is open.
    assert!(screen.has_drawer() && screen.selects_row());
    assert_eq!(screen.drawer_tab(), Some(DrawerTab::Overview));
    assert_eq!(screen.row_kind(), Some(ResourceKind::CronJobs));
    // The other screens open no menu, and a name that is no kind is no screen.
    assert!(!LaunchScreen::Kind(ResourceKind::CronJobs).opens_menu());
    assert!(parse(&["--screen", "nothings-menu"]).is_err());
}

#[test]
fn the_port_forward_fixture_screens_open_the_page_and_need_no_cluster() {
    for (name, screen, opens_dialog) in [
        ("port-forwards", LaunchScreen::PortForwards, false),
        ("port-forwards-list", LaunchScreen::PortForwardsList, false),
        (
            "port-forward-new-fixture",
            LaunchScreen::PortForwardNewFixture,
            true,
        ),
        (
            "port-forward-confirm-fixture",
            LaunchScreen::PortForwardConfirmFixture,
            true,
        ),
        (
            "port-forward-remove-fixture",
            LaunchScreen::PortForwardRemoveFixture,
            true,
        ),
    ] {
        let parsed = run_options(&["--screen", name]).screen;
        assert_eq!(parsed, screen);
        assert_eq!(parsed.screen(), Screen::PortForwarding);
        assert!(parsed.is_port_forward_fixture());
        assert_eq!(parsed.opens_dialog(), opens_dialog, "{name}");
        assert!(!parsed.has_dock() && !parsed.selects_row(), "{name}");
    }
    assert!(!LaunchScreen::Pods.is_port_forward_fixture());
}

#[test]
fn usage_lists_the_port_forward_screens() {
    for name in [
        "port-forwards",
        "port-forwards-list",
        "port-forward-new-fixture",
        "port-forward-confirm-fixture",
        "port-forward-remove-fixture",
    ] {
        assert!(USAGE.contains(name), "{name}");
    }
}

#[test]
fn screen_edit_yaml_diff_parses() {
    let screen = run_options(&["--screen", "edit-yaml-diff"]).screen;
    assert_eq!(screen, LaunchScreen::EditYamlDiff);
    assert_eq!(screen.screen(), Screen::Kind(ResourceKind::Deployments));
    // It is opened from fixed data once the shell renders, like a dialog screen, and selects no row.
    assert!(screen.opens_dialog());
    assert!(!screen.selects_row() && !screen.has_drawer() && !screen.checks_rows());
    assert!(USAGE.contains("edit-yaml-diff"));
}

#[test]
fn screen_revision_diff_parses() {
    let screen = run_options(&["--screen", "revision-diff"]).screen;
    assert_eq!(screen, LaunchScreen::RevisionDiff);
    assert_eq!(screen.screen(), Screen::Kind(ResourceKind::Deployments));
    // A dialog over fixed data: it opens once the shell renders, waits for no cluster, selects no row.
    assert!(screen.opens_dialog() && screen.is_dialog_fixture());
    assert!(!screen.selects_row() && !screen.has_drawer() && !screen.checks_rows());
    assert!(USAGE.contains("revision-diff"));
}

#[test]
fn screen_delete_confirm_parses() {
    let screen = run_options(&["--screen", "delete-confirm"]).screen;
    assert_eq!(screen, LaunchScreen::DeleteConfirm);
    assert_eq!(screen.screen(), Screen::Kind(ResourceKind::Deployments));
    // Drawn from fixed data once the shell renders, like a dialog screen, and selects no row.
    assert!(screen.opens_dialog());
    assert!(!screen.selects_row() && !screen.has_drawer() && !screen.checks_rows());
    assert!(USAGE.contains("delete-confirm"));
}

#[test]
fn screen_delete_bulk_confirm_parses() {
    let screen = run_options(&["--screen", "delete-bulk-confirm"]).screen;
    assert_eq!(screen, LaunchScreen::DeleteBulkConfirm);
    assert_eq!(screen.screen(), Screen::Pods);
    assert!(screen.opens_dialog());
    assert!(!screen.selects_row() && !screen.has_drawer() && !screen.checks_rows());
    assert!(USAGE.contains("delete-bulk-confirm"));
}

#[test]
fn the_node_shell_confirm_screen_is_a_fixed_dialog_over_nodes() {
    let parsed = run_options(&["--screen", "node-shell-confirm"]).screen;
    assert_eq!(parsed, LaunchScreen::NodeShellConfirm);
    assert_eq!(parsed.screen(), Screen::Nodes);
    assert!(parsed.opens_dialog());
    assert!(parsed.is_dialog_fixture());
    assert!(!parsed.has_dock());
    // It is drawn from fixed data, so it waits for no node metrics either.
    assert!(!parsed.shows_node_usage());
    assert!(USAGE.contains("node-shell-confirm"));
}

#[test]
fn the_options_fixtures_are_dialogs_over_pods_and_nodes() {
    for (name, screen, list) in [
        (
            "node-shell-options",
            LaunchScreen::NodeShellOptions,
            Screen::Nodes,
        ),
        (
            "debug-container-options",
            LaunchScreen::DebugContainerOptions,
            Screen::Pods,
        ),
    ] {
        let parsed = run_options(&["--screen", name]).screen;
        assert_eq!(parsed, screen);
        assert_eq!(parsed.screen(), list);
        assert!(parsed.opens_dialog());
        assert!(parsed.is_dialog_fixture());
        assert!(!parsed.has_dock());
        assert!(USAGE.contains(name), "{name}");
    }
}

#[test]
fn the_staging_confirm_and_the_sweep_are_offline_dialogs() {
    for (name, screen, list) in [
        (
            "node-shell-confirm-staging",
            LaunchScreen::NodeShellConfirmStaging,
            Screen::Nodes,
        ),
        (
            "leftover-sweep-fixture",
            LaunchScreen::LeftoverSweepFixture,
            Screen::Pods,
        ),
    ] {
        let parsed = run_options(&["--screen", name]).screen;
        assert_eq!(parsed, screen);
        assert_eq!(parsed.screen(), list);
        assert!(parsed.opens_dialog());
        assert!(parsed.is_dialog_fixture());
        assert!(!parsed.has_dock());
        assert!(USAGE.contains(name), "{name}");
    }
}

#[test]
fn the_node_editor_fixtures_are_offline_dialogs_over_nodes() {
    for (name, screen) in [
        ("node-taints-editor", LaunchScreen::NodeTaintsEditor),
        ("node-labels-editor", LaunchScreen::NodeLabelsEditor),
        (
            "node-taints-editor-invalid",
            LaunchScreen::NodeTaintsEditorInvalid,
        ),
        ("drain-dialog", LaunchScreen::DrainDialog),
    ] {
        let parsed = run_options(&["--screen", name]).screen;
        assert_eq!(parsed, screen);
        assert_eq!(parsed.screen(), Screen::Nodes);
        assert!(parsed.opens_dialog());
        assert!(parsed.is_dialog_fixture());
        assert!(!parsed.has_dock());
        assert!(!parsed.shows_node_usage());
        assert!(USAGE.contains(name), "{name}");
    }
}

#[test]
fn the_drain_progress_screen_is_an_offline_dock_over_nodes() {
    let parsed = run_options(&["--screen", "drain-progress"]).screen;
    assert_eq!(parsed, LaunchScreen::DrainProgress);
    assert_eq!(parsed.screen(), Screen::Nodes);
    assert!(parsed.has_dock());
    assert!(parsed.is_dock_fixture());
    assert!(!parsed.opens_dialog());
    assert!(!parsed.shows_node_usage());
    assert!(USAGE.contains("drain-progress"));
}

#[test]
fn the_debug_tab_fixtures_are_offline_dock_screens() {
    for (name, screen) in [
        ("node-shell-tab-fixture", LaunchScreen::NodeShellTabFixture),
        (
            "debug-shell-tab-fixture",
            LaunchScreen::DebugShellTabFixture,
        ),
    ] {
        let parsed = run_options(&["--screen", name]).screen;
        assert_eq!(parsed, screen);
        assert_eq!(parsed.screen(), Screen::Pods);
        assert!(parsed.has_dock());
        assert!(parsed.is_dock_fixture());
        assert!(!parsed.opens_dialog());
        assert!(USAGE.contains(name), "{name}");
    }
}

#[test]
fn screen_hpa_range_popover_parses() {
    let screen = run_options(&["--screen", "hpa-range-popover"]).screen;
    assert_eq!(screen, LaunchScreen::HpaRangePopover);
    assert_eq!(
        screen.screen(),
        Screen::Kind(ResourceKind::HorizontalPodAutoscalers)
    );
    // Drawn from fixed data once the shell renders, and selects no row.
    assert!(screen.opens_dialog());
    assert!(!screen.selects_row() && !screen.has_drawer() && !screen.checks_rows());
    assert!(USAGE.contains("hpa-range-popover"));
}

#[test]
fn screen_expand_confirm_parses() {
    let screen = run_options(&["--screen", "expand-confirm"]).screen;
    assert_eq!(screen, LaunchScreen::ExpandConfirm);
    assert_eq!(
        screen.screen(),
        Screen::Kind(ResourceKind::PersistentVolumeClaims)
    );
    // Drawn from fixed data once the shell renders, and selects no row.
    assert!(screen.opens_dialog());
    assert!(!screen.selects_row() && !screen.has_drawer() && !screen.checks_rows());
    assert!(USAGE.contains("expand-confirm"));
}

#[test]
fn screen_default_class_confirm_parses() {
    let screen = run_options(&["--screen", "default-class-confirm"]).screen;
    assert_eq!(screen, LaunchScreen::DefaultClassConfirm);
    assert_eq!(screen.screen(), Screen::Kind(ResourceKind::StorageClasses));
    // Drawn from fixed data once the shell renders, and selects no row.
    assert!(screen.opens_dialog());
    assert!(!screen.selects_row() && !screen.has_drawer() && !screen.checks_rows());
    assert!(USAGE.contains("default-class-confirm"));
}

#[test]
fn the_stuck_drain_screen_is_an_offline_dock_over_nodes() {
    let parsed = run_options(&["--screen", "drain-progress-stuck"]).screen;
    assert_eq!(parsed, LaunchScreen::DrainProgressStuck);
    assert_eq!(parsed.screen(), Screen::Nodes);
    assert!(parsed.has_dock());
    assert!(parsed.is_dock_fixture());
    assert!(!parsed.opens_dialog());
    assert!(!parsed.shows_node_usage());
    assert!(USAGE.contains("drain-progress-stuck"));
}

#[test]
fn screen_values_edit_parses() {
    let screen = run_options(&["--screen", "values-edit"]).screen;
    assert_eq!(screen, LaunchScreen::ValuesEdit);
    assert_eq!(screen.screen(), Screen::Kind(ResourceKind::Secrets));
    // It is opened from fixed data once the shell renders, like Edit YAML's diff, and selects no row.
    assert!(screen.opens_dialog());
    assert!(!screen.selects_row() && !screen.has_drawer() && !screen.checks_rows());
    assert!(USAGE.contains("values-edit"));
}
