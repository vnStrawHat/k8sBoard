//! X.509 certificate metadata read from a PEM chain. Only public fields are kept: names,
//! subject alternative names, and validity. Keys, signatures, and serials are never copied.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use x509_cert::Certificate;
use x509_cert::der::Decode;
use x509_cert::ext::pkix::SubjectAltName;
use x509_cert::ext::pkix::name::GeneralName;
use x509_cert::time::Time;

/// A chain longer than this is cut: real chains are three or four certificates.
pub(crate) const CHAIN_LIMIT: usize = 10;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CertificateInfo {
    /// RFC 4514 text of the subject DN, for example `CN=shop.example.test,O=Example`.
    pub subject: String,
    /// The same for the issuer.
    pub issuer: String,
    /// `dNSName` as written and `iPAddress` as address text, in file order; other kinds are
    /// skipped.
    pub alt_names: Vec<String>,
    pub not_before: jiff::Timestamp,
    pub not_after: jiff::Timestamp,
}

/// Why a `kubernetes.io/tls` secret has no certificate to show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CertificateIssue {
    /// `tls.crt` is absent or empty.
    Missing,
    /// No `CERTIFICATE` block, or a block the parser rejects.
    Unparsed,
}

/// The certificates of `pem_text`: leaf first, in file order, never empty, at most
/// `CHAIN_LIMIT`. Blocks of other kinds are skipped; one bad certificate fails the chain.
pub(crate) fn parse_certificates(
    pem_text: &[u8],
) -> Result<Vec<CertificateInfo>, CertificateIssue> {
    let blocks = pem::parse_many(pem_text).map_err(|_| CertificateIssue::Unparsed)?;
    let chain = blocks
        .iter()
        .filter(|block| block.tag() == "CERTIFICATE")
        .take(CHAIN_LIMIT)
        .map(|block| certificate_info(block.contents()))
        .collect::<Result<Vec<_>, _>>()?;
    if chain.is_empty() {
        return Err(CertificateIssue::Unparsed);
    }
    Ok(chain)
}

fn certificate_info(der: &[u8]) -> Result<CertificateInfo, CertificateIssue> {
    // The default RFC 5280 profile; a stricter rejection reads as "could not be parsed".
    let certificate = Certificate::from_der(der).map_err(|_| CertificateIssue::Unparsed)?;
    let tbs = certificate.tbs_certificate();
    let validity = tbs.validity();
    Ok(CertificateInfo {
        subject: tbs.subject().to_string(),
        issuer: tbs.issuer().to_string(),
        alt_names: alt_names(&certificate)?,
        not_before: timestamp(&validity.not_before)?,
        not_after: timestamp(&validity.not_after)?,
    })
}

fn alt_names(certificate: &Certificate) -> Result<Vec<String>, CertificateIssue> {
    let extension = certificate
        .tbs_certificate()
        .get_extension::<SubjectAltName>()
        .map_err(|_| CertificateIssue::Unparsed)?;
    let Some((_is_critical, SubjectAltName(names))) = extension else {
        return Ok(Vec::new());
    };
    Ok(names
        .iter()
        .filter_map(|name| match name {
            GeneralName::DnsName(dns) => Some(dns.as_str().to_owned()),
            GeneralName::IpAddress(octets) => ip_text(octets.as_bytes()),
            _ => None,
        })
        .collect())
}

fn ip_text(octets: &[u8]) -> Option<String> {
    if let Ok(octets) = <[u8; 4]>::try_from(octets) {
        return Some(IpAddr::V4(Ipv4Addr::from(octets)).to_string());
    }
    let octets = <[u8; 16]>::try_from(octets).ok()?;
    Some(IpAddr::V6(Ipv6Addr::from(octets)).to_string())
}

fn timestamp(time: &Time) -> Result<jiff::Timestamp, CertificateIssue> {
    let seconds =
        i64::try_from(time.to_unix_duration().as_secs()).map_err(|_| CertificateIssue::Unparsed)?;
    jiff::Timestamp::from_second(seconds).map_err(|_| CertificateIssue::Unparsed)
}

/// What a new `tls.crt` and `tls.key` look like to the app before they are sent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TlsPairInfo {
    /// RFC 4514 text of the leaf's subject.
    pub subject: String,
    pub not_after: jiff::Timestamp,
    pub key: KeyCheck,
}

