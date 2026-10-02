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
