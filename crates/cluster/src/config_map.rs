use futures::Stream;
use k8s_openapi::api::core::v1::ConfigMap;
use kube::runtime::watcher;

use crate::connection::ClusterConnection;
use crate::namespace::NamespaceScope;
use crate::resource_watch::{WatchUpdate, selected_summary_watch, summary_watch};
use crate::workload::label_terms;

/// A value this short and on one line is shown as written.
const LINE_PREVIEW_CHARS: usize = 120;

/// Key names and sizes only. Values are never copied: a config map can hold 1 MiB per
/// key, and its contents may be sensitive.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigMapSummary {
    pub namespace: String,
    pub name: String,
    pub created_at: Option<jiff::Timestamp>,
    /// `key=value` terms in key order.
    pub labels: Vec<String>,
    /// `data` and `binaryData` keys, sorted by name.
    pub keys: Vec<ConfigMapKey>,
    pub is_immutable: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigMapKey {
    pub name: String,
    /// UTF-8 length for `data`, decoded length for `binaryData`.
    pub size_bytes: usize,
    pub is_binary: bool,
}

/// The values of one config map as short previews. Values can be sensitive, so these types
/// have no `Debug`, and nothing in this module logs.
#[derive(Clone, PartialEq, Eq)]
pub struct ConfigMapValues {
    pub namespace: String,
    pub name: String,
    /// Sorted by key.
    pub entries: Vec<ConfigMapValue>,
}

#[derive(Clone, PartialEq, Eq)]
pub struct ConfigMapValue {
    pub key: String,
    pub preview: ValuePreview,
}

#[derive(Clone, PartialEq, Eq)]
pub enum ValuePreview {
    /// A single line of at most 120 characters, as written.
    Line(String),
    Json {
        size_bytes: usize,
    },
    Text {
        size_bytes: usize,
        lines: usize,
    },
    Binary {
        size_bytes: usize,
    },
}

impl ClusterConnection {
    /// Watches config maps in `scope`. Yields batched snapshots ordered by (namespace, name).
    pub fn watch_config_maps(
        &self,
        scope: NamespaceScope,
    ) -> impl Stream<Item = WatchUpdate<ConfigMapSummary>> + Send + 'static {
        summary_watch(
            self,
            self.scoped_apis(&scope),
            "watching config maps",
            config_map_summary,
        )
    }

    /// Watches one config map of `namespace` as value previews, as one drawer-scoped watch.
    pub fn watch_config_map_values(
        &self,
        namespace: &str,
        name: &str,
    ) -> impl Stream<Item = WatchUpdate<ConfigMapValues>> + Send + 'static {
        let scope = NamespaceScope::Named(namespace.to_owned());
        selected_summary_watch(
            self,
            self.scoped_apis(&scope),
            watcher::Config::default().fields(&format!("metadata.name={name}")),
            "watching config map values",
            config_map_values,
        )
    }
}

pub(crate) fn config_map_summary(config_map: &ConfigMap) -> ConfigMapSummary {
    let text_keys = config_map
        .data
        .iter()
        .flatten()
        .map(|(name, value)| ConfigMapKey {
            name: name.clone(),
            size_bytes: value.len(),
            is_binary: false,
        });
    let binary_keys = config_map
        .binary_data
        .iter()
        .flatten()
        .map(|(name, value)| ConfigMapKey {
            name: name.clone(),
            size_bytes: value.0.len(),
            is_binary: true,
        });
    let mut keys: Vec<_> = text_keys.chain(binary_keys).collect();
    keys.sort_by(|left, right| left.name.cmp(&right.name));
    ConfigMapSummary {
        namespace: config_map.metadata.namespace.clone().unwrap_or_default(),
        name: config_map.metadata.name.clone().unwrap_or_default(),
        created_at: config_map
            .metadata
            .creation_timestamp
            .as_ref()
            .map(|time| time.0),
        labels: label_terms(&config_map.metadata),
        keys,
        is_immutable: config_map.immutable == Some(true),
    }
}

fn config_map_values(config_map: &ConfigMap) -> ConfigMapValues {
    let text_entries = config_map
        .data
        .iter()
        .flatten()
        .map(|(key, value)| ConfigMapValue {
            key: key.clone(),
            preview: text_preview(value),
        });
    let binary_entries =
        config_map
            .binary_data
            .iter()
            .flatten()
            .map(|(key, value)| ConfigMapValue {
                key: key.clone(),
                preview: ValuePreview::Binary {
                    size_bytes: value.0.len(),
                },
            });
    let mut entries: Vec<_> = text_entries.chain(binary_entries).collect();
    entries.sort_by(|left, right| left.key.cmp(&right.key));
    ConfigMapValues {
        namespace: config_map.metadata.namespace.clone().unwrap_or_default(),
        name: config_map.metadata.name.clone().unwrap_or_default(),
        entries,
    }
}

