//! The ConfigMap row builder. It shows key names and sizes only: the summary never carries
//! values, and the drawer says so.

use cluster::ConfigMapSummary;

use crate::kind_row::{DetailRow, DetailSection, KindCell, KindRow, chips};
use crate::status_tone::{StatusLabel, StatusTone};

const BYTES_PER_UNIT: usize = 1024;
const UNITS: [&str; 3] = ["KiB", "MiB", "GiB"];

pub(crate) fn config_map_row(config_map: &ConfigMapSummary) -> KindRow {
    let key_count = config_map.keys.len();
    let mut data: Vec<DetailRow> = Vec::new();
    if config_map.is_immutable {
        data.push(DetailRow::field("Immutable", KindCell::Text("Yes".into())));
    }
    if config_map.keys.is_empty() {
        data.push(DetailRow::Note("No keys".into()));
    }
    data.extend(config_map.keys.iter().map(|key| {
        let binary = if key.is_binary { " · binary" } else { "" };
        DetailRow::field(
            key.name.clone(),
            KindCell::Mono(format!("{}{binary}", format_bytes(key.size_bytes)).into()),
        )
    }));
    data.push(DetailRow::Note("Values are in the YAML tab".into()));
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
            KindCell::age(config_map.created_at),
        ],
        sections: vec![DetailSection {
            title: "Data",
            rows: data,
        }],
        event: None,
        related_pods: None,
        labels: chips(&config_map.labels),
    }
}

/// 1024-based: `412 B`, `2.0 KiB`, `1.1 MiB`.
fn format_bytes(bytes: usize) -> String {
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
