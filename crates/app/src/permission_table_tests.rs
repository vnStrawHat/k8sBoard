use super::*;

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

fn table(rules: &[RbacRule]) -> PermissionTable {
    permission_table(rules)
}

fn keys(table: &PermissionTable) -> Vec<(&str, &str)> {
    table
        .rows
        .iter()
        .map(|row| (row.group.as_str(), row.resource.as_str()))
        .collect()
}

#[test]
fn expands_groups_times_resources() {
    let result = table(&[rule(&["", "apps"], &["pods", "deployments"], &["get"])]);
    assert_eq!(
        keys(&result),
        [
            ("", "deployments"),
            ("", "pods"),
            ("apps", "deployments"),
            ("apps", "pods")
        ]
    );
}

#[test]
fn wildcard_verb_fills_all_cells_and_other() {
    let result = table(&[rule(&[""], &["pods"], &["*"])]);
    let row = &result.rows[0];
    assert!(row.cells.iter().all(|cell| *cell == VerbCell::All));
    assert_eq!(row.other_verbs, ["*"]);
}

#[test]
fn unknown_verbs_go_to_other_sorted() {
    let result = table(&[rule(&[""], &["pods"], &["proxy", "get", "bind", "proxy"])]);
    let row = &result.rows[0];
    assert_eq!(row.other_verbs, ["bind", "proxy"]);
    assert_eq!(row.cells[0], VerbCell::All);
    assert_eq!(row.cells[1], VerbCell::Empty);
}

#[test]
fn resource_names_make_names_cell() {
    let result = table(&[
        named(rule(&[""], &["secrets"], &["get"]), &["web", "db"]),
        named(rule(&[""], &["secrets"], &["get"]), &["db", "cache"]),
    ]);
    assert_eq!(
        result.rows[0].cells[0],
        VerbCell::Names(texts(&["cache", "db", "web"]))
    );
}

#[test]
fn all_wins_over_names() {
    let result = table(&[
        named(rule(&[""], &["secrets"], &["get"]), &["web"]),
        rule(&[""], &["secrets"], &["get"]),
        named(rule(&[""], &["secrets"], &["get"]), &["db"]),
    ]);
    assert_eq!(result.rows[0].cells[0], VerbCell::All);
}

#[test]
fn everything_row_first() {
    let result = table(&[
        rule(&[""], &["pods"], &["get"]),
        rule(&["*"], &["*"], &["*"]),
    ]);
    assert_eq!(keys(&result), [("*", "*"), ("", "pods")]);
    assert!(result.rows[0].is_everything);
    assert!(!result.rows[1].is_everything);
}

#[test]
fn everything_needs_all_three_wildcards() {
    let result = table(&[
        rule(&["*"], &["*"], &["get"]),
        rule(&["apps"], &["*"], &["*"]),
    ]);
    assert!(result.rows.iter().all(|row| !row.is_everything));
}

#[test]
fn core_group_sorts_first() {
    let result = table(&[
        rule(&["apps"], &["deployments"], &["get"]),
        rule(&[""], &["pods"], &["get"]),
    ]);
    assert_eq!(keys(&result), [("", "pods"), ("apps", "deployments")]);
}

#[test]
fn subresource_rows_separate() {
    let result = table(&[rule(&[""], &["pods", "pods/log"], &["get"])]);
    assert_eq!(keys(&result), [("", "pods"), ("", "pods/log")]);
}

#[test]
fn url_rules_merge_by_url() {
    let url_rule = |urls: &[&str], verbs: &[&str]| RbacRule {
        non_resource_urls: texts(urls),
        ..rule(&[], &[], verbs)
    };
    let result = table(&[
        url_rule(&["/healthz", "/version"], &["get"]),
        url_rule(&["/healthz"], &["post"]),
    ]);
    assert!(result.rows.is_empty());
    let urls: Vec<_> = result
        .url_rows
        .iter()
        .map(|row| (row.url.as_str(), row.verbs.clone()))
        .collect();
    assert_eq!(
        urls,
        [
            ("/healthz", texts(&["get", "post"])),
            ("/version", texts(&["get"]))
        ]
    );
}

