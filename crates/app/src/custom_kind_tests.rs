use cluster::SchemaOutline;

use super::*;
use crate::resource_kind::ResourceKind;

fn version(name: &str, columns: Vec<PrinterColumn>) -> cluster::CrdVersion {
    cluster::CrdVersion {
        name: name.to_owned(),
        is_served: true,
        is_storage: true,
        is_deprecated: false,
        deprecation_warning: None,
        printer_columns: columns,
        schema: SchemaOutline::default(),
    }
}

fn crd(
    group: &str,
    kind: &str,
    plural: &str,
    scope: ResourceScope,
    columns: Vec<PrinterColumn>,
) -> CrdSummary {
    CrdSummary {
        name: format!("{plural}.{group}"),
        group: group.to_owned(),
        kind: kind.to_owned(),
        plural: plural.to_owned(),
        singular: kind.to_ascii_lowercase(),
        scope,
        versions: vec![version("v1", columns)],
        state: CrdState::Established,
        created_at: None,
    }
}

fn column(name: &str, column_type: ColumnType, path: &str) -> PrinterColumn {
    PrinterColumn::new(name, column_type, path, false)
}

fn certificate_columns() -> Vec<PrinterColumn> {
    vec![
        column(
            "Ready",
            ColumnType::String,
            ".status.conditions[?(@.type==\"Ready\")].status",
        ),
        column("Issuer", ColumnType::String, ".spec.issuerRef.name"),
        column("Age", ColumnType::Date, ".metadata.creationTimestamp"),
    ]
}

fn certificates() -> CrdSummary {
    crd(
        "cert-manager.io",
        "Certificate",
        "certificates",
        ResourceScope::Namespaced,
        certificate_columns(),
    )
}

fn one_kind(crd: &CrdSummary) -> CustomKind {
    let mut cache = CustomKindCache::default();
    custom_kinds(std::slice::from_ref(crd), &mut cache)
        .pop()
        .expect("an Established CRD becomes a kind")
}

fn column_names(kind: CustomKind) -> Vec<&'static str> {
    kind.spec()
        .columns
        .iter()
        .map(|column| column.name)
        .collect()
}

#[test]
fn labels_follow_kind_and_plural() {
    assert_eq!(kind_label("Certificate", "certificates"), "Certificates");
    assert_eq!(kind_label("Ingress", "ingresses"), "Ingresses");
    assert_eq!(kind_label("KafkaTopic", "kafkatopics"), "KafkaTopics");
    assert_eq!(kind_label("Policy", "policies"), "Policies");
    assert_eq!(kind_label("Ox", "oxen"), "Oxen");
    // A plural that does not start with the kind gets its first letter raised.
    assert_eq!(kind_label("Mouse", "mice"), "Mice");
    assert_eq!(kind_label("Mouse", ""), "");
}

#[test]
fn badges_use_the_next_capital() {
    assert_eq!(kind_badge("Certificate"), "Ce");
    assert_eq!(kind_badge("KafkaTopic"), "Kt");
    assert_eq!(kind_badge("Application"), "Ap");
    assert_eq!(kind_badge("X"), "X");
    assert_eq!(kind_badge(""), "");
}

#[test]
fn only_established_served_crds_become_kinds() {
    let mut not_established = certificates();
    not_established.state = CrdState::NotEstablished { reason: None };
    let mut terminating = certificates();
    terminating.state = CrdState::Terminating;
    let mut unserved = certificates();
    unserved.versions[0].is_served = false;
    let mut cache = CustomKindCache::default();
    let kinds = custom_kinds(
        &[not_established, terminating, unserved, certificates()],
        &mut cache,
    );
    assert_eq!(kinds.len(), 1);
    assert_eq!(kinds[0].crd_name(), "certificates.cert-manager.io");
}

#[test]
fn kinds_sort_by_group_then_label() {
    let widgets = crd(
        "b.io",
        "Widget",
        "widgets",
        ResourceScope::Namespaced,
        Vec::new(),
    );
    let gadgets = crd(
        "b.io",
        "Gadget",
        "gadgets",
        ResourceScope::Namespaced,
        Vec::new(),
    );
    let alpha = crd("a.io", "Zed", "zeds", ResourceScope::Cluster, Vec::new());
    let mut cache = CustomKindCache::default();
    let kinds = custom_kinds(&[widgets, gadgets, alpha], &mut cache);
    let labels: Vec<_> = kinds.iter().map(|kind| kind.spec().label).collect();
    assert_eq!(labels, ["Zeds", "Gadgets", "Widgets"]);
}

