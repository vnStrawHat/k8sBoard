//! The watched folders of the catalog (spec 0043): the scan and watch of each folder, the debounce
//! of its change events, and how its files join the list. A watched folder is a place anyone can
//! drop a file in, so its files are listed and loaded on request but never start a session or get
//! probed on their own (`start_kubeconfigs`, `ProbeCandidate.origin`), every read is bounded
//! (`kubeconfig_folder.rs`), and nothing here writes in the folder.

use std::io;
use std::path::{Path, PathBuf};

use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender};
use futures::{FutureExt as _, StreamExt as _, select_biased};
use gpui_kit::{Context, Task};

use super::{CatalogNotice, ClusterCatalog, LoadRequest, PartSource, PathStyle, same_path_text};
use crate::kubeconfig_folder::{
    FileStamp, FolderFile, FolderScan, FolderStatus, FolderSummary, RESCAN_DEBOUNCE,
    RESCAN_MAX_WAIT, diff_scan, scan_folder,
};
use crate::launch_options::standalone_files;
use crate::settings::AppSettings;

/// A watcher saw something change in `folder`. The rescan reads the folder itself, so the kind of
/// change does not matter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FolderEvent {
    pub(crate) folder: PathBuf,
}

/// One folder of `registry.kubeconfig_folders`, with what the last scan saw of it.
pub(super) struct WatchedFolder {
    path: PathBuf,
    status: FolderStatus,
    /// The candidate files of the last scan, in name order.
    files: Vec<FolderFile>,
    skipped_over_cap: usize,
    /// Dropping it stops the watch.
    _watcher: Option<notify::RecommendedWatcher>,
}

impl WatchedFolder {
    /// The watcher comes later, with the first listing: starting it can block on a folder that
    /// is slow to answer (an offline network share), and that must not stall the main thread.
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            status: FolderStatus::Scanning,
            files: Vec::new(),
            skipped_over_cap: 0,
            _watcher: None,
        }
    }
}

/// What a listing does about the watch of its folder.
enum WatchStart {
    /// The first listing of a folder: start the watcher too, on the same background thread.
    Start(UnboundedSender<FolderEvent>),
    /// A rescan: the folder is already watched.
    Keep,
}

/// Watches `folder` (not its subfolders). Every event, errors included, only sends the folder on
/// the channel; the catalog task rescans it after the debounce. `None` when the OS refuses the
/// watch (a missing folder): the folder is looked at again at the next start. Blocking: call it
/// off the UI thread.
#[cfg(not(test))]
fn start_watcher(
    folder: &Path,
    events: UnboundedSender<FolderEvent>,
) -> Option<notify::RecommendedWatcher> {
    use notify::{RecursiveMode, Watcher as _};
    let watched = folder.to_path_buf();
    let mut watcher = notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
        // Reading a file reports an access event on some systems, and the rescan reads files: it
        // would trigger itself without this.
        if result.as_ref().is_ok_and(|event| event.kind.is_access()) {
            return;
        }
        // The receiver only closes when the app is shutting down.
        let _ = events.unbounded_send(FolderEvent {
            folder: watched.clone(),
        });
    })
    .map_err(|error| tracing::warn!(%error, "cannot start a folder watcher"))
    .ok()?;
    watcher
        .watch(folder, RecursiveMode::NonRecursive)
        .map_err(|error| tracing::warn!(%error, "cannot watch a folder"))
        .ok()?;
    Some(watcher)
}

/// Tests never start a real watcher: they send `FolderEvent`s on the channel instead. The
/// number of starts on this thread is counted, so a test can see when a start happens.
#[cfg(test)]
fn start_watcher(_: &Path, _: UnboundedSender<FolderEvent>) -> Option<notify::RecommendedWatcher> {
    WATCHER_STARTS.with(|starts| starts.set(starts.get() + 1));
    None
}

#[cfg(test)]
thread_local! {
    /// The test executor runs every task on the test's own thread, so this counts one test only.
    pub(super) static WATCHER_STARTS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The task that turns events into rescans: one rescan `RESCAN_DEBOUNCE` after the last event of a
/// burst, and at the latest `RESCAN_MAX_WAIT` after its first, so a stream that never pauses still
/// gets one. The timers are the executor's, so tests drive them with the fake clock.
pub(super) fn watch_events(
    mut events: UnboundedReceiver<FolderEvent>,
    cx: &mut Context<ClusterCatalog>,
) -> Task<()> {
    cx.spawn(async move |this, cx| {
        loop {
            let Some(first) = events.next().await else {
                return;
            };
            let mut due = vec![first.folder];
            let mut quiet = cx.background_executor().timer(RESCAN_DEBOUNCE).fuse();
            let mut limit = cx.background_executor().timer(RESCAN_MAX_WAIT).fuse();
            loop {
                select_biased! {
                    event = events.next() => {
                        let Some(event) = event else {
                            break;
                        };
                        if !due.contains(&event.folder) {
                            due.push(event.folder);
                        }
                        quiet = cx.background_executor().timer(RESCAN_DEBOUNCE).fuse();
                    }
                    () = quiet => break,
                    () = limit => break,
                }
            }
            if this
                .update(cx, |catalog, cx| catalog.rescan(due, cx))
                .is_err()
            {
                return;
            }
        }
    })
}

/// The registry's folders, absolute and without duplicates, in registry order.
fn watched_paths(registered: &[PathBuf]) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = Vec::new();
    for folder in registered {
        let folder = std::path::absolute(folder).unwrap_or_else(|_| folder.clone());
        let text = folder.to_string_lossy();
        let is_known = paths
            .iter()
            .any(|known| same_path_text(&known.to_string_lossy(), &text, PathStyle::HOST));
        if !is_known {
            paths.push(folder);
        }
    }
    paths
}

