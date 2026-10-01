use std::fmt;
use std::path::{Path, PathBuf};

/// A kubeconfig file loaded from an explicit path. Holds credentials.
// Debug is manual: it prints the path and context names only.
#[derive(Clone)]
pub struct Kubeconfig {
    path: PathBuf,
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
    #[error("{origin} '{requested}' not found in kubeconfig '{}'; available contexts: {}",
            .path.display(), context_list(.available))]
    ContextNotFound {
        path: PathBuf,
        requested: String,
        origin: ContextOrigin,
        available: Vec<String>,
    },
    #[error("kubeconfig '{}' has no current-context and no context was requested; available contexts: {}",
            .path.display(), context_list(.available))]
    NoContextSelected {
        path: PathBuf,
        available: Vec<String>,
    },
}

fn context_list(names: &[String]) -> String {
    if names.is_empty() {
        return "(none)".to_owned();
    }
    names.join(", ")
}

impl Kubeconfig {
    /// Blocking file I/O: call it off the UI thread.
    pub fn load(path: &Path) -> Result<Self, KubeconfigError> {
        // `read_from` (not `from_yaml`) makes relative credential paths absolute.
        let document =
            kube::config::Kubeconfig::read_from(path).map_err(|error| load_error(path, error))?;
        Ok(Self::from_document(path, document))
    }

    fn from_document(path: &Path, document: kube::config::Kubeconfig) -> Self {
        let mut contexts: Vec<ContextSummary> = Vec::new();
        for named in &document.contexts {
            let Some(context) = &named.context else {
                continue;
            };
            // kube's lookup takes the first entry with a given name.
            if contexts.iter().any(|existing| existing.name == named.name) {
                continue;
            }
            contexts.push(ContextSummary {
                name: named.name.clone(),
                cluster: context.cluster.clone(),
                user: context.user.clone(),
                namespace: context.namespace.clone(),
            });
        }
        Self {
            path: path.to_path_buf(),
            document,
            contexts,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
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
                        path: self.path.clone(),
                        available: self.context_names(),
                    });
                }
            },
        };
        self.contexts
            .iter()
            .find(|context| context.name == name)
            .ok_or_else(|| KubeconfigError::ContextNotFound {
                path: self.path.clone(),
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
            .field("path", &self.path)
            .field("contexts", &self.context_names())
            .finish()
    }
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
