use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};

/// One or more kubeconfig files merged like kubectl. Holds credentials.
// Debug is manual: it prints the sources and context names only.
#[derive(Clone)]
pub struct Kubeconfig {
    sources: Vec<PathBuf>,
    document: kube::config::Kubeconfig,
    contexts: Vec<ContextSummary>,
}

/// One usable context of a kubeconfig. Holds names only, never credentials.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContextSummary {
    pub name: String,
    /// Kubeconfig cluster entry name.
    pub cluster: String,
    /// Kubeconfig user entry name.
    pub user: Option<String>,
    pub namespace: Option<String>,
    /// The file that defined this context.
    pub source: PathBuf,
}

/// What a context connects to, for display. Holds no credential value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConnectionInfo {
    /// `scheme://host[:port]`: userinfo, path, query, and fragment are dropped.
    pub server: Option<String>,
    pub auth: AuthKind,
}

/// How a context authenticates: the kind only, never a token, key, or argument.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuthKind {
    Token,
    TokenFile,
    ClientCertificate,
    /// `command` is the file name of the plugin executable, without arguments or environment.
    Exec {
        command: String,
    },
    AuthProvider {
        name: String,
    },
    Basic,
    None,
}

impl fmt::Display for AuthKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Token => formatter.write_str("token"),
            Self::TokenFile => formatter.write_str("token file"),
            Self::ClientCertificate => formatter.write_str("client certificate"),
            Self::Exec { command } => write!(formatter, "exec: {command}"),
            Self::AuthProvider { name } => write!(formatter, "auth provider: {name}"),
            Self::Basic => formatter.write_str("basic"),
            Self::None => formatter.write_str("none"),
        }
    }
}

/// The context, cluster, and user entry names of a kubeconfig, in file order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EntryNames {
    pub contexts: Vec<String>,
    pub clusters: Vec<String>,
    pub users: Vec<String>,
}

/// The merged kubeconfig and the errors of the files that were skipped.
#[derive(Debug)]
pub struct LoadedKubeconfig {
    pub kubeconfig: Kubeconfig,
    pub skipped: Vec<KubeconfigError>,
}

/// Where a context name came from, for error messages.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContextOrigin {
    Requested,
    CurrentContext,
}

impl fmt::Display for ContextOrigin {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Requested => formatter.write_str("context"),
            Self::CurrentContext => formatter.write_str("current-context"),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum KubeconfigError {
    #[error("cannot read kubeconfig '{}'", .path.display())]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    // No source on purpose: the parser message can quote a line that holds a token.
    #[error("kubeconfig '{}' is not a valid kubeconfig YAML document", .path.display())]
    Parse { path: PathBuf },
    #[error("kubeconfig '{}' has a different kind or apiVersion; skipped", .path.display())]
    Incompatible { path: PathBuf },
    #[error("no kubeconfig file was given")]
    NoFiles,
    #[error("{origin} '{requested}' not found in kubeconfig '{}'; available contexts: {}",
            path_list(.paths), context_list(.available))]
    ContextNotFound {
        paths: Vec<PathBuf>,
        requested: String,
        origin: ContextOrigin,
        available: Vec<String>,
    },
    #[error("kubeconfig '{}' has no current-context and no context was requested; available contexts: {}",
            path_list(.paths), context_list(.available))]
    NoContextSelected {
        paths: Vec<PathBuf>,
        available: Vec<String>,
    },
}

fn path_list(paths: &[PathBuf]) -> String {
    let names: Vec<String> = paths
        .iter()
        .map(|path| path.display().to_string())
        .collect();
    names.join(", ")
}

fn context_list(names: &[String]) -> String {
    if names.is_empty() {
        return "(none)".to_owned();
    }
    names.join(", ")
}

