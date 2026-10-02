use cluster::{RoleKind, RoleRef, Selector, SubjectKind};

use super::*;
use crate::resource_kind::ResourceKind;

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

fn role(namespace: Option<&str>, rules: Vec<RbacRule>) -> RoleSummary {
    RoleSummary {
        namespace: namespace.map(str::to_owned),
        name: "reader".to_owned(),
        created_at: None,
        labels: Vec::new(),
        rules,
        aggregation: Vec::new(),
    }
}

fn everything() -> RbacRule {
    rule(&["*"], &["*"], &["*"])
}

fn subject(kind: SubjectKind, namespace: Option<&str>, name: &str) -> Subject {
    Subject {
        kind,
        name: name.to_owned(),
        namespace: namespace.map(str::to_owned),
    }
}

fn binding(
    namespace: Option<&str>,
    role: (RoleKind, &str),
    subjects: Vec<Subject>,
) -> BindingSummary {
    BindingSummary {
        namespace: namespace.map(str::to_owned),
        name: "bind".to_owned(),
        created_at: None,
        labels: Vec::new(),
        role: RoleRef {
            kind: role.0,
            name: role.1.to_owned(),
        },
        subjects,
    }
}

fn admin_binding(subjects: Vec<Subject>) -> BindingSummary {
    binding(None, (RoleKind::ClusterRole, "cluster-admin"), subjects)
}

fn group(name: &str) -> Subject {
    subject(SubjectKind::Group, None, name)
}

#[test]
fn role_row_cells_match_column_count() {
    let row = role_row(&role(Some("shop"), vec![rule(&[""], &["pods"], &["get"])]));
    assert_eq!(row.cells.len(), ResourceKind::Roles.columns().len());
    assert_eq!(row.namespace.as_deref(), Some("shop"));
    // The Bindings cell waits for the companion join.
    assert_eq!(row.cells[1], KindCell::Absent);
}

#[test]
fn cluster_role_row_cells_match_column_count() {
    let row = cluster_role_row(&role(None, Vec::new()));
    assert_eq!(row.cells.len(), ResourceKind::ClusterRoles.columns().len());
    assert_eq!(row.namespace, None);
    assert_eq!(row.cells[1], KindCell::Text("No".into()));
    assert_eq!(row.cells[2], KindCell::Absent);
}

#[test]
fn role_binding_row_cells_match_column_count() {
    let row = role_binding_row(&binding(Some("shop"), (RoleKind::Role, "r"), Vec::new()));
    assert_eq!(row.cells.len(), ResourceKind::RoleBindings.columns().len());
}

#[test]
fn cluster_role_binding_row_cells_match_column_count() {
    let row = cluster_role_binding_row(&binding(None, (RoleKind::ClusterRole, "r"), Vec::new()));
    assert_eq!(
        row.cells.len(),
        ResourceKind::ClusterRoleBindings.columns().len()
    );
}

#[test]
fn rules_cell_marks_all() {
    let all = role_row(&role(Some("shop"), vec![everything()]));
    assert_eq!(all.cells[0], KindCell::Text("1 (all)".into()));
    let some = role_row(&role(Some("shop"), vec![rule(&[""], &["pods"], &["get"])]));
    assert_eq!(some.cells[0], KindCell::Text("1".into()));
}

fn status_of(role: &RoleSummary) -> (String, StatusTone) {
    let status = cluster_role_row(role).status;
    (status.text.to_string(), status.tone)
}

#[test]
fn role_status_order() {
    let rules = |count: usize| vec![rule(&[""], &["pods"], &["get"]); count];
    let named = |text: &str, tone| (text.to_owned(), tone);
    assert_eq!(
        status_of(&role(None, vec![everything()])),
        named("Full access", StatusTone::Warn)
    );
    assert_eq!(
        status_of(&role(None, Vec::new())),
        named("No rules", StatusTone::Warn)
    );
    assert_eq!(
        status_of(&role(None, rules(1))),
        named("1 rule", StatusTone::Ok)
    );
    assert_eq!(
        status_of(&role(None, rules(3))),
        named("3 rules", StatusTone::Ok)
    );
    let mut aggregated = role(None, rules(2));
    aggregated.aggregation = vec![Selector::everything()];
    assert_eq!(
        status_of(&aggregated),
        named("Aggregated · 2 rules", StatusTone::Ok)
    );
    // Full access beats the aggregation.
    let mut wide = role(None, vec![everything()]);
    wide.aggregation = vec![Selector::everything()];
    assert_eq!(status_of(&wide), named("Full access", StatusTone::Warn));
    assert_eq!(
        role_row(&role(Some("shop"), Vec::new()))
            .status
            .text
            .as_ref(),
        "No rules"
    );
}

