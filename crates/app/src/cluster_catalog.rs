//! The app-wide list of loaded kubeconfigs, shared by the main window and the Settings window.
//! The launch chain loads once; each registry file loads on its own and follows the registry.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use cluster::{Kubeconfig, KubeconfigError};
use gpui_kit::{App, AppContext as _, ClipboardItem, Context, Entity, Global, Subscription};
use zeroize::Zeroizing;

use crate::cluster_form::{ClusterGroup, cluster_groups, file_name_text, remove_kubeconfig};
use crate::cluster_session::error_text;
use crate::kubeconfig_import::{
    PASTED_DIR, is_app_owned, is_pasted_file_name, write_pasted_kubeconfig,
};
use crate::launch_options::standalone_files;
use crate::secret_clipboard::ClipboardMark;
use crate::settings::AppSettings;

/// The catalog entity, held as a global so both windows reach the same one.
pub(crate) struct CatalogHandle(pub(crate) Entity<ClusterCatalog>);

impl Global for CatalogHandle {}

impl CatalogHandle {
    /// Creates the catalog, which starts loading, and sets the global.
    pub(crate) fn install(chain: Vec<PathBuf>, cx: &mut App) {
        let catalog = cx.new(|cx| ClusterCatalog::new(chain, cx));
        cx.set_global(Self(catalog));
    }

    pub(crate) fn of(cx: &App) -> Entity<ClusterCatalog> {
        cx.global::<Self>().0.clone()
    }
}

/// One loaded unit: the launch chain (merged) or one registry file.
pub(crate) enum CatalogPart {
    Loading,
    Loaded(Arc<Kubeconfig>),
    /// The text of the load error; the file never reaches the list.
    Failed(String),
}

/// Something the user should read about the catalog; shown by the title-bar warning button.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CatalogNotice {
    /// A kubeconfig file that did not load. `path` is `None` when the error names no file; the
    /// notice goes away with the file's registry entry.
    Skipped {
        path: Option<PathBuf>,
        error: String,
    },
    /// A file in the pasted-files folder that the registry does not list (a crash between the
    /// write and the registry update); its Remove action deletes it.
    Unregistered(PathBuf),
    DeleteFailed(PathBuf),
    SaveFailed(io::ErrorKind),
}

impl std::fmt::Display for CatalogNotice {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Skipped { error, .. } => write!(formatter, "Skipped kubeconfig: {error}"),
            Self::Unregistered(path) => {
                write!(
                    formatter,
                    "Pasted file {} is not registered",
                    file_name(path)
                )
            }
            Self::DeleteFailed(path) => write!(
                formatter,
                "Could not delete {}; it is still listed",
                file_name(path)
            ),
            Self::SaveFailed(kind) => {
                write!(formatter, "Could not save the pasted kubeconfig ({kind})")
            }
        }
    }
}

/// Where a paste stands, for the Clusters page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PasteStatus {
    Idle,
    Saving,
    /// The pasted file was written, loaded, and registered.
    Added(PathBuf),
    Failed,
}

fn file_name(path: &Path) -> String {
    file_name_text(&path.to_string_lossy()).to_owned()
}

pub(crate) struct ClusterCatalog {
    /// The launch chain files, absolute; also what a registry file is compared against.
    chain_files: Vec<PathBuf>,
    /// `None` when no chain file could be located.
    chain: Option<CatalogPart>,
    /// Registry files in registry order.
    standalone: Vec<(PathBuf, CatalogPart)>,
    notices: Vec<CatalogNotice>,
    paste: PasteStatus,
    _settings_observer: Subscription,
}

/// What one load produced, for one part.
struct PartLoad {
    target: PartTarget,
    part: CatalogPart,
    /// One notice per file that was skipped.
    skipped: Vec<CatalogNotice>,
}

enum PartTarget {
    Chain,
    File(PathBuf),
}

