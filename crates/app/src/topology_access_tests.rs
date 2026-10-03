use cluster::RoleKind;

use super::*;
use crate::access_rows::{cluster_role_binding_row, role_binding_row};
use crate::resource_kind::ResourceKind;
use crate::topology_fixtures::{NAMESPACE, account_subject, binding, group_subject};

fn names(binding_subjects: Vec<cluster::Subject>) -> bool {
    let binding = binding(None, "b", (RoleKind::ClusterRole, "view"), binding_subjects);
    names_namespace_account(&binding, NAMESPACE)
}

#[test]
fn a_direct_subject_of_the_namespace_counts() {
    assert!(names(vec![account_subject("api")]));
}

#[test]
fn an_account_of_another_namespace_does_not() {
    let mut elsewhere = account_subject("api");
    elsewhere.namespace = Some("other".to_owned());
    assert!(!names(vec![elsewhere]));
}

#[test]
fn the_service_account_and_authenticated_groups_count() {
    for group in [
        "system:serviceaccounts",
        "system:serviceaccounts:shop",
        "system:authenticated",
    ] {
        assert!(names(vec![group_subject(group)]), "{group}");
    }
}

#[test]
fn other_groups_and_users_do_not() {
    for group in [
        "system:serviceaccounts:other",
        "system:unauthenticated",
        "system:masters",
        "devs",
    ] {
        assert!(!names(vec![group_subject(group)]), "{group}");
    }
    let user = cluster::Subject {
        kind: cluster::SubjectKind::User,
        name: "alice".to_owned(),
        namespace: None,
    };
    assert!(!names(vec![user]));
}

#[test]
fn collect_keeps_the_role_bindings_and_only_the_cluster_bindings_that_name_the_namespace() {
    let mine = binding(
        Some(NAMESPACE),
        "mine",
        (RoleKind::ClusterRole, "view"),
        vec![account_subject("api")],
    );
    let admin = binding(
        None,
        "admin",
        (RoleKind::ClusterRole, "cluster-admin"),
        vec![account_subject("api")],
    );
    let unrelated = binding(
        None,
        "unrelated",
        (RoleKind::ClusterRole, "view"),
        vec![group_subject("devs")],
    );
    let rows = [
        (TopologyKind::RoleBinding, role_binding_row(&mine)),
        (
            TopologyKind::ClusterRoleBinding,
            cluster_role_binding_row(&admin),
        ),
        (
            TopologyKind::ClusterRoleBinding,
            cluster_role_binding_row(&unrelated),
        ),
    ];
    let bindings = AccessBindings::collect(rows.iter().map(|(kind, row)| (*kind, row)), NAMESPACE);
    let lists = bindings.lists();
    assert_eq!(lists.role_bindings, [mine]);
    assert_eq!(lists.cluster_role_bindings, [admin]);
}

#[test]
fn binding_nodes_come_from_binding_keys_only() {
    let role_binding = ResourceKey::Kind {
        kind: ResourceKind::RoleBindings,
        namespace: Some(NAMESPACE.to_owned()),
        name: "b".to_owned(),
    };
    assert_eq!(
        binding_node(&role_binding),
        Some(NodeId::Object {
            kind: TopologyKind::RoleBinding,
            name: "b".to_owned()
        })
    );
    let role = ResourceKey::Kind {
        kind: ResourceKind::Roles,
        namespace: Some(NAMESPACE.to_owned()),
        name: "r".to_owned(),
    };
    assert_eq!(binding_node(&role), None);
}

#[test]
fn a_direct_cluster_admin_grant_names_the_binding_node() {
    let admin = binding(
        None,
        "ci-admin",
        (RoleKind::ClusterRole, "cluster-admin"),
        vec![account_subject("api")],
    );
    let bindings = AccessBindings::collect(
        [(
            TopologyKind::ClusterRoleBinding,
            cluster_role_binding_row(&admin),
        )]
        .iter()
        .map(|(kind, row)| (*kind, row)),
        NAMESPACE,
    );
    let index = BindingIndex::build(&bindings.lists());
    let account = NodeId::Object {
        kind: TopologyKind::ServiceAccount,
        name: "api".to_owned(),
    };
    let grant = cluster_admin_grant(&index, NAMESPACE, &account, "api").expect("a grant");
    assert_eq!(
        grant.binding,
        Some(NodeId::Object {
            kind: TopologyKind::ClusterRoleBinding,
            name: "ci-admin".to_owned()
        })
    );
    assert_eq!(grant.binding_text, "clusterrolebinding/ci-admin");
    assert_eq!(grant.group, None);
    assert!(cluster_admin_grant(&index, NAMESPACE, &account, "other").is_none());
}

fn grant_through(kind: TopologyKind, row: crate::kind_row::KindRow) -> ClusterAdminGrant {
    let bindings = AccessBindings::collect([(kind, &row)].into_iter(), NAMESPACE);
    let index = BindingIndex::build(&bindings.lists());
    let account = NodeId::Object {
        kind: TopologyKind::ServiceAccount,
        name: "api".to_owned(),
    };
    cluster_admin_grant(&index, NAMESPACE, &account, "api").expect("a grant")
}

#[test]
fn a_role_binding_grant_carries_its_namespace() {
    let local = binding(
        Some(NAMESPACE),
        "local-admin",
        (RoleKind::ClusterRole, "cluster-admin"),
        vec![account_subject("api")],
    );
    let grant = grant_through(TopologyKind::RoleBinding, role_binding_row(&local));
    assert_eq!(grant.binding_namespace.as_deref(), Some(NAMESPACE));
    assert_eq!(grant.binding_text, "rolebinding/local-admin");
}

#[test]
fn a_cluster_role_binding_grant_is_cluster_wide() {
    let wide = binding(
        None,
        "ci-admin",
        (RoleKind::ClusterRole, "cluster-admin"),
        vec![account_subject("api")],
    );
    let grant = grant_through(
        TopologyKind::ClusterRoleBinding,
        cluster_role_binding_row(&wide),
    );
    assert_eq!(grant.binding_namespace, None);
}