#[test]
fn binding_role_cell_kubectl_style() {
    let namespaced = role_binding_row(&binding(
        Some("shop"),
        (RoleKind::Role, "api-reader"),
        vec![group("devs")],
    ));
    assert_eq!(
        namespaced.cells[0],
        KindCell::Text("Role/api-reader".into())
    );
    let cluster_role = role_binding_row(&binding(
        Some("shop"),
        (RoleKind::ClusterRole, "view"),
        vec![group("devs")],
    ));
    assert_eq!(
        cluster_role.cells[0],
        KindCell::Text("ClusterRole/view".into())
    );
    let cluster = cluster_role_binding_row(&binding(
        None,
        (RoleKind::ClusterRole, "view"),
        vec![group("devs")],
    ));
    assert_eq!(cluster.cells[0], KindCell::Text("view".into()));
}

#[test]
fn cluster_admin_to_service_account_is_warn() {
    let row = cluster_role_binding_row(&admin_binding(vec![subject(
        SubjectKind::ServiceAccount,
        Some("kube-system"),
        "tiller",
    )]));
    assert_eq!(
        row.status.text.as_ref(),
        "cluster-admin to service accounts"
    );
    assert_eq!(row.status.tone, StatusTone::Warn);
    assert_eq!(
        row.cells[0],
        KindCell::Toned(labeled("cluster-admin", StatusTone::Warn))
    );
    let namespaced = role_binding_row(&BindingSummary {
        namespace: Some("shop".to_owned()),
        ..admin_binding(vec![group("system:serviceaccounts:shop")])
    });
    assert_eq!(namespaced.status.tone, StatusTone::Warn);
    assert_eq!(
        namespaced.cells[0],
        KindCell::Toned(labeled("ClusterRole/cluster-admin", StatusTone::Warn))
    );
}

#[test]
fn cluster_admin_to_authenticated_is_bad() {
    for name in ["system:authenticated", "system:unauthenticated"] {
        let row = cluster_role_binding_row(&admin_binding(vec![group(name)]));
        assert_eq!(
            row.status.text.as_ref(),
            "cluster-admin to everyone",
            "{name}"
        );
        assert_eq!(row.status.tone, StatusTone::Bad, "{name}");
        assert_eq!(
            row.cells[0],
            KindCell::Toned(labeled("cluster-admin", StatusTone::Bad))
        );
    }
}

#[test]
fn cluster_admin_to_user_is_ok() {
    let row = cluster_role_binding_row(&admin_binding(vec![
        subject(SubjectKind::User, None, "ana"),
        group("system:masters"),
    ]));
    assert_eq!(row.status.text.as_ref(), "2 subjects");
    assert_eq!(row.status.tone, StatusTone::Ok);
    assert_eq!(row.cells[0], KindCell::Text("cluster-admin".into()));
    let none = cluster_role_binding_row(&admin_binding(Vec::new()));
    assert_eq!(none.status.text.as_ref(), "No subjects");
    assert_eq!(none.status.tone, StatusTone::Warn);
    let one = role_binding_row(&binding(
        Some("shop"),
        (RoleKind::Role, "r"),
        vec![group("devs")],
    ));
    assert_eq!(one.status.text.as_ref(), "1 subject");
}

#[test]
fn other_role_kind_cell_as_written() {
    let row = role_binding_row(&binding(
        Some("shop"),
        (RoleKind::Other("Weird".to_owned()), "x"),
        Vec::new(),
    ));
    assert_eq!(row.cells[0], KindCell::Text("Weird/x".into()));
    let overview = row.section("Role").expect("role section");
    assert!(
        overview
            .rows
            .iter()
            .all(|detail| !matches!(detail, DetailRow::Link { .. }))
    );
}

#[test]
fn subjects_text_formats() {
    let row = cluster_role_binding_row(&binding(
        None,
        (RoleKind::ClusterRole, "view"),
        vec![
            subject(SubjectKind::ServiceAccount, Some("shop"), "api"),
            subject(SubjectKind::User, None, "ana"),
            group("devs"),
        ],
    ));
    assert_eq!(
        row.cells[1],
        KindCell::Text("sa shop/api, user ana, group devs".into())
    );
    let none = cluster_role_binding_row(&binding(None, (RoleKind::ClusterRole, "v"), Vec::new()));
    assert_eq!(none.cells[1], KindCell::Absent);
}

#[test]
fn binding_drawer_links_the_role() {
    let row = role_binding_row(&binding(
        Some("shop"),
        (RoleKind::Role, "api-reader"),
        Vec::new(),
    ));
    let role = row.section("Role").expect("role section");
    assert!(role.rows.iter().any(|detail| matches!(
        detail,
        DetailRow::Link {
            target: ResourceKey::Kind {
                kind: ResourceKind::Roles,
                ..
            },
            ..
        }
    )));
    let subjects = row.section("Subjects").expect("subjects section");
    assert_eq!(subjects.rows, [DetailRow::Note("No subjects".into())]);
}

