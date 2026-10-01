use cluster::ConfigMapKey;

use super::*;
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
fn format_bytes_uses_binary_units() {
    assert_eq!(format_bytes(0), "0 B");
    assert_eq!(format_bytes(412), "412 B");
    assert_eq!(format_bytes(1023), "1023 B");
    assert_eq!(format_bytes(2048), "2.0 KiB");
    assert_eq!(format_bytes(1_153_434), "1.1 MiB");
    assert_eq!(format_bytes(3 * 1024 * 1024 * 1024), "3.0 GiB");
}

#[test]
fn config_map_lists_key_sizes_and_marks_binary_keys() {
    let row = config_map_row(&config_map(vec![
        key("app.yaml", 412, false),
        key("logo.png", 2048, true),
    ]));
    let rows = data_rows(&row);
    assert_eq!(
        rows.first(),
        Some(&DetailRow::field(
            "app.yaml",
            KindCell::Mono("412 B".into())
        ))
    );
    assert_eq!(
        rows.get(1),
        Some(&DetailRow::field(
            "logo.png",
            KindCell::Mono("2.0 KiB · binary".into())
        ))
    );
}

#[test]
fn config_map_without_keys_says_so_and_notes_hidden_values() {
    let row = config_map_row(&config_map(Vec::new()));
    assert_eq!(
        data_rows(&row),
        [
            DetailRow::Note("No keys".into()),
            DetailRow::Note("Values are not shown in this version".into()),
        ]
    );
    assert_eq!(row.status.text, "0 keys");
}

#[test]
fn config_map_status_is_singular_for_one_key() {
    let row = config_map_row(&config_map(vec![key("a", 1, false)]));
    assert_eq!(row.status.text, "1 key");
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

#[test]
fn config_map_keys_stay_in_the_label_and_value_layout() {
    let row = config_map_row(&config_map(vec![
        key("statusbadge.enabled", 4, false),
        key("a-very-long-key-name-that-would-never-fit.yaml", 4, false),
    ]));
    let fields = data_rows(&row)
        .iter()
        .filter(|row| matches!(row, DetailRow::Field { .. }))
        .count();
    assert_eq!(fields, 2);
    assert!(
        !data_rows(&row)
            .iter()
            .any(|row| matches!(row, DetailRow::Stacked { .. }))
    );
}
