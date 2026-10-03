//! The bodies and names of the debug writes (spec 0037): the ephemeral container patch, the node
//! shell pod, and the names that carry them. Pure: nothing here sends a request.
//!
//! A body holds the image and the node name and nothing a session reads or writes.

use std::collections::hash_map::RandomState;
use std::hash::BuildHasher as _;
use std::time::Instant;

use serde_json::{Value, json};

use crate::dns_name::is_dns_label;
use crate::pod_shell::AUTO_SCRIPT;

/// The image of a debug container and a node shell pod unless the cluster's settings name another.
/// Pinned by digest so the tag cannot be re-pointed. The digest is the index of
/// `library/busybox:1.36.1` as Docker Hub's registry answered on 2026-10-03.
pub const DEFAULT_DEBUG_IMAGE: &str = "docker.io/library/busybox:1.36.1@sha256:73aaf090f3d85aa34ee199857f03fa3a95c8ede2ffd4cc2cdb5b94e566b11662";

/// Every node shell pod is named with this prefix; the write path refuses to create or delete a
/// pod without it, so the cleanup bypass can never reach another pod.
pub(crate) const NODE_SHELL_PREFIX: &str = "k8sboard-node-shell-";
/// The label that marks a k8sBoard object, and the purpose label the leftover sweep selects on.
pub(crate) const MANAGED_BY_LABEL: &str = "app.kubernetes.io/managed-by";
pub(crate) const PURPOSE_LABEL: &str = "k8sboard.io/purpose";
pub(crate) const INSTANCE_LABEL: &str = "k8sboard.io/instance";
pub(crate) const NODE_LABEL: &str = "k8sboard.io/node";
const USER_ANNOTATION: &str = "k8sboard.io/user";
pub(crate) const MANAGED_BY: &str = "k8sboard";
pub(crate) const NODE_SHELL_PURPOSE: &str = "node-shell";
/// If the app dies before it attaches, the privileged process still ends after this long.
// ponytail: 4 h cap; make it a setting if long sessions need it.
const ACTIVE_DEADLINE_SECONDS: u32 = 14_400;
const MAX_NAME_LENGTH: usize = 63;
const MAX_IMAGE_LENGTH: usize = 255;
const SUFFIX_LENGTH: usize = 5;

/// Five characters from `[a-z0-9]`, from a randomly keyed hash of the clock: no dependency for a
/// name that only has to differ from the last one.
pub fn random_suffix() -> String {
    let mut value = RandomState::new().hash_one(Instant::now());
    (0..SUFFIX_LENGTH)
        .map(|_| {
            let digit = (value % 36) as u32;
            value /= 36;
            char::from_digit(digit, 36).unwrap_or('0')
        })
        .collect()
}

/// The name every debug container of k8sBoard starts with.
pub(crate) const DEBUG_CONTAINER_PREFIX: &str = "k8sboard-debug-";

/// `k8sboard-debug-{5}`: the name of a new debug container. A reconnect makes a new one.
pub fn debug_container_name() -> String {
    format!("{DEBUG_CONTAINER_PREFIX}{}", random_suffix())
}

/// The id of one app run, for the `k8sboard.io/instance` label: ten characters.
pub fn run_id() -> String {
    format!("{}{}", random_suffix(), random_suffix())
}

/// `k8sboard-node-shell-{node}-{suffix}`: a valid DNS label of at most 63 characters. The node is
/// lowercased, anything outside `[a-z0-9-]` becomes `-`, and it is cut so the whole name fits and
/// does not end in `-`.
pub fn node_shell_pod_name(node: &str, suffix: &str) -> String {
    let room = MAX_NAME_LENGTH - NODE_SHELL_PREFIX.len() - suffix.len() - 1;
    let node: String = node
        .chars()
        .map(|ch| match ch.to_ascii_lowercase() {
            ch if ch.is_ascii_lowercase() || ch.is_ascii_digit() => ch,
            _ => '-',
        })
        .take(room)
        .collect();
    let node = node.trim_matches('-');
    if node.is_empty() {
        format!("{NODE_SHELL_PREFIX}{suffix}")
    } else {
        format!("{NODE_SHELL_PREFIX}{node}-{suffix}")
    }
}