/// Whether the private key belongs to the certificate's public key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyCheck {
    Matches,
    Differs,
    /// The key is encrypted or of a kind whose public part is not in the file (Ed25519, or an
    /// EC key written without it): it cannot be compared without computing with it.
    NotChecked,
}

/// Why a pair cannot be used. `Display` is fixed text: it never quotes the input.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum TlsPairIssue {
    #[error("the certificate holds no CERTIFICATE block that parses")]
    Certificate,
    #[error("the key holds no PRIVATE KEY block")]
    NoKey,
}

const KEY_TAGS: [&str; 4] = [
    "PRIVATE KEY",
    "RSA PRIVATE KEY",
    "EC PRIVATE KEY",
    "ENCRYPTED PRIVATE KEY",
];
/// `rsaEncryption` and `id-ecPublicKey`, as DER object identifier contents.
const RSA_OID: &[u8] = &[0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x01, 0x01];
const EC_OID: &[u8] = &[0x2A, 0x86, 0x48, 0xCE, 0x3D, 0x02, 0x01];
const RSA_CERTIFICATE: &str = "1.2.840.113549.1.1.1";
const EC_CERTIFICATE: &str = "1.2.840.10045.2.1";
const TAG_INTEGER: u8 = 0x02;
const TAG_BIT_STRING: u8 = 0x03;
const TAG_OCTET_STRING: u8 = 0x04;
const TAG_OID: u8 = 0x06;
const TAG_SEQUENCE: u8 = 0x30;
/// `[1]` of an `ECPrivateKey`: the public point.
const TAG_EC_PUBLIC: u8 = 0xA1;

/// Reads the leaf certificate of `certificate_pem` and checks `key_pem` against it. The key text is
/// only read: nothing of it is kept, copied, or printed.
pub fn check_tls_pair(certificate_pem: &[u8], key_pem: &[u8]) -> Result<TlsPairInfo, TlsPairIssue> {
    let blocks = pem::parse_many(certificate_pem).map_err(|_| TlsPairIssue::Certificate)?;
    let leaf = blocks
        .iter()
        .find(|block| block.tag() == "CERTIFICATE")
        .ok_or(TlsPairIssue::Certificate)?;
    let info = certificate_info(leaf.contents()).map_err(|_| TlsPairIssue::Certificate)?;
    let certificate =
        Certificate::from_der(leaf.contents()).map_err(|_| TlsPairIssue::Certificate)?;
    let key_blocks = pem::parse_many(key_pem).map_err(|_| TlsPairIssue::NoKey)?;
    let key = key_blocks
        .iter()
        .find(|block| KEY_TAGS.contains(&block.tag()))
        .ok_or(TlsPairIssue::NoKey)?;
    let public = certificate.tbs_certificate().subject_public_key_info();
    let key = key_check(
        &public.algorithm.oid.to_string(),
        public.subject_public_key.raw_bytes(),
        key.tag(),
        key.contents(),
    );
    Ok(TlsPairInfo {
        subject: info.subject,
        not_after: info.not_after,
        key,
    })
}

/// Compares the public key the certificate carries with the one inside the private key. An RSA
/// private key holds the modulus and the exponent, and an EC key usually holds the point, so
/// neither needs any arithmetic.
fn key_check(algorithm: &str, public: &[u8], tag: &str, private: &[u8]) -> KeyCheck {
    let parts = match tag {
        "RSA PRIVATE KEY" => Some((RSA_OID, private)),
        "EC PRIVATE KEY" => Some((EC_OID, private)),
        "PRIVATE KEY" => pkcs8_parts(private),
        _ => None,
    };
    let Some((key_algorithm, inner)) = parts else {
        return KeyCheck::NotChecked;
    };
    match (key_algorithm, algorithm) {
        (RSA_OID, RSA_CERTIFICATE) => same_rsa_key(public, inner),
        (EC_OID, EC_CERTIFICATE) => same_ec_key(public, inner),
        // A key of one algorithm never belongs to a certificate of the other.
        (RSA_OID | EC_OID, RSA_CERTIFICATE | EC_CERTIFICATE) => KeyCheck::Differs,
        _ => KeyCheck::NotChecked,
    }
}

