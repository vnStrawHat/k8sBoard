//! Row builders for the access-control kinds: Roles, ClusterRoles, RoleBindings,
//! ClusterRoleBindings, and ServiceAccounts. The joined cells (binding counts, Bound roles, Used
//! by) are filled later (`kind_join`).

use cluster::{BindingSummary, RbacRule, RoleSummary, ServiceAccountSummary, Subject, SubjectKind};

use crate::access_bindings::{
    BroadAdmin, broad_admin, role_key, service_account_text, subject_text,
};
use crate::kind_row::{
    DetailRow, DetailSection, KindCell, KindObject, KindRow, LiveContent, chips,
};
use crate::status_tone::{StatusLabel, StatusTone};
use crate::table_selection::ResourceKey;

/// The most rules a drawer prints; the rest is a count.
const MAX_RULES: usize = 200;
/// A rules table cell is cut here so a long list does not push the columns off the drawer.
const MAX_CELL_CHARS: usize = 40;
const RULES_HEADER: [&str; 3] = ["apiGroups", "resources", "verbs"];
/// The verbs column wraps at this width; the drawer shows about 65 mono characters.
const VERBS_LINE_CHARS: usize = 28;

fn labeled(text: impl Into<gpui_kit::SharedString>, tone: StatusTone) -> StatusLabel {
    StatusLabel {
        text: text.into(),
        tone,
    }
}

fn yes_no(is_yes: bool) -> KindCell {
    KindCell::Text(if is_yes { "Yes" } else { "No" }.into())
}