#[test]
fn cached_definitions_are_reused() {
    let mut cache = CustomKindCache::default();
    let first = custom_kinds(&[certificates()], &mut cache);
    let again = custom_kinds(&[certificates()], &mut cache);
    assert!(ptr::eq(first[0].0, again[0].0));
    // Handed to the next session, the cache still reuses it.
    let mut next = std::mem::take(&mut cache);
    let moved = custom_kinds(&[certificates()], &mut next);
    assert!(ptr::eq(first[0].0, moved[0].0));
}

#[test]
fn changed_columns_make_a_new_kind() {
    let mut cache = CustomKindCache::default();
    let before = custom_kinds(&[certificates()], &mut cache)[0];
    let mut changed = certificates();
    changed.versions[0].printer_columns.push(column(
        "Secret",
        ColumnType::String,
        ".spec.secretName",
    ));
    let after = custom_kinds(&[changed], &mut cache)[0];
    assert_ne!(before, after);
    assert!(!ptr::eq(before.0, after.0));
    assert_eq!(before.crd_name(), after.crd_name());
}

#[test]
fn equality_short_circuits_on_the_same_pointer() {
    let kind = one_kind(&certificates());
    let copy = kind;
    assert!(ptr::eq(kind.0, copy.0));
    assert_eq!(kind, copy);
    // Two independent builds of one definition are equal by source, not by pointer.
    let other = one_kind(&certificates());
    assert!(!ptr::eq(kind.0, other.0));
    assert_eq!(kind, other);
    let mut seen = std::collections::HashSet::new();
    seen.insert(kind);
    assert!(seen.contains(&other));
}

#[test]
fn columns_follow_printer_column_types() {
    let columns = vec![
        column(
            "Ready",
            ColumnType::String,
            ".status.conditions[?(@.type==\"Ready\")].status",
        ),
        column("Name", ColumnType::String, ".spec.name"),
        column("Replicas", ColumnType::Integer, ".spec.replicas"),
        column("Ratio", ColumnType::Number, ".spec.ratio"),
        column("Paused", ColumnType::Boolean, ".spec.paused"),
        column("Until", ColumnType::Date, ".spec.until"),
        column("Age", ColumnType::Date, ".metadata.creationTimestamp"),
    ];
    let kind = one_kind(&crd(
        "x.io",
        "Widget",
        "widgets",
        ResourceScope::Namespaced,
        columns,
    ));
    let spec: Vec<_> = kind
        .spec()
        .columns
        .iter()
        .map(|column| (column.name, column.width, column.align))
        .collect();
    assert_eq!(
        spec,
        [
            ("Ready", 80., Align::Left),
            ("Name", 160., Align::Left),
            ("Replicas", 90., Align::Right),
            ("Ratio", 90., Align::Right),
            ("Paused", 80., Align::Left),
            ("Until", 90., Align::Right),
            ("Age", 70., Align::Right),
        ]
    );
    assert_eq!(kind.column_rules()[0], ColumnRule::ConditionStatus);
    assert_eq!(kind.column_rules()[1], ColumnRule::Plain);
}

#[test]
fn no_printer_columns_give_age_only() {
    let kind = one_kind(&crd(
        "x.io",
        "Widget",
        "widgets",
        ResourceScope::Namespaced,
        Vec::new(),
    ));
    assert_eq!(column_names(kind), ["Age"]);
    assert!(kind.printer_columns().is_empty());
}

#[test]
fn built_in_columns_extend_cert_manager_certificates() {
    let kind = one_kind(&certificates());
    assert_eq!(column_names(kind), ["Ready", "Issuer", "Expires", "Age"]);
    assert_eq!(
        kind.column_rules(),
        [
            ColumnRule::ConditionStatus,
            ColumnRule::Plain,
            ColumnRule::Expiry,
            ColumnRule::Plain,
        ]
    );
    let expires = &kind.printer_columns()[2];
    assert_eq!(expires.json_path, ".status.notAfter");
    assert!(expires.is_supported);
}