impl ClusterCatalog {
    /// Loads the chain once and every standalone registry file, on the background executor.
    pub(crate) fn new(chain: Vec<PathBuf>, cx: &mut Context<Self>) -> Self {
        let registered = &AppSettings::get(cx).registry.kubeconfigs;
        let files = standalone_files(registered, &chain);
        let mut targets = Vec::new();
        if !chain.is_empty() {
            targets.push(LoadRequest::Chain(chain.clone()));
        }
        targets.extend(files.iter().cloned().map(LoadRequest::File));
        let catalog = Self {
            chain: (!chain.is_empty()).then_some(CatalogPart::Loading),
            chain_files: chain,
            standalone: files
                .into_iter()
                .map(|file| (file, CatalogPart::Loading))
                .collect(),
            notices: Vec::new(),
            paste: PasteStatus::Idle,
            _settings_observer: cx.observe_global::<AppSettings>(Self::follow_registry),
        };
        Self::load(targets, cx);
        Self::list_unregistered_files(cx);
        catalog
    }

    fn parts(&self) -> impl Iterator<Item = &CatalogPart> {
        self.chain
            .iter()
            .chain(self.standalone.iter().map(|(_, part)| part))
    }

    /// Every loaded kubeconfig: the chain first, then registry files in registry order.
    pub(crate) fn kubeconfigs(&self) -> impl Iterator<Item = &Arc<Kubeconfig>> {
        self.parts().filter_map(|part| match part {
            CatalogPart::Loaded(kubeconfig) => Some(kubeconfig),
            CatalogPart::Loading | CatalogPart::Failed(_) => None,
        })
    }

    /// The loaded clusters grouped by environment, as Settings and the switcher list them.
    pub(crate) fn groups(&self, cx: &App) -> Vec<ClusterGroup> {
        let kubeconfigs: Vec<&Kubeconfig> = self
            .kubeconfigs()
            .map(|kubeconfig| kubeconfig.as_ref())
            .collect();
        cluster_groups(
            &kubeconfigs,
            &AppSettings::get(cx).registry,
            |path| self.is_chain_source(path),
            AppSettings::config_dir(cx),
        )
    }

    pub(crate) fn is_loading(&self) -> bool {
        self.parts()
            .any(|part| matches!(part, CatalogPart::Loading))
    }

    /// Whether `path` is one of the launch chain files (`KUBECONFIG` or `--kubeconfig`).
    pub(crate) fn is_chain_source(&self, path: &Path) -> bool {
        let text = path.to_string_lossy();
        self.chain_files
            .iter()
            .any(|file| same_path_text(&file.to_string_lossy(), &text, PathStyle::HOST))
    }

    /// The launch chain files, absolute.
    pub(crate) fn chain_files(&self) -> &[PathBuf] {
        &self.chain_files
    }

    /// Why nothing is listed: the first load error, or that no file was located.
    pub(crate) fn failure_text(&self) -> String {
        for part in self.parts() {
            if let CatalogPart::Failed(message) = part {
                return message.clone();
            }
        }
        "no kubeconfig found: pass --kubeconfig, set KUBECONFIG, or create ~/.kube/config"
            .to_owned()
    }

    pub(crate) fn notices(&self) -> &[CatalogNotice] {
        &self.notices
    }

    pub(crate) fn clear_notices(&mut self, cx: &mut Context<Self>) {
        self.notices.clear();
        cx.notify();
    }

    /// Reloads only what the registry changed: added files load, removed files drop. The chain
    /// never reloads, and a session running from a removed file is not touched.
    fn follow_registry(&mut self, cx: &mut Context<Self>) {
        let registered = &AppSettings::get(cx).registry.kubeconfigs;
        let wanted = standalone_files(registered, &self.chain_files);
        let is_unchanged = wanted.len() == self.standalone.len()
            && wanted
                .iter()
                .zip(&self.standalone)
                .all(|(wanted, (have, _))| wanted == have);
        if is_unchanged {
            return;
        }
        let mut previous = std::mem::take(&mut self.standalone);
        let mut added = Vec::new();
        for file in wanted {
            match previous.iter().position(|(have, _)| *have == file) {
                Some(index) => self.standalone.push(previous.swap_remove(index)),
                None => {
                    added.push(LoadRequest::File(file.clone()));
                    self.standalone.push((file, CatalogPart::Loading));
                }
            }
        }
        // What is left of `previous` left the registry; a skip notice for it is stale.
        for (removed, _) in previous {
            self.notices.retain(|notice| {
                !matches!(notice, CatalogNotice::Skipped { path: Some(path), .. } if *path == removed)
            });
        }
        Self::load(added, cx);
        cx.notify();
    }

