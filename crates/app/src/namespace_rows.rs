//! The Namespaces row builder.

use cluster::{NamespaceDeletionCondition, NamespacePhase, NamespaceSummary};

use crate::kind_row::{
    DetailRow, DetailSection, KindCell, KindObject, KindRow, LiveContent, chips,
};
use crate::status_tone::{StatusLabel, StatusTone};

pub(crate) fn namespace_row(namespace: &NamespaceSummary) -> KindRow {
    let status = phase_label(namespace.phase);
    let age = KindCell::Age {
        at: namespace.created_at,
        tone: None,
    };
    KindRow {
        namespace: None,
        name: namespace.name.clone(),
        created_at: namespace.created_at,
        status: status.clone(),
        cells: vec![
            KindCell::Toned(status.clone()),
            // The pods join fills the Pods, CPU req, and Memory req cells.
            KindCell::Absent,
            KindCell::Absent,
            KindCell::Absent,
            age.clone(),
        ],
        sections: namespace_sections(namespace, status, age),
        event: None,
        related_pods: None,
        labels: chips(&namespace.labels),
        object: KindObject::Namespace(namespace.clone()),
    }
}

/// Namespace, then Remaining resources while it is Terminating, then Quota.
fn namespace_sections(
    namespace: &NamespaceSummary,
    status: StatusLabel,
    age: KindCell,
) -> Vec<DetailSection> {
    let mut sections = vec![DetailSection {
        title: "Namespace",
        rows: vec![
            DetailRow::field("Status", KindCell::Toned(status)),
            DetailRow::field("Created", age),
        ],
    }];
    if namespace.phase == NamespacePhase::Terminating {
        sections.push(DetailSection {
            title: "Remaining resources",
            rows: remaining_rows(&namespace.deletion_conditions),
        });
    }
    sections.push(DetailSection {
        title: "Quota",
        rows: vec![DetailRow::Live(LiveContent::NamespaceQuotas)],
    });
    sections
}

/// A namespace that has been Terminating for longer than this is STUCK.
pub(crate) const STUCK_AFTER: jiff::SignedDuration = jiff::SignedDuration::from_mins(5);

/// One thing the namespace controller says still blocks the deletion.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RemainingEntry {
    Resource {
        resource: String,
        count: u64,
    },
    Finalizer {
        finalizer: String,
        count: u64,
    },
    /// A message this build does not parse, kept as the API reports it.
    Unparsed(String),
}

const CONTENT_REMAINING: &str = "NamespaceContentRemaining";
const FINALIZERS_REMAINING: &str = "NamespaceFinalizersRemaining";
const INSTANCES_SUFFIX: &str = " resource instances";

/// The blockers named by the deletion conditions: resource types from `NamespaceContentRemaining`
/// (`pods. has 2 resource instances`), finalizers from `NamespaceFinalizersRemaining`
/// (`strimzi.io/topic-operator in 1 resource instances`), and every other message, or an item that
/// does not match, as `Unparsed`. A condition without a message says nothing.
pub(crate) fn remaining_entries(conditions: &[NamespaceDeletionCondition]) -> Vec<RemainingEntry> {
    let mut entries = Vec::new();
    for condition in conditions {
        let Some(message) = condition.message.as_deref() else {
            continue;
        };
        match condition.name.as_str() {
            CONTENT_REMAINING | FINALIZERS_REMAINING => {
                // The text after the first `: ` lists the items, separated by `, `.
                let Some((_, items)) = message.split_once(": ") else {
                    entries.push(RemainingEntry::Unparsed(message.to_owned()));
                    continue;
                };
                let is_resources = condition.name == CONTENT_REMAINING;
                for item in items.split(", ") {
                    entries.push(
                        remaining_item(item, is_resources)
                            .unwrap_or_else(|| RemainingEntry::Unparsed(item.to_owned())),
                    );
                }
            }
            _ => entries.push(RemainingEntry::Unparsed(message.to_owned())),
        }
    }
    entries
}