#[test]
fn built_in_columns_append_without_an_age_column() {
    let mut without_age = certificates();
    without_age.versions[0].printer_columns.pop();
    let kind = one_kind(&without_age);
    assert_eq!(column_names(kind), ["Ready", "Issuer", "Expires"]);
    let mut already = certificates();
    already.versions[0]
        .printer_columns
        .insert(0, column("Expires", ColumnType::Date, ".status.notAfter"));
    // A column that already exists is not added twice.
    assert_eq!(
        column_names(one_kind(&already))
            .iter()
            .filter(|name| **name == "Expires")
            .count(),
        1
    );
}

#[test]
fn built_in_columns_do_not_apply_to_other_crds() {
    let other = crd(
        "x.io",
        "Widget",
        "widgets",
        ResourceScope::Namespaced,
        vec![column(
            "Age",
            ColumnType::Date,
            ".metadata.creationTimestamp",
        )],
    );
    assert_eq!(column_names(one_kind(&other)), ["Age"]);
}

#[test]
fn custom_kinds_describe_their_resource() {
    let kind = one_kind(&certificates());
    let spec = kind.spec();
    assert_eq!(spec.label, "Certificates");
    assert_eq!(spec.singular, "certificate");
    assert_eq!(spec.plural, "certificates");
    assert_eq!(spec.badge, "Ce");
    assert_eq!(spec.delete_label, "Delete certificate…");
    assert!(spec.is_namespaced);
    assert!(spec.has_labels);
    assert!(!spec.has_port_forward);
    assert!(spec.read_only_actions.is_empty());
    assert_eq!(kind.resource().group, "cert-manager.io");
    assert_eq!(
        format!("{kind:?}"),
        "CustomKind(certificates.cert-manager.io)"
    );
}

#[test]
fn cluster_scoped_crds_are_not_namespaced() {
    let kind = one_kind(&crd(
        "x.io",
        "Zed",
        "zeds",
        ResourceScope::Cluster,
        Vec::new(),
    ));
    assert!(!kind.spec().is_namespaced);
    assert!(!ResourceKind::Custom(kind).is_namespaced());
}

#[test]
fn only_custom_kinds_lack_an_access_check() {
    let custom = ResourceKind::Custom(one_kind(&certificates()));
    assert_eq!(custom.access_check(), None);
    assert_eq!(custom.builtin_object(), None);
    assert_eq!(custom.object_kind(), "Certificate");
    assert!(!custom.has_count());
    for kind in ResourceKind::ALL {
        assert!(kind.access_check().is_some(), "{}", kind.label());
        assert!(kind.builtin_object().is_some(), "{}", kind.label());
    }
}

#[test]
fn custom_kinds_build_object_refs_that_fit_their_scope() {
    let namespaced = ResourceKind::Custom(one_kind(&certificates()));
    assert!(
        namespaced
            .object_ref(Some("shop".to_owned()), "web".to_owned())
            .is_some()
    );
    assert!(namespaced.object_ref(None, "web".to_owned()).is_none());
    let cluster = ResourceKind::Custom(one_kind(&crd(
        "x.io",
        "Zed",
        "zeds",
        ResourceScope::Cluster,
        Vec::new(),
    )));
    assert!(cluster.object_ref(None, "z".to_owned()).is_some());
    assert!(
        cluster
            .object_ref(Some("shop".to_owned()), "z".to_owned())
            .is_none()
    );
}

#[test]
fn cache_hits_do_not_grow_the_cache() {
    let mut cache = CustomKindCache::default();
    custom_kinds(&[certificates()], &mut cache);
    custom_kinds(&[certificates()], &mut cache);
    assert_eq!(cache.kinds.len(), 1);
}

#[test]
fn cache_warns_once_past_the_limit() {
    let mut cache = CustomKindCache::default();
    let crds: Vec<CrdSummary> = (0..=CACHE_WARN_LIMIT)
        .map(|index| {
            crd(
                "x.io",
                "Widget",
                &format!("widgets{index}"),
                ResourceScope::Namespaced,
                Vec::new(),
            )
        })
        .collect();
    custom_kinds(&crds[..CACHE_WARN_LIMIT], &mut cache);
    assert!(!cache.has_warned);
    custom_kinds(&crds, &mut cache);
    assert!(cache.has_warned);
    assert_eq!(cache.kinds.len(), CACHE_WARN_LIMIT + 1);
}
