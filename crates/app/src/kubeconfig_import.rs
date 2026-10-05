//! Adding a kubeconfig from the Settings window: the preview with its name-collision warnings,
//! the checks on a picked file or the clipboard text, and the pasted file's name and storage.
//! Pure logic and blocking file I/O; the dialogs live in `clusters_page.rs`.
//!
//! No log call here: file paths and error kinds at most, never text, user names, or servers.

use std::fmt;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};

use cluster::{AuthKind, Kubeconfig, KubeconfigError};

use crate::cluster_catalog::{PathStyle, same_path_text};
use crate::cluster_form::{ClusterRow, file_name_text};
use crate::environment::{EnvironmentTier, guess_environment};

/// The folder under the config dir that holds pasted kubeconfig files.
pub(crate) const PASTED_DIR: &str = "kubeconfigs";
/// A kubeconfig is a few KiB; anything bigger is not one, and it must not be parsed.
pub(crate) const MAX_CLIPBOARD_BYTES: usize = 1024 * 1024;
const MAX_SLUG_CHARS: usize = 40;
/// Shown where pasting is off, and by `ImportError::NoConfigDir`.
pub(crate) const NO_CONFIG_DIR_MESSAGE: &str =
    "Settings are not saved this session, so a pasted kubeconfig cannot be stored.";

#[derive(Debug)]
pub(crate) struct ImportPreview {
    pub(crate) source: ImportSource,
    pub(crate) contexts: Vec<ContextPreview>,
    pub(crate) collisions: Vec<NameCollision>,
}

#[derive(Debug)]
pub(crate) enum ImportSource {
    /// Only the path is stored; the file is never copied.
    File(PathBuf),
    /// The clipboard text is written to `target`.
    Pasted { target: PathBuf },
}

/// Names, the server without userinfo, and the auth kind: nothing that is a credential.
#[derive(Debug)]
pub(crate) struct ContextPreview {
    pub(crate) name: String,
    pub(crate) server: Option<String>,
    pub(crate) auth: AuthKind,
    /// Guessed from the names, as the list will show it.
    pub(crate) environment: EnvironmentTier,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct NameCollision {
    pub(crate) kind: EntryKind,
    pub(crate) name: String,
    pub(crate) place: CollisionPlace,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EntryKind {
    Context,
    DisplayName,
    Cluster,
    User,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CollisionPlace {
    File(PathBuf),
    KubeconfigChain,
}

impl fmt::Display for NameCollision {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = &self.name;
        let place = match &self.place {
            CollisionPlace::File(path) => file_name_text(&path.to_string_lossy()).to_owned(),
            CollisionPlace::KubeconfigChain => "your KUBECONFIG chain".to_owned(),
        };
        match self.kind {
            EntryKind::Context => write!(
                formatter,
                "Context '{name}' also exists in {place}. Both are listed; the switcher adds the file name."
            ),
            EntryKind::DisplayName => write!(
                formatter,
                "Context '{name}' matches the name shown for a cluster from {place}. Both are listed; rename one in Settings."
            ),
            EntryKind::Cluster | EntryKind::User => {
                let kind = if self.kind == EntryKind::Cluster {
                    "Cluster"
                } else {
                    "User"
                };
                write!(
                    formatter,
                    "{kind} '{name}' also exists in {place}. k8sBoard reads this file on its own, so nothing is replaced; kubectl would use only the first one if both were in KUBECONFIG."
                )
            }
        }
    }
}

/// Why a kubeconfig cannot be added; the text is shown in the dialog. None of them quotes
/// content.
#[derive(Debug)]
pub(crate) enum ImportError {
    AlreadyAdded,
    InChain,
    NoContexts,
    NoClipboardText,
    /// Writes are off, so there is no folder for the pasted file.
    NoConfigDir,
    ClipboardTooLarge,
    InvalidClipboard,
    /// A picked file that cannot be read or parsed; the cluster crate's text holds no content.
    Load(KubeconfigError),
}

impl fmt::Display for ImportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyAdded => formatter.write_str("This file is already added."),
            Self::InChain => {
                formatter.write_str("This file is already loaded from KUBECONFIG or --kubeconfig.")
            }
            Self::NoContexts => formatter.write_str("This kubeconfig has no contexts."),
            Self::NoClipboardText => formatter.write_str("The clipboard has no text."),
            Self::NoConfigDir => formatter.write_str(NO_CONFIG_DIR_MESSAGE),
            Self::ClipboardTooLarge => formatter
                .write_str("The clipboard text is larger than 1 MiB; that is not a kubeconfig."),
            Self::InvalidClipboard => {
                formatter.write_str("The clipboard text is not a valid kubeconfig.")
            }
            Self::Load(error) => write!(formatter, "{error}"),
        }
    }
}