/// `{name} has {n} resource instances` or `{name} in {n} resource instances`.
fn remaining_item(item: &str, is_resource: bool) -> Option<RemainingEntry> {
    let counted = item.strip_suffix(INSTANCES_SUFFIX)?;
    let separator = if is_resource { " has " } else { " in " };
    let (name, count) = counted.rsplit_once(separator)?;
    let count = count.parse().ok()?;
    if is_resource {
        // The core group reads `pods.`: nothing follows the dot.
        let resource = name.trim_end_matches('.').to_owned();
        return Some(RemainingEntry::Resource { resource, count });
    }
    Some(RemainingEntry::Finalizer {
        finalizer: name.to_owned(),
        count,
    })
}

/// The Remaining resources section of a Terminating namespace.
fn remaining_rows(conditions: &[NamespaceDeletionCondition]) -> Vec<DetailRow> {
    let entries = remaining_entries(conditions);
    // Messages this build cannot parse are what the STUCK box already shows; repeating them here
    // would only say it twice.
    let only_unparsed = entries
        .iter()
        .all(|entry| matches!(entry, RemainingEntry::Unparsed(_)));
    if only_unparsed {
        return vec![DetailRow::Note(
            "The namespace reports no remaining content.".into(),
        )];
    }
    entries
        .into_iter()
        .map(|entry| match entry {
            RemainingEntry::Resource { resource, count } => DetailRow::field(
                resource,
                KindCell::Text(format!("{count} remaining").into()),
            ),
            RemainingEntry::Finalizer { finalizer, count } => DetailRow::field(
                format!("finalizer {finalizer}"),
                KindCell::Toned(StatusLabel {
                    text: format!("{count} objects").into(),
                    tone: StatusTone::Warn,
                }),
            ),
            RemainingEntry::Unparsed(text) => DetailRow::Note(text.into()),
        })
        .collect()
}

