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
