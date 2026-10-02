use cluster::{RbacRule, ResourceRequest, RoleKind, RoleRef, RoleSummary, Subject, SubjectKind};

use super::*;

fn rule(resources: &[&str], names: &[&str]) -> RbacRule {
    RbacRule {
        api_groups: vec![String::new()],
        resources: resources.iter().map(|value| (*value).to_owned()).collect(),
        resource_names: names.iter().map(|value| (*value).to_owned()).collect(),
        verbs: vec!["get".to_owned()],
        non_resource_urls: Vec::new(),
    }
}

fn cluster_role(name: &str, rules: Vec<RbacRule>) -> RoleSummary {
    RoleSummary {
        namespace: None,
        name: name.to_owned(),
        created_at: None,
        labels: Vec::new(),
        rules,
        aggregation: Vec::new(),
    }
}

fn subject(kind: SubjectKind, name: &str, namespace: Option<&str>) -> Subject {
    Subject {
        kind,
        name: name.to_owned(),
        namespace: namespace.map(str::to_owned),
    }
}

fn binding(name: &str, role: &str, subjects: Vec<Subject>) -> BindingSummary {
    BindingSummary {
        namespace: None,
        name: name.to_owned(),
        created_at: None,
        labels: Vec::new(),
        role: RoleRef {
            kind: RoleKind::ClusterRole,
            name: role.to_owned(),
        },
        subjects,
    }
}

fn coverage() -> RbacCoverage {
    RbacCoverage {
        cluster_roles: true,
        cluster_bindings: true,
        roles: NamespaceCoverage::AllNamespaces,
        role_bindings: NamespaceCoverage::AllNamespaces,
    }
}

fn snapshot(roles: Vec<RoleSummary>, bindings: Vec<BindingSummary>) -> RbacSnapshot {
    RbacSnapshot {
        roles: Vec::new(),
        cluster_roles: roles,
        role_bindings: Vec::new(),
        cluster_role_bindings: bindings,
        coverage: coverage(),
    }
}

fn get_pods() -> AccessRequest {
    AccessRequest {
        verb: "get".to_owned(),
        target: RequestTarget::Resource(ResourceRequest {
            group: String::new(),
            resource: "pods".to_owned(),
            subresource: None,
            name: None,
            namespace: Some("shop".to_owned()),
        }),
    }
}

fn groups_of(snapshot: &RbacSnapshot) -> WhoCanGroups {
    who_can_groups(&snapshot.who_can(&get_pods()))
}

fn texts(groups: &[SubjectGroup]) -> Vec<&str> {
    groups.iter().map(|group| group.text.as_str()).collect()
}

#[test]
fn groups_grants_by_subject() {
    let ann = subject(SubjectKind::User, "ann", None);
    let data = snapshot(
        vec![cluster_role("reader", vec![rule(&["pods"], &[])])],
        vec![
            binding("first", "reader", vec![ann.clone()]),
            binding("second", "reader", vec![ann]),
        ],
    );
    let groups = groups_of(&data);
    let [group] = groups.full.as_slice() else {
        panic!("one subject");
    };
    assert_eq!(group.text, "user ann");
    let bindings: Vec<_> = group
        .lines
        .iter()
        .map(|line| line.binding_text.as_str())
        .collect();
    assert_eq!(
        bindings,
        ["clusterrolebinding/first", "clusterrolebinding/second"]
    );
    assert_eq!(group.lines[0].role_text, "clusterrole/reader");
    assert_eq!(group.lines[0].scope, "cluster-wide");
}

#[test]
fn orders_broad_groups_users_groups_accounts() {
    let subjects = vec![
        subject(SubjectKind::ServiceAccount, "robot", Some("shop")),
        subject(SubjectKind::Group, "devs", None),
        subject(SubjectKind::User, "zed", None),
        subject(SubjectKind::User, "ann", None),
        subject(SubjectKind::Group, "system:authenticated", None),
    ];
    let data = snapshot(
        vec![cluster_role("reader", vec![rule(&["pods"], &[])])],
        vec![binding("b", "reader", subjects)],
    );
    let groups = groups_of(&data);
    assert_eq!(
        texts(&groups.full),
        [
            "group system:authenticated",
            "user ann",
            "user zed",
            "group devs",
            "sa shop/robot"
        ]
    );
    assert_eq!(groups.full[0].tone, Some(StatusTone::Bad));
    assert_eq!(
        groups.full[4].account,
        Some(ResourceKey::Kind {
            kind: ResourceKind::ServiceAccounts,
            namespace: Some("shop".to_owned()),
            name: "robot".to_owned(),
        })
    );
}

#[test]
fn only_named_grants_go_to_second_group() {
    let data = snapshot(
        vec![
            cluster_role("all-pods", vec![rule(&["pods"], &[])]),
            cluster_role("some-pods", vec![rule(&["pods"], &["web", "db"])]),
        ],
        vec![
            binding(
                "a",
                "all-pods",
                vec![subject(SubjectKind::User, "ann", None)],
            ),
            binding(
                "b",
                "some-pods",
                vec![subject(SubjectKind::User, "bob", None)],
            ),
        ],
    );
    let groups = groups_of(&data);
    assert_eq!(texts(&groups.full), ["user ann"]);
    assert_eq!(texts(&groups.named_only), ["user bob"]);
    assert_eq!(
        groups.named_only[0].lines[0].only.as_deref(),
        Some("only web, db")
    );
}

#[test]
fn only_text_caps_names_at_five() {
    let names: Vec<String> = (1..=7).map(|index| format!("n{index}")).collect();
    assert_eq!(only_text(&names), "only n1, n2, n3, n4, n5, +2");
}

