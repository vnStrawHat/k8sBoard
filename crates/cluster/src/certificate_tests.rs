use super::*;

const LEAF: &[u8] = include_bytes!("../tests/fixtures/tls/leaf.pem");
const CHAIN: &[u8] = include_bytes!("../tests/fixtures/tls/chain.pem");
const GARBAGE: &[u8] = include_bytes!("../tests/fixtures/tls/garbage.pem");

fn at(text: &str) -> jiff::Timestamp {
    text.parse().expect("test timestamp")
}

#[test]
fn leaf_fields_are_read() {
    let chain = parse_certificates(LEAF).expect("leaf parses");
    assert_eq!(chain.len(), 1);
    // Dates as `openssl x509 -noout -dates` printed them when the fixture was made.
    assert_eq!(chain[0].subject, "CN=shop.example.test,O=Example");
    assert_eq!(chain[0].issuer, "CN=Example Test CA,O=Example");
    assert_eq!(chain[0].alt_names, ["shop.example.test", "10.0.0.1"]);
    assert_eq!(chain[0].not_before, at("2026-10-02T07:42:08Z"));
    assert_eq!(chain[0].not_after, at("2125-04-26T07:42:08Z"));
}

#[test]
fn chain_keeps_file_order() {
    let chain = parse_certificates(CHAIN).expect("chain parses");
    assert_eq!(chain.len(), 2);
    assert_eq!(chain[0].subject, "CN=shop.example.test,O=Example");
    assert_eq!(chain[1].subject, "CN=Example Test CA,O=Example");
    assert_eq!(chain[1].not_after, at("2076-01-13T07:42:08Z"));
    assert!(chain[1].not_after < chain[0].not_after);
}

#[test]
fn chain_is_cut_at_limit() {
    let text = LEAF.repeat(CHAIN_LIMIT + 3);
    let chain = parse_certificates(&text).expect("repeated leaf parses");
    assert_eq!(chain.len(), CHAIN_LIMIT);
}

#[test]
fn non_certificate_blocks_are_skipped() {
    let mut text = b"-----BEGIN PRIVATE KEY-----\nAAAA\n-----END PRIVATE KEY-----\n".to_vec();
    text.extend_from_slice(LEAF);
    let chain = parse_certificates(&text).expect("certificate after the key block parses");
    assert_eq!(chain.len(), 1);
}

#[test]
fn garbage_is_unparsed() {
    assert_eq!(parse_certificates(GARBAGE), Err(CertificateIssue::Unparsed));
    assert_eq!(
        parse_certificates(b"not pem at all"),
        Err(CertificateIssue::Unparsed)
    );
}

#[test]
fn empty_input_is_unparsed() {
    assert_eq!(parse_certificates(b""), Err(CertificateIssue::Unparsed));
}

#[test]
fn one_bad_block_makes_chain_unparsed() {
    let mut text = LEAF.to_vec();
    text.extend_from_slice(GARBAGE);
    assert_eq!(parse_certificates(&text), Err(CertificateIssue::Unparsed));
}

#[test]
fn ip_addresses_of_both_families_are_text() {
    assert_eq!(ip_text(&[10, 0, 0, 1]).as_deref(), Some("10.0.0.1"));
    let mut v6 = [0_u8; 16];
    v6[15] = 1;
    assert_eq!(ip_text(&v6).as_deref(), Some("::1"));
    assert_eq!(ip_text(&[1, 2, 3]), None);
}

const RSA_CERT: &[u8] = include_bytes!("../tests/fixtures/tls/pair-rsa.crt");
const RSA_PKCS8: &[u8] = include_bytes!("../tests/fixtures/tls/pair-rsa-pkcs8.key");
const RSA_PKCS1: &[u8] = include_bytes!("../tests/fixtures/tls/pair-rsa-pkcs1.key");
const EC_CERT: &[u8] = include_bytes!("../tests/fixtures/tls/pair-ec.crt");
const EC_SEC1: &[u8] = include_bytes!("../tests/fixtures/tls/pair-ec-sec1.key");
const EC_PKCS8: &[u8] = include_bytes!("../tests/fixtures/tls/pair-ec-pkcs8.key");
const OTHER_RSA_PKCS8: &[u8] = include_bytes!("../tests/fixtures/tls/other-rsa-pkcs8.key");

fn key_of(certificate: &[u8], key: &[u8]) -> KeyCheck {
    check_tls_pair(certificate, key).expect("a usable pair").key
}

#[test]
fn a_pair_reads_the_subject_and_the_not_after_of_the_leaf() {
    let pair = check_tls_pair(RSA_CERT, RSA_PKCS8).expect("a usable pair");
    assert_eq!(pair.subject, "O=Example,CN=api.example.test");
    assert_eq!(pair.not_after, at("2126-09-13T03:21:16Z"));
}

#[test]
fn an_rsa_key_matches_its_certificate_in_both_encodings() {
    assert_eq!(key_of(RSA_CERT, RSA_PKCS8), KeyCheck::Matches);
    assert_eq!(key_of(RSA_CERT, RSA_PKCS1), KeyCheck::Matches);
}

#[test]
fn an_ec_key_matches_its_certificate_in_both_encodings() {
    assert_eq!(key_of(EC_CERT, EC_SEC1), KeyCheck::Matches);
    assert_eq!(key_of(EC_CERT, EC_PKCS8), KeyCheck::Matches);
}

#[test]
fn another_rsa_key_differs() {
    assert_eq!(key_of(RSA_CERT, OTHER_RSA_PKCS8), KeyCheck::Differs);
}

#[test]
fn a_key_of_the_other_algorithm_differs() {
    assert_eq!(key_of(RSA_CERT, EC_SEC1), KeyCheck::Differs);
    assert_eq!(key_of(EC_CERT, RSA_PKCS8), KeyCheck::Differs);
}

#[test]
fn a_chain_is_judged_by_its_first_certificate() {
    let mut chain = RSA_CERT.to_vec();
    chain.extend_from_slice(LEAF);
    assert_eq!(key_of(&chain, RSA_PKCS8), KeyCheck::Matches);
}

#[test]
fn an_encrypted_or_unreadable_key_is_not_checked() {
    let encrypted =
        b"-----BEGIN ENCRYPTED PRIVATE KEY-----\nAAAA\n-----END ENCRYPTED PRIVATE KEY-----\n";
    assert_eq!(key_of(RSA_CERT, encrypted), KeyCheck::NotChecked);
    let broken = b"-----BEGIN PRIVATE KEY-----\nAAAA\n-----END PRIVATE KEY-----\n";
    assert_eq!(key_of(RSA_CERT, broken), KeyCheck::NotChecked);
}

#[test]
fn text_without_a_block_is_an_issue() {
    assert_eq!(
        check_tls_pair(b"not a certificate", RSA_PKCS8),
        Err(TlsPairIssue::Certificate)
    );
    assert_eq!(
        check_tls_pair(GARBAGE, RSA_PKCS8),
        Err(TlsPairIssue::Certificate)
    );
    assert_eq!(check_tls_pair(RSA_CERT, b""), Err(TlsPairIssue::NoKey));
    // The certificate is not a key.
    assert_eq!(check_tls_pair(RSA_CERT, RSA_CERT), Err(TlsPairIssue::NoKey));
}

#[test]
fn an_issue_names_no_input() {
    for issue in [TlsPairIssue::Certificate, TlsPairIssue::NoKey] {
        assert!(!format!("{issue}").contains("BEGIN"));
    }
}