    /// A notice that is already shown is not shown twice.
    fn push_notice(&mut self, notice: CatalogNotice) {
        if !self.notices.contains(&notice) {
            self.notices.push(notice);
        }
    }

    /// The loaded launch chain, merged; `None` while it loads, failed, or there is no chain.
    pub(crate) fn chain(&self) -> Option<&Arc<Kubeconfig>> {
        match self.chain.as_ref()? {
            CatalogPart::Loaded(kubeconfig) => Some(kubeconfig),
            CatalogPart::Loading | CatalogPart::Failed(_) => None,
        }
    }

    /// The loaded registry files, in registry order; the chain is not among them.
    pub(crate) fn standalone_kubeconfigs(&self) -> impl Iterator<Item = &Arc<Kubeconfig>> {
        self.standalone.iter().filter_map(|(_, part)| match part {
            CatalogPart::Loaded(kubeconfig) => Some(kubeconfig),
            CatalogPart::Loading | CatalogPart::Failed(_) => None,
        })
    }

    pub(crate) fn paste_status(&self) -> &PasteStatus {
        &self.paste
    }

    pub(crate) fn reset_paste_status(&mut self, cx: &mut Context<Self>) {
        self.paste = PasteStatus::Idle;
        cx.notify();
    }

    /// Writes the pasted `text` to a new file under the config folder, loads it, and registers
    /// it. The catalog lives as long as the app, so closing the Settings window mid-save cannot
    /// leave a credential file without a registry entry. `clipboard` marks the text the user
    /// pasted: when the clipboard still holds it afterwards, it is cleared.
    pub(crate) fn add_pasted(
        &mut self,
        text: String,
        first_context: Option<String>,
        clipboard: ClipboardMark,
        cx: &mut Context<Self>,
    ) {
        let Some(dir) = AppSettings::config_dir(cx).map(Path::to_path_buf) else {
            self.push_notice(CatalogNotice::SaveFailed(io::ErrorKind::Unsupported));
            self.paste = PasteStatus::Failed;
            cx.notify();
            return;
        };
        self.paste = PasteStatus::Saving;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let saved = cx
                .background_executor()
                .spawn(async move { save_pasted(&dir, first_context.as_deref(), text) })
                .await;
            let _ = this.update(cx, |catalog, cx| {
                catalog.finish_paste(saved, &clipboard, cx)
            });
        })
        .detach();
    }

    fn finish_paste(
        &mut self,
        saved: Result<(PathBuf, Arc<Kubeconfig>), io::ErrorKind>,
        clipboard: &ClipboardMark,
        cx: &mut Context<Self>,
    ) {
        match saved {
            Ok((path, kubeconfig)) => {
                // Listed before the registry names it, so the registry observer finds nothing
                // to load.
                self.standalone
                    .push((path.clone(), CatalogPart::Loaded(kubeconfig)));
                let registered = path.clone();
                AppSettings::update(cx, |settings| {
                    settings.registry.kubeconfigs.push(registered)
                });
                clear_clipboard_if_marked(clipboard, cx);
                self.paste = PasteStatus::Added(path);
            }
            Err(kind) => {
                tracing::warn!(?kind, "cannot save the pasted kubeconfig");
                self.push_notice(CatalogNotice::SaveFailed(kind));
                self.paste = PasteStatus::Failed;
            }
        }
        cx.notify();
    }

    /// Drops `path` from the registry with its entries. A file k8sBoard wrote is deleted first,
    /// and the entries go only after the delete succeeded: a pasted credential must not stay
    /// on disk unlisted.
    pub(crate) fn remove_kubeconfig(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let is_owned = AppSettings::config_dir(cx).is_some_and(|dir| is_app_owned(&path, dir));
        if !is_owned {
            AppSettings::update(cx, |settings| {
                remove_kubeconfig(&mut settings.registry, &path)
            });
            return;
        }
        cx.spawn(async move |this, cx| {
            let deleted = cx
                .background_executor()
                .spawn({
                    let path = path.clone();
                    async move { delete_file(&path) }
                })
                .await;
            let _ = this.update(cx, |catalog, cx| catalog.finish_remove(path, deleted, cx));
        })
        .detach();
    }

    fn finish_remove(
        &mut self,
        path: PathBuf,
        deleted: Result<(), io::ErrorKind>,
        cx: &mut Context<Self>,
    ) {
        match deleted {
            Ok(()) => {
                self.notices.retain(|notice| {
                    !matches!(notice, CatalogNotice::Unregistered(other) | CatalogNotice::DeleteFailed(other) if *other == path)
                });
                AppSettings::update(cx, |settings| {
                    remove_kubeconfig(&mut settings.registry, &path)
                });
            }
            Err(kind) => {
                tracing::warn!(?kind, "cannot delete the pasted kubeconfig");
                self.push_notice(CatalogNotice::DeleteFailed(path));
            }
        }
        cx.notify();
    }

    /// At start, lists the pasted-files folder and raises a notice for each file the registry
    /// does not name (a crash between the write and the registry update).
    fn list_unregistered_files(cx: &mut Context<Self>) {
        let Some(dir) = AppSettings::config_dir(cx).map(|dir| dir.join(PASTED_DIR)) else {
            return;
        };
        cx.spawn(async move |this, cx| {
            let files = cx
                .background_executor()
                .spawn(async move { yaml_files(&dir) })
                .await;
            let _ = this.update(cx, |catalog, cx| catalog.raise_unregistered(files, cx));
        })
        .detach();
    }

    fn raise_unregistered(&mut self, files: Vec<PathBuf>, cx: &mut Context<Self>) {
        let registered = &AppSettings::get(cx).registry.kubeconfigs;
        let unlisted: Vec<PathBuf> = files
            .into_iter()
            .filter(|file| {
                let text = file.to_string_lossy();
                !registered.iter().any(|registered| {
                    same_path_text(&registered.to_string_lossy(), &text, PathStyle::HOST)
                })
            })
            .collect();
        if unlisted.is_empty() {
            return;
        }
        for file in unlisted {
            self.push_notice(CatalogNotice::Unregistered(file));
        }
        cx.notify();
    }

    fn load(requests: Vec<LoadRequest>, cx: &mut Context<Self>) {
        if requests.is_empty() {
            return;
        }
        cx.spawn(async move |this, cx| {
            let loads = cx
                .background_executor()
                .spawn(async move { requests.into_iter().map(load_part).collect::<Vec<_>>() })
                .await;
            let _ = this.update(cx, |catalog, cx| catalog.finish_load(loads, cx));
        })
        .detach();
    }

    fn finish_load(&mut self, loads: Vec<PartLoad>, cx: &mut Context<Self>) {
        for load in loads {
            match load.target {
                PartTarget::Chain => {
                    if self.chain.is_some() {
                        self.chain = Some(load.part);
                    }
                    for notice in load.skipped {
                        self.push_notice(notice);
                    }
                }
                PartTarget::File(path) => {
                    // The file may have left the registry while it loaded: then its part and
                    // its notices are dropped.
                    if let Some((_, part)) =
                        self.standalone.iter_mut().find(|(have, _)| *have == path)
                    {
                        *part = load.part;
                        for notice in load.skipped {
                            self.push_notice(notice);
                        }
                    }
                }
            }
        }
        cx.notify();
    }
}

