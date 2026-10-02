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
