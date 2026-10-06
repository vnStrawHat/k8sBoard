use super::*;

use EnvironmentTier::{Development, Local, Production, Staging};

#[test]
fn guess_table() {
    let rows = [
        ("prod-eu-1", "", Production),
        (
            "arn:aws:eks:eu-central-1:4471:cluster/prod-eu-1",
            "",
            Production,
        ),
        ("stg-us-1", "", Staging),
        ("uat", "", Staging),
        ("dev-shared", "", Development),
        ("kind-k8sboard", "", Local),
        ("minikube", "", Local),
        ("docker-desktop", "", Local),
        ("kind-prod", "", Production),
        ("latest", "", Staging),
        ("readonly@Monitor", "", Staging),
        ("admin", "prod-1", Production),
        ("staging", "", Staging),
        ("test-cluster", "", Development),
        ("k3d-dev", "", Development),
        ("localhost", "", Local),
    ];
    for (context, cluster, expected) in rows {
        assert_eq!(
            guess_environment(context, cluster),
            expected,
            "context {context:?}, cluster {cluster:?}"
        );
    }
}

#[test]
fn riskiest_match_wins() {
    assert_eq!(guess_environment("dev-prod", ""), Production);
    assert_eq!(guess_environment("dev-stg", ""), Staging);
    assert_eq!(guess_environment("kind-dev", ""), Development);
}

#[test]
fn tokens_ignore_substrings() {
    assert_eq!(guess_environment("latest", ""), Staging);
    assert_eq!(guess_environment("contest", ""), Staging);
}

#[test]
fn trailing_digits_are_trimmed() {
    assert_eq!(guess_environment("prod1", ""), Production);
    assert_eq!(guess_environment("dev01", ""), Development);
}

#[test]
fn cluster_name_counts_when_context_is_unknown() {
    assert_eq!(guess_environment("admin@x", "prod-eu"), Production);
    assert_eq!(guess_environment("admin@x", "dev-eu"), Development);
}

#[test]
fn uat_monitor_context_is_staging() {
    assert_eq!(
        guess_environment("readonly@Monitor", "cluster.local"),
        Staging
    );
}

#[test]
fn badge_text_per_tier() {
    assert_eq!(Local.badge(), "LOCAL");
    assert_eq!(Development.badge(), "DEV");
    assert_eq!(Staging.badge(), "STG");
    assert_eq!(Production.badge(), "PROD");
}

#[test]
fn tier_order_is_risk() {
    assert!(Local < Development && Development < Staging && Staging < Production);
    let riskiest = [Development, Production, Local].into_iter().max();
    assert_eq!(riskiest, Some(Production));
}

#[test]
fn tier_serializes_lowercase() {
    let text = serde_json::to_string(&Production).expect("serializes");
    assert_eq!(text, "\"production\"");
    let back: EnvironmentTier = serde_json::from_str("\"local\"").expect("parses");
    assert_eq!(back, Local);
}

fn custom(name: &str, tier: EnvironmentTier) -> CustomEnvironment {
    CustomEnvironment {
        name: name.to_owned(),
        color: EnvironmentColor::Teal,
        tier,
    }
}

fn key(name: &str) -> EnvironmentKey {
    EnvironmentKey::Custom(name.to_owned())
}

#[test]
fn tier_colors() {
    assert_eq!(Production.color(), EnvironmentColor::Red);
    assert_eq!(Staging.color(), EnvironmentColor::Amber);
    assert_eq!(Development.color(), EnvironmentColor::Blue);
    assert_eq!(Local.color(), EnvironmentColor::Gray);
}

#[test]
fn environment_key_round_trips() {
    let built_in = EnvironmentKey::BuiltIn(Production);
    assert_eq!(
        serde_json::to_value(&built_in).expect("serializes"),
        serde_json::json!("production")
    );
    assert_eq!(
        serde_json::from_value::<EnvironmentKey>(serde_json::json!("production")).expect("parses"),
        built_in
    );
    assert_eq!(
        serde_json::to_value(key("QA")).expect("serializes"),
        serde_json::json!("QA")
    );
    // The built-in strings are lower case; any other spelling is a custom name.
    assert_eq!(
        serde_json::from_value::<EnvironmentKey>(serde_json::json!("Production")).expect("parses"),
        key("Production")
    );
}

#[test]
fn custom_environment_round_trips() {
    let json = serde_json::json!({"name": "QA", "color": "green", "tier": "staging"});
    let parsed: CustomEnvironment = serde_json::from_value(json.clone()).expect("parses");
    assert_eq!(
        parsed,
        CustomEnvironment {
            name: "QA".to_owned(),
            color: EnvironmentColor::Green,
            tier: Staging,
        }
    );
    assert_eq!(serde_json::to_value(&parsed).expect("serializes"), json);
}

