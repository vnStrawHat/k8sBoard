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
