use super::*;

fn api_subject(kind: &str, name: &str, namespace: Option<&str>) -> ApiSubject {
    ApiSubject {
        kind: kind.to_owned(),
        name: name.to_owned(),
        namespace: namespace.map(str::to_owned),
        ..Default::default()
    }
}

fn api_role_ref(kind: &str, name: &str) -> ApiRoleRef {
    ApiRoleRef {
        api_group: "rbac.authorization.k8s.io".to_owned(),
        kind: kind.to_owned(),
        name: name.to_owned(),
    }
}

fn role_binding(role_kind: &str, subjects: Vec<ApiSubject>) -> BindingSummary {
    let mut binding = RoleBinding {
        role_ref: api_role_ref(role_kind, "reader"),
        subjects: Some(subjects),
        ..Default::default()
    };
    binding.metadata.name = Some("read".to_owned());
    binding.metadata.namespace = Some("shop".to_owned());
    role_binding_summary(&binding)
}

fn cluster_binding(subjects: Vec<ApiSubject>) -> BindingSummary {
    cluster_role_binding_summary(&ClusterRoleBinding {
        role_ref: api_role_ref("ClusterRole", "view"),
        subjects: Some(subjects),
        ..Default::default()
    })
}

fn group(name: &str) -> Subject {
    Subject {
        kind: SubjectKind::Group,
        name: name.to_owned(),
        namespace: None,
    }
}

fn with_subjects(subjects: Vec<Subject>) -> BindingSummary {
    BindingSummary {
        subjects,
        ..role_binding("Role", Vec::new())
    }
}

#[test]
fn binding_reads_role_ref_and_subjects() {
    let summary = role_binding(
        "Role",
        vec![
            api_subject("User", "ana", None),
            api_subject("Group", "devs", None),
            api_subject("ServiceAccount", "api", Some("shop")),
        ],
    );
    assert_eq!(summary.name, "read");
    assert_eq!(summary.namespace.as_deref(), Some("shop"));
    assert_eq!(summary.role.kind, RoleKind::Role);
    assert_eq!(summary.role.name, "reader");
    let kinds: Vec<_> = summary
        .subjects
        .iter()
        .map(|subject| subject.kind)
        .collect();
    assert_eq!(
        kinds,
        [
            SubjectKind::User,
            SubjectKind::Group,
            SubjectKind::ServiceAccount
        ]
    );
    assert!(summary.has_service_account_subject());
    assert!(!with_subjects(vec![group("devs")]).has_service_account_subject());
}

#[test]
fn service_account_subject_defaults_to_binding_namespace() {
    let summary = role_binding("Role", vec![api_subject("ServiceAccount", "api", None)]);
    assert_eq!(summary.subjects[0].namespace.as_deref(), Some("shop"));
    assert_eq!(
        summary.binds_service_account("shop", "api"),
        Some(SubjectMatch::Direct)
    );
}

#[test]
fn cluster_binding_service_account_without_namespace_never_matches() {
    let summary = cluster_binding(vec![api_subject("ServiceAccount", "api", None)]);
    assert_eq!(summary.namespace, None);
    assert_eq!(summary.subjects[0].namespace, None);
    assert_eq!(summary.binds_service_account("shop", "api"), None);
    assert_eq!(summary.binds_service_account("", "api"), None);
}

#[test]
fn binds_service_account_direct() {
    let summary = cluster_binding(vec![api_subject("ServiceAccount", "api", Some("shop"))]);
    assert_eq!(
        summary.binds_service_account("shop", "api"),
        Some(SubjectMatch::Direct)
    );
    assert_eq!(summary.binds_service_account("shop", "worker"), None);
    assert_eq!(summary.binds_service_account("other", "api"), None);
}

#[test]
fn binds_service_account_through_service_account_groups() {
    let all = cluster_binding(vec![api_subject("Group", "system:serviceaccounts", None)]);
    assert_eq!(
        all.binds_service_account("shop", "api"),
        Some(SubjectMatch::Group("system:serviceaccounts".to_owned()))
    );
    let namespace = cluster_binding(vec![api_subject(
        "Group",
        "system:serviceaccounts:shop",
        None,
    )]);
    assert_eq!(
        namespace.binds_service_account("shop", "api"),
        Some(SubjectMatch::Group(
            "system:serviceaccounts:shop".to_owned()
        ))
    );
    assert_eq!(namespace.binds_service_account("other", "api"), None);
}

#[test]
fn direct_match_wins_over_group() {
    let summary = cluster_binding(vec![
        api_subject("Group", "system:serviceaccounts", None),
        api_subject("ServiceAccount", "api", Some("shop")),
    ]);
    assert_eq!(
        summary.binds_service_account("shop", "api"),
        Some(SubjectMatch::Direct)
    );
}

#[test]
fn authenticated_group_does_not_bind() {
    let summary = cluster_binding(vec![
        api_subject("Group", "system:authenticated", None),
        api_subject("Group", "system:unauthenticated", None),
    ]);
    assert_eq!(summary.binds_service_account("shop", "api"), None);
}

#[test]
fn unknown_subject_kind_is_dropped() {
    let summary = cluster_binding(vec![
        api_subject("Robot", "r2", None),
        api_subject("User", "ana", None),
    ]);
    assert_eq!(summary.subjects.len(), 1);
    assert_eq!(summary.subjects[0].name, "ana");
}

#[test]
fn role_kind_maps_role_cluster_role_and_other() {
    assert_eq!(role_binding("Role", Vec::new()).role.kind, RoleKind::Role);
    assert_eq!(
        role_binding("ClusterRole", Vec::new()).role.kind,
        RoleKind::ClusterRole
    );
    let other = role_binding("Weird", Vec::new()).role.kind;
    assert_eq!(other, RoleKind::Other("Weird".to_owned()));
    assert_eq!(other.to_string(), "Weird");
    assert_eq!(RoleKind::ClusterRole.to_string(), "ClusterRole");
    assert_eq!(RoleKind::Role.to_string(), "Role");
}

#[test]
fn broad_groups_recognized() {
    assert_eq!(
        group("system:authenticated").broad_group(),
        Some(BroadGroup::Authenticated)
    );
    assert_eq!(
        group("system:unauthenticated").broad_group(),
        Some(BroadGroup::Unauthenticated)
    );
    assert_eq!(
        group("system:serviceaccounts").broad_group(),
        Some(BroadGroup::AllServiceAccounts)
    );
    assert_eq!(
        group("system:serviceaccounts:shop").broad_group(),
        Some(BroadGroup::NamespaceServiceAccounts("shop".to_owned()))
    );
    assert_eq!(group("system:masters").broad_group(), None);
    assert_eq!(group("system:serviceaccounts:").broad_group(), None);
    let user = Subject {
        kind: SubjectKind::User,
        ..group("system:authenticated")
    };
    assert_eq!(user.broad_group(), None);
}