impl ClusterCatalog {
    /// Starts watching and scanning the folders the registry names at start.
    pub(super) fn start_watching_folders(mut self, cx: &mut Context<Self>) -> Self {
        self.sync_folders(cx);
        self
    }

    /// Starts the watch of a folder the registry gained and stops the watch of one it lost.
    /// A lost folder's files leave the list in `reconcile_standalone`.
    pub(super) fn sync_folders(&mut self, cx: &mut Context<Self>) {
        let wanted = watched_paths(&AppSettings::get(cx).registry.kubeconfig_folders);
        let is_unchanged = wanted.len() == self.folders.len()
            && wanted
                .iter()
                .zip(&self.folders)
                .all(|(wanted, have)| *wanted == have.path);
        if is_unchanged {
            return;
        }
        let mut previous = std::mem::take(&mut self.folders);
        let mut added = Vec::new();
        for path in wanted {
            match previous.iter().position(|have| have.path == path) {
                Some(index) => self.folders.push(previous.swap_remove(index)),
                None => {
                    self.folders.push(WatchedFolder::new(path.clone()));
                    added.push(path);
                }
            }
        }
        for removed in previous {
            self.notices.retain(|notice| {
                !matches!(notice, CatalogNotice::FolderMissing { path, .. } if *path == removed.path)
            });
        }
        for folder in added {
            Self::scan(folder, WatchStart::Start(self.folder_events.clone()), cx);
        }
        cx.notify();
    }

