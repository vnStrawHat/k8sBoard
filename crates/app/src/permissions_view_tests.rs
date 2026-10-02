use cluster::{BindingSummary, RbacRule, ResourceRequest, RoleKind, RoleRef, Subject, SubjectKind};

use super::*;

fn denied(reason: Option<&str>) -> AccessDecision {
    AccessDecision::Denied {
        reason: reason.map(str::to_owned),
    }
}

#[test]
fn answer_you_allowed() {
    assert_eq!(
        you_answer(&AccessDecision::Allowed),
        Answer {
            tone: StatusTone::Ok,
            text: "Yes".to_owned()
        }
    );
}

#[test]
fn answer_you_denied_with_reason() {
    let with_reason = you_answer(&denied(Some("RBAC: no rule")));
    assert_eq!(with_reason.tone, StatusTone::Bad);
    assert_eq!(with_reason.text, "No: RBAC: no rule");
}

#[test]
fn answer_you_denied_without_reason() {
    assert_eq!(you_answer(&denied(None)).text, "No");
}

fn other(text: &str) -> SubjectQuery {
    SubjectQuery::Other {
        text: text.to_owned(),
        identity: Identity::user("x"),
        account: None,
    }
}

#[test]
fn user_caveat_names_the_group_the_identity_has() {
    assert_eq!(user_caveat(&other("user ann")), USER_CAVEAT);
    assert_eq!(
        user_caveat(&other("user system:anonymous")),
        ANONYMOUS_CAVEAT
    );
    assert!(ANONYMOUS_CAVEAT.contains("system:unauthenticated"));
    assert_eq!(user_caveat(&other("group devs")), "");
    assert_eq!(user_caveat(&other("sa shop/robot")), "");
    assert_eq!(user_caveat(&SubjectQuery::You), "");
}

#[test]
fn a_failed_table_shows_its_error_for_every_subject() {
    let failed = RequestState::<()>::Failed("denied".to_owned());
    for is_you in [true, false] {
        assert_eq!(
            waiting_line(&failed, is_you),
            Waiting::Bad("denied".to_owned())
        );
    }
}

#[test]
fn a_loading_review_says_so_for_you_only() {
    let loading = RequestState::<()>::Loading {
        _task: gpui_kit::Task::ready(()),
    };
    assert_eq!(
        waiting_line(&loading, true),
        Waiting::Muted("Asking the API server…")
    );
    assert_eq!(waiting_line(&loading, false), Waiting::Snapshot);
}

#[test]
fn an_idle_table_waits_for_the_snapshot_only_for_other_subjects() {
    let idle = RequestState::<()>::Idle;
    assert_eq!(waiting_line(&idle, true), Waiting::Nothing);
    assert_eq!(waiting_line(&idle, false), Waiting::Snapshot);
}

#[test]
fn answer_other_via_first_grant() {
    let answer = other_answer(Some((
        "clusterrolebinding/admins",
        "clusterrole/cluster-admin",
    )));
    assert_eq!(answer.tone, StatusTone::Ok);
    assert_eq!(
        answer.text,
        "Yes · via clusterrolebinding/admins → clusterrole/cluster-admin"
    );
}

#[test]
fn answer_other_denied() {
    let answer = other_answer(None);
    assert_eq!(answer.tone, StatusTone::Bad);
    assert_eq!(answer.text, "No RBAC binding grants this.");
}

#[test]
fn masters_always_answer_yes() {
    assert_eq!(masters_answer().tone, StatusTone::Ok);
    assert!(masters_answer().text.contains("system:masters"));
}

fn review(is_incomplete: bool, error: Option<&str>) -> RulesReview {
    RulesReview {
        rules: Vec::new(),
        is_incomplete,
        evaluation_error: error.map(str::to_owned),
    }
}

#[test]
fn incomplete_note_text() {
    assert_eq!(incomplete_note(&review(false, Some("ignored"))), None);
    assert_eq!(
        incomplete_note(&review(true, Some("webhook down"))).as_deref(),
        Some("Incomplete: rules shown are granted; others may be missing (webhook down)")
    );
    assert_eq!(
        incomplete_note(&review(true, None)).as_deref(),
        Some("Incomplete: rules shown are granted; others may be missing")
    );
}

#[test]
fn you_gets_namespaces_only_and_others_also_all() {
    let listed = vec!["a".to_owned(), "b".to_owned()];
    assert_eq!(
        permission_namespace_options(listed.clone(), Some("c"), true),
        ["c", "a", "b"]
    );
    let others = permission_namespace_options(listed, Some("b"), false);
    assert_eq!(others[0], ALL_NAMESPACES);
    assert_eq!(others[1..], ["a", "b"]);
}

