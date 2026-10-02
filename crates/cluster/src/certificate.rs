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

#[cfg(test)]
#[path = "certificate_tests.rs"]
mod certificate_tests;
