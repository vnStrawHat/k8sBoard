//! The ConfigMap row builder. The row holds the summary (key names and sizes, never values); the
//! drawer's Data section shows value previews from the related watch, and Used by comes from the
//! live pods, both at paint time (`live_sections.rs`).

use cluster::{ConfigMapKey, ConfigMapSummary};

use crate::kind_row::{
    DetailRow, DetailSection, KindCell, KindObject, KindRow, LiveContent, chips,
};
use crate::status_tone::{StatusLabel, StatusTone};

const BYTES_PER_UNIT: usize = 1024;
const UNITS: [&str; 3] = ["KiB", "MiB", "GiB"];

pub(crate) fn config_map_row(config_map: &ConfigMapSummary) -> KindRow {
    let key_count = config_map.keys.len();
    let mut data: Vec<DetailRow> = Vec::new();
    if config_map.is_immutable {
        data.push(DetailRow::field("Immutable", KindCell::Text("Yes".into())));
    }
    data.push(DetailRow::Live(LiveContent::ConfigMapData));
    let status_text = match key_count {
        1 => "1 key".to_owned(),
        count => format!("{count} keys"),
    };
    KindRow {
        namespace: Some(config_map.namespace.clone()),
        name: config_map.name.clone(),
        created_at: config_map.created_at,
        status: StatusLabel {
            text: status_text.into(),
            tone: StatusTone::Ok,
        },
        cells: vec![
            KindCell::count(key_count),
            // The pods join fills it once the pods list has loaded.
            KindCell::Absent,
            KindCell::age(config_map.created_at),
        ],
        sections: vec![
            DetailSection {
                title: "Data",
                rows: data,
            },
            DetailSection {
                title: "Used by",
                rows: vec![DetailRow::Live(LiveContent::UsedBy)],
            },
        ],
        event: None,
        related_pods: None,
        labels: chips(&config_map.labels),
        object: KindObject::ConfigMap(config_map.clone()),
    }
}

/// What a key reads before its value preview arrives: `412 B`, `2.0 KiB · binary`.
pub(crate) fn key_size_text(key: &ConfigMapKey) -> String {
    let binary = if key.is_binary { " · binary" } else { "" };
    format!("{}{binary}", format_bytes(key.size_bytes))
}

/// 1024-based: `412 B`, `2.0 KiB`, `1.1 MiB`.
pub(crate) fn format_bytes(bytes: usize) -> String {
    if bytes < BYTES_PER_UNIT {
        return format!("{bytes} B");
    }
    let mut size = bytes as f64;
    let mut unit = "B";
    for next in UNITS {
        size /= BYTES_PER_UNIT as f64;
        unit = next;
        if size < BYTES_PER_UNIT as f64 {
            break;
        }
    }
    format!("{size:.1} {unit}")
}

#[cfg(test)]
#[path = "config_map_rows_tests.rs"]
mod config_map_rows_tests;
