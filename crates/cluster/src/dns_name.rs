//! The shape of an object name that is safe to put in a request path.

/// DNS-1123 subdomain: 1-253 chars; dot-separated labels of 1-63 chars from `[a-z0-9-]`, each
/// starting and ending alphanumeric. Load-bearing: kube does not percent-encode a name in a path,
/// so `a/../x` or `a#b` would otherwise change the path a request goes to.
pub(crate) fn is_dns_subdomain(name: &str) -> bool {
    (1..=253).contains(&name.len()) && name.split('.').all(is_dns_label)
}

/// A name that is one path segment, for the RBAC kinds, whose names use `:` (`system:aggregate-to-admin`):
/// 1-253 printable ASCII characters (no space, control, or non-ASCII character), not `.` or `..`, and
/// none of `/ \ % ? #`. kube does not percent-encode a name, so the characters that change the path
/// are refused, and ASCII only keeps look-alike characters out of an object name.
pub(crate) fn is_path_segment_name(name: &str) -> bool {
    (1..=253).contains(&name.len())
        && name != "."
        && name != ".."
        && name
            .chars()
            .all(|ch| ch.is_ascii_graphic() && !matches!(ch, '/' | '\\' | '%' | '?' | '#'))
}

fn is_dns_label(label: &str) -> bool {
    (1..=63).contains(&label.len())
        && label
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && !label.starts_with('-')
        && !label.ends_with('-')
}

#[cfg(test)]
#[path = "dns_name_tests.rs"]
mod dns_name_tests;