/// The algorithm identifier and the inner key of a PKCS#8 `PrivateKeyInfo`.
fn pkcs8_parts(der: &[u8]) -> Option<(&[u8], &[u8])> {
    let (_, info, _) = der_element(der, TAG_SEQUENCE)?;
    let (_, _version, rest) = der_element(info, TAG_INTEGER)?;
    let (_, algorithm, rest) = der_element(rest, TAG_SEQUENCE)?;
    let (_, oid, _) = der_element(algorithm, TAG_OID)?;
    let (_, inner, _) = der_element(rest, TAG_OCTET_STRING)?;
    Some((oid, inner))
}

/// `RSAPublicKey` (`SEQUENCE { n, e }`) against `RSAPrivateKey` (`SEQUENCE { version, n, e, … }`).
fn same_rsa_key(public: &[u8], private: &[u8]) -> KeyCheck {
    let public_parts = (|| {
        let (_, key, _) = der_element(public, TAG_SEQUENCE)?;
        let (_, modulus, rest) = der_element(key, TAG_INTEGER)?;
        let (_, exponent, _) = der_element(rest, TAG_INTEGER)?;
        Some((modulus, exponent))
    })();
    let private_parts = (|| {
        let (_, key, _) = der_element(private, TAG_SEQUENCE)?;
        let (_, _version, rest) = der_element(key, TAG_INTEGER)?;
        let (_, modulus, rest) = der_element(rest, TAG_INTEGER)?;
        let (_, exponent, _) = der_element(rest, TAG_INTEGER)?;
        Some((modulus, exponent))
    })();
    let (Some((modulus, exponent)), Some((private_modulus, private_exponent))) =
        (public_parts, private_parts)
    else {
        return KeyCheck::NotChecked;
    };
    // A leading zero byte keeps an unsigned integer positive; it is not part of the value.
    let value = |bytes: &[u8]| {
        let start = bytes
            .iter()
            .position(|byte| *byte != 0)
            .unwrap_or(bytes.len());
        bytes[start..].to_vec()
    };
    if value(modulus) == value(private_modulus) && value(exponent) == value(private_exponent) {
        KeyCheck::Matches
    } else {
        KeyCheck::Differs
    }
}

/// The point of the certificate against the `[1] publicKey` of the `ECPrivateKey`, when the file
/// has it.
fn same_ec_key(public: &[u8], private: &[u8]) -> KeyCheck {
    let point = (|| {
        let (_, key, _) = der_element(private, TAG_SEQUENCE)?;
        let (_, _version, mut rest) = der_element(key, TAG_INTEGER)?;
        while !rest.is_empty() {
            let (tag, content, after) = der_element_any(rest)?;
            if tag == TAG_EC_PUBLIC {
                let (_, bits, _) = der_element(content, TAG_BIT_STRING)?;
                // The first byte counts unused bits, which a point has none of.
                return bits.split_first().map(|(_, point)| point);
            }
            rest = after;
        }
        None
    })();
    match point {
        Some(point) if point == public => KeyCheck::Matches,
        Some(_) => KeyCheck::Differs,
        None => KeyCheck::NotChecked,
    }
}

/// The next DER element if it has `expected` as its tag: `(tag, content, what follows)`.
fn der_element(input: &[u8], expected: u8) -> Option<(u8, &[u8], &[u8])> {
    der_element_any(input).filter(|(tag, _, _)| *tag == expected)
}

fn der_element_any(input: &[u8]) -> Option<(u8, &[u8], &[u8])> {
    let (&tag, rest) = input.split_first()?;
    let (&first, rest) = rest.split_first()?;
    let (length, rest) = if first < 0x80 {
        (usize::from(first), rest)
    } else {
        let count = usize::from(first & 0x7F);
        if count == 0 || count > 4 || rest.len() < count {
            return None;
        }
        let (length_bytes, rest) = rest.split_at(count);
        let length = length_bytes
            .iter()
            .fold(0_usize, |sum, byte| (sum << 8) | usize::from(*byte));
        (length, rest)
    };
    (rest.len() >= length).then(|| (tag, &rest[..length], &rest[length..]))
}

#[cfg(test)]
#[path = "certificate_tests.rs"]
mod certificate_tests;