/// Non-empty, no whitespace, at most 255 characters: no registry parsing (air-gapped mirrors use
/// any host).
pub fn is_valid_debug_image(image: &str) -> bool {
    !image.is_empty()
        && image.len() <= MAX_IMAGE_LENGTH
        && !image.chars().any(char::is_whitespace)
        && !image.chars().any(char::is_control)
}

/// A name the server accepts as a container name.
pub(crate) fn is_container_name(name: &str) -> bool {
    is_dns_label(name)
}

/// A value the server accepts in a label: at most 63 characters of `[A-Za-z0-9-_.]`, starting and
/// ending alphanumeric.
pub(crate) fn is_label_value(value: &str) -> bool {
    let is_edge = |ch: char| ch.is_ascii_alphanumeric();
    value.len() <= MAX_NAME_LENGTH
        && value.chars().next().is_none_or(is_edge)
        && value.chars().next_back().is_none_or(is_edge)
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
}

/// The strategic merge patch that appends one ephemeral container (`spec.ephemeralContainers`
/// merges on `name`, so no read comes first). `stdinOnce` ends the shell when the attach ends.
pub(crate) fn debug_container_patch(name: &str, image: &str, target_container: &str) -> Value {
    json!({"spec": {"ephemeralContainers": [{
        "name": name,
        "image": image,
        "command": ["sh"],
        "stdin": true,
        "stdinOnce": true,
        "tty": true,
        "targetContainerName": target_container,
        "imagePullPolicy": "IfNotPresent",
        "terminationMessagePolicy": "File",
    }]}})
}

/// What a node shell pod is made of.
pub(crate) struct NodeShellPod<'a> {
    pub(crate) namespace: &'a str,
    pub(crate) name: &'a str,
    pub(crate) node: &'a str,
    pub(crate) image: &'a str,
    pub(crate) user: Option<&'a str>,
    pub(crate) instance: &'a str,
}

/// The privileged pod of a node shell: on the node, with the host's PID namespace, entering PID 1's
/// namespaces with `nsenter`. No service account token, no resources, and a 4 h deadline.
pub(crate) fn node_shell_pod(pod: &NodeShellPod<'_>) -> Value {
    let mut labels = json!({
        MANAGED_BY_LABEL: MANAGED_BY,
        PURPOSE_LABEL: NODE_SHELL_PURPOSE,
        INSTANCE_LABEL: pod.instance,
    });
    if is_label_value(pod.node) && !pod.node.is_empty() {
        labels[NODE_LABEL] = json!(pod.node);
    }
    let mut metadata = json!({
        "name": pod.name,
        "namespace": pod.namespace,
        "labels": labels,
    });
    if let Some(user) = pod.user {
        metadata["annotations"] = json!({ USER_ANNOTATION: user });
    }
    json!({
        "apiVersion": "v1",
        "kind": "Pod",
        "metadata": metadata,
        "spec": {
            "nodeName": pod.node,
            "hostPID": true,
            "restartPolicy": "Never",
            "terminationGracePeriodSeconds": 0,
            "activeDeadlineSeconds": ACTIVE_DEADLINE_SECONDS,
            "automountServiceAccountToken": false,
            "enableServiceLinks": false,
            "tolerations": [{"operator": "Exists"}],
            "containers": [{
                "name": "shell",
                "image": pod.image,
                "command": [
                    "nsenter", "-t", "1", "-m", "-u", "-i", "-n", "-p", "--", "sh", "-c",
                    AUTO_SCRIPT,
                ],
                "stdin": true,
                "stdinOnce": true,
                "tty": true,
                "securityContext": {"privileged": true},
                "imagePullPolicy": "IfNotPresent",
                "terminationMessagePolicy": "File",
            }],
        },
    })
}

#[cfg(test)]
#[path = "debug_pod_bodies_tests.rs"]
mod debug_pod_bodies_tests;
