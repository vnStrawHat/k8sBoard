use k8s_openapi::api::core::v1::LimitRangeSpec;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;

use super::*;

fn quantities(pairs: &[(&str, &str)]) -> Option<BTreeMap<String, Quantity>> {
    Some(
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), Quantity((*value).to_owned())))
            .collect(),
    )
}

fn limit_range(limits: Vec<LimitRangeItem>) -> LimitRange {
    LimitRange {
        metadata: ObjectMeta {
            namespace: Some("shop".to_owned()),
            name: Some("defaults".to_owned()),
            ..Default::default()
        },
        spec: Some(LimitRangeSpec { limits }),
    }
}

#[test]
fn summary_keeps_quantities_as_written() {
    let summary = limit_range_summary(&limit_range(vec![LimitRangeItem {
        type_: "Container".to_owned(),
        default: quantities(&[("cpu", "500m"), ("memory", "512Mi")]),
        default_request: quantities(&[("cpu", "0.1")]),
        max: quantities(&[("cpu", "2")]),
        min: quantities(&[("memory", "64Mi")]),
        ..Default::default()
    }]));
    assert_eq!(summary.namespace, "shop");
    assert_eq!(summary.name, "defaults");
    let [limit] = summary.limits.as_slice() else {
        panic!("one limit");
    };
    assert_eq!(limit.kind, "Container");
    // `0.1` stays `0.1`: the text is not normalized to `100m`.
    assert_eq!(
        limit.default_request.get("cpu").map(String::as_str),
        Some("0.1")
    );
    assert_eq!(
        limit.default.get("memory").map(String::as_str),
        Some("512Mi")
    );
    assert_eq!(limit.max.get("cpu").map(String::as_str), Some("2"));
    assert_eq!(limit.min.get("memory").map(String::as_str), Some("64Mi"));
}

#[test]
fn summary_orders_limits_as_listed() {
    let item = |kind: &str| LimitRangeItem {
        type_: kind.to_owned(),
        ..Default::default()
    };
    let summary = limit_range_summary(&limit_range(vec![
        item("Pod"),
        item("Container"),
        item("PersistentVolumeClaim"),
    ]));
    let kinds: Vec<&str> = summary
        .limits
        .iter()
        .map(|limit| limit.kind.as_str())
        .collect();
    assert_eq!(kinds, ["Pod", "Container", "PersistentVolumeClaim"]);
    // An item with no quantities has empty maps.
    assert!(summary.limits[0].default.is_empty() && summary.limits[0].max.is_empty());
}

#[test]
fn empty_spec_has_no_limits() {
    assert!(
        limit_range_summary(&limit_range(Vec::new()))
            .limits
            .is_empty()
    );
    let no_spec = LimitRange {
        metadata: ObjectMeta {
            name: Some("bare".to_owned()),
            ..Default::default()
        },
        spec: None,
    };
    let summary = limit_range_summary(&no_spec);
    assert_eq!(summary.name, "bare");
    assert!(summary.limits.is_empty());
}