#[test]
fn cluster_role_drawer_sections() {
    let mut aggregated = role(None, vec![rule(&[""], &["pods"], &["get"])]);
    aggregated.aggregation = vec![
        Selector::of_labels(&["rbac/aggregate=true".to_owned()]).expect("terms are not empty"),
    ];
    aggregated.labels = vec!["kubernetes.io/bootstrapping=rbac-defaults".to_owned()];
    let row = cluster_role_row(&aggregated);
    let titles: Vec<&str> = row.sections.iter().map(|section| section.title).collect();
    assert_eq!(titles, ["Cluster role", "Aggregation", "Rules", "Bound to"]);
    assert_eq!(
        row.section("Cluster role").expect("section").rows[0],
        DetailRow::field("Built-in", KindCell::Text("Yes".into()))
    );
    let plain = cluster_role_row(&role(None, Vec::new()));
    let titles: Vec<&str> = plain.sections.iter().map(|section| section.title).collect();
    assert_eq!(titles, ["Cluster role", "Rules", "Bound to"]);
    assert_eq!(
        plain.section("Rules").expect("section").rows,
        [DetailRow::Note("No rules".into())]
    );
}

fn table(rules: &[RbacRule]) -> Vec<String> {
    rules_table(rules).lines().map(str::to_owned).collect()
}

#[test]
fn rules_table_aligns_columns() {
    let lines = table(&[
        rule(
            &["apps"],
            &["deployments", "statefulsets"],
            &["get", "list"],
        ),
        rule(&["batch"], &["jobs"], &["*"]),
    ]);
    // Widths: apiGroups 9, resources 25 (`deployments, statefulsets`).
    assert_eq!(
        lines,
        [
            format!("{:<9}  {:<25}  {}", "apiGroups", "resources", "verbs"),
            format!(
                "{:<9}  {:<25}  {}",
                "apps", "deployments, statefulsets", "get, list"
            ),
            format!("{:<9}  {:<25}  {}", "batch", "jobs", "*"),
        ]
    );
}

#[test]
fn rules_table_prints_empty_group_and_resource_names() {
    let mut named = rule(&[""], &["pods"], &["get"]);
    named.resource_names = vec!["api".to_owned(), "web".to_owned()];
    let lines = table(&[named]);
    assert_eq!(
        lines[1],
        format!("{:<9}  {:<15}  {}", "\"\"", "pods (api, web)", "get")
    );
}

#[test]
fn rules_table_lists_non_resource_urls_last() {
    let mut urls = rule(&[], &[], &["get"]);
    urls.non_resource_urls = vec!["/healthz".to_owned(), "/version".to_owned()];
    let lines = table(&[urls, rule(&[""], &["pods"], &["list"])]);
    assert_eq!(lines.len(), 3);
    assert!(lines[0].starts_with("apiGroups"));
    assert_eq!(lines[2], "nonResourceURLs: /healthz, /version  verbs: get");
}

#[test]
fn rules_table_caps_at_200() {
    let rules = vec![rule(&[""], &["pods"], &["get"]); 203];
    let lines = table(&rules);
    // The header, 200 rules, and the remainder line.
    assert_eq!(lines.len(), 202);
    assert_eq!(lines[201], "… 3 more rules");
    let exact = table(&vec![rule(&[""], &["pods"], &["get"]); 200]);
    assert_eq!(exact.len(), 201);
}

#[test]
fn rules_table_cuts_long_cells() {
    let long = "a".repeat(60);
    let lines = table(&[rule(&[""], &[&long], &["get"])]);
    assert!(
        lines[1].contains(&format!("{}…", "a".repeat(39))),
        "{}",
        lines[1]
    );
}

// ---- ServiceAccounts ----

fn account(name: &str) -> cluster::ServiceAccountSummary {
    cluster::ServiceAccountSummary {
        namespace: "shop".to_owned(),
        name: name.to_owned(),
        created_at: None,
        labels: Vec::new(),
        secrets: Vec::new(),
        image_pull_secrets: Vec::new(),
        automount_token: None,
        cloud_identities: Vec::new(),
    }
}

fn titles(row: &KindRow) -> Vec<&'static str> {
    row.sections.iter().map(|section| section.title).collect()
}

