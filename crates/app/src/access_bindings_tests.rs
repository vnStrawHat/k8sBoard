use cluster::{RoleKind, RoleRef, SubjectKind};

use super::*;

fn subject(kind: SubjectKind, namespace: Option<&str>, name: &str) -> Subject {
    Subject {
        kind,
        name: name.to_owned(),
        namespace: namespace.map(str::to_owned),
    }
}

fn account(namespace: &str, name: &str) -> Subject {
    subject(SubjectKind::ServiceAccount, Some(namespace), name)
}

fn binding(
    namespace: Option<&str>,
    name: &str,
    role: (RoleKind, &str),
    subjects: Vec<Subject>,
) -> BindingSummary {
    BindingSummary {
        namespace: namespace.map(str::to_owned),
        name: name.to_owned(),
        created_at: None,
        labels: Vec::new(),
        role: RoleRef {
            kind: role.0,
            name: role.1.to_owned(),
        },
        subjects,
    }
}

fn role(namespace: Option<&str>, name: &str) -> RoleSummary {
    RoleSummary {
        namespace: namespace.map(str::to_owned),
        name: name.to_owned(),
        created_at: None,
        labels: Vec::new(),
        rules: Vec::new(),
        aggregation: Vec::new(),
    }
}

fn lists<'a>(
    role_bindings: &'a [BindingSummary],
    cluster: &'a [BindingSummary],
) -> BindingLists<'a> {
    BindingLists {
        role_bindings,
        cluster_role_bindings: cluster,
    }
}

fn names(bindings: &[&BindingSummary]) -> Vec<String> {
    bindings
        .iter()
        .map(|binding| binding.name.clone())
        .collect()
}

#[test]
fn binding_text_per_binding_kind() {
    let reader = binding(Some("shop"), "read", (RoleKind::Role, "reader"), Vec::new());
    assert_eq!(binding_text(&reader), "rolebinding/read");
    let admin = binding(
        None,
        "root",
        (RoleKind::ClusterRole, "cluster-admin"),
        Vec::new(),
    );
    assert_eq!(binding_text(&admin), "clusterrolebinding/root");
}

#[test]
fn subject_text_formats() {
    assert_eq!(subject_text(&account("shop", "api")), "sa shop/api");
    assert_eq!(
        subject_text(&subject(SubjectKind::User, None, "ana")),
        "user ana"
    );
    assert_eq!(
        subject_text(&subject(SubjectKind::Group, None, "devs")),
        "group devs"
    );
    assert_eq!(
        subject_text(&subject(SubjectKind::ServiceAccount, None, "api")),
        "sa api"
    );
}

#[test]
fn binding_key_per_binding_kind() {
    let namespaced = binding(Some("shop"), "read", (RoleKind::Role, "r"), Vec::new());
    assert_eq!(
        binding_key(&namespaced),
        ResourceKey::Kind {
            kind: ResourceKind::RoleBindings,
            namespace: Some("shop".to_owned()),
            name: "read".to_owned(),
        }
    );
    let cluster_wide = binding(None, "root", (RoleKind::ClusterRole, "r"), Vec::new());
    assert_eq!(
        binding_key(&cluster_wide),
        ResourceKey::Kind {
            kind: ResourceKind::ClusterRoleBindings,
            namespace: None,
            name: "root".to_owned(),
        }
    );
}

#[test]
fn role_key_per_role_kind() {
    let key = |kind, namespace| role_key(&binding(namespace, "b", (kind, "r"), Vec::new()));
    assert_eq!(
        key(RoleKind::Role, Some("shop")),
        Some(ResourceKey::Kind {
            kind: ResourceKind::Roles,
            namespace: Some("shop".to_owned()),
            name: "r".to_owned(),
        })
    );
    assert_eq!(
        key(RoleKind::ClusterRole, Some("shop")),
        Some(ResourceKey::Kind {
            kind: ResourceKind::ClusterRoles,
            namespace: None,
            name: "r".to_owned(),
        })
    );
    assert_eq!(key(RoleKind::Other("Weird".to_owned()), Some("shop")), None);
}

#[test]
fn role_is_bound_only_in_its_namespace() {
    let role_bindings = [
        binding(Some("shop"), "here", (RoleKind::Role, "reader"), Vec::new()),
        binding(
            Some("other"),
            "there",
            (RoleKind::Role, "reader"),
            Vec::new(),
        ),
        binding(
            Some("shop"),
            "cluster",
            (RoleKind::ClusterRole, "reader"),
            Vec::new(),
        ),
    ];
    let index = BindingIndex::build(&lists(&role_bindings, &[]));
    assert_eq!(
        names(&index.bindings_of_role(&role(Some("shop"), "reader"))),
        ["here"]
    );
    assert_eq!(
        names(&index.bindings_of_role(&role(Some("other"), "reader"))),
        ["there"]
    );
    assert!(
        index
            .bindings_of_role(&role(Some("shop"), "unbound"))
            .is_empty()
    );
}

