//! Custom resource kinds: one `CustomKind` per Established CRD with a served version, built from
//! the session's CRD watch. A definition is leaked once and reused by every session, so
//! `ResourceKind` stays `Copy` and every accessor stays `&'static`.

use std::collections::HashMap;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::ptr;

use cluster::{ColumnType, CrdState, CrdSummary, CustomResourceType, PrinterColumn, ResourceScope};
use gpui_kit::assets::IconName;

use crate::resource_actions::ResourceAction;
use crate::resource_kind::{Align, KindAction, KindApi, KindColumn, KindSpec, NameColumn, column};

/// How the app paints one printer column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ColumnRule {
    Plain,
    /// A condition status: True, False, and Unknown are toned.
    ConditionStatus,
    /// A date toned like a TLS expiry.
    Expiry,
}

/// A column the app adds to a well-known CRD because its printer columns lack it.
struct BuiltInColumn {
    crd_name: &'static str,
    name: &'static str,
    column_type: ColumnType,
    json_path: &'static str,
    rule: ColumnRule,
}

/// cert-manager has no NotAfter printer column, and W7 shows when a Certificate expires.
static BUILT_IN_COLUMNS: [BuiltInColumn; 1] = [BuiltInColumn {
    crd_name: "certificates.cert-manager.io",
    name: "Expires",
    column_type: ColumnType::Date,
    json_path: ".status.notAfter",
    rule: ColumnRule::Expiry,
}];

/// The one CRD whose objects offer Renew now (spec 0018 step 6), and the only version the write
/// path reaches.
const CERT_MANAGER_CERTIFICATES: &str = "certificates.cert-manager.io";
const CERT_MANAGER_VERSION: &str = "v1";
static CERTIFICATE_ACTIONS: [KindAction; 1] = [KindAction::keyed(
    "Renew now",
    ResourceAction::RenewCertificate,
)];

const AGE_COLUMN_NAME: &str = "Age";
const AGE_COLUMN: KindColumn = column(AGE_COLUMN_NAME, 70., Align::Right);

/// What makes two definitions the same kind.
#[derive(Clone, PartialEq, Eq, Hash)]
struct CustomKindSource {
    crd_name: String,
    resource: CustomResourceType,
    /// Built-in columns included.
    printer_columns: Vec<PrinterColumn>,
}

pub(crate) struct CustomKindSpec {
    source: CustomKindSource,
    rules: Vec<ColumnRule>,
    spec: KindSpec,
}

/// A served custom resource kind. Two kinds are equal when their source is, with a pointer
/// check first; a changed definition is a new kind, so views and keys never mix column sets.
#[derive(Clone, Copy)]
pub(crate) struct CustomKind(&'static CustomKindSpec);

impl PartialEq for CustomKind {
    fn eq(&self, other: &Self) -> bool {
        ptr::eq(self.0, other.0) || self.0.source == other.0.source
    }
}

impl Eq for CustomKind {}

impl Hash for CustomKind {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.source.hash(state);
    }
}

impl fmt::Debug for CustomKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "CustomKind({})", self.0.source.crd_name)
    }
}

impl CustomKind {
    pub(crate) fn crd_name(self) -> &'static str {
        &self.0.source.crd_name
    }

    pub(crate) fn resource(self) -> &'static CustomResourceType {
        &self.0.source.resource
    }

    /// The columns the objects watch evaluates, built-in ones included.
    pub(crate) fn printer_columns(self) -> &'static [PrinterColumn] {
        &self.0.source.printer_columns
    }

    /// One rule per printer column.
    pub(crate) fn column_rules(self) -> &'static [ColumnRule] {
        &self.0.rules
    }

    pub(crate) fn spec(self) -> &'static KindSpec {
        &self.0.spec
    }

    /// Whether this is the cert-manager Certificate kind, at whatever version it is served.
    pub(crate) fn is_cert_manager_certificate(self) -> bool {
        self.crd_name() == CERT_MANAGER_CERTIFICATES
    }

    /// Whether Renew now can reach this kind: the write path accepts `cert-manager.io/v1` only.
    pub(crate) fn is_cert_manager_v1(self) -> bool {
        self.is_cert_manager_certificate() && self.resource().version == CERT_MANAGER_VERSION
    }
}

/// Every definition ever built, keyed by source. The session owns it and hands it to the next
/// session, so a context switch reuses what it has seen.
#[derive(Default)]
pub(crate) struct CustomKindCache {
    kinds: HashMap<CustomKindSource, CustomKind>,
    /// The size warning was logged.
    has_warned: bool,
}

/// Definitions after which the cache warns once: each leaks about 1 KiB for the process.
const CACHE_WARN_LIMIT: usize = 2_000;

/// The Established CRDs with a served version as kinds, sorted by (group, label). A cached
/// source is reused; only definitions never seen before are leaked.
pub(crate) fn custom_kinds(crds: &[CrdSummary], cache: &mut CustomKindCache) -> Vec<CustomKind> {
    let mut kinds: Vec<CustomKind> = crds
        .iter()
        .filter(|crd| crd.state == CrdState::Established)
        .filter_map(|crd| {
            let (source, rules) = kind_source(crd)?;
            Some(cached_kind(cache, source, rules, crd))
        })
        .collect();
    kinds.sort_by(|left, right| {
        (&left.resource().group, left.spec().label)
            .cmp(&(&right.resource().group, right.spec().label))
    });
    kinds
}

