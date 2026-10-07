//! The Secret row builder. A row holds the summary (key names, sizes, type, certificate metadata)
//! and never a value; the drawer's Data, Certificate, and Used by sections are `Live` placeholders
//! that read the summary and the live lists at paint time (`live_sections.rs`).

use cluster::{CertificateInfo, CertificateIssue, SecretDetails, SecretKey, SecretSummary};

use crate::certificate_expiry::{date_text, date_time_text, intermediate_expires_first};
use crate::config_map_rows::format_bytes;
use crate::kind_row::{
    DetailRow, DetailSection, KindCell, KindObject, KindRow, LiveContent, chips,
};
use crate::status_tone::{StatusLabel, StatusTone};
use crate::table_selection::ResourceKey;

/// What every masked value reads as. It is fixed, so it leaks nothing beyond the size beside it.
pub(crate) const MASK: &str = "••••••••••";
/// How many alternative names a certificate field lists before `+{n}`.
const MAX_LISTED_ALT_NAMES: usize = 20;

/// The Type cell: a type with a domain (`bootstrap.kubernetes.io/token`) is cut in the middle like
/// a qualified name, so the part after the slash, which tells the types apart, stays visible.
fn type_cell(secret_type: &str) -> KindCell {
    match secret_type.rsplit_once('/') {
        Some((domain, kind)) => KindCell::Qualified {
            prefix: Some(domain.to_owned().into()),
            text: kind.to_owned().into(),
        },
        None => KindCell::Mono(secret_type.to_owned().into()),
    }
}

pub(crate) fn secret_row(secret: &SecretSummary) -> KindRow {
    let data = if secret.keys.is_empty() {
        DetailRow::Note("No data".into())
    } else {
        DetailRow::Live(LiveContent::SecretData)
    };
    let mut sections = vec![
        DetailSection {
            title: "Secret",
            rows: secret_rows(secret),
        },
        DetailSection {
            title: "Data",
            rows: vec![data],
        },
    ];
    if matches!(
        secret.details,
        SecretDetails::Certificate { .. } | SecretDetails::NoCertificate(_)
    ) {
        sections.push(DetailSection {
            title: "Certificate",
            rows: vec![DetailRow::Live(LiveContent::Certificate)],
        });
    }
    sections.push(DetailSection {
        title: "Used by",
        rows: vec![DetailRow::Live(LiveContent::UsedBy)],
    });
    KindRow {
        namespace: Some(secret.namespace.clone()),
        name: secret.name.clone(),
        created_at: secret.created_at,
        status: secret_status(secret),
        cells: vec![
            type_cell(&secret.secret_type),
            KindCell::count(secret.keys.len()),
            // The pods and Ingresses join fills it once both lists have loaded.
            KindCell::Absent,
            expires_cell(&secret.details),
            KindCell::age(secret.created_at),
        ],
        sections,
        event: None,
        related_pods: None,
        labels: chips(&secret.labels),
        object: KindObject::Secret(secret.clone()),
    }
}

/// The leaf's not-after of a TLS secret; other secrets have no expiry.
fn expires_cell(details: &SecretDetails) -> KindCell {
    match details {
        SecretDetails::Certificate { chain } => {
            chain
                .first()
                .map_or(KindCell::Absent, |leaf| KindCell::Expiry {
                    not_after: leaf.not_after,
                })
        }
        _ => KindCell::Absent,
    }
}

/// The type, unless the certificate is unusable. Time-dependent states live in the box.
pub(crate) fn secret_status(secret: &SecretSummary) -> StatusLabel {
    let (text, tone) = match &secret.details {
        SecretDetails::NoCertificate(CertificateIssue::Unparsed) => {
            ("Certificate not parsed", StatusTone::Warn)
        }
        SecretDetails::NoCertificate(CertificateIssue::Missing) => {
            ("No certificate", StatusTone::Warn)
        }
        _ => (secret.secret_type.as_str(), StatusTone::Ok),
    };
    StatusLabel {
        text: text.to_owned().into(),
        tone,
    }
}

/// The type and the details that hold names only: the token's account and the registry hosts.
fn secret_rows(secret: &SecretSummary) -> Vec<DetailRow> {
    let mut rows = vec![
        DetailRow::field("Type", KindCell::Mono(secret.secret_type.clone().into())),
        DetailRow::field(
            "Immutable",
            KindCell::Text(if secret.is_immutable { "yes" } else { "no" }.into()),
        ),
    ];
    match &secret.details {
        SecretDetails::ServiceAccountToken {
            account: Some(account),
        } => rows.push(account_row(&secret.namespace, account)),
        SecretDetails::Registries(hosts) if hosts.is_empty() => {
            rows.push(DetailRow::Note("No registry hosts".into()));
        }
        SecretDetails::Registries(hosts) => {
            rows.push(DetailRow::field("Registries", KindCell::count(hosts.len())));
            rows.push(DetailRow::Chips(chips(hosts)));
        }
        SecretDetails::None
        | SecretDetails::Certificate { .. }
        | SecretDetails::NoCertificate(_)
        | SecretDetails::ServiceAccountToken { account: None } => {}
    }
    rows
}