#[test]
fn cluster_role_bound_by_both_kinds() {
    let role_bindings = [
        binding(
            Some("shop"),
            "local",
            (RoleKind::ClusterRole, "view"),
            Vec::new(),
        ),
        binding(
            Some("shop"),
            "other-role",
            (RoleKind::Role, "view"),
            Vec::new(),
        ),
    ];
    let cluster = [binding(
        None,
        "global",
        (RoleKind::ClusterRole, "view"),
        Vec::new(),
    )];
    let index = BindingIndex::build(&lists(&role_bindings, &cluster));
    assert_eq!(
        names(&index.bindings_of_role(&role(None, "view"))),
        ["local", "global"]
    );
}

#[test]
fn index_skips_other_role_kinds() {
    let cluster = [
        binding(
            None,
            "weird",
            (RoleKind::Other("Weird".to_owned()), "view"),
            Vec::new(),
        ),
        // A cluster binding cannot name a Role; it must not land on a ClusterRole of that name.
        binding(None, "invalid", (RoleKind::Role, "view"), Vec::new()),
    ];
    let index = BindingIndex::build(&lists(&[], &cluster));
    assert!(index.bindings_of_role(&role(None, "view")).is_empty());
}

#[test]
fn broad_admin_needs_cluster_admin_and_a_broad_subject() {
    let admin = (RoleKind::ClusterRole, "cluster-admin");
    let group = |name| subject(SubjectKind::Group, None, name);
    let with = |subjects| broad_admin(&binding(None, "b", admin.clone(), subjects));
    assert_eq!(
        with(vec![account("kube-system", "tiller")]),
        Some(BroadAdmin::ServiceAccounts)
    );
    assert_eq!(
        with(vec![group("system:serviceaccounts")]),
        Some(BroadAdmin::ServiceAccounts)
    );
    assert_eq!(
        with(vec![group("system:authenticated")]),
        Some(BroadAdmin::Everyone)
    );
    assert_eq!(
        with(vec![account("a", "b"), group("system:unauthenticated")]),
        Some(BroadAdmin::Everyone)
    );
    assert_eq!(with(vec![subject(SubjectKind::User, None, "ana")]), None);
    assert_eq!(with(vec![group("system:masters")]), None);
    let view = binding(
        None,
        "b",
        (RoleKind::ClusterRole, "view"),
        vec![account("a", "b")],
    );
    assert_eq!(broad_admin(&view), None);
    let namespaced = binding(
        Some("a"),
        "b",
        (RoleKind::Role, "cluster-admin"),
        vec![account("a", "b")],
    );
    assert_eq!(broad_admin(&namespaced), None);
}

#[test]
fn role_subjects_service_accounts_first_by_text() {
    let bindings = [
        binding(
            None,
            "b1",
            (RoleKind::ClusterRole, "r"),
            vec![
                subject(SubjectKind::User, None, "ana"),
                account("z", "last"),
            ],
        ),
        binding(
            None,
            "b2",
            (RoleKind::ClusterRole, "r"),
            vec![account("a", "first")],
        ),
    ];
    let refs: Vec<&BindingSummary> = bindings.iter().collect();
    let subjects = role_subjects(&refs);
    let texts: Vec<&str> = subjects.iter().map(|row| row.text.as_str()).collect();
    assert_eq!(texts, ["sa a/first", "sa z/last", "user ana"]);
    assert_eq!(subjects[0].binding.name, "b2");
    assert!(subjects[0].is_service_account);
    assert!(!subjects[2].is_service_account);
}

mod status {
    use cluster::{AccessCheck, AccessDecision, AccessReport, AccessReview, WatchUpdate};

    use super::*;
    use crate::cluster_session::LiveList;

    fn ready(items: Vec<BindingSummary>) -> LiveList<BindingSummary> {
        let mut list = LiveList::Loading;
        list.apply(WatchUpdate::Snapshot(items));
        list
    }

    fn bindings(
        role_bindings: LiveList<BindingSummary>,
        cluster_role_bindings: Option<LiveList<BindingSummary>>,
    ) -> CompanionLists {
        CompanionLists::Bindings {
            role_bindings,
            cluster_role_bindings,
        }
    }

    fn denying(denied: &[AccessCheck]) -> AccessState {
        AccessState::Known(AccessReport {
            reviews: AccessCheck::ALL
                .into_iter()
                .map(|check| AccessReview {
                    check,
                    decision: if denied.contains(&check) {
                        AccessDecision::Denied { reason: None }
                    } else {
                        AccessDecision::Allowed
                    },
                })
                .collect(),
        })
    }