/// How a path string is split and compared: Windows paths use both separators and ignore case.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PathStyle {
    Windows,
    Unix,
}

impl PathStyle {
    pub(crate) const HOST: Self = if cfg!(windows) {
        Self::Windows
    } else {
        Self::Unix
    };
}

/// Whether two path strings name the same place. Windows: split on `/` and the backslash and
/// compare the parts case-insensitively. Unix: split on `/` only and compare exactly. Both skip
/// empty and `.` parts. String-level on purpose: `Path` follows the host's rules, so a Windows
/// path would split wrongly on another host.
pub(crate) fn same_path_text(a: &str, b: &str, style: PathStyle) -> bool {
    let parts = |text: &str| -> Vec<String> {
        let is_separator = |c: char| c == '/' || (style == PathStyle::Windows && c == '\\');
        text.split(is_separator)
            .filter(|part| !part.is_empty() && *part != ".")
            .map(|part| match style {
                PathStyle::Windows => part.to_lowercase(),
                PathStyle::Unix => part.to_owned(),
            })
            .collect()
    };
    parts(a) == parts(b)
}

enum LoadRequest {
    Chain(Vec<PathBuf>),
    File(PathBuf),
}

/// Writes the pasted text, drops it, then loads the new file; a file that does not load is
/// deleted again. Blocking file I/O: call it off the UI thread.
fn save_pasted(
    config_dir: &Path,
    first_context: Option<&str>,
    text: String,
) -> Result<(PathBuf, Arc<Kubeconfig>), io::ErrorKind> {
    let path =
        write_pasted_kubeconfig(config_dir, first_context, &text).map_err(|error| error.kind())?;
    drop(text);
    match Kubeconfig::load(std::slice::from_ref(&path)) {
        Ok(loaded) => Ok((path, Arc::new(loaded.kubeconfig))),
        Err(_) => {
            // The parse already passed on the same text, so this is a read failure; the file
            // must not stay behind unlisted.
            let _ = std::fs::remove_file(&path);
            Err(io::ErrorKind::InvalidData)
        }
    }
}