/// A link to the service account's row, or its name as text when the kind has no screen.
fn account_row(namespace: &str, account: &str) -> DetailRow {
    match ResourceKey::of_object("ServiceAccount", Some(namespace), account) {
        Some(target) => DetailRow::Link {
            label: "Service account".into(),
            text: account.to_owned().into(),
            target,
        },
        None => DetailRow::field("Service account", KindCell::Mono(account.to_owned().into())),
    }
}

/// One key of the Data section before any value is revealed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MaskedKeyRow {
    pub(crate) key: String,
    /// `412 B`.
    pub(crate) size: String,
    pub(crate) is_binary: bool,
}

/// The masked rows of `keys`, in key order. The mask itself is `MASK`.
pub(crate) fn secret_data_rows(keys: &[SecretKey]) -> Vec<MaskedKeyRow> {
    keys.iter()
        .map(|key| MaskedKeyRow {
            key: key.name.clone(),
            size: format_bytes(key.size_bytes),
            is_binary: key.is_binary,
        })
        .collect()
}

/// The Certificate section of a TLS secret: the leaf's fields, then chain facts. Dates are UTC.
pub(crate) fn certificate_rows(details: &SecretDetails) -> Vec<DetailRow> {
    let chain = match details {
        SecretDetails::Certificate { chain } => chain,
        SecretDetails::NoCertificate(CertificateIssue::Missing) => {
            return vec![DetailRow::Note("The secret has no tls.crt.".into())];
        }
        SecretDetails::NoCertificate(CertificateIssue::Unparsed) => {
            return vec![DetailRow::Note(
                "tls.crt could not be parsed as an X.509 certificate.".into(),
            )];
        }
        SecretDetails::None
        | SecretDetails::Registries(_)
        | SecretDetails::ServiceAccountToken { .. } => return Vec::new(),
    };
    let Some(leaf) = chain.first() else {
        return Vec::new();
    };
    let mut rows = vec![
        DetailRow::field("Subject", KindCell::Mono(leaf.subject.clone().into())),
        DetailRow::field("Issuer", KindCell::Mono(leaf.issuer.clone().into())),
    ];
    if !leaf.alt_names.is_empty() {
        rows.push(DetailRow::field(
            "Alt names",
            KindCell::count(leaf.alt_names.len()),
        ));
        rows.push(DetailRow::Chips(alt_name_chips(&leaf.alt_names)));
    }
    rows.push(DetailRow::field(
        "Not before",
        KindCell::Text(date_time_text(leaf.not_before).into()),
    ));
    rows.push(DetailRow::field(
        "Not after",
        KindCell::Expiry {
            not_after: leaf.not_after,
        },
    ));
    rows.extend(intermediate_row(chain));
    if chain.len() > 1 {
        rows.push(DetailRow::field(
            "Chain",
            KindCell::Text(format!("{} certificates", chain.len()).into()),
        ));
    }
    rows
}

/// The Intermediate warning, only when a CA of the chain expires before the leaf.
fn intermediate_row(chain: &[CertificateInfo]) -> Option<DetailRow> {
    let earliest = intermediate_expires_first(chain)?;
    Some(DetailRow::field(
        "Intermediate",
        KindCell::Toned(StatusLabel {
            text: format!("expires {}, before the leaf", date_text(earliest)).into(),
            tone: StatusTone::Warn,
        }),
    ))
}

/// The leaf facts an Ingress shows for a secret it names: Subject, Issuer, Not after, and the
/// Intermediate warning. Notes for a secret without a usable certificate.
pub(crate) fn certificate_summary_rows(details: &SecretDetails) -> Vec<DetailRow> {
    let SecretDetails::Certificate { chain } = details else {
        return certificate_rows(details);
    };
    let Some(leaf) = chain.first() else {
        return Vec::new();
    };
    let mut rows = vec![
        DetailRow::field("Subject", KindCell::Mono(leaf.subject.clone().into())),
        DetailRow::field("Issuer", KindCell::Mono(leaf.issuer.clone().into())),
        DetailRow::field(
            "Not after",
            KindCell::Expiry {
                not_after: leaf.not_after,
            },
        ),
    ];
    rows.extend(intermediate_row(chain));
    rows
}

fn alt_name_chips(names: &[String]) -> Vec<gpui_kit::SharedString> {
    let mut terms = chips(&names[..names.len().min(MAX_LISTED_ALT_NAMES)]);
    if names.len() > MAX_LISTED_ALT_NAMES {
        terms.push(format!("+{}", names.len() - MAX_LISTED_ALT_NAMES).into());
    }
    terms
}

#[cfg(test)]
#[path = "secret_rows_tests.rs"]
mod secret_rows_tests;
