use super::*;
use crate::rbac_snapshot::{NamespaceCoverage, RbacCoverage};

fn texts(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

fn rule(groups: &[&str], resources: &[&str], verbs: &[&str]) -> RbacRule {
    RbacRule {
        api_groups: texts(groups),
        resources: texts(resources),
        resource_names: Vec::new(),
        verbs: texts(verbs),
        non_resource_urls: Vec::new(),
    }
}

fn named(mut rule: RbacRule, names: &[&str]) -> RbacRule {
    rule.resource_names = texts(names);
    rule
}

fn url_rule(urls: &[&str], verbs: &[&str]) -> RbacRule {
    RbacRule {
        api_groups: Vec::new(),
        resources: Vec::new(),
        resource_names: Vec::new(),
        verbs: texts(verbs),
        non_resource_urls: texts(urls),
    }
}

fn resource_request(
    verb: &str,
    group: &str,
    resource: &str,
    subresource: Option<&str>,
    name: Option<&str>,
    namespace: Option<&str>,
) -> AccessRequest {
    AccessRequest {
        verb: verb.to_owned(),
        target: RequestTarget::Resource(ResourceRequest {
            group: group.to_owned(),
            resource: resource.to_owned(),
            subresource: subresource.map(str::to_owned),
            name: name.map(str::to_owned),
            namespace: namespace.map(str::to_owned),
        }),
    }
}

fn get_in(group: &str, resource: &str, namespace: Option<&str>) -> AccessRequest {
    resource_request("get", group, resource, None, None, namespace)
}

fn url_request(verb: &str, path: &str) -> AccessRequest {
    AccessRequest {
        verb: verb.to_owned(),
        target: RequestTarget::NonResource {
            path: path.to_owned(),
        },
    }
}

fn role(namespace: Option<&str>, name: &str, rules: Vec<RbacRule>) -> RoleSummary {
    RoleSummary {
        namespace: namespace.map(str::to_owned),
        name: name.to_owned(),
        created_at: None,
        labels: Vec::new(),
        rules,
        aggregation: Vec::new(),
    }
}

fn binding(
    namespace: Option<&str>,
    name: &str,
    kind: RoleKind,
    role_name: &str,
    subjects: Vec<Subject>,
) -> BindingSummary {
    BindingSummary {
        namespace: namespace.map(str::to_owned),
        name: name.to_owned(),
        created_at: None,
        labels: Vec::new(),
        role: crate::role_binding::RoleRef {
            kind,
            name: role_name.to_owned(),
        },
        subjects,
    }
}

fn user(name: &str) -> Subject {
    Subject {
        kind: SubjectKind::User,
        name: name.to_owned(),
        namespace: None,
    }
}

fn group(name: &str) -> Subject {
    Subject {
        kind: SubjectKind::Group,
        name: name.to_owned(),
        namespace: None,
    }
}

fn account(namespace: &str, name: &str) -> Subject {
    Subject {
        kind: SubjectKind::ServiceAccount,
        name: name.to_owned(),
        namespace: Some(namespace.to_owned()),
    }
}

fn snapshot(
    roles: Vec<RoleSummary>,
    cluster_roles: Vec<RoleSummary>,
    role_bindings: Vec<BindingSummary>,
    cluster_role_bindings: Vec<BindingSummary>,
) -> RbacSnapshot {
    RbacSnapshot {
        roles,
        cluster_roles,
        role_bindings,
        cluster_role_bindings,
        coverage: RbacCoverage {
            cluster_roles: true,
            cluster_bindings: true,
            roles: NamespaceCoverage::AllNamespaces,
            role_bindings: NamespaceCoverage::AllNamespaces,
        },
    }
}

/// One ClusterRole `reader` (get pods) bound by `binding_namespace` to `subjects`.
fn reader_snapshot(binding_namespace: Option<&str>, subjects: Vec<Subject>) -> RbacSnapshot {
    let reader = role(None, "reader", vec![rule(&[""], &["pods"], &["get"])]);
    let bound = binding(
        binding_namespace,
        "b",
        RoleKind::ClusterRole,
        "reader",
        subjects,
    );
    match binding_namespace {
        Some(_) => snapshot(Vec::new(), vec![reader], vec![bound], Vec::new()),
        None => snapshot(Vec::new(), vec![reader], Vec::new(), vec![bound]),
    }
}

fn matches(rule: &RbacRule, request: &AccessRequest) -> bool {
    rule_match(rule, request).is_some()
}

fn subject_names(grants: &[Grant<'_>]) -> Vec<String> {
    grants
        .iter()
        .map(|grant| grant.subject.name.clone())
        .collect()
}

// Rule matching.

#[test]
fn verb_wildcard_matches_any_verb() {
    let any = rule(&[""], &["pods"], &["*"]);
    for verb in ["get", "delete", "deletecollection"] {
        let request = resource_request(verb, "", "pods", None, None, Some("a"));
        assert!(matches(&any, &request), "{verb}");
    }
}

#[test]
fn verb_must_match_exactly() {
    let list = rule(&[""], &["pods"], &["list", "watch"]);
    assert!(matches(
        &list,
        &resource_request("list", "", "pods", None, None, None)
    ));
    assert!(!matches(
        &list,
        &resource_request("get", "", "pods", None, None, None)
    ));
}

#[test]
fn verb_prefix_does_not_match() {
    let list = rule(&[""], &["pods"], &["list"]);
    assert!(!matches(
        &list,
        &resource_request("lis", "", "pods", None, None, None)
    ));
}

#[test]
fn group_wildcard_and_exact() {
    let apps = rule(&["apps"], &["deployments"], &["get"]);
    assert!(matches(&apps, &get_in("apps", "deployments", None)));
    assert!(!matches(&apps, &get_in("batch", "deployments", None)));
}

#[test]
fn group_wildcard_matches_any_group() {
    let any = rule(&["*"], &["deployments"], &["get"]);
    assert!(matches(&any, &get_in("batch", "deployments", None)));
}

#[test]
fn core_group_is_empty_string() {
    let core = rule(&[""], &["pods"], &["get"]);
    assert!(matches(&core, &get_in("", "pods", None)));
    assert!(!matches(&core, &get_in("metrics.k8s.io", "pods", None)));
}

#[test]
fn resource_wildcard_matches_subresources_too() {
    let everything = rule(&["*"], &["*"], &["get"]);
    let log = resource_request("get", "", "pods", Some("log"), None, None);
    assert!(matches(&everything, &log));
}

#[test]
fn resource_wildcard_matches_any_resource() {
    let everything = rule(&["*"], &["*"], &["get"]);
    assert!(matches(&everything, &get_in("", "nodes", None)));
}

#[test]
fn empty_subresource_is_no_subresource() {
    let pods = rule(&[""], &["pods"], &["get"]);
    let request = resource_request("get", "", "pods", Some(""), None, None);
    assert!(matches(&pods, &request));
    let logs = rule(&[""], &["pods/log"], &["get"]);
    assert!(!matches(&logs, &request));
}

#[test]
fn plain_resource_does_not_match_subresource() {
    let pods = rule(&[""], &["pods"], &["get"]);
    let log = resource_request("get", "", "pods", Some("log"), None, None);
    assert!(!matches(&pods, &log));
}

#[test]
fn subresource_matches_exactly() {
    let logs = rule(&[""], &["pods/log"], &["get"]);
    let log = resource_request("get", "", "pods", Some("log"), None, None);
    let exec = resource_request("get", "", "pods", Some("exec"), None, None);
    assert!(matches(&logs, &log));
    assert!(!matches(&logs, &exec));
}

#[test]
fn subresource_rule_does_not_match_plain_resource() {
    let logs = rule(&[""], &["pods/log"], &["get"]);
    assert!(!matches(&logs, &get_in("", "pods", None)));
}

#[test]
fn star_slash_subresource_matches_any_resource() {
    let any_log = rule(&[""], &["*/log"], &["get"]);
    let pod_log = resource_request("get", "", "pods", Some("log"), None, None);
    let node_log = resource_request("get", "", "nodes", Some("log"), None, None);
    let pod_exec = resource_request("get", "", "pods", Some("exec"), None, None);
    assert!(matches(&any_log, &pod_log));
    assert!(matches(&any_log, &node_log));
    assert!(!matches(&any_log, &pod_exec));
}

#[test]
fn star_slash_subresource_does_not_match_plain_resource() {
    let any_log = rule(&[""], &["*/log"], &["get"]);
    assert!(!matches(&any_log, &get_in("", "pods", None)));
}

#[test]
fn resource_slash_star_is_not_a_wildcard() {
    let pods_star = rule(&[""], &["pods/*"], &["get"]);
    let log = resource_request("get", "", "pods", Some("log"), None, None);
    assert!(!matches(&pods_star, &log));
}

#[test]
fn resource_slash_star_does_not_match_plain_resource() {
    let pods_star = rule(&[""], &["pods/*"], &["get"]);
    assert!(!matches(&pods_star, &get_in("", "pods", None)));
}

#[test]
fn resource_names_any_when_empty() {
    let pods = rule(&[""], &["pods"], &["get"]);
    assert_eq!(
        rule_match(&pods, &get_in("", "pods", None)),
        Some(GrantNames::Any)
    );
}

#[test]
fn empty_resource_names_match_named_request() {
    let pods = rule(&[""], &["pods"], &["get"]);
    let named_request = resource_request("get", "", "pods", None, Some("web"), None);
    assert_eq!(rule_match(&pods, &named_request), Some(GrantNames::Any));
}

#[test]
fn resource_names_match_named_request() {
    let pods = named(rule(&[""], &["pods"], &["get"]), &["web", "db"]);
    let request = resource_request("get", "", "pods", None, Some("web"), None);
    assert_eq!(rule_match(&pods, &request), Some(GrantNames::Any));
}

#[test]
fn resource_names_other_name_no_match() {
    let pods = named(rule(&[""], &["pods"], &["get"]), &["web"]);
    let request = resource_request("get", "", "pods", None, Some("other"), None);
    assert_eq!(rule_match(&pods, &request), None);
}

#[test]
fn resource_names_without_request_name_is_only() {
    let pods = named(rule(&[""], &["pods"], &["get"]), &["web", "db"]);
    assert_eq!(
        rule_match(&pods, &get_in("", "pods", None)),
        Some(GrantNames::Only(texts(&["web", "db"])))
    );
}

#[test]
fn non_resource_exact_star_and_prefix() {
    let cases = [
        ("/healthz", "/healthz", true),
        ("/healthz", "/livez", false),
        ("*", "/anything", true),
        ("/apis/*", "/apis/apps", true),
        ("/apis/*", "/apis", false),
        ("/apis/**", "/apis/apps", true),
    ];
    for (url, path, expected) in cases {
        let url_grant = url_rule(&[url], &["get"]);
        assert_eq!(
            matches(&url_grant, &url_request("get", path)),
            expected,
            "{url} vs {path}"
        );
    }
}

#[test]
fn non_resource_verb_must_match() {
    assert!(!matches(
        &url_rule(&["/healthz"], &["get"]),
        &url_request("post", "/healthz")
    ));
}

#[test]
fn non_resource_rule_never_matches_resource_request() {
    let urls = url_rule(&["*"], &["*"]);
    assert!(!matches(&urls, &get_in("", "pods", None)));
}

#[test]
fn resource_rule_never_matches_non_resource_request() {
    let everything = rule(&["*"], &["*"], &["*"]);
    assert!(!matches(&everything, &url_request("get", "/healthz")));
}

// Who can.

#[test]
fn cluster_role_binding_grants_in_every_namespace() {
    let data = reader_snapshot(None, vec![user("ann")]);
    for namespace in [Some("a"), Some("b"), None] {
        let grants = data.who_can(&get_in("", "pods", namespace));
        assert_eq!(subject_names(&grants), ["ann"], "{namespace:?}");
    }
}

#[test]
fn role_binding_grants_only_in_its_namespace() {
    let data = reader_snapshot(Some("a"), vec![user("ann")]);
    assert_eq!(data.who_can(&get_in("", "pods", Some("a"))).len(), 1);
    assert!(data.who_can(&get_in("", "pods", Some("b"))).is_empty());
}

#[test]
fn role_binding_never_grants_cluster_wide_request() {
    let data = reader_snapshot(Some("a"), vec![user("ann")]);
    assert!(data.who_can(&get_in("", "pods", None)).is_empty());
}

#[test]
fn role_binding_to_cluster_role_scopes_rules_to_namespace() {
    let both = role(
        None,
        "both",
        vec![rule(&[""], &["pods", "nodes"], &["get"])],
    );
    let bound = binding(
        Some("a"),
        "b",
        RoleKind::ClusterRole,
        "both",
        vec![user("ann")],
    );
    let data = snapshot(Vec::new(), vec![both], vec![bound], Vec::new());
    assert_eq!(data.who_can(&get_in("", "pods", Some("a"))).len(), 1);
    assert!(data.who_can(&get_in("", "pods", Some("b"))).is_empty());
}

#[test]
fn role_binding_to_cluster_role_misses_cluster_scoped_request() {
    let nodes = role(None, "nodes", vec![rule(&[""], &["nodes"], &["get"])]);
    let bound = binding(
        Some("a"),
        "b",
        RoleKind::ClusterRole,
        "nodes",
        vec![user("ann")],
    );
    let data = snapshot(Vec::new(), vec![nodes], vec![bound], Vec::new());
    // A cluster-scoped request carries no namespace, so the RoleBinding never reaches it.
    assert!(data.who_can(&get_in("", "nodes", None)).is_empty());
}

#[test]
fn role_binding_ignores_non_resource_rules() {
    let health = role(None, "health", vec![url_rule(&["/healthz"], &["get"])]);
    let bound = binding(
        Some("a"),
        "b",
        RoleKind::ClusterRole,
        "health",
        vec![user("ann")],
    );
    let data = snapshot(Vec::new(), vec![health], vec![bound], Vec::new());
    assert!(data.who_can(&url_request("get", "/healthz")).is_empty());
}

#[test]
fn role_ref_resolves_role_in_binding_namespace() {
    let in_a = role(Some("a"), "r", vec![rule(&[""], &["pods"], &["get"])]);
    let in_b = role(Some("b"), "r", vec![rule(&[""], &["secrets"], &["get"])]);
    let bound = binding(Some("b"), "x", RoleKind::Role, "r", vec![user("ann")]);
    let data = snapshot(vec![in_a, in_b], Vec::new(), vec![bound], Vec::new());
    assert!(data.who_can(&get_in("", "pods", Some("b"))).is_empty());
    assert_eq!(data.who_can(&get_in("", "secrets", Some("b"))).len(), 1);
}

#[test]
fn missing_role_grants_nothing() {
    let bound = binding(None, "x", RoleKind::ClusterRole, "gone", vec![user("ann")]);
    let data = snapshot(Vec::new(), Vec::new(), Vec::new(), vec![bound]);
    assert!(data.who_can(&get_in("", "pods", None)).is_empty());
}

#[test]
fn other_role_kind_grants_nothing() {
    let reader = role(None, "reader", vec![rule(&[""], &["pods"], &["get"])]);
    let bound = binding(
        None,
        "x",
        RoleKind::Other("Strange".to_owned()),
        "reader",
        vec![user("ann")],
    );
    let data = snapshot(Vec::new(), vec![reader], Vec::new(), vec![bound]);
    assert!(data.who_can(&get_in("", "pods", None)).is_empty());
}

#[test]
fn aggregated_cluster_role_uses_listed_rules() {
    let mut aggregated = role(None, "agg", vec![rule(&[""], &["pods"], &["get"])]);
    aggregated.aggregation = vec![crate::Selector::everything()];
    let bound = binding(None, "x", RoleKind::ClusterRole, "agg", vec![user("ann")]);
    let data = snapshot(Vec::new(), vec![aggregated], Vec::new(), vec![bound]);
    assert_eq!(data.who_can(&get_in("", "pods", None)).len(), 1);
    assert!(data.who_can(&get_in("", "secrets", None)).is_empty());
}

#[test]
fn one_grant_per_subject_and_binding() {
    let data = reader_snapshot(
        None,
        vec![user("ann"), group("devs"), account("a", "robot")],
    );
    let grants = data.who_can(&get_in("", "pods", None));
    assert_eq!(subject_names(&grants), ["ann", "devs", "robot"]);
    assert!(grants.iter().all(|grant| grant.binding.name == "b"));
}

#[test]
fn several_rules_merge_names_any_wins() {
    let merged = role(
        None,
        "r",
        vec![
            named(rule(&[""], &["pods"], &["get"]), &["web"]),
            rule(&[""], &["pods"], &["get"]),
        ],
    );
    let bound = binding(None, "x", RoleKind::ClusterRole, "r", vec![user("ann")]);
    let data = snapshot(Vec::new(), vec![merged], Vec::new(), vec![bound]);
    let grants = data.who_can(&get_in("", "pods", None));
    assert_eq!(grants.len(), 1);
    assert_eq!(grants[0].names, GrantNames::Any);
}

#[test]
fn only_names_union_sorted() {
    let merged = role(
        None,
        "r",
        vec![
            named(rule(&[""], &["pods"], &["get"]), &["web", "db"]),
            named(rule(&[""], &["pods"], &["*"]), &["cache", "db"]),
        ],
    );
    let bound = binding(None, "x", RoleKind::ClusterRole, "r", vec![user("ann")]);
    let data = snapshot(Vec::new(), vec![merged], Vec::new(), vec![bound]);
    let grants = data.who_can(&get_in("", "pods", None));
    assert_eq!(
        grants[0].names,
        GrantNames::Only(texts(&["cache", "db", "web"]))
    );
}

#[test]
fn grant_order_cluster_bindings_first() {
    let reader = role(None, "reader", vec![rule(&[""], &["pods"], &["get"])]);
    let namespaced = binding(
        Some("a"),
        "rb",
        RoleKind::ClusterRole,
        "reader",
        vec![user("second")],
    );
    let cluster = binding(
        None,
        "crb",
        RoleKind::ClusterRole,
        "reader",
        vec![user("first")],
    );
    let data = snapshot(Vec::new(), vec![reader], vec![namespaced], vec![cluster]);
    let grants = data.who_can(&get_in("", "pods", Some("a")));
    assert_eq!(subject_names(&grants), ["first", "second"]);
}

// Identity.

#[test]
fn service_account_identity_user_and_groups() {
    let identity = Identity::service_account("shop", "robot");
    assert_eq!(
        identity.user.as_deref(),
        Some("system:serviceaccount:shop:robot")
    );
    assert_eq!(
        identity.groups,
        [
            "system:serviceaccounts",
            "system:serviceaccounts:shop",
            "system:authenticated"
        ]
    );
}

#[test]
fn service_account_matches_direct_subject() {
    let identity = Identity::service_account("shop", "robot");
    assert!(identity.applies_to(&account("shop", "robot")));
    assert!(!identity.applies_to(&account("shop", "other")));
}

#[test]
fn service_account_ignores_same_name_in_other_namespace() {
    let identity = Identity::service_account("shop", "robot");
    assert!(!identity.applies_to(&account("other", "robot")));
}

#[test]
fn service_account_ignores_subject_without_namespace() {
    let identity = Identity::service_account("shop", "robot");
    let no_namespace = Subject {
        namespace: None,
        ..account("shop", "robot")
    };
    assert!(!identity.applies_to(&no_namespace));
}

#[test]
fn service_account_matches_service_account_groups() {
    let identity = Identity::service_account("shop", "robot");
    assert!(identity.applies_to(&group("system:serviceaccounts")));
    assert!(identity.applies_to(&group("system:serviceaccounts:shop")));
}

#[test]
fn service_account_matches_authenticated_group() {
    let identity = Identity::service_account("shop", "robot");
    assert!(identity.applies_to(&group("system:authenticated")));
}

#[test]
fn service_account_ignores_other_namespace_group() {
    let identity = Identity::service_account("shop", "robot");
    assert!(!identity.applies_to(&group("system:serviceaccounts:other")));
}

#[test]
fn unauthenticated_group_never_applies_to_service_account() {
    let identity = Identity::service_account("shop", "robot");
    assert!(!identity.applies_to(&group("system:unauthenticated")));
}

#[test]
fn user_identity_matches_name_and_authenticated() {
    let identity = Identity::user("ann");
    assert!(identity.applies_to(&user("ann")));
    assert!(identity.applies_to(&group("system:authenticated")));
    assert!(!identity.applies_to(&user("bob")));
}

#[test]
fn user_identity_ignores_groups_it_cannot_know() {
    let identity = Identity::user("ann");
    assert!(!identity.applies_to(&group("system:masters")));
}

#[test]
fn anonymous_user_gets_unauthenticated_group() {
    let identity = Identity::user("system:anonymous");
    assert!(identity.applies_to(&group("system:unauthenticated")));
    assert!(identity.applies_to(&user("system:anonymous")));
}

#[test]
fn anonymous_user_is_not_authenticated() {
    let identity = Identity::user("system:anonymous");
    assert!(!identity.applies_to(&group("system:authenticated")));
}

#[test]
fn group_identity_matches_only_group() {
    let identity = Identity::group("devs");
    assert!(identity.applies_to(&group("devs")));
    assert!(!identity.applies_to(&group("system:authenticated")));
}

#[test]
fn group_identity_has_no_user_name() {
    let identity = Identity::group("devs");
    assert!(!identity.applies_to(&user("devs")));
}

// Rules of, decide.

fn robot_snapshot() -> RbacSnapshot {
    let reader = role(None, "reader", vec![rule(&[""], &["pods"], &["get"])]);
    let local = role(
        Some("a"),
        "local",
        vec![rule(&[""], &["secrets"], &["list"])],
    );
    let health = role(None, "health", vec![url_rule(&["/healthz"], &["get"])]);
    let cluster_binding = binding(
        None,
        "crb",
        RoleKind::ClusterRole,
        "reader",
        vec![group("system:serviceaccounts")],
    );
    let namespaced = binding(
        Some("a"),
        "rb",
        RoleKind::Role,
        "local",
        vec![account("a", "robot")],
    );
    let other_namespace = binding(
        Some("b"),
        "rb-b",
        RoleKind::Role,
        "local",
        vec![account("a", "robot")],
    );
    let with_health = binding(
        Some("a"),
        "rb-health",
        RoleKind::ClusterRole,
        "health",
        vec![account("a", "robot")],
    );
    snapshot(
        vec![local],
        vec![reader, health],
        vec![namespaced, other_namespace, with_health],
        vec![cluster_binding],
    )
}

#[test]
fn rules_of_namespace_includes_cluster_and_namespace_bindings() {
    let data = robot_snapshot();
    let rules = data.rules_of(&Identity::service_account("a", "robot"), Some("a"));
    let names: Vec<_> = rules
        .iter()
        .map(|effective| effective.binding.name.as_str())
        .collect();
    assert_eq!(names, ["crb", "rb"]);
}

#[test]
fn rules_of_cluster_wide_uses_cluster_bindings_only() {
    let data = robot_snapshot();
    let rules = data.rules_of(&Identity::service_account("a", "robot"), None);
    let names: Vec<_> = rules
        .iter()
        .map(|effective| effective.binding.name.as_str())
        .collect();
    assert_eq!(names, ["crb"]);
}

#[test]
fn rules_of_subject_follows_binding_order() {
    let reader = role(None, "reader", vec![rule(&[""], &["pods"], &["get"])]);
    let bound = binding(
        None,
        "crb",
        RoleKind::ClusterRole,
        "reader",
        vec![
            user("someone-else"),
            group("system:authenticated"),
            account("a", "robot"),
        ],
    );
    let data = snapshot(Vec::new(), vec![reader], Vec::new(), vec![bound]);
    let rules = data.rules_of(&Identity::service_account("a", "robot"), None);
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].subject.name, "system:authenticated");
}