#[test]
fn a_lone_star_group_is_hidden_under_a_star_resource() {
    let row = |group: &str, resource: &str| crate::permission_table::PermissionRow {
        group: group.to_owned(),
        resource: resource.to_owned(),
        cells: std::array::from_fn(|_| VerbCell::Empty),
        other_verbs: Vec::new(),
        named_other_verbs: Vec::new(),
        is_everything: false,
    };
    assert!(!shows_group(&row("*", "*")));
    assert!(!shows_group(&row("", "pods")));
    assert!(shows_group(&row("apps", "*")));
    assert!(shows_group(&row("*", "pods")));
}

#[test]
fn you_namespaces_fall_back_to_scope() {
    let listed = ["a".to_owned(), "b".to_owned()];
    let scope = ["c".to_owned()];
    assert_eq!(you_namespaces(Some(&listed), &scope), ["a", "b"]);
    assert_eq!(you_namespaces(None, &scope), ["c"]);
    assert!(you_namespaces(None, &[]).is_empty());
}

fn binding(name: &str, role: &str) -> BindingSummary {
    BindingSummary {
        namespace: None,
        name: name.to_owned(),
        created_at: None,
        labels: Vec::new(),
        role: RoleRef {
            kind: RoleKind::ClusterRole,
            name: role.to_owned(),
        },
        subjects: Vec::new(),
    }
}

#[test]
fn granted_by_has_one_row_per_binding() {
    let subject = Subject {
        kind: SubjectKind::Group,
        name: "system:serviceaccounts".to_owned(),
        namespace: None,
    };
    let first = binding("b1", "reader");
    let second = binding("b2", "editor");
    let rule = RbacRule {
        api_groups: vec![String::new()],
        resources: vec!["pods".to_owned()],
        resource_names: Vec::new(),
        verbs: vec!["get".to_owned()],
        non_resource_urls: Vec::new(),
    };
    let effective = |binding| EffectiveRule {
        rule: &rule,
        binding,
        subject: &subject,
    };
    let rules = [effective(&first), effective(&first), effective(&second)];
    let rows = granted_by(&rules);
    let texts: Vec<_> = rows.iter().map(|row| row.binding_text.as_str()).collect();
    assert_eq!(texts, ["clusterrolebinding/b1", "clusterrolebinding/b2"]);
    assert_eq!(rows[0].via, "group system:serviceaccounts");
    assert_eq!(rows[0].role_text, "clusterrole/reader");
}

#[test]
fn question_reads_the_request_in_words() {
    let request = AccessRequest {
        verb: "get".to_owned(),
        target: RequestTarget::Resource(ResourceRequest {
            group: "apps".to_owned(),
            resource: "deployments".to_owned(),
            subresource: Some("scale".to_owned()),
            name: Some("web".to_owned()),
            namespace: Some("shop".to_owned()),
        }),
    };
    assert_eq!(
        question_text(&request),
        "get deployments.apps/scale web in shop"
    );
    let url = AccessRequest {
        verb: "get".to_owned(),
        target: RequestTarget::NonResource {
            path: "/healthz".to_owned(),
        },
    };
    assert_eq!(question_text(&url), "get /healthz");
}

#[test]
fn only_a_service_account_gets_its_group_grants() {
    // Regression guard for the identity wiring of `other_table`: a user query lists no group
    // beyond system:authenticated, an account query lists its account groups too.
    let subject = SubjectQuery::Other {
        text: "user ann".to_owned(),
        identity: Identity::user("ann"),
        account: None,
    };
    let snapshot = RbacSnapshot {
        roles: Vec::new(),
        cluster_roles: Vec::new(),
        role_bindings: Vec::new(),
        cluster_role_bindings: Vec::new(),
        coverage: cluster::RbacCoverage {
            cluster_roles: true,
            cluster_bindings: true,
            roles: cluster::NamespaceCoverage::AllNamespaces,
            role_bindings: cluster::NamespaceCoverage::AllNamespaces,
        },
    };
    let SubjectQuery::Other { identity, .. } = &subject else {
        panic!("other");
    };
    let shown = other_table(
        &snapshot,
        jiff::Timestamp::UNIX_EPOCH,
        &subject,
        identity,
        Some("shop"),
    );
    assert!(shown.table.rows.is_empty());
    assert_eq!(shown.scope_text, "in shop");
    assert!(shown.caveats.ends_with("system:authenticated count."));
}

#[test]
fn masters_table_has_no_rules() {
    let shown = masters_table();
    assert!(shown.is_masters);
    assert!(shown.table.rows.is_empty() && shown.granted_by.is_empty());
}

#[test]
fn cluster_wide_scope_reads_cluster_wide() {
    assert_eq!(scope_text(None), "cluster-wide");
    assert_eq!(scope_text(Some("shop")), "in shop");
}