/// The text to parse, or why the clipboard cannot be a kubeconfig.
pub(crate) fn check_clipboard_text(text: Option<&str>) -> Result<&str, ImportError> {
    let text = text.ok_or(ImportError::NoClipboardText)?;
    if text.len() > MAX_CLIPBOARD_BYTES {
        return Err(ImportError::ClipboardTooLarge);
    }
    Ok(text)
}

/// `path` may be added: not already registered and not a launch chain file.
pub(crate) fn check_new_file(
    path: &Path,
    registered: &[PathBuf],
    is_chain_source: bool,
) -> Result<(), ImportError> {
    let text = path.to_string_lossy();
    let is_registered = registered
        .iter()
        .any(|file| same_path_text(&file.to_string_lossy(), &text, PathStyle::HOST));
    if is_registered {
        return Err(ImportError::AlreadyAdded);
    }
    if is_chain_source {
        return Err(ImportError::InChain);
    }
    Ok(())
}

/// The preview of `candidate`; an error when it lists no context.
pub(crate) fn import_preview(
    source: ImportSource,
    candidate: &Kubeconfig,
    rows: &[ClusterRow],
    chain: Option<&Kubeconfig>,
    standalone: &[&Kubeconfig],
) -> Result<ImportPreview, ImportError> {
    if candidate.contexts().is_empty() {
        return Err(ImportError::NoContexts);
    }
    let contexts = candidate
        .contexts()
        .iter()
        .map(|context| {
            let info = candidate.connection_info(context);
            ContextPreview {
                name: context.name.clone(),
                server: info.server,
                auth: info.auth,
                environment: guess_environment(&context.name, &context.cluster),
            }
        })
        .collect();
    Ok(ImportPreview {
        source,
        contexts,
        collisions: name_collisions(candidate, rows, chain, standalone),
    })
}

/// The names `candidate` shares with what is already listed. They are warnings, never blockers.
pub(crate) fn name_collisions(
    candidate: &Kubeconfig,
    rows: &[ClusterRow],
    chain: Option<&Kubeconfig>,
    standalone: &[&Kubeconfig],
) -> Vec<NameCollision> {
    let mut collisions = Vec::new();
    for context in candidate.contexts() {
        let lowered = context.name.to_lowercase();
        for row in rows {
            let place = || CollisionPlace::File(row.cluster.kubeconfig.clone());
            if row.cluster.context == context.name {
                collisions.push(NameCollision {
                    kind: EntryKind::Context,
                    name: context.name.clone(),
                    place: place(),
                });
            } else if row.profile.display_name.to_lowercase() == lowered {
                // A different context name with this display name can only be a stored one.
                collisions.push(NameCollision {
                    kind: EntryKind::DisplayName,
                    name: context.name.clone(),
                    place: place(),
                });
            }
        }
    }
    let names = candidate.entry_names();
    let others = chain
        .map(|chain| (CollisionPlace::KubeconfigChain, chain))
        .into_iter()
        .chain(standalone.iter().map(|kubeconfig| {
            let source = kubeconfig.sources().first().cloned().unwrap_or_default();
            (CollisionPlace::File(source), *kubeconfig)
        }));
    for (place, other) in others {
        let other_names = other.entry_names();
        let shared = |kind, ours: &[String], theirs: &[String]| {
            ours.iter()
                .filter(|name| theirs.contains(name))
                .map(|name| (kind, name.clone()))
                .collect::<Vec<_>>()
        };
        let clusters = shared(EntryKind::Cluster, &names.clusters, &other_names.clusters);
        let users = shared(EntryKind::User, &names.users, &other_names.users);
        for (kind, name) in clusters.into_iter().chain(users) {
            collisions.push(NameCollision {
                kind,
                name,
                place: place.clone(),
            });
        }
    }
    collisions
}