#[test]
fn caps_at_300_rows() {
    let resources: Vec<String> = (0..305).map(|index| format!("r{index:03}")).collect();
    let many = RbacRule {
        resources,
        ..rule(&[""], &[], &["get"])
    };
    let result = table(&[many]);
    assert_eq!(result.rows.len(), 300);
    assert_eq!(result.hidden_rows, 5);
}

fn chip_texts(table: &PermissionTable) -> Vec<String> {
    can_do_chips(table)
        .chips
        .into_iter()
        .map(|(text, _)| text.to_string())
        .collect()
}

#[test]
fn chip_text_lists_verbs_in_column_order() {
    let result = table(&[rule(&[""], &["pods"], &["list", "get", "bind"])]);
    assert_eq!(chip_texts(&result), ["get, list, bind pods"]);
    let grouped = table(&[rule(&["apps"], &["deployments"], &["patch", "update"])]);
    assert_eq!(chip_texts(&grouped), ["update, patch deployments.apps"]);
}

#[test]
fn chip_all_verbs_and_all_resources() {
    let all_verbs = table(&[rule(&[""], &["pods"], &["*"])]);
    assert_eq!(chip_texts(&all_verbs), ["all verbs pods"]);
    let all_resources = table(&[rule(&["apps"], &["*"], &["get"])]);
    assert_eq!(chip_texts(&all_resources), ["get all resources"]);
}

#[test]
fn chip_named_suffix() {
    let result = table(&[named(rule(&[""], &["secrets"], &["get"]), &["web"])]);
    assert_eq!(chip_texts(&result), ["get secrets (named)"]);
}

#[test]
fn other_verb_names_make_the_row_named() {
    let result = table(&[named(rule(&[""], &["users"], &["impersonate"]), &["alice"])]);
    let row = &result.rows[0];
    assert_eq!(row.other_verbs, ["impersonate"]);
    assert_eq!(row.named_other_verbs, ["impersonate"]);
    assert_eq!(row.other_text(), "impersonate (named)");
    assert_eq!(chip_texts(&result), ["impersonate users (named)"]);
}

#[test]
fn unrestricted_other_verb_is_not_named() {
    let result = table(&[rule(&[""], &["users"], &["impersonate"])]);
    let row = &result.rows[0];
    assert!(row.named_other_verbs.is_empty());
    assert_eq!(row.other_text(), "impersonate");
    assert_eq!(chip_texts(&result), ["impersonate users"]);
}

#[test]
fn unrestricted_other_verb_wins_over_names() {
    let result = table(&[
        named(rule(&[""], &["users"], &["impersonate"]), &["alice"]),
        rule(&[""], &["users"], &["impersonate"]),
    ]);
    assert!(result.rows[0].named_other_verbs.is_empty());
}

#[test]
fn one_unrestricted_verb_keeps_the_row_unnamed() {
    let result = table(&[
        named(rule(&[""], &["pods"], &["get"]), &["web"]),
        rule(&[""], &["pods"], &["bind"]),
    ]);
    assert_eq!(chip_texts(&result), ["get, bind pods"]);
}

#[test]
fn chip_everything_is_warn() {
    let result = table(&[rule(&["*"], &["*"], &["*"])]);
    let chips = can_do_chips(&result);
    assert_eq!(chips.chips, [("everything".into(), Some(StatusTone::Warn))]);
}

#[test]
fn chips_cap_at_12_with_more() {
    let resources: Vec<String> = (0..15).map(|index| format!("r{index:02}")).collect();
    let many = RbacRule {
        resources,
        ..rule(&[""], &[], &["get"])
    };
    let chips = can_do_chips(&table(&[many]));
    assert_eq!(chips.chips.len(), 12);
    assert_eq!(chips.more, 3);
}
