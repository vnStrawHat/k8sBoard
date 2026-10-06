use super::*;

fn choices() -> Vec<DebugChoice> {
    vec![
        DebugChoice {
            name: "proxy".to_owned(),
            tag: "SIDECAR",
        },
        DebugChoice {
            name: "web".to_owned(),
            tag: "MAIN",
        },
    ]
}

#[test]
fn debug_image_validation() {
    let valid = |image: &str| validate_debug(Some("web"), image, &choices()).is_ok();
    assert!(valid("busybox:1.36"));
    assert!(valid(cluster::DEFAULT_DEBUG_IMAGE));
    assert!(valid(&"a".repeat(255)));
    assert!(!valid(""));
    assert!(!valid("   "));
    assert!(!valid("bus ybox"));
    assert!(!valid(&"a".repeat(256)));
    let errors = validate_debug(Some("web"), "", &choices()).expect_err("an empty image");
    assert_eq!(errors.image, Some(IMAGE_ERROR));
    assert_eq!(errors.target, None);
}

#[test]
fn the_image_is_trimmed() {
    let chosen = validate_debug(Some("web"), "  busybox:1  ", &choices()).expect("valid");
    assert_eq!(chosen.image, "busybox:1");
    assert_eq!(chosen.target_container, "web");
}

#[test]
fn the_target_must_be_a_listed_running_container() {
    assert!(validate_debug(Some("proxy"), "busybox", &choices()).is_ok());
    for target in [None, Some("init"), Some("")] {
        let errors = validate_debug(target, "busybox", &choices()).expect_err("no such target");
        assert_eq!(errors.target, Some("Pick a running container"));
    }
}

#[test]
fn node_shell_options_need_a_namespace_and_an_image() {
    let chosen = validate_node_shell(" kube-system ", "busybox:1").expect("valid");
    assert_eq!(
        chosen,
        NodeShellChosen {
            namespace: "kube-system".to_owned(),
            image: "busybox:1".to_owned(),
        }
    );
    let blank = validate_node_shell("", "busybox:1").expect_err("no namespace");
    assert_eq!(blank.namespace, Some("Enter a namespace"));
    let bad = validate_node_shell("Kube System", "busybox:1").expect_err("not a name");
    assert_eq!(bad.namespace, Some("Not a valid namespace name"));
    let both = validate_node_shell("ok", "a b").expect_err("a bad image");
    assert_eq!(both.namespace, None);
    assert_eq!(both.image, Some(IMAGE_ERROR));
}

#[test]
fn the_select_starts_on_the_preselected_container_else_the_first_main_one() {
    let list = choices();
    assert_eq!(first_selected(&list, Some("proxy")), 0);
    assert_eq!(first_selected(&list, Some("web")), 1);
    // A container that is no longer listed falls back to the default.
    assert_eq!(first_selected(&list, Some("gone")), 1);
    assert_eq!(first_selected(&list, None), 1);
    assert_eq!(first_selected(&[], None), 0);
}

#[test]
fn the_dialog_texts_are_the_spec_texts() {
    assert_eq!(
        DEBUG_WARNING,
        "Ephemeral containers cannot be removed. It stays in the pod spec until the pod is deleted. Each Reconnect adds another container."
    );
    assert_eq!(
        CLOSING_NOTE,
        "Closing the tab ends the shell and everything started from it."
    );
    assert_eq!(
        NODE_SHELL_NOTE,
        "Closing the tab ends the shell and everything started from it. The pod is deleted when the shell ends."
    );
    assert_eq!(
        node_shell_warning("wk-03"),
        "Creates a privileged pod with host PID access on wk-03. Anything you run affects the node."
    );
}

#[test]
fn an_image_pinned_by_digest_gets_a_short_digest_line() {
    let image = "docker.io/library/busybox:1.36.1@sha256:73aaf090f3d85aa34ee199857f03fa3a95c8ede2ffd4cc2cdb5b94e566b11662";
    assert_eq!(
        digest_summary(image).as_deref(),
        Some("digest sha256:73aa…62")
    );
    assert_eq!(
        digest_summary("  busybox@sha256:abcdef  ").as_deref(),
        Some("digest sha256:abcdef"),
        "a digest this short is shown whole"
    );
}

#[test]
fn an_image_without_a_digest_has_no_digest_line() {
    for image in [
        "busybox:1.36",
        "  busybox:1.36  ",
        "reg.io:5000/a/b:v1",
        "a@",
        "a@sha256:",
    ] {
        assert_eq!(digest_summary(image), None, "{image}");
    }
}