fn count_noun(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

// ---- Roles and ClusterRoles ----

/// Full access beats everything else; a role without rules grants nothing, which is rarely meant.
fn role_status(role: &RoleSummary) -> StatusLabel {
    let rules = role.rules.len();
    if role.grants_everything() {
        return labeled("Full access", StatusTone::Warn);
    }
    if role.is_aggregated() {
        return labeled(
            format!("Aggregated · {}", count_noun(rules, "rule", "rules")),
            StatusTone::Ok,
        );
    }
    if rules == 0 {
        return labeled("No rules", StatusTone::Warn);
    }
    labeled(count_noun(rules, "rule", "rules"), StatusTone::Ok)
}

fn rules_cell(role: &RoleSummary) -> KindCell {
    let all = if role.grants_everything() {
        " (all)"
    } else {
        ""
    };
    KindCell::Text(format!("{}{all}", role.rules.len()).into())
}

fn rules_rows(role: &RoleSummary) -> Vec<DetailRow> {
    if role.rules.is_empty() {
        return vec![DetailRow::Note("No rules".into())];
    }
    vec![DetailRow::Table(rules_table(&role.rules).into())]
}

pub(crate) fn role_row(role: &RoleSummary) -> KindRow {
    let status = role_status(role);
    KindRow {
        namespace: role.namespace.clone(),
        name: role.name.clone(),
        created_at: role.created_at,
        status,
        cells: vec![
            rules_cell(role),
            // The Bindings companion join fills the binding count.
            KindCell::Absent,
            KindCell::age(role.created_at),
        ],
        sections: vec![
            DetailSection {
                title: "Rules",
                rows: rules_rows(role),
            },
            DetailSection {
                title: "Bindings",
                rows: vec![DetailRow::Live(LiveContent::RoleBindings)],
            },
        ],
        event: None,
        related_pods: None,
        labels: chips(&role.labels),
        object: KindObject::Role(role.clone()),
    }
}

pub(crate) fn cluster_role_row(role: &RoleSummary) -> KindRow {
    let status = role_status(role);
    let mut sections = vec![DetailSection {
        title: "Cluster role",
        rows: vec![
            DetailRow::field("Built-in", yes_no(role.is_built_in())),
            DetailRow::field("Aggregated", yes_no(role.is_aggregated())),
        ],
    }];
    if role.is_aggregated() {
        let mut rows: Vec<DetailRow> = role
            .aggregation
            .iter()
            .map(|selector| DetailRow::Chips(chips(&selector.terms())))
            .collect();
        rows.push(DetailRow::Note(
            "Rules are combined from ClusterRoles with these labels".into(),
        ));
        sections.push(DetailSection {
            title: "Aggregation",
            rows,
        });
    }
    sections.push(DetailSection {
        title: "Rules",
        rows: rules_rows(role),
    });
    sections.push(DetailSection {
        title: "Bound to",
        rows: vec![DetailRow::Live(LiveContent::RoleSubjects)],
    });
    KindRow {
        namespace: None,
        name: role.name.clone(),
        created_at: role.created_at,
        status,
        cells: vec![
            rules_cell(role),
            yes_no(role.is_aggregated()),
            // The Bindings companion join fills the binding count.
            KindCell::Absent,
            KindCell::age(role.created_at),
        ],
        sections,
        event: None,
        related_pods: None,
        labels: chips(&role.labels),
        object: KindObject::Role(role.clone()),
    }
}

// ---- RoleBindings and ClusterRoleBindings ----

fn binding_status(binding: &BindingSummary) -> StatusLabel {
    match broad_admin(binding) {
        Some(BroadAdmin::Everyone) => labeled("cluster-admin to everyone", StatusTone::Bad),
        Some(BroadAdmin::ServiceAccounts) => {
            labeled("cluster-admin to service accounts", StatusTone::Warn)
        }
        None if binding.subjects.is_empty() => labeled("No subjects", StatusTone::Warn),
        None => labeled(
            count_noun(binding.subjects.len(), "subject", "subjects"),
            StatusTone::Ok,
        ),
    }
}

/// The role cell: toned as the status when the binding is a broad cluster-admin grant.
fn role_cell(binding: &BindingSummary, text: String) -> KindCell {
    match broad_admin(binding) {
        Some(_) => KindCell::Toned(labeled(text, binding_status(binding).tone)),
        None => KindCell::Text(text.into()),
    }
}

fn subjects_cell(binding: &BindingSummary) -> KindCell {
    let texts: Vec<String> = binding.subjects.iter().map(subject_text).collect();
    if texts.is_empty() {
        return KindCell::Absent;
    }
    KindCell::Text(texts.join(", ").into())
}

fn subject_row(subject: &Subject) -> DetailRow {
    let label = match subject.kind {
        SubjectKind::User => "User",
        SubjectKind::Group => "Group",
        SubjectKind::ServiceAccount => "ServiceAccount",
    };
    let text = match subject.kind {
        SubjectKind::ServiceAccount => service_account_text(subject),
        SubjectKind::User | SubjectKind::Group => subject.name.clone(),
    };
    let target = (subject.kind == SubjectKind::ServiceAccount)
        .then(|| {
            ResourceKey::of_object(
                "ServiceAccount",
                subject.namespace.as_deref(),
                &subject.name,
            )
        })
        .flatten();
    // Stacked: a service account reads `namespace/name`, which does not fit beside its label.
    match target {
        Some(target) => DetailRow::StackedLink {
            label: label.into(),
            text: text.into(),
            target,
        },
        None => DetailRow::stacked(label, KindCell::Mono(text.into())),
    }
}

fn binding_sections(binding: &BindingSummary) -> Vec<DetailSection> {
    let name_row = match role_key(binding) {
        Some(target) => DetailRow::Link {
            label: "Name".into(),
            text: binding.role.name.clone().into(),
            target,
        },
        None => DetailRow::field("Name", KindCell::Text(binding.role.name.clone().into())),
    };
    let subject_rows = if binding.subjects.is_empty() {
        vec![DetailRow::Note("No subjects".into())]
    } else {
        binding.subjects.iter().map(subject_row).collect()
    };
    vec![
        DetailSection {
            title: "Role",
            rows: vec![
                DetailRow::field("Kind", KindCell::Text(binding.role.kind.to_string().into())),
                name_row,
            ],
        },
        DetailSection {
            title: "Subjects",
            rows: subject_rows,
        },
    ]
}

fn binding_row(binding: &BindingSummary, role_cell: KindCell) -> KindRow {
    KindRow {
        namespace: binding.namespace.clone(),
        name: binding.name.clone(),
        created_at: binding.created_at,
        status: binding_status(binding),
        cells: vec![
            role_cell,
            subjects_cell(binding),
            KindCell::age(binding.created_at),
        ],
        sections: binding_sections(binding),
        event: None,
        related_pods: None,
        labels: chips(&binding.labels),
        object: KindObject::Binding(binding.clone()),
    }
}

pub(crate) fn role_binding_row(binding: &BindingSummary) -> KindRow {
    let text = format!("{}/{}", binding.role.kind, binding.role.name);
    binding_row(binding, role_cell(binding, text))
}

pub(crate) fn cluster_role_binding_row(binding: &BindingSummary) -> KindRow {
    binding_row(binding, role_cell(binding, binding.role.name.clone()))
}

// ---- ServiceAccounts ----

fn automount_text(account: &ServiceAccountSummary) -> &'static str {
    match account.automount_token {
        Some(true) => "Yes",
        Some(false) => "No",
        None => "Default (yes)",
    }
}