#[test]
fn resolve_finds_custom_by_name() {
    let list = [custom("QA", Staging)];
    let resolved = resolve_environment(&key("QA"), &list);
    assert_eq!(resolved, Environment::Custom(list[0].clone()));
    assert_eq!(resolved.tier(), Staging);
    assert_eq!(resolved.color(), EnvironmentColor::Teal);
}

#[test]
fn built_in_key_resolves_to_the_built_in() {
    assert_eq!(
        resolve_environment(&EnvironmentKey::BuiltIn(Local), &[custom("QA", Staging)]),
        Environment::LOCAL
    );
}

#[test]
fn missing_custom_resolves_to_production() {
    assert_eq!(
        resolve_environment(&key("Gone"), &[]),
        Environment::PRODUCTION
    );
}

#[test]
fn reserved_custom_name_is_ignored_at_resolve() {
    let list = [custom("Production", Local), custom("PROD", Local)];
    for name in ["Production", "PROD"] {
        let resolved = resolve_environment(&key(name), &list);
        assert_eq!(resolved, Environment::PRODUCTION, "{name}");
        assert_eq!(resolved.tier(), Production, "{name}");
    }
}

#[test]
fn repeated_custom_name_is_ignored() {
    let list = [custom("QA", Staging), custom("qa", Local)];
    assert_eq!(
        resolve_environment(&key("qa"), &list),
        Environment::PRODUCTION
    );
    let names: Vec<_> = usable_environments(&list)
        .map(|environment| environment.name.as_str())
        .collect();
    assert_eq!(names, ["QA"]);
}

#[test]
fn reference_match_is_case_sensitive() {
    assert_eq!(
        resolve_environment(&key("qa"), &[custom("QA", Staging)]),
        Environment::PRODUCTION
    );
}

#[test]
fn reserved_words_follow_the_tiers() {
    let mut words: Vec<&str> = vec!["auto"];
    words.extend(BUILT_IN_GROUP_TITLES);
    for tier in EnvironmentTier::ALL {
        words.push(tier.name());
        words.push(tier.badge());
    }
    for word in words {
        assert!(is_reserved(word), "{word}");
        assert!(is_reserved(&word.to_lowercase()), "{word}");
        assert!(
            is_reserved(&format!("  {} ", word.to_uppercase())),
            "{word}"
        );
    }
    assert!(!is_reserved("QA"));
}

#[test]
fn custom_badge_is_upper_case_name() {
    assert_eq!(
        Environment::Custom(custom("Pre-prod", Staging)).badge(),
        "PRE-PROD"
    );
    assert_eq!(Environment::STAGING.badge(), "STG");
}

#[gpui_kit::test]
fn palette_color_uses_theme_tokens(cx: &mut gpui_kit::TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        let theme = cx.theme().clone();
        let expected = [
            (EnvironmentColor::Red, theme.danger),
            (EnvironmentColor::Amber, theme.warning),
            (EnvironmentColor::Green, theme.success),
            (EnvironmentColor::Blue, theme.info),
            (EnvironmentColor::Teal, theme.cyan),
            (EnvironmentColor::Purple, theme.magenta),
            (EnvironmentColor::Gray, theme.muted_foreground),
        ];
        for (color, token) in expected {
            assert_eq!(palette_color(color, cx), token, "{color:?}");
        }
    });
}

#[gpui_kit::test]
fn environment_color_follows_the_environment(cx: &mut gpui_kit::TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        // The badge of a built-in is the risk signal: PROD is the danger token.
        assert_eq!(
            environment_color(&Environment::PRODUCTION, cx),
            cx.theme().danger
        );
        let teal = Environment::Custom(custom("QA", Staging));
        assert_eq!(environment_color(&teal, cx), cx.theme().cyan);
    });
}

#[test]
fn palette_lists_seven_colors_with_names() {
    let names: Vec<_> = EnvironmentColor::ALL
        .iter()
        .map(|color| color.name())
        .collect();
    assert_eq!(
        names,
        ["Red", "Amber", "Green", "Blue", "Teal", "Purple", "Gray"]
    );
}

#[test]
fn cluster_environment_label_pairs_the_name_with_the_badge() {
    assert_eq!(
        cluster_environment_label("uat-monitor", &Environment::PRODUCTION),
        "uat-monitor · PROD"
    );
    let qa = Environment::Custom(custom("qa", Staging));
    assert_eq!(cluster_environment_label("eu-1", &qa), "eu-1 · QA");
}

#[test]
fn production_follows_the_tier() {
    assert!(Environment::PRODUCTION.is_production());
    assert!(Environment::Custom(custom("DR", Production)).is_production());
    assert!(!Environment::STAGING.is_production());
    assert!(!Environment::Custom(custom("QA", Staging)).is_production());
}
