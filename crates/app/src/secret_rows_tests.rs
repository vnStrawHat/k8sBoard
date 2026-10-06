use cluster::{CertificateInfo, SecretSummary};

use super::*;
use crate::resource_kind::ResourceKind;

fn at(text: &str) -> jiff::Timestamp {
    text.parse().expect("test timestamp")
}

fn secret(
    secret_type: &str,
    details: SecretDetails,
    keys: &[(&str, usize, bool)],
) -> SecretSummary {
    SecretSummary {
        namespace: "shop".to_owned(),
        name: "credentials".to_owned(),
        created_at: None,
        labels: vec!["app=shop".to_owned()],
        secret_type: secret_type.to_owned(),
        keys: keys
            .iter()
            .map(|(name, size_bytes, is_binary)| SecretKey {
                name: (*name).to_owned(),
                size_bytes: *size_bytes,
                is_binary: *is_binary,
            })
            .collect(),
        details,
        is_immutable: false,
        is_owned: false,
    }
}

fn opaque(keys: &[(&str, usize, bool)]) -> SecretSummary {
    secret("Opaque", SecretDetails::None, keys)
}

fn certificate(subject: &str, not_after: &str) -> CertificateInfo {
    CertificateInfo {
        subject: subject.to_owned(),
        issuer: "CN=ca".to_owned(),
        alt_names: vec!["shop.example.test".to_owned()],
        not_before: at("2026-01-01T00:00:00Z"),
        not_after: at(not_after),
    }
}

fn labels_of(rows: &[DetailRow]) -> Vec<String> {
    rows.iter()
        .filter_map(|row| match row {
            DetailRow::Field { label, .. } | DetailRow::Link { label, .. } => {
                Some(label.to_string())
            }
            _ => None,
        })
        .collect()
}

#[test]
fn secret_row_cells_match_column_count() {
    let row = secret_row(&opaque(&[("password", 12, false)]));
    assert_eq!(row.cells.len(), ResourceKind::Secrets.columns().len());
    assert_eq!(row.cells[0], KindCell::Mono("Opaque".into()));
    assert_eq!(row.cells[1], KindCell::count(1));
    assert_eq!(row.cells[2], KindCell::Absent);
    assert_eq!(row.namespace.as_deref(), Some("shop"));
}

#[test]
fn expires_cell_is_the_leaf_not_after_for_tls_only() {
    let opaque_row = secret_row(&opaque(&[]));
    assert_eq!(opaque_row.cells[3], KindCell::Absent);
    let tls = secret(
        "kubernetes.io/tls",
        SecretDetails::Certificate {
            chain: vec![certificate("CN=a", "2027-01-01T00:00:00Z")],
        },
        &[],
    );
    assert_eq!(
        secret_row(&tls).cells[3],
        KindCell::Expiry {
            not_after: at("2027-01-01T00:00:00Z")
        }
    );
    let unparsed = secret(
        "kubernetes.io/tls",
        SecretDetails::NoCertificate(CertificateIssue::Unparsed),
        &[],
    );
    assert_eq!(secret_row(&unparsed).cells[3], KindCell::Absent);
}

#[test]
fn secret_status_is_type_or_not_parsed() {
    let ok = secret_status(&opaque(&[]));
    assert_eq!((ok.text.as_ref(), ok.tone), ("Opaque", StatusTone::Ok));
    let tls = secret(
        "kubernetes.io/tls",
        SecretDetails::Certificate {
            chain: vec![certificate("CN=a", "2027-01-01T00:00:00Z")],
        },
        &[],
    );
    assert_eq!(secret_status(&tls).text, "kubernetes.io/tls");
    let unparsed = secret(
        "kubernetes.io/tls",
        SecretDetails::NoCertificate(CertificateIssue::Unparsed),
        &[],
    );
    let status = secret_status(&unparsed);
    assert_eq!(
        (status.text.as_ref(), status.tone),
        ("Certificate not parsed", StatusTone::Warn)
    );
    let missing = secret(
        "kubernetes.io/tls",
        SecretDetails::NoCertificate(CertificateIssue::Missing),
        &[],
    );
    let status = secret_status(&missing);
    assert_eq!(
        (status.text.as_ref(), status.tone),
        ("No certificate", StatusTone::Warn)
    );
}

#[test]
fn data_section_is_live_only() {
    let row = secret_row(&opaque(&[("password", 12, false)]));
    let data = row.section("Data").expect("a Data section");
    assert_eq!(data.rows, [DetailRow::Live(LiveContent::SecretData)]);
    let empty = secret_row(&opaque(&[]));
    let data = empty.section("Data").expect("a Data section");
    assert_eq!(data.rows, [DetailRow::Note("No data".into())]);
}

#[test]
fn masked_rows_show_mask_size_and_binary() {
    let keys = [
        SecretKey {
            name: "blob".to_owned(),
            size_bytes: 2048,
            is_binary: true,
        },
        SecretKey {
            name: "user".to_owned(),
            size_bytes: 5,
            is_binary: false,
        },
    ];
    let rows = secret_data_rows(&keys);
    assert_eq!(
        rows,
        [
            MaskedKeyRow {
                key: "blob".to_owned(),
                size: "2.0 KiB".to_owned(),
                is_binary: true
            },
            MaskedKeyRow {
                key: "user".to_owned(),
                size: "5 B".to_owned(),
                is_binary: false
            },
        ]
    );
    assert_eq!(MASK, "••••••••••");
}