/// Clears the clipboard when it still holds the text `mark` was made from, and nothing else.
fn clear_clipboard_if_marked(mark: &ClipboardMark, cx: &mut App) {
    // The clipboard text is the credential just pasted: wiped when dropped.
    let holds_text = cx
        .read_from_clipboard()
        .and_then(|item| item.text())
        .map(Zeroizing::new)
        .is_some_and(|text| mark.matches(&text));
    if holds_text {
        cx.write_to_clipboard(ClipboardItem::new_string(String::new()));
    }
}

/// A missing file counts as deleted.
fn delete_file(path: &Path) -> Result<(), io::ErrorKind> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.kind()),
    }
}

/// The pasted-looking `*.yaml` files of `dir`; a missing or unreadable folder lists nothing.
fn yaml_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    // Only names k8sBoard gives its own files: a file the user put here is not ours to offer for
    // deletion.
    let is_pasted = |path: &PathBuf| {
        path.file_name()
            .and_then(|name| name.to_str())
            .is_some_and(is_pasted_file_name)
    };
    entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(is_pasted)
        .collect()
}

fn skipped_notice(error: &KubeconfigError) -> CatalogNotice {
    let path = match error {
        KubeconfigError::Read { path, .. }
        | KubeconfigError::Parse { path }
        | KubeconfigError::Incompatible { path } => Some(path.clone()),
        _ => None,
    };
    CatalogNotice::Skipped {
        path,
        error: error.to_string(),
    }
}

/// Blocking file I/O: call it off the UI thread.
fn load_part(request: LoadRequest) -> PartLoad {
    let (target, files) = match request {
        LoadRequest::Chain(chain) => (PartTarget::Chain, chain),
        LoadRequest::File(file) => (PartTarget::File(file.clone()), vec![file]),
    };
    match Kubeconfig::load(&files) {
        Ok(loaded) => PartLoad {
            target,
            part: CatalogPart::Loaded(Arc::new(loaded.kubeconfig)),
            skipped: loaded.skipped.iter().map(skipped_notice).collect(),
        },
        Err(error) => PartLoad {
            target,
            skipped: vec![skipped_notice(&error)],
            part: CatalogPart::Failed(error_text(&error)),
        },
    }
}

#[cfg(test)]
#[path = "cluster_catalog_tests.rs"]
mod cluster_catalog_tests;