fn text_preview(value: &str) -> ValuePreview {
    let single_line = value
        .strip_suffix("\r\n")
        .or_else(|| value.strip_suffix('\n'))
        .unwrap_or(value);
    if !single_line.contains('\n') && single_line.chars().count() <= LINE_PREVIEW_CHARS {
        return ValuePreview::Line(single_line.to_owned());
    }
    let size_bytes = value.len();
    if value.trim().starts_with(['{', '[']) {
        return ValuePreview::Json { size_bytes };
    }
    ValuePreview::Text {
        size_bytes,
        lines: value.lines().count(),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use k8s_openapi::ByteString;

    use super::*;

    fn key(name: &str, size_bytes: usize, is_binary: bool) -> ConfigMapKey {
        ConfigMapKey {
            name: name.to_owned(),
            size_bytes,
            is_binary,
        }
    }

    #[test]
    fn config_map_keys_merge_data_and_binary_sorted() {
        let config_map = ConfigMap {
            data: Some(BTreeMap::from([
                ("settings.yaml".to_owned(), "a: 1".to_owned()),
                ("empty".to_owned(), String::new()),
                ("zeta".to_owned(), "\u{e9}".to_owned()),
            ])),
            binary_data: Some(BTreeMap::from([(
                "cert.bin".to_owned(),
                ByteString(vec![0, 1, 2]),
            )])),
            ..Default::default()
        };
        let summary = config_map_summary(&config_map);
        assert_eq!(
            summary.keys,
            [
                key("cert.bin", 3, true),
                key("empty", 0, false),
                key("settings.yaml", 4, false),
                key("zeta", 2, false),
            ]
        );
    }

    #[test]
    fn config_map_summary_reads_immutable_flag() {
        let immutable = ConfigMap {
            immutable: Some(true),
            ..Default::default()
        };
        assert!(config_map_summary(&immutable).is_immutable);
        assert!(!config_map_summary(&ConfigMap::default()).is_immutable);
    }

    #[test]
    fn config_map_summary_drops_values() {
        let config_map = ConfigMap {
            data: Some(BTreeMap::from([(
                "app.properties".to_owned(),
                "db.password=distinctive-text-value".to_owned(),
            )])),
            binary_data: Some(BTreeMap::from([(
                "blob".to_owned(),
                ByteString(b"distinctive-binary-value".to_vec()),
            )])),
            ..Default::default()
        };
        let text = format!("{:?}", config_map_summary(&config_map));
        assert!(text.contains("app.properties"));
        assert!(!text.contains("distinctive-text-value"));
        assert!(!text.contains("distinctive-binary-value"));
    }

    fn values_of(data: &[(&str, &str)], binary: &[(&str, &[u8])]) -> ConfigMapValues {
        let config_map = ConfigMap {
            data: Some(
                data.iter()
                    .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
                    .collect(),
            ),
            binary_data: Some(
                binary
                    .iter()
                    .map(|(key, value)| ((*key).to_owned(), ByteString(value.to_vec())))
                    .collect(),
            ),
            ..Default::default()
        };
        config_map_values(&config_map)
    }

    fn preview_of(entry: &ConfigMapValue) -> &ValuePreview {
        &entry.preview
    }

    #[test]
    fn value_preview_kinds() {
        let long_line = "x".repeat(121);
        let exact_line = "y".repeat(120);
        let json = "{\n  \"a\": 1\n}\n";
        let text = "first\nsecond\nthird\n";
        let values = values_of(
            &[
                ("1-line", "level=info"),
                ("2-trailing", "level=info\n"),
                ("3-exact", &exact_line),
                ("4-long", &long_line),
                ("5-json", json),
                ("6-text", text),
                ("7-empty", ""),
            ],
            &[("8-binary", &[0, 1, 2, 3])],
        );
        let previews: Vec<_> = values.entries.iter().map(preview_of).collect();
        assert!(matches!(previews[0], ValuePreview::Line(line) if line == "level=info"));
        assert!(matches!(previews[1], ValuePreview::Line(line) if line == "level=info"));
        assert!(matches!(previews[2], ValuePreview::Line(line) if line == &exact_line));
        assert!(matches!(
            previews[3],
            ValuePreview::Text {
                size_bytes: 121,
                lines: 1
            }
        ));
        assert!(matches!(
            previews[4],
            ValuePreview::Json { size_bytes } if *size_bytes == json.len()
        ));
        assert!(matches!(
            previews[5],
            ValuePreview::Text {
                size_bytes: 19,
                lines: 3
            }
        ));
        assert!(matches!(previews[6], ValuePreview::Line(line) if line.is_empty()));
        assert!(matches!(
            previews[7],
            ValuePreview::Binary { size_bytes: 4 }
        ));
    }

    #[test]
    fn values_sorted_by_key() {
        let values = values_of(&[("zeta", "1"), ("alpha", "2")], &[("middle", b"3")]);
        let keys: Vec<_> = values
            .entries
            .iter()
            .map(|entry| entry.key.as_str())
            .collect();
        assert_eq!(keys, ["alpha", "middle", "zeta"]);
    }

    #[test]
    fn crlf_line_endings_stay_a_single_line() {
        let values = values_of(&[("crlf", "level=info\r\n"), ("two", "a\r\nb\r\n")], &[]);
        assert!(matches!(
            &values.entries[0].preview,
            ValuePreview::Line(line) if line == "level=info"
        ));
        assert!(matches!(
            &values.entries[1].preview,
            ValuePreview::Text { lines: 2, .. }
        ));
    }
}