impl Kubeconfig {
    /// Loads `paths` in order and merges them like kubectl: the first file wins per named
    /// entry and for `current-context`. A file that cannot be read, parsed, or merged is
    /// skipped and reported; `Err` only when no file loads (the first error).
    /// Blocking file I/O: call it off the UI thread.
    pub fn load(paths: &[PathBuf]) -> Result<LoadedKubeconfig, KubeconfigError> {
        let mut merged: Option<kube::config::Kubeconfig> = None;
        let mut sources = Vec::new();
        let mut origins = HashMap::new();
        let mut skipped = Vec::new();
        for path in paths {
            // `read_from` (not `from_yaml`) makes relative credential paths absolute.
            let next = match kube::config::Kubeconfig::read_from(path) {
                Ok(next) => next,
                Err(error) => {
                    skipped.push(load_error(path, error));
                    continue;
                }
            };
            if let Some(accumulated) = &merged
                && is_incompatible(accumulated, &next)
            {
                // `merge` consumes the accumulator and drops it on `Err`, so the check that
                // would make it fail runs first.
                skipped.push(KubeconfigError::Incompatible { path: path.clone() });
                continue;
            }
            for named in &next.contexts {
                origins
                    .entry(named.name.clone())
                    .or_insert_with(|| path.clone());
            }
            let accumulated = match merged.take() {
                None => next,
                Some(accumulated) => accumulated
                    .merge(next)
                    .map_err(|_| KubeconfigError::Incompatible { path: path.clone() })?,
            };
            sources.push(path.clone());
            merged = Some(accumulated);
        }
        match merged {
            Some(document) => Ok(LoadedKubeconfig {
                kubeconfig: Self::from_document(sources, document, &origins),
                skipped,
            }),
            None => Err(skipped
                .into_iter()
                .next()
                .unwrap_or(KubeconfigError::NoFiles)),
        }
    }

    /// Parses YAML text in memory (pasted content). `origin` is used in errors and as the
    /// context source only; relative credential paths stay relative (the file is re-loaded with
    /// `load` after it is written).
    pub fn parse(text: &str, origin: &Path) -> Result<Kubeconfig, KubeconfigError> {
        // The parser error is dropped: it can quote a line that holds a token.
        let document =
            kube::config::Kubeconfig::from_yaml(text).map_err(|_| KubeconfigError::Parse {
                path: origin.to_path_buf(),
            })?;
        Ok(Self::from_document(
            vec![origin.to_path_buf()],
            document,
            &HashMap::new(),
        ))
    }

    /// Server and auth kind of a context; never a credential value.
    pub fn connection_info(&self, context: &ContextSummary) -> ConnectionInfo {
        let server = self
            .document
            .clusters
            .iter()
            .find(|named| named.name == context.cluster)
            .and_then(|named| named.cluster.as_ref())
            .and_then(|cluster| cluster.server.as_deref())
            .map(display_server);
        let auth_info = context.user.as_deref().and_then(|user| {
            self.document
                .auth_infos
                .iter()
                .find(|named| named.name == user)
                .and_then(|named| named.auth_info.as_ref())
        });
        ConnectionInfo {
            server,
            auth: auth_info.map_or(AuthKind::None, auth_kind),
        }
    }

    /// Context, cluster, and user entry names, in file order.
    pub fn entry_names(&self) -> EntryNames {
        EntryNames {
            contexts: self.context_names(),
            clusters: self
                .document
                .clusters
                .iter()
                .map(|named| named.name.clone())
                .collect(),
            users: self
                .document
                .auth_infos
                .iter()
                .map(|named| named.name.clone())
                .collect(),
        }
    }

    fn from_document(
        sources: Vec<PathBuf>,
        document: kube::config::Kubeconfig,
        origins: &HashMap<String, PathBuf>,
    ) -> Self {
        let mut contexts: Vec<ContextSummary> = Vec::new();
        for named in &document.contexts {
            let Some(context) = &named.context else {
                continue;
            };
            // kube's lookup takes the first entry with a given name.
            if contexts.iter().any(|existing| existing.name == named.name) {
                continue;
            }
            let source = origins
                .get(&named.name)
                .or_else(|| sources.first())
                .cloned()
                .unwrap_or_default();
            contexts.push(ContextSummary {
                name: named.name.clone(),
                cluster: context.cluster.clone(),
                user: context.user.clone(),
                namespace: context.namespace.clone(),
                source,
            });
        }
        Self {
            sources,
            document,
            contexts,
        }
    }

    /// The files that loaded, in merge order.
    pub fn sources(&self) -> &[PathBuf] {
        &self.sources
    }

    pub fn contexts(&self) -> &[ContextSummary] {
        &self.contexts
    }

    /// The raw `current-context` value. It may name a context that does not exist.
    pub fn current_context(&self) -> Option<&str> {
        self.document
            .current_context
            .as_deref()
            .filter(|name| !name.is_empty())
    }