#[test]
fn no_section_of_a_secret_row_holds_a_value() {
    let row = secret_row(&opaque(&[("password", 12, false)]));
    let text = format!("{:?}", row.sections);
    assert!(text.contains("SecretData"));
    // Rows are built from the summary, which holds sizes only.
    assert!(!text.contains("hunter2"));
}

#[test]
fn token_secret_links_service_account() {
    let token = secret(
        "kubernetes.io/service-account-token",
        SecretDetails::ServiceAccountToken {
            account: Some("builder".to_owned()),
        },
        &[("token", 900, false)],
    );
    let row = secret_row(&token);
    let section = row.section("Secret").expect("a Secret section");
    let link = section
        .rows
        .iter()
        .find_map(|row| match row {
            DetailRow::Link { text, target, .. } => Some((text.to_string(), target.clone())),
            _ => None,
        })
        .expect("a service account link");
    assert_eq!(link.0, "builder");
    assert_eq!(
        link.1,
        ResourceKey::of_object("ServiceAccount", Some("shop"), "builder").expect("a key")
    );
}

#[test]
fn registries_as_chips() {
    let docker = secret(
        "kubernetes.io/dockerconfigjson",
        SecretDetails::Registries(vec![
            "a.example.test".to_owned(),
            "b.example.test".to_owned(),
        ]),
        &[],
    );
    let row = secret_row(&docker);
    let section = row.section("Secret").expect("a Secret section");
    assert!(section.rows.contains(&DetailRow::Chips(vec![
        "a.example.test".into(),
        "b.example.test".into()
    ])));
    let none = secret(
        "kubernetes.io/dockerconfigjson",
        SecretDetails::Registries(Vec::new()),
        &[],
    );
    let row = secret_row(&none);
    let section = row.section("Secret").expect("a Secret section");
    assert!(
        section
            .rows
            .contains(&DetailRow::Note("No registry hosts".into()))
    );
}

#[test]
fn certificate_section_only_for_tls_secrets() {
    assert!(secret_row(&opaque(&[])).section("Certificate").is_none());
    let tls = secret(
        "kubernetes.io/tls",
        SecretDetails::NoCertificate(CertificateIssue::Missing),
        &[],
    );
    let row = secret_row(&tls);
    let section = row.section("Certificate").expect("a Certificate section");
    assert_eq!(section.rows, [DetailRow::Live(LiveContent::Certificate)]);
}

#[test]
fn certificate_section_rows() {
    let details = SecretDetails::Certificate {
        chain: vec![certificate(
            "CN=shop.example.test,O=Example",
            "2027-01-01T00:00:00Z",
        )],
    };
    let rows = certificate_rows(&details);
    assert_eq!(
        labels_of(&rows),
        ["Subject", "Issuer", "Alt names", "Not before", "Not after"]
    );
    assert!(rows.contains(&DetailRow::field(
        "Subject",
        KindCell::Mono("CN=shop.example.test,O=Example".into())
    )));
    assert!(rows.contains(&DetailRow::field(
        "Not before",
        KindCell::Text("2026-01-01 00:00 UTC".into())
    )));
    assert!(rows.contains(&DetailRow::field(
        "Not after",
        KindCell::Expiry {
            not_after: at("2027-01-01T00:00:00Z")
        }
    )));
    assert!(rows.contains(&DetailRow::Chips(vec!["shop.example.test".into()])));
    // One certificate: no Chain row.
    assert!(!labels_of(&rows).contains(&"Chain".to_owned()));
}

#[test]
fn certificate_section_warns_early_intermediate() {
    let details = SecretDetails::Certificate {
        chain: vec![
            certificate("CN=leaf", "2027-01-01T00:00:00Z"),
            certificate("CN=ca", "2026-11-05T00:00:00Z"),
        ],
    };
    let rows = certificate_rows(&details);
    assert!(rows.contains(&DetailRow::field(
        "Intermediate",
        KindCell::Toned(StatusLabel {
            text: "expires Nov 5, 2026, before the leaf".into(),
            tone: StatusTone::Warn
        })
    )));
    assert!(rows.contains(&DetailRow::field(
        "Chain",
        KindCell::Text("2 certificates".into())
    )));
}

#[test]
fn certificate_alt_names_capped() {
    let mut leaf = certificate("CN=leaf", "2027-01-01T00:00:00Z");
    leaf.alt_names = (0..23)
        .map(|index| format!("host-{index}.example.test"))
        .collect();
    let rows = certificate_rows(&SecretDetails::Certificate { chain: vec![leaf] });
    let chips = rows
        .iter()
        .find_map(|row| match row {
            DetailRow::Chips(chips) => Some(chips),
            _ => None,
        })
        .expect("alt name chips");
    assert_eq!(chips.len(), MAX_LISTED_ALT_NAMES + 1);
    assert_eq!(chips.last().map(|chip| chip.as_ref()), Some("+3"));
}

#[test]
fn certificate_notes_for_unusable_certificates() {
    let missing = certificate_rows(&SecretDetails::NoCertificate(CertificateIssue::Missing));
    assert_eq!(
        missing,
        [DetailRow::Note("The secret has no tls.crt.".into())]
    );
    let unparsed = certificate_rows(&SecretDetails::NoCertificate(CertificateIssue::Unparsed));
    assert_eq!(
        unparsed,
        [DetailRow::Note(
            "tls.crt could not be parsed as an X.509 certificate.".into()
        )]
    );
    assert!(certificate_rows(&SecretDetails::None).is_empty());
}