fn phase_label(phase: NamespacePhase) -> StatusLabel {
    let (text, tone) = match phase {
        NamespacePhase::Active => ("Active", StatusTone::Ok),
        NamespacePhase::Terminating => ("Terminating", StatusTone::Info),
        NamespacePhase::Unknown => ("Unknown", StatusTone::Warn),
    };
    StatusLabel {
        text: text.into(),
        tone,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resource_kind::ResourceKind;

    fn summary(phase: NamespacePhase) -> NamespaceSummary {
        NamespaceSummary {
            name: "team-a".to_owned(),
            phase,
            labels: vec!["env=dev".to_owned()],
            created_at: None,
            deleting_since: None,
            deletion_conditions: Vec::new(),
        }
    }

    #[test]
    fn namespace_row_cells_match_column_count() {
        let row = namespace_row(&summary(NamespacePhase::Active));
        assert_eq!(row.cells.len(), ResourceKind::Namespaces.columns().len());
        assert_eq!(row.namespace, None);
        assert_eq!(row.labels, ["env=dev"]);
    }

    #[test]
    fn namespace_status_tone_follows_phase() {
        let tone = |phase| namespace_row(&summary(phase)).status.tone;
        assert_eq!(tone(NamespacePhase::Active), StatusTone::Ok);
        assert_eq!(tone(NamespacePhase::Terminating), StatusTone::Info);
        assert_eq!(tone(NamespacePhase::Unknown), StatusTone::Warn);
    }

    #[test]
    fn namespace_row_has_quota_section() {
        let row = namespace_row(&summary(NamespacePhase::Active));
        let titles: Vec<&str> = row.sections.iter().map(|section| section.title).collect();
        assert_eq!(titles, ["Namespace", "Quota"]);
        assert_eq!(
            row.section("Quota").map(|section| section.rows.as_slice()),
            Some([DetailRow::Live(LiveContent::NamespaceQuotas)].as_slice())
        );
    }

    fn condition(name: &str, message: Option<&str>) -> NamespaceDeletionCondition {
        NamespaceDeletionCondition {
            name: name.to_owned(),
            reason: None,
            message: message.map(str::to_owned),
        }
    }

    fn terminating(conditions: Vec<NamespaceDeletionCondition>) -> NamespaceSummary {
        NamespaceSummary {
            phase: NamespacePhase::Terminating,
            deleting_since: jiff::Timestamp::from_second(1_000).ok(),
            deletion_conditions: conditions,
            ..summary(NamespacePhase::Terminating)
        }
    }

    #[test]
    fn remaining_resources_parse_content_and_finalizers() {
        let entries = remaining_entries(&[
            condition(
                "NamespaceContentRemaining",
                Some(
                    "Some resources are remaining: kafkatopics.kafka.strimzi.io has 1 resource instances, pods. has 2 resource instances",
                ),
            ),
            condition(
                "NamespaceFinalizersRemaining",
                Some(
                    "Some content in the namespace has finalizers remaining: strimzi.io/topic-operator in 1 resource instances",
                ),
            ),
        ]);
        assert_eq!(
            entries,
            [
                RemainingEntry::Resource {
                    resource: "kafkatopics.kafka.strimzi.io".to_owned(),
                    count: 1
                },
                // The core group reads `pods.`: the dot is trimmed.
                RemainingEntry::Resource {
                    resource: "pods".to_owned(),
                    count: 2
                },
                RemainingEntry::Finalizer {
                    finalizer: "strimzi.io/topic-operator".to_owned(),
                    count: 1
                },
            ]
        );
    }

    #[test]
    fn unparsed_messages_become_notes() {
        let conditions = [
            condition(
                "NamespaceDeletionDiscoveryFailure",
                Some("Discovery failed for some groups"),
            ),
            condition(
                "NamespaceContentRemaining",
                Some("Some resources are remaining: pods. has 2 resource instances, odd item"),
            ),
            condition("NamespaceContentRemaining", Some("no separator here")),
            condition("NamespaceDeletionContentFailure", None),
        ];
        assert_eq!(
            remaining_entries(&conditions),
            [
                RemainingEntry::Unparsed("Discovery failed for some groups".to_owned()),
                RemainingEntry::Resource {
                    resource: "pods".to_owned(),
                    count: 2
                },
                RemainingEntry::Unparsed("odd item".to_owned()),
                RemainingEntry::Unparsed("no separator here".to_owned()),
            ]
        );
    }

    #[test]
    fn remaining_resources_section_only_while_terminating() {
        let active = namespace_row(&summary(NamespacePhase::Active));
        assert!(active.section("Remaining resources").is_none());
        let row = namespace_row(&terminating(vec![
            condition(
                "NamespaceContentRemaining",
                Some("Some resources are remaining: pods. has 2 resource instances"),
            ),
            condition(
                "NamespaceFinalizersRemaining",
                Some("Some content has finalizers remaining: a.io/f in 3 resource instances"),
            ),
        ]));
        let titles: Vec<&str> = row.sections.iter().map(|section| section.title).collect();
        assert_eq!(titles, ["Namespace", "Remaining resources", "Quota"]);
        let rows = &row.section("Remaining resources").expect("section").rows;
        assert_eq!(
            rows[0],
            DetailRow::field("pods", KindCell::Text("2 remaining".into()))
        );
        assert_eq!(
            rows[1],
            DetailRow::field(
                "finalizer a.io/f",
                KindCell::Toned(StatusLabel {
                    text: "3 objects".into(),
                    tone: StatusTone::Warn
                })
            )
        );
    }

    #[test]
    fn unparsed_only_messages_are_left_to_the_stuck_box() {
        let row = namespace_row(&terminating(vec![condition(
            "NamespaceDeletionDiscoveryFailure",
            Some("Discovery failed for some groups"),
        )]));
        let rows = &row.section("Remaining resources").expect("section").rows;
        assert_eq!(
            rows,
            &[DetailRow::Note(
                "The namespace reports no remaining content.".into()
            )]
        );
        // A parsed entry beside an unparsed one still lists both.
        let mixed = namespace_row(&terminating(vec![
            condition(
                "NamespaceDeletionDiscoveryFailure",
                Some("Discovery failed"),
            ),
            condition(
                "NamespaceContentRemaining",
                Some("Some resources are remaining: pods. has 2 resource instances"),
            ),
        ]));
        let rows = &mixed.section("Remaining resources").expect("section").rows;
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn terminating_without_conditions_reports_no_remaining_content() {
        let row = namespace_row(&terminating(Vec::new()));
        assert_eq!(
            row.section("Remaining resources")
                .map(|section| section.rows.as_slice()),
            Some(
                [DetailRow::Note(
                    "The namespace reports no remaining content.".into()
                )]
                .as_slice()
            )
        );
        assert!(matches!(row.object, KindObject::Namespace(_)));
    }
}
