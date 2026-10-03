use serde_json::json;

use super::*;

fn pod<'a>(node: &'a str, name: &'a str) -> NodeShellPod<'a> {
    NodeShellPod {
        namespace: "kube-system",
        name,
        node,
        image: DEFAULT_DEBUG_IMAGE,
        user: Some("readonly"),
        instance: "q4m7x2k9pa",
    }
}

#[test]
fn random_suffix_is_five_base36_chars() {
    for _ in 0..50 {
        let suffix = random_suffix();
        assert_eq!(suffix.len(), 5, "{suffix}");
        assert!(
            suffix
                .chars()
                .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit()),
            "{suffix}"
        );
    }
}

#[test]
fn suffixes_differ_between_calls() {
    let all: std::collections::HashSet<_> = (0..50).map(|_| random_suffix()).collect();
    assert!(all.len() > 40, "{all:?}");
}

#[test]
fn run_id_is_ten_chars() {
    assert_eq!(run_id().len(), 10);
}

#[test]
fn debug_container_names_carry_the_prefix() {
    let name = debug_container_name();
    assert!(name.starts_with(DEBUG_CONTAINER_PREFIX), "{name}");
    assert!(is_container_name(&name), "{name}");
}

#[test]
fn node_shell_pod_name_is_valid() {
    let long = "N".repeat(63);
    let nodes = [
        "wk-03",
        "Worker.Node_01",
        "ip-10-0-1-5.ec2.internal",
        long.as_str(),
        "-edge-",
        "..",
    ];
    for node in nodes {
        let name = node_shell_pod_name(node, "x7k2q");
        assert!(name.len() <= 63, "{name}");
        assert!(is_dns_label(&name), "{name}");
        assert!(name.starts_with(NODE_SHELL_PREFIX), "{name}");
        assert!(name.ends_with("-x7k2q"), "{name}");
    }
    assert_eq!(
        node_shell_pod_name("wk-03", "x7k2q"),
        "k8sboard-node-shell-wk-03-x7k2q"
    );
    assert_eq!(
        node_shell_pod_name("Worker.Node_01", "x7k2q"),
        "k8sboard-node-shell-worker-node-01-x7k2q"
    );
    assert_eq!(
        node_shell_pod_name("..", "x7k2q"),
        "k8sboard-node-shell-x7k2q"
    );
}

#[test]
fn a_cut_node_name_does_not_end_in_a_dash() {
    // The cut lands right after a dash: 37 characters are kept.
    let node = format!("{}-{}", "a".repeat(36), "b".repeat(10));
    let name = node_shell_pod_name(&node, "x7k2q");
    assert_eq!(name.len(), 20 + 36 + 1 + 5);
    assert!(is_dns_label(&name), "{name}");
}

#[test]
fn image_text_rules() {
    assert!(is_valid_debug_image(DEFAULT_DEBUG_IMAGE));
    assert!(is_valid_debug_image("registry.local:5000/tools/busybox:1"));
    assert!(!is_valid_debug_image(""));
    assert!(!is_valid_debug_image("busy box"));
    assert!(!is_valid_debug_image("busybox\n"));
    assert!(!is_valid_debug_image(&"a".repeat(256)));
    assert!(is_valid_debug_image(&"a".repeat(255)));
}

#[test]
fn default_image_is_pinned_by_digest() {
    let (name, digest) = DEFAULT_DEBUG_IMAGE
        .split_once("@sha256:")
        .expect("a digest-pinned image");
    assert!(name.starts_with("docker.io/library/busybox:"));
    assert_eq!(digest.len(), 64);
    assert!(digest.chars().all(|ch| ch.is_ascii_hexdigit()));
}

#[test]
fn label_values_follow_the_server_rules() {
    assert!(is_label_value("wk-03"));
    assert!(is_label_value("a.b_c-d"));
    assert!(!is_label_value("-a"));
    assert!(!is_label_value("a-"));
    assert!(!is_label_value("a b"));
    assert!(!is_label_value(&"a".repeat(64)));
}

