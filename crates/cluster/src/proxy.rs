//! The proxy a client connects through (spec 0043). A `ProxyUrl` is valid by construction and holds
//! no credential: `parse` refuses userinfo, so neither the settings file nor any `Debug` output can
//! carry a proxy password. A proxy that needs one takes it from the kubeconfig `proxy-url`.

use std::fmt;

/// The longest proxy URL text `ProxyUrl::parse` accepts.
const MAX_URL_CHARS: usize = 2048;

/// How the client reaches the API server.
#[derive(Clone, PartialEq, Eq)]
pub enum ProxyChoice {
    /// The `proxy-url` of the kubeconfig cluster entry, else none. `HTTPS_PROXY` is never used.
    /// Its userinfo, when present, stays inside the kubeconfig document.
    Kubeconfig,
    /// No proxy, even when the kubeconfig sets one.
    Direct,
    /// A proxy picked in Settings.
    Url(ProxyUrl),
}

// Manual: the variant and `scheme://host:port` only.
impl fmt::Debug for ProxyChoice {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Kubeconfig => formatter.write_str("Kubeconfig"),
            Self::Direct => formatter.write_str("Direct"),
            Self::Url(url) => formatter.debug_tuple("Url").field(url).finish(),
        }
    }
}

impl ProxyChoice {
    /// The URI the client config gets. `has_proxy_url` says whether the kubeconfig cluster sets its
    /// own `proxy-url`, and `loaded` is what kube read (the kubeconfig's, else `HTTPS_PROXY`).
    pub(crate) fn config_uri(
        &self,
        has_proxy_url: bool,
        loaded: Option<http::Uri>,
    ) -> Option<http::Uri> {
        match self {
            Self::Kubeconfig if has_proxy_url => loaded,
            Self::Kubeconfig | Self::Direct => None,
            Self::Url(url) => Some(url.0.clone()),
        }
    }
}

/// An `http://` or `socks5://` proxy with a host and an optional port, and nothing else.
#[derive(Clone, PartialEq, Eq)]
pub struct ProxyUrl(http::Uri);

// Manual, though a `ProxyUrl` holds no credential: the shape stays `scheme://host:port`.
impl fmt::Debug for ProxyUrl {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("ProxyUrl")
            .field(&self.display())
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ProxyUrlError {
    #[error("the proxy URL must start with http:// or socks5://")]
    Scheme,
    #[error("the proxy URL must not carry a user name or password")]
    Credentials,
    #[error("the proxy URL has no host")]
    Host,
    #[error("the proxy port must be from 1 to 65535")]
    Port,
    #[error("the proxy URL must not have a path, query, or fragment")]
    Path,
    #[error("the proxy URL is longer than {MAX_URL_CHARS} characters")]
    TooLong,
    #[error("the proxy URL is not a valid URL")]
    Malformed,
}

impl ProxyUrl {
    /// Checks `text` in this order: length and characters, scheme (case-insensitive; stored
    /// lowercase, because kube matches `socks5` case-sensitively), userinfo, host, port, and that
    /// nothing follows the authority but an optional `/`. No error carries the text.
    pub fn parse(text: &str) -> Result<Self, ProxyUrlError> {
        let text = text.trim();
        if text.chars().count() > MAX_URL_CHARS {
            return Err(ProxyUrlError::TooLong);
        }
        if text.is_empty() || text.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err(ProxyUrlError::Malformed);
        }
        let (scheme, rest) = split_scheme(text).ok_or(ProxyUrlError::Scheme)?;
        let (authority, tail) = rest.split_at(rest.find(['/', '?', '#']).unwrap_or(rest.len()));
        if authority.contains('@') {
            return Err(ProxyUrlError::Credentials);
        }
        check_host_and_port(authority)?;
        if !matches!(tail, "" | "/") {
            return Err(ProxyUrlError::Path);
        }
        format!("{scheme}://{authority}")
            .parse::<http::Uri>()
            .map(Self)
            .map_err(|_| ProxyUrlError::Malformed)
    }

    /// `scheme://host[:port]`, the scheme in lowercase.
    pub fn display(&self) -> String {
        let scheme = self.0.scheme_str().unwrap_or_default();
        let authority = self.0.authority().map_or("", http::uri::Authority::as_str);
        format!("{scheme}://{authority}")
    }
}

/// The lowercase scheme and the text after `://`, for the two schemes Settings accepts.
fn split_scheme(text: &str) -> Option<(&'static str, &str)> {
    ["http", "socks5"].into_iter().find_map(|scheme| {
        // `get` fails inside a multi-byte character, which no scheme has.
        let head = text.get(..scheme.len() + 3)?.as_bytes();
        let is_scheme = head[..scheme.len()].eq_ignore_ascii_case(scheme.as_bytes())
            && &head[scheme.len()..] == b"://";
        is_scheme.then(|| (scheme, &text[scheme.len() + 3..]))
    })
}

/// `host`, `host:port`, `[v6]`, or `[v6]:port`, with a host and a port of 1 to 65535.
fn check_host_and_port(authority: &str) -> Result<(), ProxyUrlError> {
    let (host, port) = match authority.strip_prefix('[') {
        Some(inner) => {
            let (host, after) = inner.split_once(']').ok_or(ProxyUrlError::Malformed)?;
            match after {
                "" => (host, None),
                _ => (
                    host,
                    Some(after.strip_prefix(':').ok_or(ProxyUrlError::Malformed)?),
                ),
            }
        }
        None => match authority.split_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (authority, None),
        },
    };
    if host.is_empty() {
        return Err(ProxyUrlError::Host);
    }
    if host.contains(['[', ']']) {
        return Err(ProxyUrlError::Malformed);
    }
    match port {
        None => Ok(()),
        Some(port) => match port.parse::<u16>() {
            Ok(number) if number > 0 && port.bytes().all(|b| b.is_ascii_digit()) => Ok(()),
            _ => Err(ProxyUrlError::Port),
        },
    }
}

#[cfg(test)]
#[path = "proxy_tests.rs"]
mod proxy_tests;
