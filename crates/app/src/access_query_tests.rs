use std::collections::HashSet;

use super::*;

fn resource(text: &str, namespace: Option<&str>) -> ResourceRequest {
    match parse_request(text, namespace)
        .expect("parses")
        .request
        .target
    {
        RequestTarget::Resource(resource) => resource,
        RequestTarget::NonResource { .. } => panic!("expected a resource request"),
    }
}

fn hint(text: &str, namespace: Option<&str>) -> Option<QueryHint> {
    parse_request(text, namespace).expect("parses").hint
}

fn rule(groups: &[&str], resources: &[&str], verbs: &[&str]) -> RbacRule {
    let texts = |values: &[&str]| values.iter().map(|value| (*value).to_owned()).collect();
    RbacRule {
        api_groups: texts(groups),
        resources: texts(resources),
        resource_names: Vec::new(),
        verbs: texts(verbs),
        non_resource_urls: Vec::new(),
    }
}

#[test]
fn trims_and_lowercases_verb() {
    let parsed = parse_request("  GET   pods  ", None).expect("parses");
    assert_eq!(parsed.request.verb, "get");
}

#[test]
fn empty_parts_are_errors() {
    for text in [
        "get .apps",
        "get pods/",
        "get pods.",
        "get deployments./log",
    ] {
        assert_eq!(
            parse_request(text, None),
            Err(QueryError::EmptyPart),
            "{text}"
        );
    }
}

#[test]
fn star_resource_means_star_group() {
    let parsed = resource("get *", None);
    assert_eq!(parsed.group, "*");
    assert_eq!(parsed.resource, "*");
    assert_eq!(hint("get *", None), None);
}

#[test]
fn parses_verb_and_resource() {
    let parsed = resource("list pods", Some("shop"));
    assert_eq!(parsed.resource, "pods");
    assert_eq!(parsed.namespace.as_deref(), Some("shop"));
    assert_eq!(parsed.name, None);
}

#[test]
fn parses_group_suffix() {
    let parsed = resource("get deployments.apps", Some("shop"));
    assert_eq!(parsed.resource, "deployments");
    assert_eq!(parsed.group, "apps");
    let dotted = resource("list pods.metrics.k8s.io", None);
    assert_eq!(dotted.resource, "pods");
    assert_eq!(dotted.group, "metrics.k8s.io");
    assert_eq!(hint("list pods.metrics.k8s.io", None), None);
}

#[test]
fn parses_subresource() {
    let parsed = resource("get pods/log", Some("shop"));
    assert_eq!(parsed.resource, "pods");
    assert_eq!(parsed.subresource.as_deref(), Some("log"));
    let grouped = resource("get deployments.apps/scale", Some("shop"));
    assert_eq!(grouped.group, "apps");
    assert_eq!(grouped.subresource.as_deref(), Some("scale"));
}

#[test]
fn parses_object_name() {
    let parsed = resource("get secrets db-password", Some("shop"));
    assert_eq!(parsed.name.as_deref(), Some("db-password"));
}

#[test]
fn infers_group_from_built_in_table() {
    assert_eq!(resource("get deployments", None).group, "apps");
    assert_eq!(resource("get cronjobs", None).group, "batch");
    assert_eq!(resource("get ingresses", None).group, "networking.k8s.io");
    assert_eq!(resource("get pods", None).group, "");
    assert_eq!(hint("get deployments", None), None);
}

#[test]
fn unknown_resource_assumes_core_with_hint() {
    let parsed = resource("get widgets", Some("shop"));
    assert_eq!(parsed.group, "");
    assert_eq!(parsed.namespace.as_deref(), Some("shop"));
    assert_eq!(
        hint("get widgets", Some("shop")),
        Some(QueryHint::AssumedCoreGroup)
    );
}

#[test]
fn cluster_scoped_drops_namespace_with_hint() {
    let parsed = resource("list nodes", Some("shop"));
    assert_eq!(parsed.namespace, None);
    assert_eq!(
        hint("list nodes", Some("shop")),
        Some(QueryHint::ClusterScoped)
    );
    // No namespace was picked, so nothing was ignored.
    assert_eq!(hint("list nodes", None), None);
}

#[test]
fn typed_group_must_match_the_built_in_kind() {
    let rbac = resource("get clusterroles.rbac.authorization.k8s.io", Some("shop"));
    assert_eq!(rbac.namespace, None);
    // The metrics group has a `pods` of its own: not the core kind, so the namespace stays.
    let metrics = resource("get pods.metrics.k8s.io", Some("shop"));
    assert_eq!(metrics.namespace.as_deref(), Some("shop"));
}

#[test]
fn parses_non_resource_url() {
    let parsed = parse_request("get /healthz", Some("shop")).expect("parses");
    assert_eq!(parsed.hint, None);
    assert_eq!(
        parsed.request.target,
        RequestTarget::NonResource {
            path: "/healthz".to_owned()
        }
    );
}

#[test]
fn url_with_name_is_error() {
    assert_eq!(
        parse_request("get /healthz extra", None),
        Err(QueryError::NameWithUrl)
    );
}

#[test]
fn empty_and_missing_target_errors() {
    assert_eq!(parse_request("", None), Err(QueryError::Empty));
    assert_eq!(parse_request("   ", None), Err(QueryError::Empty));
    assert_eq!(parse_request("get", None), Err(QueryError::MissingTarget));
}

#[test]
fn too_many_words_is_error() {
    assert_eq!(
        parse_request("get pods web extra", None),
        Err(QueryError::TooManyWords)
    );
}