    pub fn resolve_context(
        &self,
        requested: Option<&str>,
    ) -> Result<&ContextSummary, KubeconfigError> {
        let (name, origin) = match requested {
            Some(name) => (name, ContextOrigin::Requested),
            None => match self.current_context() {
                Some(name) => (name, ContextOrigin::CurrentContext),
                None => {
                    return Err(KubeconfigError::NoContextSelected {
                        paths: self.sources.clone(),
                        available: self.context_names(),
                    });
                }
            },
        };
        self.contexts
            .iter()
            .find(|context| context.name == name)
            .ok_or_else(|| KubeconfigError::ContextNotFound {
                paths: self.sources.clone(),
                requested: name.to_owned(),
                origin,
                available: self.context_names(),
            })
    }

    /// Whether the cluster entry sets its own `proxy-url`.
    pub(crate) fn has_proxy_url(&self, cluster_name: &str) -> bool {
        self.document
            .clusters
            .iter()
            .find(|named| named.name == cluster_name)
            .and_then(|named| named.cluster.as_ref())
            .and_then(|cluster| cluster.proxy_url.as_deref())
            .is_some_and(|url| !url.is_empty())
    }

    pub(crate) fn document(&self) -> &kube::config::Kubeconfig {
        &self.document
    }

    fn context_names(&self) -> Vec<String> {
        self.contexts
            .iter()
            .map(|context| context.name.clone())
            .collect()
    }
}

impl fmt::Debug for Kubeconfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Kubeconfig")
            .field("sources", &self.sources)
            .field("contexts", &self.context_names())
            .finish()
    }
}

/// `scheme://host[:port]` of a server URL. Done by hand: the value is free text, and a URL
/// parser dependency is not worth it for display.
fn display_server(server: &str) -> String {
    let (scheme, rest) = match server.split_once("://") {
        Some((scheme, rest)) => (Some(scheme), rest),
        None => (None, server),
    };
    let (authority, tail) = rest.split_at(rest.find(['/', '?', '#']).unwrap_or(rest.len()));
    // An `@` after the first delimiter means the delimiter may be part of a raw password
    // (`u:pa/ss@host`) or the path holds an `@`. Which one cannot be told, so no host is shown:
    // part of a password must never reach the screen.
    if tail.contains('@') {
        return scheme.map_or_else(|| "…".to_owned(), |scheme| format!("{scheme}://…"));
    }
    let host = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    match scheme {
        Some(scheme) => format!("{scheme}://{host}"),
        None => host.to_owned(),
    }
}

fn auth_kind(auth: &kube::config::AuthInfo) -> AuthKind {
    if let Some(exec) = &auth.exec {
        let command = exec.command.as_deref().unwrap_or_default();
        return AuthKind::Exec {
            command: command
                .rsplit(['/', '\\'])
                .next()
                .unwrap_or_default()
                .to_owned(),
        };
    }
    if let Some(provider) = &auth.auth_provider {
        return AuthKind::AuthProvider {
            name: provider.name.clone(),
        };
    }
    if auth.client_certificate.is_some() || auth.client_certificate_data.is_some() {
        return AuthKind::ClientCertificate;
    }
    if auth.token.is_some() {
        return AuthKind::Token;
    }
    if auth.token_file.is_some() {
        return AuthKind::TokenFile;
    }
    if auth.username.is_some() {
        return AuthKind::Basic;
    }
    AuthKind::None
}

fn is_incompatible(merged: &kube::config::Kubeconfig, next: &kube::config::Kubeconfig) -> bool {
    let differs = |left: &Option<String>, right: &Option<String>| {
        left.is_some() && right.is_some() && left != right
    };
    differs(&merged.kind, &next.kind) || differs(&merged.api_version, &next.api_version)
}

fn load_error(path: &Path, error: kube::config::KubeconfigError) -> KubeconfigError {
    match error {
        kube::config::KubeconfigError::ReadConfig(source, _) => KubeconfigError::Read {
            path: path.to_path_buf(),
            source,
        },
        // The parser message is dropped: it can quote the failing line, and that line may hold a token.
        _ => KubeconfigError::Parse {
            path: path.to_path_buf(),
        },
    }
}

// `#[path]` keeps the tests a sibling file; without it they would live in src/kubeconfig/.
#[cfg(test)]
#[path = "kubeconfig_tests.rs"]
mod kubeconfig_tests;
