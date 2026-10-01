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
            namespace: Some("team-a".to_owned()),
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
    for screen in [LaunchScreen::LogsDock, LaunchScreen::LogsZoomed] {
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