/// Secret references are names only: no value is read here. Each name links to its Secrets row.
fn secrets_rows(account: &ServiceAccountSummary) -> Vec<DetailRow> {
    let references = account
        .secrets
        .iter()
        .map(|name| (name, "Token reference"))
        .chain(
            account
                .image_pull_secrets
                .iter()
                .map(|name| (name, "Image pull secret")),
        );
    let mut rows: Vec<DetailRow> = references
        .map(|(name, way)| secret_reference_row(&account.namespace, name, way))
        .collect();
    let has_references = !rows.is_empty();
    if !has_references {
        rows.push(DetailRow::Note("No secret references".into()));
    }
    rows.push(DetailRow::field(
        "Automount token",
        KindCell::Text(automount_text(account).into()),
    ));
    if has_references {
        rows.push(DetailRow::Note("Secret contents are never read".into()));
    }
    rows
}

/// How the account references a secret, as a label above a link to the secret's row.
fn secret_reference_row(namespace: &str, name: &str, way: &str) -> DetailRow {
    match ResourceKey::of_object("Secret", Some(namespace), name) {
        Some(target) => DetailRow::Link {
            label: way.to_owned().into(),
            text: name.to_owned().into(),
            target,
        },
        None => DetailRow::field(way.to_owned(), KindCell::Mono(name.to_owned().into())),
    }
}

pub(crate) fn service_account_row(account: &ServiceAccountSummary) -> KindRow {
    let mut sections = vec![DetailSection {
        title: "Bound roles",
        rows: vec![DetailRow::Live(LiveContent::BoundRoles)],
    }];
    if !account.cloud_identities.is_empty() {
        sections.push(DetailSection {
            title: "Cloud identity",
            rows: account
                .cloud_identities
                .iter()
                .map(|identity| {
                    DetailRow::field(
                        identity.provider.label(),
                        KindCell::Mono(identity.value.clone().into()),
                    )
                })
                .collect(),
        });
    }
    sections.push(DetailSection {
        title: "Can do",
        rows: vec![DetailRow::Live(LiveContent::CanDo)],
    });
    sections.push(DetailSection {
        title: "Used by",
        rows: vec![DetailRow::Live(LiveContent::ServiceAccountPods)],
    });
    sections.push(DetailSection {
        title: "Secrets",
        rows: secrets_rows(account),
    });
    KindRow {
        namespace: Some(account.namespace.clone()),
        name: account.name.clone(),
        created_at: account.created_at,
        status: service_account_status(),
        cells: vec![
            // The Bindings companion join fills Bound roles, and the pods join fills Used by.
            KindCell::Absent,
            KindCell::Absent,
            KindCell::age(account.created_at),
        ],
        sections,
        event: None,
        related_pods: None,
        labels: chips(&account.labels),
        object: KindObject::ServiceAccount(account.clone()),
    }
}

