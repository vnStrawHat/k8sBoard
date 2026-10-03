use serde_json::json;

use super::*;

fn taint(key: &str, value: Option<&str>, effect: &str) -> NodeTaint {
    NodeTaint {
        key: key.to_owned(),
        value: value.map(str::to_owned),
        effect: effect.to_owned(),
        time_added: None,
    }
}

fn set(key: &str, value: &str) -> LabelChange {
    LabelChange {
        key: key.to_owned(),
        value: Some(value.to_owned()),
    }
}

fn remove(key: &str) -> LabelChange {
    LabelChange {
        key: key.to_owned(),
        value: None,
    }
}

#[test]
fn eviction_body_pins_the_uid_in_camel_case() {
    let body = eviction_body("payments", "api-1", "u-1", GracePeriod::Seconds(30));
    assert_eq!(
        body,
        json!({
            "apiVersion": "policy/v1",
            "kind": "Eviction",
            "metadata": {"name": "api-1", "namespace": "payments"},
            "deleteOptions": {"preconditions": {"uid": "u-1"}, "gracePeriodSeconds": 30},
        })
    );
}

#[test]
fn eviction_body_omits_grace_for_the_pod_default() {
    let body = eviction_body("payments", "api-1", "u-1", GracePeriod::PodDefault);
    assert!(body["deleteOptions"].get("gracePeriodSeconds").is_none());
    assert_eq!(body["deleteOptions"]["preconditions"]["uid"], "u-1");
}

#[test]
fn taints_patch_sends_the_list_with_the_version_and_time_added() {
    let added: jiff::Timestamp = "2026-10-02T08:00:00.123Z".parse().expect("a timestamp");
    let mut unreachable = taint("node.kubernetes.io/unreachable", None, "NoExecute");
    unreachable.time_added = Some(added);
    let taints = [
        taint("dedicated", Some("ingress"), "NoSchedule"),
        unreachable,
    ];
    assert_eq!(
        taints_patch(&taints, "42"),
        json!({
            "metadata": {"resourceVersion": "42"},
            "spec": {"taints": [
                {"key": "dedicated", "value": "ingress", "effect": "NoSchedule"},
                {
                    "key": "node.kubernetes.io/unreachable",
                    "effect": "NoExecute",
                    "timeAdded": "2026-10-02T08:00:00Z",
                },
            ]},
        })
    );
}

#[test]
fn no_taints_is_an_empty_list_not_null() {
    let body = taints_patch(&[], "7");
    assert_eq!(body["spec"]["taints"], json!([]));
}

#[test]
fn labels_patch_sets_and_removes_keys() {
    let body = labels_patch(&[set("team", "infra"), remove("old-key")]);
    assert_eq!(
        body,
        json!({"metadata": {"labels": {"team": "infra", "old-key": null}}})
    );
}

#[test]
fn qualified_names_follow_the_kubernetes_rule() {
    for valid in [
        "dedicated",
        "example.com/gpu",
        "a.b-c_d",
        "node-role.kubernetes.io/infra",
    ] {
        assert!(is_qualified_name(valid), "{valid}");
    }
    let too_long = "a".repeat(64);
    for invalid in [
        "",
        "/name",
        "prefix/",
        "-a",
        "a-",
        "a b",
        "Example.com/x",
        "a/b/c",
        too_long.as_str(),
    ] {
        assert!(!is_qualified_name(invalid), "{invalid}");
    }
}

#[test]
fn taints_need_a_known_effect_and_valid_text() {
    assert!(are_valid_taints(&[taint("a", Some("b"), "NoSchedule")]));
    assert!(are_valid_taints(&[]));
    assert!(!are_valid_taints(&[taint("a", None, "Evict")]));
    assert!(!are_valid_taints(&[taint("", None, "NoSchedule")]));
    assert!(!are_valid_taints(&[taint(
        "a",
        Some("has space"),
        "NoSchedule"
    )]));
}

#[test]
fn a_taint_repeats_neither_key_nor_effect() {
    let twice = [
        taint("a", None, "NoSchedule"),
        taint("a", Some("x"), "NoSchedule"),
    ];
    assert!(!are_valid_taints(&twice));
    let other_effect = [
        taint("a", None, "NoSchedule"),
        taint("a", None, "NoExecute"),
    ];
    assert!(are_valid_taints(&other_effect));
}

#[test]
fn label_changes_need_valid_unique_keys() {
    assert!(are_valid_label_changes(&[
        set("team", "infra"),
        remove("old")
    ]));
    assert!(!are_valid_label_changes(&[]));
    assert!(!are_valid_label_changes(&[
        set("team", "a"),
        remove("team")
    ]));
    assert!(!are_valid_label_changes(&[set("bad key", "a")]));
    assert!(!are_valid_label_changes(&[set("team", "not valid!")]));
}

#[test]
fn uids_are_short_dashed_alphanumerics() {
    assert!(is_valid_uid("5b6c0d4e-1234-4abc-9def-0123456789ab"));
    assert!(!is_valid_uid(""));
    assert!(!is_valid_uid("u/1"));
    assert!(!is_valid_uid(&"a".repeat(65)));
}

#[test]
fn label_change_debug_hides_the_value() {
    let text = format!("{:?}", set("team", "secret-value"));
    assert!(text.contains("team"), "{text}");
    assert!(!text.contains("secret-value"), "{text}");
}