#[test]
fn debug_patch_holds_one_container_that_ends_with_its_attach() {
    let patch = debug_container_patch("k8sboard-debug-x7k2q", DEFAULT_DEBUG_IMAGE, "api");
    assert_eq!(
        patch,
        json!({"spec": {"ephemeralContainers": [{
            "name": "k8sboard-debug-x7k2q",
            "image": DEFAULT_DEBUG_IMAGE,
            "command": ["sh"],
            "stdin": true,
            "stdinOnce": true,
            "tty": true,
            "targetContainerName": "api",
            "imagePullPolicy": "IfNotPresent",
            "terminationMessagePolicy": "File",
        }]}})
    );
}

#[test]
fn node_shell_pod_is_privileged_on_the_node() {
    let body = node_shell_pod(&pod("wk-03", "k8sboard-node-shell-wk-03-x7k2q"));
    assert_eq!(
        body,
        json!({
            "apiVersion": "v1", "kind": "Pod",
            "metadata": {
                "name": "k8sboard-node-shell-wk-03-x7k2q", "namespace": "kube-system",
                "labels": {
                    "app.kubernetes.io/managed-by": "k8sboard",
                    "k8sboard.io/purpose": "node-shell",
                    "k8sboard.io/node": "wk-03",
                    "k8sboard.io/instance": "q4m7x2k9pa",
                },
                "annotations": {"k8sboard.io/user": "readonly"},
            },
            "spec": {
                "nodeName": "wk-03", "hostPID": true, "restartPolicy": "Never",
                "terminationGracePeriodSeconds": 0, "activeDeadlineSeconds": 14400,
                "automountServiceAccountToken": false, "enableServiceLinks": false,
                "tolerations": [{"operator": "Exists"}],
                "containers": [{
                    "name": "shell", "image": DEFAULT_DEBUG_IMAGE,
                    "command": [
                        "nsenter", "-t", "1", "-m", "-u", "-i", "-n", "-p", "--", "sh", "-c",
                        AUTO_SCRIPT,
                    ],
                    "stdin": true, "stdinOnce": true, "tty": true,
                    "securityContext": {"privileged": true},
                    "imagePullPolicy": "IfNotPresent", "terminationMessagePolicy": "File",
                }],
            },
        })
    );
}

#[test]
fn node_shell_command_enters_pid_1() {
    let body = node_shell_pod(&pod("wk-03", "k8sboard-node-shell-wk-03-x7k2q"));
    let command = body["spec"]["containers"][0]["command"]
        .as_array()
        .expect("a command");
    let words: Vec<&str> = command.iter().filter_map(|word| word.as_str()).collect();
    assert_eq!(
        words[..10],
        [
            "nsenter", "-t", "1", "-m", "-u", "-i", "-n", "-p", "--", "sh"
        ]
    );
    assert_eq!(words[10], "-c");
    assert_eq!(words[11], AUTO_SCRIPT);
    assert_eq!(words.len(), 12);
}

#[test]
fn the_pod_declares_no_host_network_and_no_token() {
    let spec = node_shell_pod(&pod("wk-03", "k8sboard-node-shell-wk-03-x7k2q"))["spec"].clone();
    assert!(spec.get("hostNetwork").is_none());
    assert!(spec.get("hostIPC").is_none());
    assert_eq!(spec["automountServiceAccountToken"], json!(false));
}

#[test]
fn a_node_that_is_no_label_value_has_no_node_label_and_no_user_means_no_annotation() {
    let mut input = pod("wk-03", "k8sboard-node-shell-x-x7k2q");
    input.node = "a.b-";
    input.user = None;
    let body = node_shell_pod(&input);
    assert!(body["metadata"]["labels"].get("k8sboard.io/node").is_none());
    assert!(body["metadata"].get("annotations").is_none());
    // The sweep still finds it: the other three labels stay.
    assert_eq!(
        body["metadata"]["labels"]["k8sboard.io/instance"],
        json!("q4m7x2k9pa")
    );
}