#[test]
fn rules_of_drops_non_resource_rules_from_role_bindings() {
    let data = robot_snapshot();
    let rules = data.rules_of(&Identity::service_account("a", "robot"), Some("a"));
    assert!(
        rules
            .iter()
            .all(|effective| effective.binding.name != "rb-health")
    );
}

#[test]
fn rules_of_keeps_non_resource_rules_from_cluster_bindings() {
    let health = role(None, "health", vec![url_rule(&["/healthz"], &["get"])]);
    let bound = binding(
        None,
        "crb",
        RoleKind::ClusterRole,
        "health",
        vec![user("ann")],
    );
    let cluster_wide = snapshot(Vec::new(), vec![health], Vec::new(), vec![bound]);
    assert_eq!(cluster_wide.rules_of(&Identity::user("ann"), None).len(), 1);
}

#[test]
fn decide_allows_through_group() {
    let data = reader_snapshot(None, vec![group("devs")]);
    let grants = data.decide(&Identity::group("devs"), &get_in("", "pods", None));
    assert_eq!(subject_names(&grants), ["devs"]);
}

fn only_names_snapshot() -> RbacSnapshot {
    let narrow = role(
        None,
        "r",
        vec![named(rule(&[""], &["pods"], &["get"]), &["web"])],
    );
    let bound = binding(None, "x", RoleKind::ClusterRole, "r", vec![user("ann")]);
    snapshot(Vec::new(), vec![narrow], Vec::new(), vec![bound])
}

#[test]
fn decide_ignores_only_names_grants() {
    let nameless = get_in("", "pods", None);
    assert!(
        only_names_snapshot()
            .decide(&Identity::user("ann"), &nameless)
            .is_empty()
    );
}

#[test]
fn decide_allows_named_request_for_named_rule() {
    let named_request = resource_request("get", "", "pods", None, Some("web"), None);
    let data = only_names_snapshot();
    assert_eq!(data.decide(&Identity::user("ann"), &named_request).len(), 1);
}

#[test]
fn decide_empty_when_nothing_grants() {
    let data = reader_snapshot(None, vec![user("ann")]);
    assert!(
        data.decide(&Identity::user("bob"), &get_in("", "pods", None))
            .is_empty()
    );
}

#[test]
fn decide_empty_for_unlisted_resource() {
    let data = reader_snapshot(None, vec![user("ann")]);
    assert!(
        data.decide(&Identity::user("ann"), &get_in("", "secrets", None))
            .is_empty()
    );
}