/// The cached kind of `source`, or a new one. A hit clones nothing.
fn cached_kind(
    cache: &mut CustomKindCache,
    source: CustomKindSource,
    rules: Vec<ColumnRule>,
    crd: &CrdSummary,
) -> CustomKind {
    if let Some(kind) = cache.kinds.get(&source) {
        return *kind;
    }
    let kind = leak_kind(source.clone(), rules, crd);
    cache.kinds.insert(source, kind);
    if cache.kinds.len() > CACHE_WARN_LIMIT && !cache.has_warned {
        cache.has_warned = true;
        tracing::warn!(
            definitions = cache.kinds.len(),
            "the custom kind cache holds many definitions, each leaked for the process"
        );
    }
    kind
}

fn kind_source(crd: &CrdSummary) -> Option<(CustomKindSource, Vec<ColumnRule>)> {
    let version = crd.preferred_version()?;
    let mut columns: Vec<(PrinterColumn, ColumnRule)> = version
        .printer_columns
        .iter()
        .map(|column| {
            let rule = if column.is_condition_status {
                ColumnRule::ConditionStatus
            } else {
                ColumnRule::Plain
            };
            (column.clone(), rule)
        })
        .collect();
    for built_in in BUILT_IN_COLUMNS
        .iter()
        .filter(|built_in| built_in.crd_name == crd.name)
    {
        if columns
            .iter()
            .any(|(column, _)| column.name == built_in.name)
        {
            continue;
        }
        let column = PrinterColumn::new(
            built_in.name,
            built_in.column_type,
            built_in.json_path,
            false,
        );
        let position = columns
            .iter()
            .position(|(column, _)| is_age(column))
            .unwrap_or(columns.len());
        columns.insert(position, (column, built_in.rule));
    }
    let (printer_columns, rules) = columns.into_iter().unzip();
    let source = CustomKindSource {
        crd_name: crd.name.clone(),
        resource: crd.resource(version),
        printer_columns,
    };
    Some((source, rules))
}

fn is_age(column: &PrinterColumn) -> bool {
    column.name == AGE_COLUMN_NAME && column.column_type == ColumnType::Date
}

// ponytail: one spec (about 1 KiB) leaks per distinct CRD definition ever seen in the process,
// bounded by the cache; evict the definitions no session uses if that ever grows.
fn leak_kind(source: CustomKindSource, rules: Vec<ColumnRule>, crd: &CrdSummary) -> CustomKind {
    let label = kind_label(&crd.kind, &crd.plural);
    let singular = crd.singular.clone();
    let columns: Vec<KindColumn> = if source.printer_columns.is_empty() {
        vec![AGE_COLUMN]
    } else {
        source
            .printer_columns
            .iter()
            .zip(&rules)
            .map(|(printer, rule)| kind_column(printer, *rule))
            .collect()
    };
    let spec = KindSpec {
        label: label.leak(),
        name_column: NameColumn::Flexible,
        has_labels: true,
        singular: singular.clone().leak(),
        plural: crd.plural.clone().leak(),
        badge: kind_badge(&crd.kind).leak(),
        icon: IconName::Puzzle,
        is_namespaced: crd.scope == ResourceScope::Namespaced,
        api: KindApi::Custom,
        columns: columns.leak(),
        read_only_actions: if crd.name == CERT_MANAGER_CERTIFICATES {
            &CERTIFICATE_ACTIONS
        } else {
            &[]
        },
        delete_label: format!("Delete {singular}…").leak(),
        has_port_forward: false,
    };
    CustomKind(Box::leak(Box::new(CustomKindSpec {
        source,
        rules,
        spec,
    })))
}

fn kind_column(printer: &PrinterColumn, rule: ColumnRule) -> KindColumn {
    if is_age(printer) {
        return AGE_COLUMN;
    }
    let name = printer.name.clone().leak();
    match printer.column_type {
        ColumnType::Date | ColumnType::Integer | ColumnType::Number => {
            column(name, 90., Align::Right)
        }
        ColumnType::Boolean => column(name, 80., Align::Left),
        ColumnType::String if rule == ColumnRule::ConditionStatus => column(name, 80., Align::Left),
        ColumnType::String => column(name, 160., Align::Left),
    }
}

/// The sidebar label: the kind plus the plural's suffix (`Certificate` + `s`), so `Ingress` +
/// `es` and `Policy` + `ies` read right; any other plural gets its first letter raised.
pub(crate) fn kind_label(kind: &str, plural: &str) -> String {
    let lower = kind.to_ascii_lowercase();
    if let Some(suffix) = plural.strip_prefix(&lower) {
        return format!("{kind}{suffix}");
    }
    if let (Some(stem), Some(kind_stem)) = (lower.strip_suffix('y'), kind.strip_suffix('y'))
        && plural == format!("{stem}ies")
    {
        return format!("{kind_stem}ies");
    }
    let mut chars = plural.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// Two letters for the drawer header: the first, then the next capital lowercased, else the
/// second letter (`Certificate` gives `Ce`, `KafkaTopic` gives `Kt`, `X` gives `X`).
pub(crate) fn kind_badge(kind: &str) -> String {
    let mut chars = kind.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };
    let rest: Vec<char> = chars.collect();
    let second = rest
        .iter()
        .find(|ch| ch.is_ascii_uppercase())
        .map(char::to_ascii_lowercase)
        .or_else(|| rest.first().copied());
    first.to_string() + &second.map(String::from).unwrap_or_default()
}

#[cfg(test)]
#[path = "custom_kind_tests.rs"]
mod custom_kind_tests;
