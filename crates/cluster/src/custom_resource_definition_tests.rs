use serde_json::{Value, json};

use super::*;

/// A cert-manager-like CRD, decoded through the lean heads like a watch event.
fn crd(value: Value) -> CrdSummary {
    let object: CrdObject = serde_json::from_value(value).expect("lean CRD decodes");
    crd_summary(&object)
}

fn certificate_crd() -> Value {
    json!({
        "apiVersion": "apiextensions.k8s.io/v1",
        "kind": "CustomResourceDefinition",
        "metadata": {
            "name": "certificates.cert-manager.io",
            "creationTimestamp": "2026-01-02T03:04:05Z",
        },
        "spec": {
            "group": "cert-manager.io",
            "scope": "Namespaced",
            "names": {
                "kind": "Certificate",
                "plural": "certificates",
                "singular": "certificate",
            },
            "versions": [{
                "name": "v1",
                "served": true,
                "storage": true,
                "additionalPrinterColumns": [
                    {
                        "name": "Ready",
                        "type": "string",
                        "jsonPath": ".status.conditions[?(@.type==\"Ready\")].status",
                    },
                    {"name": "Issuer", "type": "string", "jsonPath": ".spec.issuerRef.name"},
                    {"name": "Odd", "type": "weird", "jsonPath": ".spec.ports[0:2]"},
                    {
                        "name": "Key",
                        "type": "string",
                        "format": "password",
                        "jsonPath": ".spec.key",
                    },
                ],
                "schema": {"openAPIV3Schema": {
                    "type": "object",
                    "properties": {
                        "metadata": {"type": "object"},
                        "spec": {
                            "type": "object",
                            "properties": {
                                "secretName": {"type": "string", "description": "deep"},
                                "issuerRef": {"type": "object", "properties": {"name": {"type": "string"}}},
                                "duration": {"x-kubernetes-int-or-string": true},
                            },
                        },
                        "status": {
                            "type": "object",
                            "properties": {"notAfter": {"type": "string"}},
                        },
                    },
                }},
            }],
        },
        "status": {"conditions": [
            {"type": "NamesAccepted", "status": "True", "reason": "NoConflicts"},
            {"type": "Established", "status": "True", "reason": "InitialNamesAccepted"},
        ]},
    })
}

fn with_conditions(conditions: Value) -> CrdSummary {
    let mut value = certificate_crd();
    value["status"] = json!({ "conditions": conditions });
    crd(value)
}

fn version(name: &str, is_served: bool) -> CrdVersion {
    CrdVersion {
        name: name.to_owned(),
        is_served,
        is_storage: false,
        is_deprecated: false,
        deprecation_warning: None,
        printer_columns: Vec::new(),
        schema: SchemaOutline::default(),
    }
}

fn summary_with_versions(versions: Vec<CrdVersion>) -> CrdSummary {
    CrdSummary {
        versions,
        ..crd(certificate_crd())
    }
}

#[test]
fn crd_summary_reads_names_scope_and_versions() {
    let summary = crd(certificate_crd());
    assert_eq!(summary.name, "certificates.cert-manager.io");
    assert_eq!(summary.group, "cert-manager.io");
    assert_eq!(summary.kind, "Certificate");
    assert_eq!(summary.plural, "certificates");
    assert_eq!(summary.singular, "certificate");
    assert_eq!(summary.scope, ResourceScope::Namespaced);
    assert_eq!(summary.state, CrdState::Established);
    assert_eq!(
        summary.created_at,
        Some("2026-01-02T03:04:05Z".parse().expect("timestamp"))
    );
    assert_eq!(summary.versions.len(), 1);
    let v1 = &summary.versions[0];
    assert_eq!(
        (v1.name.as_str(), v1.is_served, v1.is_storage),
        ("v1", true, true)
    );
    assert!(!v1.is_deprecated);
}

#[test]
fn singular_defaults_to_lowercase_kind() {
    let mut value = certificate_crd();
    value["spec"]["names"]["singular"] = json!("");
    assert_eq!(crd(value).singular, "certificate");
    let mut value = certificate_crd();
    value["spec"]["names"]
        .as_object_mut()
        .expect("names")
        .remove("singular");
    assert_eq!(crd(value).singular, "certificate");
}

