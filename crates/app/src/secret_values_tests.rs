use std::sync::atomic::{AtomicUsize, Ordering};

use gpui_kit::{AppContext as _, TestAppContext};

use super::*;
use crate::launch_options::LaunchScreen;
use crate::settings::ThemePreference;

/// Values used by tests only; no real secret appears anywhere in this file.
const FIXTURE_TEXT: &str = "fixture-text-0016";

fn value(key: &str, bytes: &[u8]) -> SecretValue {
    SecretValue::new(key.to_owned(), bytes.to_vec())
}

fn values() -> Vec<SecretValue> {
    vec![
        value("alpha", FIXTURE_TEXT.as_bytes()),
        value("beta", b"second"),
        value("gamma", &[0xff, 0xfe]),
    ]
}

fn key_of(values: &[SecretValue]) -> Vec<&str> {
    values.iter().map(SecretValue::key).collect()
}

fn revealed_keys(revealed: &[RevealedValue]) -> Vec<&str> {
    revealed.iter().map(|entry| entry.value.key()).collect()
}

#[test]
fn reveal_all_keeps_every_value() {
    let kept = kept_values(values(), &SecretAction::RevealAll).expect("all kept");
    assert_eq!(key_of(&kept), ["alpha", "beta", "gamma"]);
}

#[test]
fn reveal_one_keeps_only_that_key() {
    let kept = kept_values(values(), &SecretAction::Reveal("beta".to_owned())).expect("kept");
    assert_eq!(key_of(&kept), ["beta"]);
}

#[test]
fn copy_keeps_only_that_key() {
    let kept = kept_values(values(), &SecretAction::Copy("alpha".to_owned())).expect("kept");
    assert_eq!(key_of(&kept), ["alpha"]);
    assert_eq!(kept[0].as_text(), Some(FIXTURE_TEXT));
}

#[test]
fn missing_key_is_an_error() {
    let Err(missing) = kept_values(values(), &SecretAction::Reveal("gone".to_owned())) else {
        panic!("a key the secret lost must be an error");
    };
    assert_eq!(missing.text(), "Key gone no longer exists in this secret.");
}

#[test]
fn reveal_replaces_same_key_and_resets_timer() {
    let start = Instant::now();
    let mut revealed = Vec::new();
    reveal(&mut revealed, vec![value("beta", b"one")], start);
    reveal(&mut revealed, vec![value("alpha", b"x")], start);
    let later = start + Duration::from_secs(20);
    reveal(&mut revealed, vec![value("beta", b"two")], later);
    assert_eq!(revealed_keys(&revealed), ["alpha", "beta"]);
    assert_eq!(revealed[1].value.as_text(), Some("two"));
    assert_eq!(revealed[1].hides_at, later + REVEAL_DURATION);
    assert_eq!(revealed[0].hides_at, start + REVEAL_DURATION);
}

#[test]
fn expire_drops_only_due_values() {
    let start = Instant::now();
    let mut revealed = Vec::new();
    reveal(&mut revealed, vec![value("old", b"1")], start);
    reveal(
        &mut revealed,
        vec![value("new", b"2")],
        start + Duration::from_secs(10),
    );
    let changed = expire(&mut revealed, start + REVEAL_DURATION);
    assert!(changed);
    assert_eq!(revealed_keys(&revealed), ["new"]);
}

#[test]
fn expire_reports_no_change() {
    let start = Instant::now();
    let mut revealed = Vec::new();
    reveal(&mut revealed, vec![value("a", b"1")], start);
    assert!(!expire(&mut revealed, start + Duration::from_secs(29)));
    assert_eq!(revealed.len(), 1);
    assert!(!expire(&mut Vec::new(), start));
}

#[test]
fn display_cuts_at_char_boundary() {
    // A three-byte character straddles the 4096-byte limit.
    let mut text = "a".repeat(REVEAL_DISPLAY_LIMIT - 1);
    text.push('\u{20ac}');
    let secret = value("long", text.as_bytes());
    let ValueDisplay::Text { shown, is_cut } = value_display(&secret) else {
        panic!("a UTF-8 value is text");
    };
    assert!(is_cut);
    assert_eq!(shown.len(), REVEAL_DISPLAY_LIMIT - 1);
    let short = value("short", b"abc");
    assert!(matches!(
        value_display(&short),
        ValueDisplay::Text {
            shown: "abc",
            is_cut: false
        }
    ));
}

#[test]
fn binary_value_displays_size() {
    let secret = value("blob", &[0xff, 0x00, 0xfe]);
    assert!(matches!(
        value_display(&secret),
        ValueDisplay::Binary { size_bytes: 3 }
    ));
}

#[test]
fn seconds_left_rounds_up() {
    let now = Instant::now();
    assert_eq!(seconds_left(now + Duration::from_millis(22_100), now), 23);
    assert_eq!(seconds_left(now + Duration::from_secs(30), now), 30);
    assert_eq!(seconds_left(now, now + Duration::from_secs(1)), 0);
}

#[test]
fn forbidden_reads_as_the_missing_right() {
    let forbidden = ClusterError::Forbidden {
        context: "uat".to_owned(),
        action: "reading secret values",
        message: "secrets is forbidden".to_owned(),
    };
    assert_eq!(failure_text(&forbidden), "Not permitted: get secrets");
    let timed_out = ClusterError::TimedOut {
        context: "uat".to_owned(),
        action: "reading secret values",
    };
    assert!(failure_text(&timed_out).contains("did not answer in time"));
}