    #[test]
    fn roles_need_only_role_bindings() {
        let companion = bindings(ready(Vec::new()), None);
        let status = bindings_status(ResourceKind::Roles, &AccessState::Unknown, Some(&companion));
        assert!(matches!(status, BindingsStatus::Ready(_)));
        assert!(ready_binding_lists(Some(&companion)).is_some());
    }

    #[test]
    fn loading_until_every_list_is_ready() {
        let companion = bindings(ready(Vec::new()), Some(LiveList::Loading));
        let status = bindings_status(
            ResourceKind::ClusterRoles,
            &AccessState::Unknown,
            Some(&companion),
        );
        assert!(matches!(status, BindingsStatus::Loading));
        let status = bindings_status(ResourceKind::ClusterRoles, &AccessState::Unknown, None);
        assert!(matches!(status, BindingsStatus::Loading));
        assert!(ready_binding_lists(Some(&companion)).is_none());
        assert!(ready_binding_lists(None).is_none());
    }

    #[test]
    fn a_failed_list_names_its_message() {
        let mut failed = LiveList::Loading;
        failed.apply(WatchUpdate::Failed(cluster::ClusterError::TimedOut {
            context: "ctx".to_owned(),
            action: "watching role bindings",
        }));
        let companion = bindings(ready(Vec::new()), Some(failed));
        let status = bindings_status(
            ResourceKind::ClusterRoles,
            &AccessState::Unknown,
            Some(&companion),
        );
        assert!(matches!(status, BindingsStatus::Failed(_)));
    }

    #[test]
    fn denied_names_every_denied_list() {
        let access = denying(&[
            AccessCheck::ListRoleBindings,
            AccessCheck::ListClusterRoleBindings,
        ]);
        let BindingsStatus::Denied(checks) =
            bindings_status(ResourceKind::ClusterRoles, &access, None)
        else {
            panic!("both lists are denied");
        };
        assert_eq!(
            checks,
            [
                AccessCheck::ListRoleBindings,
                AccessCheck::ListClusterRoleBindings
            ]
        );
        let only_cluster = denying(&[AccessCheck::ListClusterRoleBindings]);
        let status = bindings_status(ResourceKind::ClusterRoles, &only_cluster, None);
        assert!(
            matches!(status, BindingsStatus::Denied(checks) if checks == [AccessCheck::ListClusterRoleBindings])
        );
        // Roles do not need the cluster role bindings.
        let status = bindings_status(ResourceKind::Roles, &only_cluster, None);
        assert!(matches!(status, BindingsStatus::Loading));
    }
}

// ---- Service accounts ----

fn index_of<'a>(
    role_bindings: &'a [BindingSummary],
    cluster: &'a [BindingSummary],
) -> BindingIndex<'a> {
    BindingIndex::build(&lists(role_bindings, cluster))
}

fn group(name: &str) -> Subject {
    subject(SubjectKind::Group, None, name)
}

fn roles_of(index: &BindingIndex, namespace: &str, name: &str) -> Vec<(String, Option<String>)> {
    index
        .bound_roles(namespace, name)
        .iter()
        .map(|bound| (role_text(&bound.role), bound.group.clone()))
        .collect()
}

#[test]
fn role_text_formats() {
    let role = |kind, name: &str| RoleRef {
        kind,
        name: name.to_owned(),
    };
    assert_eq!(role_text(&role(RoleKind::Role, "reader")), "role/reader");
    assert_eq!(
        role_text(&role(RoleKind::ClusterRole, "cluster-admin")),
        "clusterrole/cluster-admin"
    );
    assert_eq!(
        role_text(&role(RoleKind::Other("Weird".to_owned()), "x")),
        "weird/x"
    );
}

#[test]
fn bound_roles_direct_and_group_sorted() {
    let role_bindings = [
        binding(
            Some("shop"),
            "b-read",
            (RoleKind::Role, "reader"),
            vec![account("shop", "api")],
        ),
        binding(
            Some("shop"),
            "b-ns-group",
            (RoleKind::ClusterRole, "edit"),
            vec![group("system:serviceaccounts:shop")],
        ),
    ];
    let cluster = [
        binding(
            None,
            "b-view",
            (RoleKind::ClusterRole, "view"),
            vec![account("shop", "api")],
        ),
        binding(
            None,
            "b-all",
            (RoleKind::ClusterRole, "basic"),
            vec![group("system:serviceaccounts")],
        ),
    ];
    let index = index_of(&role_bindings, &cluster);
    // Direct roles first (by role text), then those reached through a group.
    assert_eq!(
        roles_of(&index, "shop", "api"),
        [
            ("clusterrole/view".to_owned(), None),
            ("role/reader".to_owned(), None),
            (
                "clusterrole/basic".to_owned(),
                Some("system:serviceaccounts".to_owned())
            ),
            (
                "clusterrole/edit".to_owned(),
                Some("system:serviceaccounts:shop".to_owned())
            ),
        ]
    );
    let direct = &index.bound_roles("shop", "api")[0];
    assert_eq!(direct.binding_namespace(), None);
    assert_eq!(direct.binding_text, "clusterrolebinding/b-view");
    assert_eq!(direct.binding_namespace(), None);
    let local = index.bound_roles("shop", "api")[1];
    assert_eq!(local.binding_namespace(), Some("shop"));
    assert_eq!(
        local.role_key,
        Some(ResourceKey::Kind {
            kind: ResourceKind::Roles,
            namespace: Some("shop".to_owned()),
            name: "reader".to_owned(),
        })
    );
}

