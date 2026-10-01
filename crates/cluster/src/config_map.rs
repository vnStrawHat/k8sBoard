use futures::Stream;
use k8s_openapi::api::core::v1::ConfigMap;

use crate::connection::ClusterConnection;
use crate::namespace::NamespaceScope;
use crate::resource_watch::{WatchUpdate, summary_watch};
use crate::workload::label_terms;

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

impl ClusterConnection {
    /// Watches config maps in `scope`. Yields batched snapshots ordered by (namespace, name).
    pub fn watch_config_maps(
        &self,
        scope: NamespaceScope,
    ) -> impl Stream<Item = WatchUpdate<ConfigMapSummary>> + Send + 'static {
        summary_watch(
            self,
            self.scoped_api(scope),
            "watching config maps",
            config_map_summary,
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
}