    /// Lists `folder` on the background executor, starting its watcher there first when asked,
    /// and applies the result when it arrives.
    fn scan(folder: PathBuf, watch: WatchStart, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let (watcher, listed) = cx
                .background_executor()
                .spawn({
                    let folder = folder.clone();
                    async move {
                        // Watching first: a change between the two steps then costs one more
                        // rescan, never a missed one.
                        let watcher = match watch {
                            WatchStart::Start(events) => start_watcher(&folder, events),
                            WatchStart::Keep => None,
                        };
                        (watcher, scan_folder(&folder))
                    }
                })
                .await;
            let _ = this.update(cx, |catalog, cx| {
                catalog.finish_scan(&folder, listed, watcher, cx);
            });
        })
        .detach();
    }

    /// A debounced burst of events: the folders it named are listed again. A folder that was
    /// stopped meanwhile is ignored.
    fn rescan(&mut self, folders: Vec<PathBuf>, cx: &mut Context<Self>) {
        #[cfg(test)]
        {
            self.folder_rescans += 1;
        }
        let watched: Vec<PathBuf> = folders
            .into_iter()
            .filter(|folder| self.folders.iter().any(|have| have.path == *folder))
            .collect();
        // A count only: a folder path is the user's, and nothing here traces file names.
        tracing::debug!(folders = watched.len(), "rescanning watched folders");
        for folder in watched {
            Self::scan(folder, WatchStart::Keep, cx);
        }
    }

    /// Applies one listing: new files load through `reconcile_standalone`, a changed file loads
    /// again (keeping its last good version when it no longer loads), a vanished file leaves the
    /// list, and a folder that cannot be listed drops its files and raises a notice.
    fn finish_scan(
        &mut self,
        folder: &Path,
        listed: io::Result<FolderScan>,
        watcher: Option<notify::RecommendedWatcher>,
        cx: &mut Context<Self>,
    ) {
        let Some(watched) = self.folders.iter_mut().find(|have| have.path == folder) else {
            return;
        };
        if watcher.is_some() {
            watched._watcher = watcher;
        }
        let mut reloads = Vec::new();
        match listed {
            Err(error) => {
                watched.files.clear();
                watched.skipped_over_cap = 0;
                watched.status = FolderStatus::Missing;
                self.push_notice(CatalogNotice::FolderMissing {
                    path: folder.to_path_buf(),
                    kind: error.kind(),
                });
            }
            Ok(scan) => {
                let change = diff_scan(&watched.files, &scan.files);
                watched.files = scan.files;
                watched.skipped_over_cap = scan.skipped_over_cap;
                watched.status = FolderStatus::Watching;
                reloads = change
                    .changed
                    .into_iter()
                    .map(|path| LoadRequest::FolderFile {
                        path,
                        is_reload: true,
                    })
                    .collect();
                self.notices.retain(|notice| {
                    !matches!(notice, CatalogNotice::FolderMissing { path, .. } if path == folder)
                });
            }
        }
        self.reconcile_standalone(cx);
        Self::load(reloads, cx);
        cx.notify();
    }

    /// The files to load on their own, in list order: the registry files, then the files of the
    /// watched folders, without the chain's files and without duplicates.
    pub(super) fn wanted_standalone(&self, cx: &Context<Self>) -> Vec<(PathBuf, PartSource)> {
        let registered = &AppSettings::get(cx).registry.kubeconfigs;
        let in_folders: Vec<PathBuf> = self
            .folders
            .iter()
            .flat_map(|folder| folder.files.iter().map(|file| file.path.clone()))
            .collect();
        let registry_files = standalone_files(registered, &[], &self.chain_files);
        standalone_files(registered, &in_folders, &self.chain_files)
            .into_iter()
            .map(|path| {
                let source = if registry_files.contains(&path) {
                    PartSource::Registry
                } else {
                    PartSource::Folder
                };
                (path, source)
            })
            .collect()
    }

    pub(super) fn has_scanning_folder(&self) -> bool {
        self.folders
            .iter()
            .any(|folder| folder.status == FolderStatus::Scanning)
    }

    /// Whether `path` is listed because a watched folder holds it (not the chain, not the
    /// registry): the rows of such a file cannot be removed and are never probed on their own.
    pub(crate) fn is_folder_source(&self, path: &Path) -> bool {
        let text = path.to_string_lossy();
        self.standalone.iter().any(|standalone| {
            standalone.source == PartSource::Folder
                && same_path_text(&standalone.path.to_string_lossy(), &text, PathStyle::HOST)
        })
    }

    /// Whether the file at `path` is a listed standalone file that is still loading. A start that
    /// waits for the file `last_used` names reads this: until the load ends, the cluster is not
    /// there to find, and the start would pick another one.
    pub(crate) fn is_file_loading(&self, path: &Path) -> bool {
        let text = path.to_string_lossy();
        self.standalone.iter().any(|standalone| {
            matches!(standalone.part, super::CatalogPart::Loading)
                && same_path_text(&standalone.path.to_string_lossy(), &text, PathStyle::HOST)
        })
    }

    /// What the last scan saw of the watched-folder file at `path`; `None` for any other file.
    pub(crate) fn folder_stamp_of(&self, path: &Path) -> Option<FileStamp> {
        let text = path.to_string_lossy();
        self.folders
            .iter()
            .flat_map(|folder| &folder.files)
            .find(|file| same_path_text(&file.path.to_string_lossy(), &text, PathStyle::HOST))
            .map(FileStamp::of)
    }

    /// The watched folder that lists `path` as its own file: `None` for a chain or registry file,
    /// even one that lies in a watched folder.
    pub(crate) fn folder_source_of(&self, path: &Path) -> Option<&Path> {
        if self.is_folder_source(path) {
            self.watched_folder_of(path)
        } else {
            None
        }
    }

    /// The watched folder that holds `path`.
    pub(crate) fn watched_folder_of(&self, path: &Path) -> Option<&Path> {
        let text = path.to_string_lossy();
        self.folders
            .iter()
            .find(|folder| {
                folder.files.iter().any(|file| {
                    same_path_text(&file.path.to_string_lossy(), &text, PathStyle::HOST)
                })
            })
            .map(|folder| folder.path.as_path())
    }

    /// One line per watched folder, for the Clusters page.
    pub(crate) fn folder_summaries(&self) -> Vec<FolderSummary> {
        self.folders
            .iter()
            .map(|folder| {
                let parts = folder.files.iter().filter_map(|file| {
                    self.standalone
                        .iter()
                        .find(|standalone| standalone.path == file.path)
                });
                let (mut loaded, mut others) = (0, 0);
                for standalone in parts {
                    match standalone.part {
                        super::CatalogPart::Loaded(_) => loaded += 1,
                        super::CatalogPart::Failed(_) | super::CatalogPart::Missing(_) => {
                            others += 1
                        }
                        super::CatalogPart::Loading => {}
                    }
                }
                FolderSummary {
                    path: folder.path.clone(),
                    status: folder.status,
                    kubeconfigs: loaded,
                    not_kubeconfigs: others,
                    not_loaded: folder.skipped_over_cap,
                }
            })
            .collect()
    }

    /// The sender the watchers use, for tests that inject events instead of touching the OS.
    #[cfg(test)]
    pub(crate) fn folder_events_for_test(&self) -> UnboundedSender<FolderEvent> {
        self.folder_events.clone()
    }

    /// How many debounced rescans ran.
    #[cfg(test)]
    pub(crate) fn folder_rescans(&self) -> usize {
        self.folder_rescans
    }
}