/// Whether `path` is a file k8sBoard wrote: its parent is `<config_dir>/kubeconfigs` and its
/// name is one `pasted_file_path` could give. Remove deletes such files, so a file the user put
/// in the folder under another name is never deleted. One that happens to look like ours still
/// would be: a ceiling of keeping no separate list of what was written.
pub(crate) fn is_app_owned(path: &Path, config_dir: &Path) -> bool {
    let path = path.to_string_lossy();
    let Some((parent, file)) = path.rsplit_once(['/', '\\']) else {
        return false;
    };
    if !is_pasted_file_name(file) {
        return false;
    }
    let owned = format!("{}/{PASTED_DIR}", config_dir.to_string_lossy());
    same_path_text(parent, &owned, PathStyle::HOST)
}

/// Whether `name` has the shape `pasted_file_path` gives: `{slug}.yaml` or `{slug}-{n}.yaml`,
/// only `[a-z0-9._-]`, at most 40 chars of slug and 10 more for the counter.
pub(crate) fn is_pasted_file_name(name: &str) -> bool {
    let Some(stem) = name.strip_suffix(".yaml") else {
        return false;
    };
    !stem.is_empty()
        && stem.len() <= MAX_SLUG_CHARS + 10
        && stem
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'))
}

/// `<dir>/kubeconfigs/{slug}.yaml`: the slug is the first context name lowercased, every char
/// outside `[a-z0-9._-]` replaced by `-`, cut to 40 chars; `pasted` when there is none. `-2`,
/// `-3`, ... is added while the name exists.
pub(crate) fn pasted_file_path(config_dir: &Path, first_context: Option<&str>) -> PathBuf {
    let slug: String = first_context
        .unwrap_or_default()
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '-'
            }
        })
        .take(MAX_SLUG_CHARS)
        .collect();
    let slug = if slug.is_empty() { "pasted" } else { &slug };
    let folder = config_dir.join(PASTED_DIR);
    let first = folder.join(format!("{slug}.yaml"));
    if !first.exists() {
        return first;
    }
    (2..)
        .map(|number| folder.join(format!("{slug}-{number}.yaml")))
        .find(|path| !path.exists())
        .unwrap_or(first)
}

/// Creates the folder and writes the text to a new file (never an existing one), then syncs it.
/// Unix: the file is `0o600` and a new folder `0o700`. Windows: the file inherits the profile
/// ACL of the config folder, like kubectl's own file (decision 9 of spec 0025).
/// Blocking file I/O: call it off the UI thread.
pub(crate) fn write_pasted_kubeconfig(
    config_dir: &Path,
    first_context: Option<&str>,
    text: &str,
) -> io::Result<PathBuf> {
    create_pasted_folder(&config_dir.join(PASTED_DIR))?;
    // Another process could take the name between the check and the create; the next name is
    // tried then.
    let mut attempts = 0;
    loop {
        let path = pasted_file_path(config_dir, first_context);
        match create_new_private(&path) {
            Ok(mut file) => {
                // A partial file would sit in the pasted folder without a registry entry.
                let written = file
                    .write_all(text.as_bytes())
                    .and_then(|()| file.sync_all());
                if let Err(error) = written {
                    drop(file);
                    let _ = std::fs::remove_file(&path);
                    return Err(error);
                }
                return Ok(path);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists && attempts < 8 => {
                attempts += 1;
            }
            Err(error) => return Err(error),
        }
    }
}

fn create_pasted_folder(folder: &Path) -> io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(folder)
}

fn create_new_private(path: &Path) -> io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(path)
}

#[cfg(test)]
#[path = "kubeconfig_import_tests.rs"]
mod kubeconfig_import_tests;