#[test]
fn service_account_row_cells_match_column_count() {
    let row = service_account_row(&account("api"));
    assert_eq!(
        row.cells.len(),
        ResourceKind::ServiceAccounts.columns().len()
    );
    assert_eq!(row.namespace.as_deref(), Some("shop"));
    // The joins fill Bound roles and Used by.
    assert_eq!(row.cells[0], KindCell::Absent);
    assert_eq!(row.cells[1], KindCell::Absent);
    assert_eq!(row.status.text.as_ref(), "Service account");
    assert_eq!(row.status.tone, StatusTone::Ok);
}

#[test]
fn binding_service_account_subject_is_a_link() {
    let row = role_binding_row(&binding(
        Some("shop"),
        (RoleKind::Role, "r"),
        vec![
            subject(SubjectKind::ServiceAccount, Some("shop"), "api"),
            subject(SubjectKind::User, None, "ana"),
        ],
    ));
    let subjects = row.section("Subjects").expect("subjects section");
    assert_eq!(
        subjects.rows[0],
        DetailRow::StackedLink {
            label: "ServiceAccount".into(),
            text: "shop/api".into(),
            target: ResourceKey::Kind {
                kind: ResourceKind::ServiceAccounts,
                namespace: Some("shop".to_owned()),
                name: "api".to_owned(),
            },
        }
    );
    // Only a service account has a screen; a user stays text.
    assert!(matches!(subjects.rows[1], DetailRow::Stacked { .. }));
}

/// The link row of a secret in the test account's namespace.
fn secret_link_row(name: &str, way: &str) -> DetailRow {
    DetailRow::Link {
        label: way.to_owned().into(),
        text: name.to_owned().into(),
        target: ResourceKey::of_object("Secret", Some("shop"), name).expect("a Secrets key"),
    }
}

#[test]
fn service_account_secret_names_link_to_secrets() {
    let mut with_secrets = account("api");
    with_secrets.secrets = vec!["api-token-x1".to_owned()];
    with_secrets.image_pull_secrets = vec!["registry".to_owned()];
    with_secrets.automount_token = Some(false);
    let row = service_account_row(&with_secrets);
    let secrets = row.section("Secrets").expect("secrets section");
    assert_eq!(
        secrets.rows,
        [
            secret_link_row("api-token-x1", "Token reference"),
            secret_link_row("registry", "Image pull secret"),
            DetailRow::field("Automount token", KindCell::Text("No".into())),
            DetailRow::Note("Secret contents are never read".into()),
        ]
    );
    let empty = service_account_row(&account("api"));
    assert_eq!(
        empty.section("Secrets").expect("secrets section").rows[0],
        DetailRow::Note("No secret references".into())
    );
}

#[test]
fn cloud_identity_section_after_bound_roles() {
    let mut irsa = account("api");
    irsa.cloud_identities = vec![cluster::CloudIdentity {
        provider: cluster::CloudProvider::Aws,
        value: "arn:aws:iam::123456789012:role/api".to_owned(),
    }];
    let row = service_account_row(&irsa);
    assert_eq!(
        titles(&row),
        [
            "Bound roles",
            "Cloud identity",
            "Can do",
            "Used by",
            "Secrets"
        ]
    );
    assert_eq!(
        row.section("Cloud identity").expect("section").rows,
        [DetailRow::field(
            "IAM role",
            KindCell::Mono("arn:aws:iam::123456789012:role/api".into())
        )]
    );
}

#[test]
fn no_cloud_identity_section_when_empty() {
    let row = service_account_row(&account("api"));
    assert_eq!(
        titles(&row),
        ["Bound roles", "Can do", "Used by", "Secrets"]
    );
}

#[test]
fn automount_default_text() {
    let text = |value: Option<bool>| {
        let mut account = account("api");
        account.automount_token = value;
        let row = service_account_row(&account);
        let secrets = row.section("Secrets").expect("section").rows.clone();
        secrets
            .into_iter()
            .find_map(|detail| match detail {
                DetailRow::Field {
                    label,
                    value: KindCell::Text(text),
                } if label.as_ref() == "Automount token" => Some(text.to_string()),
                _ => None,
            })
            .expect("automount row")
    };
    assert_eq!(text(None), "Default (yes)");
    assert_eq!(text(Some(true)), "Yes");
    assert_eq!(text(Some(false)), "No");
}

#[test]
fn service_account_sections_put_can_do_after_cloud_identity() {
    let irsa = ServiceAccountSummary {
        cloud_identities: vec![cluster::CloudIdentity {
            provider: cluster::CloudProvider::Aws,
            value: "arn:aws:iam::1:role/x".to_owned(),
        }],
        ..account("api")
    };
    assert_eq!(
        titles(&service_account_row(&irsa)),
        [
            "Bound roles",
            "Cloud identity",
            "Can do",
            "Used by",
            "Secrets"
        ]
    );
    assert_eq!(
        titles(&service_account_row(&account("api"))),
        ["Bound roles", "Can do", "Used by", "Secrets"]
    );
}
