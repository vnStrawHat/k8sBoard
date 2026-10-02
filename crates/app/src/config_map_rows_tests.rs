use cluster::ConfigMapKey;

use super::*;
use crate::kind_join::CONFIG_MAP_USED_BY;
use crate::resource_kind::ResourceKind;

fn config_map(keys: Vec<ConfigMapKey>) -> ConfigMapSummary {
    ConfigMapSummary {
        namespace: "team-a".to_owned(),
        name: "settings".to_owned(),
        created_at: None,
        labels: vec!["app=api".to_owned()],
        keys,
        is_immutable: false,
    }
}

fn key(name: &str, size_bytes: usize, is_binary: bool) -> ConfigMapKey {
    ConfigMapKey {
        name: name.to_owned(),
        size_bytes,
        is_binary,
    }
}

fn data_rows(row: &KindRow) -> &[DetailRow] {
    &row.sections.first().expect("data section").rows
}

#[test]
fn config_map_row_cells_match_column_count() {
    let row = config_map_row(&config_map(vec![key("a", 1, false)]));
    assert_eq!(row.cells.len(), ResourceKind::ConfigMaps.columns().len());
    assert_eq!(row.cells.first(), Some(&KindCell::count(1)));
}

#[test]
fn config_map_used_by_cell_waits_for_the_join() {
    let row = config_map_row(&config_map(Vec::new()));
    assert_eq!(row.cells.get(CONFIG_MAP_USED_BY), Some(&KindCell::Absent));
}

#[test]
fn config_map_sections_are_live_data_and_used_by() {
    let summary = config_map(vec![key("a", 1, false)]);
    let row = config_map_row(&summary);
    let titles: Vec<&str> = row.sections.iter().map(|section| section.title).collect();
    assert_eq!(titles, ["Data", "Used by"]);
    assert_eq!(
        data_rows(&row),
        [DetailRow::Live(LiveContent::ConfigMapData)]
    );
    assert_eq!(
        row.section("Used by")
            .map(|section| section.rows.as_slice()),
        Some(&[DetailRow::Live(LiveContent::UsedBy)][..])
    );
    assert_eq!(row.object, KindObject::ConfigMap(summary));
}

#[test]
fn format_bytes_uses_binary_units() {
    assert_eq!(format_bytes(0), "0 B");
    assert_eq!(format_bytes(412), "412 B");
    assert_eq!(format_bytes(1023), "1023 B");
    assert_eq!(format_bytes(2048), "2.0 KiB");
    assert_eq!(format_bytes(1_153_434), "1.1 MiB");
    assert_eq!(format_bytes(3 * 1024 * 1024 * 1024), "3.0 GiB");
}

#[test]
fn key_size_text_marks_binary_keys() {
    assert_eq!(key_size_text(&key("app.yaml", 412, false)), "412 B");
    assert_eq!(
        key_size_text(&key("logo.png", 2048, true)),
        "2.0 KiB · binary"
    );
}

#[test]
fn config_map_status_counts_keys() {
    assert_eq!(
        config_map_row(&config_map(Vec::new())).status.text,
        "0 keys"
    );
    assert_eq!(
        config_map_row(&config_map(vec![key("a", 1, false)]))
            .status
            .text,
        "1 key"
    );
    assert_eq!(
        config_map_row(&config_map(vec![key("a", 1, false), key("b", 1, false)]))
            .status
            .text,
        "2 keys"
    );
}

#[test]
fn config_map_immutable_field_appears_only_when_true() {
    let mut summary = config_map(vec![key("a", 1, false)]);
    let has_immutable = |summary: &ConfigMapSummary| {
        data_rows(&config_map_row(summary))
            .iter()
            .any(|row| matches!(row, DetailRow::Field { label, .. } if label == "Immutable"))
    };
    assert!(!has_immutable(&summary));
    summary.is_immutable = true;
    assert!(has_immutable(&summary));
}