#[test]
fn headline_counts_full_subjects() {
    let request = get_pods();
    assert_eq!(headline_text(&request, 1), "1 subject can get pods in shop");
    assert_eq!(
        headline_text(&request, 3),
        "3 subjects can get pods in shop"
    );
    let cluster_wide = AccessRequest {
        verb: "list".to_owned(),
        target: RequestTarget::Resource(ResourceRequest {
            group: "apps".to_owned(),
            resource: "deployments".to_owned(),
            subresource: Some("scale".to_owned()),
            name: None,
            namespace: None,
        }),
    };
    assert_eq!(
        headline_text(&cluster_wide, 0),
        "0 subjects can list deployments.apps/scale cluster-wide"
    );
}

#[test]
fn headline_warns_on_broad_group() {
    let data = snapshot(
        vec![cluster_role("reader", vec![rule(&["pods"], &[])])],
        vec![binding(
            "b",
            "reader",
            vec![
                subject(SubjectKind::User, "ann", None),
                subject(SubjectKind::Group, "system:serviceaccounts", None),
            ],
        )],
    );
    let result = evaluate_request(&data, jiff::Timestamp::UNIX_EPOCH, &get_pods());
    assert!(result.has_broad_group);
    assert_eq!(result.groups.full[0].tone, Some(StatusTone::Warn));
    let plain = snapshot(
        vec![cluster_role("reader", vec![rule(&["pods"], &[])])],
        vec![binding(
            "b",
            "reader",
            vec![subject(SubjectKind::User, "ann", None)],
        )],
    );
    let result = evaluate_request(&plain, jiff::Timestamp::UNIX_EPOCH, &get_pods());
    assert!(!result.has_broad_group);
}

#[test]
fn coverage_notes_per_gap() {
    assert!(coverage_notes(&coverage(), Some("shop")).is_empty());
    let mut gaps = coverage();
    gaps.cluster_bindings = false;
    assert_eq!(
        coverage_notes(&gaps, None),
        ["ClusterRoleBindings were not listed; only namespace grants are shown."]
    );
    let mut gaps = coverage();
    gaps.cluster_roles = false;
    assert_eq!(
        coverage_notes(&gaps, None),
        ["ClusterRoles were not listed; grants through them are not shown."]
    );
    let mut gaps = coverage();
    gaps.roles = NamespaceCoverage::Namespaces(vec!["a".to_owned(), "b".to_owned()]);
    gaps.role_bindings = NamespaceCoverage::Namespaces(vec!["a".to_owned()]);
    assert_eq!(
        coverage_notes(&gaps, Some("a")),
        ["Roles and RoleBindings were listed in 1 namespace only (not permitted cluster-wide)."]
    );
    assert_eq!(coverage_notes(&gaps, Some("b")).len(), 2);
    assert_eq!(
        coverage_notes(&gaps, Some("b"))[1],
        "RoleBindings of b were not listed; only cluster-wide grants are shown."
    );
}

#[test]
fn namespace_options_add_a_missing_namespace() {
    let listed = vec!["a".to_owned(), "b".to_owned()];
    assert_eq!(
        namespace_options(listed.clone(), Some("c")),
        [ALL_NAMESPACES, "c", "a", "b"]
    );
    assert_eq!(
        namespace_options(listed.clone(), Some("b")),
        [ALL_NAMESPACES, "a", "b"]
    );
    assert_eq!(namespace_options(listed, None), [ALL_NAMESPACES, "a", "b"]);
    assert_eq!(
        namespace_options(Vec::new(), Some("x")),
        [ALL_NAMESPACES, "x"]
    );
}

#[test]
fn coverage_note_pluralizes_the_namespace_count() {
    let mut gaps = coverage();
    gaps.roles = NamespaceCoverage::Namespaces(vec!["a".to_owned(), "b".to_owned()]);
    assert_eq!(
        coverage_notes(&gaps, None),
        ["Roles and RoleBindings were listed in 2 namespaces only (not permitted cluster-wide)."]
    );
}

#[test]
fn masters_grant_moves_to_the_fixed_row() {
    let data = snapshot(
        vec![cluster_role("reader", vec![rule(&["pods"], &[])])],
        vec![binding(
            "admins",
            "reader",
            vec![
                subject(SubjectKind::Group, "system:masters", None),
                subject(SubjectKind::User, "ann", None),
            ],
        )],
    );
    let result = evaluate_request(&data, jiff::Timestamp::UNIX_EPOCH, &get_pods());
    assert_eq!(texts(&result.groups.full), ["user ann"]);
    let [line] = result.masters.as_slice() else {
        panic!("one masters grant line");
    };
    assert_eq!(line.binding_text, "clusterrolebinding/admins");
    // The subject still counts: the headline is about who can, not how many rows show.
    assert!(result.headline.starts_with("2 subjects can"));
}

#[test]
fn masters_row_has_no_lines_without_a_binding() {
    let data = snapshot(
        vec![cluster_role("reader", vec![rule(&["pods"], &[])])],
        vec![binding(
            "b",
            "reader",
            vec![subject(SubjectKind::User, "ann", None)],
        )],
    );
    let result = evaluate_request(&data, jiff::Timestamp::UNIX_EPOCH, &get_pods());
    assert!(result.masters.is_empty());
}

#[test]
fn only_idle_and_loading_listings_are_awaited() {
    assert!(awaits_listing(&RbacState::Idle));
    assert!(awaits_listing(&RbacState::Loading {
        _task: gpui_kit::Task::ready(())
    }));
    assert!(!awaits_listing(&RbacState::Failed("no".to_owned())));
}
