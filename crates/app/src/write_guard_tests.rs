use super::*;
use crate::environment::{Environment, guess_environment};

fn profile(environment: Environment, read_only: bool) -> ClusterProfile {
    ClusterProfile {
        display_name: "uat-monitor".to_owned(),
        confirm: ConfirmMode::for_tier(environment.tier()),
        environment,
        default_namespace: None,
        read_only,
        allow_node_shell: false,
        debug_image: cluster::DEFAULT_DEBUG_IMAGE.to_owned(),
        node_shell_namespace: "kube-system".to_owned(),
        proxy: Ok(cluster::ProxyChoice::Kubeconfig),
        metrics: None,
    }
}

#[test]
fn confirm_mode_follows_tier() {
    assert_eq!(
        ConfirmMode::for_tier(EnvironmentTier::Production),
        ConfirmMode::TypeName
    );
    for environment in [
        EnvironmentTier::Staging,
        EnvironmentTier::Development,
        EnvironmentTier::Local,
    ] {
        assert_eq!(
            ConfirmMode::for_tier(environment),
            ConfirmMode::Click,
            "{environment:?}"
        );
    }
}

#[test]
fn confirm_step_table() {
    let typed = DialogConfirm::TypeName {
        expected: "prod-eu-1".to_owned(),
    };
    for risk in [ActionRisk::Change, ActionRisk::Destructive] {
        assert_eq!(
            confirm_step(ConfirmMode::TypeName, risk, "prod-eu-1"),
            typed
        );
        assert_eq!(
            confirm_step(ConfirmMode::Click, risk, "prod-eu-1"),
            DialogConfirm::Click
        );
    }
}

#[test]
fn non_prod_tiers_always_show_a_dialog() {
    // An unknown context name is classified Staging, so it clicks like the others.
    let unknown = guess_environment("mystery", "other");
    for environment in [
        EnvironmentTier::Staging,
        EnvironmentTier::Development,
        EnvironmentTier::Local,
        unknown,
    ] {
        let mode = ConfirmMode::for_tier(environment);
        for risk in [ActionRisk::Change, ActionRisk::Destructive] {
            assert_eq!(
                confirm_step(mode, risk, "name"),
                DialogConfirm::Click,
                "{environment:?} {risk:?}"
            );
        }
    }
}

#[test]
fn confirm_mode_serializes_kebab_case() {
    assert_eq!(
        serde_json::to_string(&ConfirmMode::TypeName).expect("serializes"),
        r#""type-name""#
    );
    assert_eq!(
        serde_json::to_string(&ConfirmMode::Click).expect("serializes"),
        r#""click""#
    );
    let parsed: ConfirmMode = serde_json::from_str(r#""type-name""#).expect("parses");
    assert_eq!(parsed, ConfirmMode::TypeName);
}

#[test]
fn lock_at_open_follows_the_profile() {
    assert_eq!(
        WriteLock::at_open(&profile(Environment::PRODUCTION, true), false),
        WriteLock::Locked
    );
    assert_eq!(
        WriteLock::at_open(&profile(Environment::PRODUCTION, false), false),
        WriteLock::Unlocked
    );
    assert_eq!(
        WriteLock::at_open(&profile(Environment::DEVELOPMENT, true), false),
        WriteLock::Locked
    );
}

#[test]
fn a_settings_reset_opens_every_session_locked() {
    assert_eq!(
        WriteLock::at_open(&profile(Environment::STAGING, false), true),
        WriteLock::Locked
    );
}

#[test]
fn the_guard_names_its_own_cluster() {
    let access = AccessState::Unknown;
    let guard = test_guard(
        &access,
        WriteLock::Locked,
        "prod-eu-1",
        Environment::PRODUCTION,
    );
    assert_eq!(guard.cluster.context, "prod-eu-1");
    assert_eq!(guard.display_name(), "prod-eu-1");
    assert_eq!(guard.profile.confirm, ConfirmMode::TypeName);
    assert_eq!(guard.summary.name, "prod-eu-1");
}

#[test]
fn app_has_no_kube_dependency() {
    // The app can only write through `ClusterConnection::write` because it cannot name `kube`.
    // Every spelling counts: a plain key, a renamed package, and a table header.
    let manifest = include_str!("../Cargo.toml");
    let offenders: Vec<&str> = manifest
        .lines()
        .map(str::trim)
        .filter(|line| {
            line.starts_with("kube") || line.contains("package = \"kube") || line.contains(".kube]")
        })
        .collect();
    assert!(offenders.is_empty(), "{offenders:?}");
}

#[test]
fn the_screenshot_build_blocks_writes() {
    // `ClusterConnection::open` forces `WritePolicy::Blocked` when the cluster crate's
    // `block-writes` feature is on, and only this line turns it on for the screenshot build.
    let manifest = include_str!("../Cargo.toml");
    let screenshot = manifest
        .lines()
        .find(|line| line.trim_start().starts_with("screenshot = "))
        .expect("the screenshot feature exists");
    assert!(
        screenshot.contains("k8sboard-cluster/block-writes"),
        "{screenshot}"
    );
}

#[test]
fn lab_writes_only_forwards_to_the_cluster_crate() {
    // The feature adds no other feature: without `screenshot` it does nothing, and the kind-only
    // gate lives in the cluster crate.
    let manifest = include_str!("../Cargo.toml");
    let lab = manifest
        .lines()
        .find(|line| line.trim_start().starts_with("lab-writes = "))
        .expect("the lab-writes feature exists");
    assert_eq!(lab, r#"lab-writes = ["k8sboard-cluster/lab-writes"]"#);
}

#[test]
fn privileged_risk_always_types_the_name() {
    let typed = DialogConfirm::TypeName {
        expected: "wk-03".to_owned(),
    };
    for environment in [
        EnvironmentTier::Production,
        EnvironmentTier::Staging,
        EnvironmentTier::Development,
        EnvironmentTier::Local,
    ] {
        for mode in [ConfirmMode::TypeName, ConfirmMode::Click] {
            assert_eq!(
                confirm_step(mode, ActionRisk::Privileged, "wk-03"),
                typed,
                "{environment:?} {mode:?}"
            );
        }
    }
}
