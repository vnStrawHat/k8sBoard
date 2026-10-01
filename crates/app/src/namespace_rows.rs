//! The Namespaces row builder.

use cluster::{NamespacePhase, NamespaceSummary};

use crate::kind_row::{DetailRow, DetailSection, KindCell, KindRow, chips};
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
        cells: vec![KindCell::Toned(status.clone()), age.clone()],
        sections: vec![DetailSection {
            title: "Namespace",
            rows: vec![
                DetailRow::field("Status", KindCell::Toned(status)),
                DetailRow::field("Created", age),
            ],
        }],
        related_pods: None,
        labels: chips(&namespace.labels),
    }
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
}