#[test]
fn cluster_scope_is_read() {
    let mut value = certificate_crd();
    value["spec"]["scope"] = json!("Cluster");
    assert_eq!(crd(value).scope, ResourceScope::Cluster);
}

#[test]
fn printer_columns_keep_order_type_and_support() {
    let summary = crd(certificate_crd());
    let columns = &summary.versions[0].printer_columns;
    let names: Vec<_> = columns.iter().map(|column| column.name.as_str()).collect();
    assert_eq!(names, ["Ready", "Issuer", "Odd", "Key"]);
    assert_eq!(columns[1].column_type, ColumnType::String);
    // An unknown type reads as a string, and an unsupported path is flagged.
    assert_eq!(columns[2].column_type, ColumnType::String);
    assert!(columns[0].is_supported && columns[1].is_supported);
    assert!(!columns[2].is_supported);
    assert_eq!(columns[1].json_path, ".spec.issuerRef.name");
}

#[test]
fn column_types_map_from_openapi_names() {
    let mut value = certificate_crd();
    value["spec"]["versions"][0]["additionalPrinterColumns"] = json!([
        {"name": "A", "type": "integer", "jsonPath": ".a"},
        {"name": "B", "type": "number", "jsonPath": ".b"},
        {"name": "C", "type": "boolean", "jsonPath": ".c"},
        {"name": "D", "type": "date", "jsonPath": ".d"},
    ]);
    let types: Vec<_> = crd(value).versions[0]
        .printer_columns
        .iter()
        .map(|column| column.column_type)
        .collect();
    assert_eq!(
        types,
        [
            ColumnType::Integer,
            ColumnType::Number,
            ColumnType::Boolean,
            ColumnType::Date
        ]
    );
}

#[test]
fn password_format_is_read() {
    let summary = crd(certificate_crd());
    let columns = &summary.versions[0].printer_columns;
    assert!(columns[3].is_password);
    assert!(!columns[1].is_password);
}

#[test]
fn condition_status_columns_are_flagged() {
    let summary = crd(certificate_crd());
    let columns = &summary.versions[0].printer_columns;
    assert!(columns[0].is_condition_status);
    assert!(!columns[1].is_condition_status);
}

#[test]
fn printer_column_new_parses_its_path() {
    let column = PrinterColumn::new("Expires", ColumnType::Date, ".status.notAfter", false);
    assert!(column.is_supported);
    assert!(!column.is_condition_status);
    let column = PrinterColumn::new("Bad", ColumnType::String, "{.spec.x}", false);
    assert!(!column.is_supported);
    assert!(!column.is_condition_status);
}

#[test]
fn schema_outline_lists_spec_and_status_fields() {
    let outline = &crd(certificate_crd()).versions[0].schema;
    let spec: Vec<_> = outline
        .spec
        .iter()
        .map(|field| (field.name.as_str(), field.field_type.as_deref()))
        .collect();
    assert_eq!(
        spec,
        [
            ("duration", None),
            ("issuerRef", Some("object")),
            ("secretName", Some("string")),
        ]
    );
    assert_eq!(outline.status.len(), 1);
    assert_eq!(outline.status[0].name, "notAfter");
    assert_eq!(outline.omitted, 0);
}

#[test]
fn schema_outline_caps_at_40_and_counts_omitted() {
    let spec: serde_json::Map<String, Value> = (0..45)
        .map(|index| (format!("spec{index:02}"), json!({"type": "string"})))
        .collect();
    let status: serde_json::Map<String, Value> = (0..41)
        .map(|index| (format!("status{index:02}"), json!({"type": "string"})))
        .collect();
    let mut value = certificate_crd();
    value["spec"]["versions"][0]["schema"]["openAPIV3Schema"]["properties"] = json!({
        "spec": {"properties": spec},
        "status": {"properties": status},
    });
    let outline = crd(value).versions[0].schema.clone();
    assert_eq!(outline.spec.len(), 40);
    assert_eq!(outline.status.len(), 40);
    assert_eq!(outline.omitted, 5 + 1);
    assert_eq!(outline.spec[0].name, "spec00");
}