#[test]
fn bound_roles_ignore_other_accounts() {
    let cluster = [
        binding(
            None,
            "b",
            (RoleKind::ClusterRole, "view"),
            vec![account("shop", "api")],
        ),
        binding(
            None,
            "everyone",
            (RoleKind::ClusterRole, "basic"),
            vec![
                group("system:authenticated"),
                group("system:serviceaccounts:db"),
            ],
        ),
        // A cluster binding subject without a namespace never matches.
        binding(
            None,
            "nameless",
            (RoleKind::ClusterRole, "edit"),
            vec![subject(SubjectKind::ServiceAccount, None, "worker")],
        ),
    ];
    let index = index_of(&[], &cluster);
    assert!(roles_of(&index, "shop", "worker").is_empty());
    assert!(roles_of(&index, "other", "api").is_empty());
    assert!(roles_of(&index, "shop", "unknown").is_empty());
    // The roles of a kind without a screen still show, as text.
    let weird = [binding(
        None,
        "w",
        (RoleKind::Other("Weird".to_owned()), "x"),
        vec![account("shop", "api")],
    )];
    let index = index_of(&[], &weird);
    let bound = index.bound_roles("shop", "api");
    assert_eq!(bound.len(), 1);
    assert_eq!(bound[0].role_key, None);
}

#[test]
fn index_built_once_serves_every_account() {
    let cluster = [
        binding(
            None,
            "a",
            (RoleKind::ClusterRole, "view"),
            vec![account("shop", "api")],
        ),
        binding(
            None,
            "b",
            (RoleKind::ClusterRole, "edit"),
            vec![account("shop", "web")],
        ),
        binding(
            None,
            "c",
            (RoleKind::ClusterRole, "basic"),
            vec![group("system:serviceaccounts:db")],
        ),
    ];
    let index = index_of(&[], &cluster);
    assert_eq!(roles_of(&index, "shop", "api").len(), 1);
    assert_eq!(roles_of(&index, "shop", "web")[0].0, "clusterrole/edit");
    assert_eq!(roles_of(&index, "db", "pg")[0].0, "clusterrole/basic");
}

#[test]
fn index_agrees_with_the_cluster_crate_matching_rule() {
    let cases = [
        vec![account("shop", "api")],
        vec![group("system:serviceaccounts")],
        vec![group("system:serviceaccounts:shop")],
        vec![group("system:serviceaccounts:other")],
        vec![group("system:authenticated"), group("system:masters")],
        vec![subject(SubjectKind::User, None, "api")],
    ];
    for subjects in cases {
        let bindings = [binding(
            None,
            "b",
            (RoleKind::ClusterRole, "view"),
            subjects,
        )];
        let index = index_of(&[], &bindings);
        let matched = bindings[0].binds_service_account("shop", "api");
        let found = index.bound_roles("shop", "api");
        assert_eq!(
            matched.is_some(),
            !found.is_empty(),
            "{:?}",
            bindings[0].subjects
        );
        if let Some(cluster::SubjectMatch::Group(name)) = matched {
            assert_eq!(found[0].group.as_deref(), Some(name.as_str()));
        }
    }
}

#[test]
fn authenticated_bindings_are_kept_apart() {
    let cluster = [
        binding(
            None,
            "basic-user",
            (RoleKind::ClusterRole, "system:basic-user"),
            vec![group("system:authenticated")],
        ),
        binding(
            None,
            "root",
            (RoleKind::ClusterRole, "cluster-admin"),
            vec![group("system:authenticated")],
        ),
    ];
    let index = index_of(&[], &cluster);
    // Not a binding of one account...
    assert!(index.bound_roles("shop", "api").is_empty());
    assert_eq!(index.everyone_roles().len(), 2);
    // ...and only cluster-admin is held by every account, listed last as a group match.
    let held = index.roles_held("shop", "api");
    assert_eq!(held.len(), 1);
    assert_eq!(role_text(&held[0].role), "clusterrole/cluster-admin");
    assert_eq!(held[0].group.as_deref(), Some("system:authenticated"));
}