/// The status before the joins.
pub(crate) fn service_account_status() -> StatusLabel {
    labeled("Service account", StatusTone::Ok)
}

// ---- Rules table ----

/// A cell cut to `MAX_CELL_CHARS` with an ellipsis.
fn cut(text: String) -> String {
    if text.chars().count() <= MAX_CELL_CHARS {
        return text;
    }
    let kept: String = text.chars().take(MAX_CELL_CHARS - 1).collect();
    format!("{kept}…")
}

fn group_cell(groups: &[String]) -> String {
    let shown: Vec<&str> = groups
        .iter()
        .map(|group| if group.is_empty() { "\"\"" } else { group })
        .collect();
    cut(shown.join(", "))
}

fn resource_cell(rule: &RbacRule) -> String {
    let mut text = rule.resources.join(", ");
    if !rule.resource_names.is_empty() {
        text.push_str(&format!(" ({})", rule.resource_names.join(", ")));
    }
    cut(text)
}

/// The verbs in lines of about `VERBS_LINE_CHARS`, broken after a comma, so a long list wraps in
/// the drawer instead of running off its edge. A rule without verbs still takes one empty line.
fn wrap_verbs(verbs: &[String]) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for (index, verb) in verbs.iter().enumerate() {
        let word = if index + 1 < verbs.len() {
            format!("{verb},")
        } else {
            verb.clone()
        };
        match lines.last_mut() {
            Some(line) if line.chars().count() + 1 + word.chars().count() <= VERBS_LINE_CHARS => {
                line.push(' ');
                line.push_str(&word);
            }
            _ => lines.push(word),
        }
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

/// The rules as an aligned text table: resource rules under an `apiGroups resources verbs`
/// header, then one line per non-resource rule.
pub(crate) fn rules_table(rules: &[RbacRule]) -> String {
    let shown = &rules[..rules.len().min(MAX_RULES)];
    let (urls, resources): (Vec<&RbacRule>, Vec<&RbacRule>) = shown
        .iter()
        .partition(|rule| rule.resources.is_empty() && !rule.non_resource_urls.is_empty());
    let mut lines = Vec::new();
    if !resources.is_empty() {
        let mut table: Vec<[String; 2]> =
            vec![[RULES_HEADER[0].to_owned(), RULES_HEADER[1].to_owned()]];
        table.extend(
            resources
                .iter()
                .map(|rule| [group_cell(&rule.api_groups), resource_cell(rule)]),
        );
        let widths: [usize; 2] = std::array::from_fn(|column| {
            table
                .iter()
                .map(|cells| cells[column].chars().count())
                .max()
                .unwrap_or(0)
        });
        let verbs = std::iter::once(vec![RULES_HEADER[2].to_owned()])
            .chain(resources.iter().map(|rule| wrap_verbs(&rule.verbs)));
        for ([groups, resources], verbs) in table.iter().zip(verbs) {
            for (index, verbs) in verbs.iter().enumerate() {
                // A wrapped line leaves the first two columns empty.
                let (groups, resources) = if index == 0 {
                    (groups.as_str(), resources.as_str())
                } else {
                    ("", "")
                };
                lines.push(format!(
                    "{groups:<width_0$}  {resources:<width_1$}  {verbs}",
                    width_0 = widths[0],
                    width_1 = widths[1]
                ));
            }
        }
    }
    lines.extend(urls.iter().map(|rule| {
        format!(
            "nonResourceURLs: {}  verbs: {}",
            cut(rule.non_resource_urls.join(", ")),
            cut(rule.verbs.join(", "))
        )
    }));
    let hidden = rules.len() - shown.len();
    if hidden > 0 {
        lines.push(format!("… {hidden} more rules"));
    }
    lines.join("\n")
}

#[cfg(test)]
#[path = "access_rows_tests.rs"]
mod access_rows_tests;