#[test]
fn missing_schema_gives_empty_outline() {
    let mut value = certificate_crd();
    value["spec"]["versions"][0]
        .as_object_mut()
        .expect("version")
        .remove("schema");
    assert_eq!(crd(value).versions[0].schema, SchemaOutline::default());
}

#[test]
fn crd_state_follows_conditions() {
    let rejected = with_conditions(json!([
        {"type": "NamesAccepted", "status": "False", "message": "name conflict"},
        {"type": "Established", "status": "True"},
    ]));
    assert_eq!(
        rejected.state,
        CrdState::NamesNotAccepted {
            message: Some("name conflict".to_owned())
        }
    );
    let pending = with_conditions(json!([
        {"type": "Established", "status": "False", "reason": "Installing"},
    ]));
    assert_eq!(
        pending.state,
        CrdState::NotEstablished {
            reason: Some("Installing".to_owned())
        }
    );
    assert_eq!(
        with_conditions(json!([])).state,
        CrdState::NotEstablished { reason: None }
    );
    let mut value = certificate_crd();
    value["metadata"]["deletionTimestamp"] = json!("2026-02-01T00:00:00Z");
    assert_eq!(crd(value).state, CrdState::Terminating);
}

#[test]
fn preferred_version_is_highest_priority_served() {
    let summary = summary_with_versions(vec![
        version("v1alpha3", true),
        version("v2beta1", true),
        version("v1", true),
        version("v2", false),
    ]);
    let preferred = summary.preferred_version().expect("a served version");
    assert_eq!(preferred.name, "v1");
    let beta_only =
        summary_with_versions(vec![version("v1alpha3", true), version("v2beta1", true)]);
    assert_eq!(
        beta_only
            .preferred_version()
            .map(|version| version.name.as_str()),
        Some("v2beta1")
    );
}

#[test]
fn no_served_version_has_no_preference() {
    let summary = summary_with_versions(vec![version("v1", false)]);
    assert!(summary.preferred_version().is_none());
    assert!(
        summary_with_versions(Vec::new())
            .preferred_version()
            .is_none()
    );
}

#[test]
fn resource_names_group_version_kind_plural_scope() {
    let summary = crd(certificate_crd());
    let resource = summary.resource(&summary.versions[0]);
    assert_eq!(
        resource,
        CustomResourceType {
            group: "cert-manager.io".to_owned(),
            version: "v1".to_owned(),
            kind: "Certificate".to_owned(),
            plural: "certificates".to_owned(),
            scope: ResourceScope::Namespaced,
        }
    );
    let api = custom_api_resource(&resource);
    assert_eq!(api.api_version, "cert-manager.io/v1");
    assert_eq!(api.plural, "certificates");
    assert_eq!(api.kind, "Certificate");
}

#[test]
fn crd_watch_config_pages_most_recent() {
    let config = crd_watch_config();
    assert_eq!(config.list_semantic, ListSemantic::MostRecent);
    assert_eq!(config.page_size, Some(20));
    assert_eq!(config.field_selector, None);
}

#[test]
fn crd_resource_is_apiextensions_v1() {
    let resource = crd_resource();
    assert_eq!(resource.api_version, "apiextensions.k8s.io/v1");
    assert_eq!(resource.plural, "customresourcedefinitions");
}

#[test]
fn lean_crd_decoding_ignores_deep_schema() {
    let mut value = certificate_crd();
    value["spec"]["versions"][0]["schema"]["openAPIV3Schema"]["properties"]["spec"]["properties"]
        ["issuerRef"] = json!({
        "type": "object",
        "properties": {"deep": {"type": "object", "properties": {"deeper": {"type": "array", "items": {"type": "string"}}}}},
        "x-kubernetes-preserve-unknown-fields": true,
    });
    value["spec"]["conversion"] = json!({"strategy": "Webhook"});
    let summary = crd(value);
    let outline = &summary.versions[0].schema;
    assert_eq!(outline.spec[1].name, "issuerRef");
    assert_eq!(outline.spec[1].field_type.as_deref(), Some("object"));
}