fn options(screenshot: Option<&str>) -> LaunchOptions {
    LaunchOptions {
        kubeconfig: None,
        context: None,
        namespace: None,
        filter: None,
        select: None,
        theme: Some(ThemePreference::Light),
        color_theme: None,
        config_dir: None,
        screen: LaunchScreen::Kind(ResourceKind::Secrets),
        screenshot: screenshot.map(Into::into),
        window_width: None,
        palette: None,
    }
}

#[test]
fn screenshot_runs_block_values() {
    assert_eq!(
        value_access(&options(Some("out.png"))),
        ValueAccess::Blocked
    );
    assert_eq!(value_access(&options(None)), ValueAccess::Enabled);
}

fn secret_key() -> ResourceKey {
    ResourceKey::Kind {
        kind: ResourceKind::Secrets,
        namespace: Some("shop".to_owned()),
        name: "credentials".to_owned(),
    }
}

fn key(name: &str) -> SecretKey {
    SecretKey {
        name: name.to_owned(),
        size_bytes: 3,
        is_binary: false,
    }
}

/// A view whose fetcher counts calls and returns fixture values.
fn counting_view(access: ValueAccess, calls: &Arc<AtomicUsize>) -> SecretValuesView {
    let calls = Arc::clone(calls);
    let fetch: FetchValues = Arc::new(move || {
        calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(values()) })
    });
    SecretValuesView::new(
        fetch,
        secret_object("ctx"),
        vec![key("alpha"), key("beta")],
        access,
    )
}

/// The fixture Secret in the cluster whose context is `context`.
fn secret_object(context: &str) -> ClusterObject {
    let cluster = crate::cluster_registry::ClusterRef {
        kubeconfig: std::path::PathBuf::from("kube.yaml"),
        context: context.to_owned(),
    };
    ClusterObject::new(cluster, secret_key())
}

#[test]
fn values_view_belongs_to_one_cluster() {
    let calls = Arc::new(AtomicUsize::new(0));
    let view = counting_view(ValueAccess::Enabled, &calls);
    assert!(view.is_for(&secret_object("ctx")));
    // The same namespace and name in another cluster is another Secret.
    assert!(!view.is_for(&secret_object("other")));
}

#[gpui_kit::test]
fn blocked_view_ignores_actions(cx: &mut TestAppContext) {
    let calls = Arc::new(AtomicUsize::new(0));
    let view = cx.new(|_| counting_view(ValueAccess::Blocked, &calls));
    for action in [
        SecretAction::RevealAll,
        SecretAction::Reveal("alpha".to_owned()),
        SecretAction::Copy("alpha".to_owned()),
    ] {
        view.update(cx, |view, cx| view.run(action, cx));
    }
    view.read_with(cx, |view, _| {
        assert!(!view.is_running());
        assert!(view.revealed.is_empty());
        assert!(view.copied.is_none());
    });
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn set_keys_drops_values_of_removed_keys() {
    let calls = Arc::new(AtomicUsize::new(0));
    let mut view = counting_view(ValueAccess::Enabled, &calls);
    let now = Instant::now();
    reveal(
        &mut view.revealed,
        vec![value("alpha", b"1"), value("beta", b"2")],
        now,
    );
    view.set_keys(&[key("beta"), key("delta")]);
    assert_eq!(revealed_keys(&view.revealed), ["beta"]);
    assert_eq!(view.keys.len(), 2);
}

#[test]
fn view_is_for_its_secret_only() {
    let calls = Arc::new(AtomicUsize::new(0));
    let view = counting_view(ValueAccess::Enabled, &calls);
    assert!(view.is_for(&secret_object("ctx")));
    let other = ResourceKey::Kind {
        kind: ResourceKind::Secrets,
        namespace: Some("shop".to_owned()),
        name: "other".to_owned(),
    };
    let cluster = secret_object("ctx").cluster;
    assert!(!view.is_for(&ClusterObject::new(cluster, other)));
}

#[test]
fn secret_view_drops_on_tab_change() {
    let key = secret_key();
    assert_eq!(
        values_subject(Some(&key), DrawerTab::Overview),
        Some(key.clone())
    );
    for tab in [DrawerTab::Yaml, DrawerTab::Events, DrawerTab::Monitor] {
        assert_eq!(values_subject(Some(&key), tab), None);
    }
}

#[test]
fn secret_view_drops_on_subject_change() {
    assert_eq!(values_subject(None, DrawerTab::Overview), None);
    let config_map = ResourceKey::Kind {
        kind: ResourceKind::ConfigMaps,
        namespace: Some("shop".to_owned()),
        name: "settings".to_owned(),
    };
    assert_eq!(values_subject(Some(&config_map), DrawerTab::Overview), None);
    let pod = ResourceKey::Pod {
        namespace: "shop".to_owned(),
        name: "api".to_owned(),
    };
    assert_eq!(values_subject(Some(&pod), DrawerTab::Overview), None);
}

#[test]
fn pending_action_runs_for_its_subject_only() {
    let key = secret_key();
    let other = ResourceKey::Kind {
        kind: ResourceKind::Secrets,
        namespace: Some("shop".to_owned()),
        name: "other".to_owned(),
    };
    assert_eq!(pending_action(&key, Some(&key)), PendingAction::Run);
    assert_eq!(pending_action(&key, Some(&other)), PendingAction::Drop);
    assert_eq!(pending_action(&key, None), PendingAction::Drop);
}