#[test]
fn wildcards_pass_through() {
    let parsed = parse_request("* *", None).expect("parses");
    assert_eq!(parsed.request.verb, "*");
    let RequestTarget::Resource(wildcard) = parsed.request.target else {
        panic!("resource request");
    };
    assert_eq!(
        (wildcard.resource.as_str(), wildcard.group.as_str()),
        ("*", "*")
    );
    assert_eq!(resource("get *.apps", None).group, "apps");
}

#[test]
fn built_in_table_has_unique_plurals() {
    let plurals: HashSet<_> = BUILT_IN_RESOURCES
        .iter()
        .map(|(plural, ..)| *plural)
        .collect();
    assert_eq!(plurals.len(), BUILT_IN_RESOURCES.len());
}

#[test]
fn who_can_prefill_uses_first_resource_rule() {
    let rules = [
        rule(&[""], &["secrets", "pods"], &["get", "list"]),
        rule(&["apps"], &["deployments"], &["delete"]),
    ];
    assert_eq!(who_can_prefill(&rules).as_deref(), Some("get secrets"));
    let grouped = [rule(&["apps"], &["deployments"], &["delete"])];
    assert_eq!(
        who_can_prefill(&grouped).as_deref(),
        Some("delete deployments.apps")
    );
}

#[test]
fn who_can_prefill_skips_url_rules() {
    let url_rule = RbacRule {
        non_resource_urls: vec!["/healthz".to_owned()],
        ..rule(&[], &[], &["get"])
    };
    let rules = [url_rule, rule(&[""], &["nodes"], &["list"])];
    assert_eq!(who_can_prefill(&rules).as_deref(), Some("list nodes"));
    assert_eq!(who_can_prefill(&[]), None);
}

#[test]
fn short_names_resolve_to_built_in_kinds() {
    let cases = [
        ("po", "pods", ""),
        ("svc", "services", ""),
        ("deploy", "deployments", "apps"),
        ("cm", "configmaps", ""),
        ("ns", "namespaces", ""),
        ("sa", "serviceaccounts", ""),
        ("no", "nodes", ""),
        ("pv", "persistentvolumes", ""),
        ("pvc", "persistentvolumeclaims", ""),
        ("ds", "daemonsets", "apps"),
        ("sts", "statefulsets", "apps"),
        ("rs", "replicasets", "apps"),
        ("cj", "cronjobs", "batch"),
        ("ing", "ingresses", "networking.k8s.io"),
        ("netpol", "networkpolicies", "networking.k8s.io"),
        ("crd", "customresourcedefinitions", "apiextensions.k8s.io"),
        ("sc", "storageclasses", "storage.k8s.io"),
        ("pdb", "poddisruptionbudgets", "policy"),
        ("hpa", "horizontalpodautoscalers", "autoscaling"),
        ("ep", "endpoints", ""),
        ("ev", "events", ""),
    ];
    for (short, plural, group) in cases {
        let parsed = resource(&format!("get {short}"), None);
        assert_eq!(
            (parsed.resource.as_str(), parsed.group.as_str()),
            (plural, group),
            "{short}"
        );
        assert_eq!(hint(&format!("get {short}"), None), None, "{short}");
    }
}

#[test]
fn singular_forms_resolve_to_plurals() {
    let cases = [
        ("pod", "pods"),
        ("secret", "secrets"),
        ("deployment", "deployments"),
        ("ingress", "ingresses"),
        ("networkpolicy", "networkpolicies"),
        ("storageclass", "storageclasses"),
        ("endpoint", "endpoints"),
        ("clusterrolebinding", "clusterrolebindings"),
    ];
    for (singular, plural) in cases {
        assert_eq!(
            resource(&format!("get {singular}"), None).resource,
            plural,
            "{singular}"
        );
    }
}

#[test]
fn unknown_singular_is_left_as_typed() {
    let parsed = resource("get widget", None);
    assert_eq!(parsed.resource, "widget");
    assert_eq!(hint("get widget", None), Some(QueryHint::AssumedCoreGroup));
}

#[test]
fn resource_group_and_subresource_are_lowercased_but_names_are_kept() {
    let parsed = resource("get Pods/Log Web-1", Some("shop"));
    assert_eq!(parsed.resource, "pods");
    assert_eq!(parsed.subresource.as_deref(), Some("log"));
    assert_eq!(parsed.name.as_deref(), Some("Web-1"));
    assert_eq!(resource("get Deployments.Apps", None).group, "apps");
}

#[test]
fn unknown_subresource_gets_a_hint() {
    assert_eq!(
        hint("get pods/logs", Some("shop")),
        Some(QueryHint::UnknownSubresource)
    );
    assert_eq!(hint("get pods/log", Some("shop")), None);
    assert_eq!(hint("get deployments/scale", Some("shop")), None);
}

#[test]
fn bindings_and_component_statuses_are_built_in() {
    let component = resource("get componentstatuses", Some("shop"));
    assert_eq!(component.namespace, None);
    let binding = resource("create bindings", Some("shop"));
    assert_eq!(binding.group, "");
    assert_eq!(binding.namespace.as_deref(), Some("shop"));
}

#[test]
fn who_can_prefill_puts_the_group_before_the_subresource() {
    let rules = [rule(&["apps"], &["deployments/scale"], &["get"])];
    assert_eq!(
        who_can_prefill(&rules).as_deref(),
        Some("get deployments.apps/scale")
    );
    let core = [rule(&[""], &["pods/log"], &["get"])];
    assert_eq!(who_can_prefill(&core).as_deref(), Some("get pods/log"));
}
