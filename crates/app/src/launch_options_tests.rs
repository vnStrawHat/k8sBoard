use super::*;
use crate::drawer::DrawerTab;
use crate::resource_kind::ResourceKind;

fn parse(args: &[&str]) -> Result<LaunchRequest, String> {
    parse_launch_options(args.iter().map(|arg| (*arg).to_owned()))
}

fn run_options(args: &[&str]) -> LaunchOptions {
    match parse(args) {
        Ok(LaunchRequest::Run(options)) => options,
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
            theme: Some(ThemeChoice::Dark),
            screen: LaunchScreen::PodDrawer(DrawerTab::Containers),
            screenshot: Some(PathBuf::from("out.png")),
        }
    );
}

#[test]
fn defaults_without_flags() {
    let options = run_options(&[]);
    assert_eq!(options.screen, LaunchScreen::Pods);
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
    assert!(parse(&["--screen", "overview"]).is_err());
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

#[test]
fn kubeconfig_flag_wins_over_env_and_home() {
    let path = kubeconfig_path(
        Some(PathBuf::from("flag.yml")),
        Some(join(&["env.yml"])),
        Some(PathBuf::from("home")),
    );
    assert_eq!(path, Some(PathBuf::from("flag.yml")));
}

#[test]
fn kubeconfig_env_uses_first_entry() {
    let env = join(&["first.yml", "second.yml"]);
    assert!(has_ignored_kubeconfig_entries(Some(env.as_os_str())));
    let path = kubeconfig_path(None, Some(env), Some(PathBuf::from("home")));
    assert_eq!(path, Some(PathBuf::from("first.yml")));
    assert!(!has_ignored_kubeconfig_entries(Some(
        join(&["only.yml"]).as_os_str()
    )));
}

#[test]
fn kubeconfig_falls_back_to_home_dot_kube_config() {
    let home = PathBuf::from("home");
    let path = kubeconfig_path(None, None, Some(home.clone()));
    assert_eq!(path, Some(home.join(".kube").join("config")));
    let from_empty_env = kubeconfig_path(None, Some(OsString::new()), Some(home.clone()));
    assert_eq!(from_empty_env, Some(home.join(".kube").join("config")));
}

#[test]
fn kubeconfig_none_when_nothing_available() {
    assert_eq!(kubeconfig_path(None, None, None), None);
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
        run_options(&["--screen", "logs-workload"]).screen,
        LaunchScreen::LogsWorkload
    );
    for screen in [
        LaunchScreen::LogsDock,
        LaunchScreen::LogsZoomed,
        LaunchScreen::LogsWorkload,
    ] {
        assert_eq!(screen.screen(), Screen::Pods);
        assert!(screen.has_log_dock());
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
        assert!(!launch.has_log_dock(), "{name}");
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
    assert!(screen.has_log_dock());
    assert_eq!(screen.screen(), Screen::Pods);
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
    assert!(!screen.has_drawer() && !screen.has_log_dock());
    assert_eq!(screen.screen(), Screen::Kind(ResourceKind::ClusterRoles));
    assert!(!LaunchScreen::Pods.opens_dialog());
}
