use super::*;

#[test]
fn names_follow_dns_subdomain_rules() {
    for name in ["ip-10-0-1-23", "a.b", "a", "0", "a-b.c-d"] {
        assert!(is_dns_subdomain(name), "{name}");
    }
    let too_long = "a".repeat(254);
    let long_label = "a".repeat(64);
    for name in [
        "",
        "A",
        "-a",
        "a-",
        "a/b",
        "a/../x",
        "a..b",
        ".a",
        "a.",
        "a?x",
        "a#b",
        "a%2Fb",
        "a b",
        "\u{e9}",
        too_long.as_str(),
        long_label.as_str(),
    ] {
        assert!(!is_dns_subdomain(name), "{name}");
    }
    let longest = format!("{0}.{0}.{0}.{1}", "a".repeat(63), "a".repeat(61));
    assert_eq!(longest.len(), 253);
    assert!(is_dns_subdomain(&longest));
}

#[test]
fn path_segment_names_allow_rbac_colons_only() {
    for name in ["system:aggregate-to-admin", "A", "a_b", "a.b", ".a"] {
        assert!(is_path_segment_name(name), "{name}");
    }
    let too_long = "a".repeat(254);
    for name in [
        "",
        ".",
        "..",
        "a/b",
        "a%2F",
        "a?b",
        "a#b",
        "a b",
        "a\\b",
        "\u{e9}",
        "a\u{202e}b",
        "a\tb",
        "a\nb",
        too_long.as_str(),
    ] {
        assert!(!is_path_segment_name(name), "{name:?}");
    }
    assert!(is_path_segment_name(&"a".repeat(253)));
}
