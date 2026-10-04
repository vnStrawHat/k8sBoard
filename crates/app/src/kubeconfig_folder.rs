//! One watched kubeconfig folder, without any view code (spec 0043): which files are candidates,
//! what changed between two scans, and the bounded read of one file. A watched folder is a place
//! anyone can drop a file in, so every read here has a cap, and nothing here writes.

use std::cmp::Ordering;
use std::ffi::OsStr;
use std::fs::File;
use std::io::{self, Read as _};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use cluster::{Kubeconfig, KubeconfigError};
use zeroize::Zeroizing;

/// The files of one folder that are listed; the rest are counted and not loaded.
pub(crate) const MAX_FOLDER_FILES: usize = 50;
/// The most one file may hold: the paste limit (0025 decision 10).
pub(crate) const MAX_FILE_BYTES: u64 = 1024 * 1024;
/// A rescan waits this long after the last event of a burst, so an editor's several writes cost
/// one.
pub(crate) const RESCAN_DEBOUNCE: Duration = Duration::from_millis(500);
/// A stream of events that never pauses (a log file) still gets a rescan this often.
pub(crate) const RESCAN_MAX_WAIT: Duration = Duration::from_secs(2);

/// The extensions of a kubeconfig file; a file with no extension (`config`) counts too.
const EXTENSIONS: [&str; 5] = ["yaml", "yml", "conf", "config", "kubeconfig"];

/// What a scan saw of one candidate file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FolderFile {
    pub(crate) path: PathBuf,
    pub(crate) len: u64,
    pub(crate) modified: Option<SystemTime>,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct FolderScan {
    /// At most `MAX_FOLDER_FILES`, in name order.
    pub(crate) files: Vec<FolderFile>,
    pub(crate) skipped_over_cap: usize,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct FolderChange {
    pub(crate) added: Vec<PathBuf>,
    pub(crate) changed: Vec<PathBuf>,
    pub(crate) removed: Vec<PathBuf>,
}

/// Where a watched folder stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FolderStatus {
    /// The first listing is still running.
    Scanning,
    Watching,
    /// The folder cannot be listed; it is looked at again at the next start.
    Missing,
}

/// What the Clusters page says about one watched folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FolderSummary {
    pub(crate) path: PathBuf,
    pub(crate) status: FolderStatus,
    /// The files that loaded and are listed.
    pub(crate) kubeconfigs: usize,
    /// Candidate files that did not parse as a kubeconfig.
    pub(crate) not_kubeconfigs: usize,
    /// Candidates over the cap of `MAX_FOLDER_FILES`.
    pub(crate) not_loaded: usize,
}

impl FolderSummary {
    /// `Watching {path} · {k} kubeconfig files{, m not kubeconfigs}{, n more files not loaded}`.
    pub(crate) fn line_text(&self) -> String {
        let path = self.path.display();
        match self.status {
            FolderStatus::Scanning => return format!("Watching {path} · scanning…"),
            FolderStatus::Missing => return format!("Watching {path} · folder not found"),
            FolderStatus::Watching => {}
        }
        let files = if self.kubeconfigs == 1 {
            "file"
        } else {
            "files"
        };
        let mut text = format!("Watching {path} · {} kubeconfig {files}", self.kubeconfigs);
        match self.not_kubeconfigs {
            0 => {}
            1 => text.push_str(", 1 not a kubeconfig"),
            count => text.push_str(&format!(", {count} not kubeconfigs")),
        }
        if self.not_loaded > 0 {
            text.push_str(&format!(", {} more files not loaded", self.not_loaded));
        }
        text
    }
}

/// Lists the candidate files of `folder` (not recursive): regular files (a symlink is not one),
/// no dot name, a kubeconfig extension or none, at most `MAX_FILE_BYTES`. Blocking file I/O: call
/// it off the UI thread.
pub(crate) fn scan_folder(folder: &Path) -> io::Result<FolderScan> {
    let mut files: Vec<FolderFile> = Vec::new();
    for entry in std::fs::read_dir(folder)? {
        let Ok(entry) = entry else {
            continue;
        };
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        let path = entry.path();
        if metadata.is_file() && is_candidate_name(&path) && metadata.len() <= MAX_FILE_BYTES {
            files.push(FolderFile {
                path,
                len: metadata.len(),
                modified: metadata.modified().ok(),
            });
        }
    }
    files.sort_by(|a, b| by_name(&a.path, &b.path));
    let skipped_over_cap = files.len().saturating_sub(MAX_FOLDER_FILES);
    files.truncate(MAX_FOLDER_FILES);
    Ok(FolderScan {
        files,
        skipped_over_cap,
    })
}

fn by_name(a: &Path, b: &Path) -> Ordering {
    a.file_name().cmp(&b.file_name())
}

fn is_candidate_name(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(OsStr::to_str) else {
        return false;
    };
    if name.starts_with('.') {
        return false;
    }
    match path.extension().and_then(OsStr::to_str) {
        Some(extension) => EXTENSIONS
            .iter()
            .any(|known| known.eq_ignore_ascii_case(extension)),
        None => true,
    }
}

/// What differs between two scans of one folder: by path, then by size or modification time.
pub(crate) fn diff_scan(before: &[FolderFile], after: &[FolderFile]) -> FolderChange {
    let mut change = FolderChange::default();
    for file in after {
        match before.iter().find(|old| old.path == file.path) {
            None => change.added.push(file.path.clone()),
            Some(old) if old != file => change.changed.push(file.path.clone()),
            Some(_) => {}
        }
    }
    for old in before {
        if !after.iter().any(|file| file.path == old.path) {
            change.removed.push(old.path.clone());
        }
    }
    change
}

/// Reads at most `MAX_FILE_BYTES + 1` bytes of `reader`; `None` means the content is over the cap.
/// The bound is on the read itself, so a file that grew since the scan (or never ends) costs one
/// buffer of the cap.
fn read_bounded(reader: impl io::Read) -> io::Result<Option<Zeroizing<Vec<u8>>>> {
    let mut bytes = Zeroizing::new(Vec::new());
    reader.take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes)?;
    Ok((bytes.len() as u64 <= MAX_FILE_BYTES).then_some(bytes))
}

/// Opens `path`, reads it within the cap, and parses it with the path rule of kube's `read_from`
/// (`Kubeconfig::parse_file`). Never kube's own `read_from`, which reads without a bound. The text
/// is wiped when dropped. Blocking file I/O: call it off the UI thread.
pub(crate) fn load_folder_file(path: &Path) -> Result<Kubeconfig, KubeconfigError> {
    let read_error = |source| KubeconfigError::Read {
        path: path.to_path_buf(),
        source,
    };
    let file = File::open(path).map_err(read_error)?;
    let Some(bytes) = read_bounded(file).map_err(read_error)? else {
        return Err(KubeconfigError::TooLarge {
            path: path.to_path_buf(),
        });
    };
    let text = decode_text(&bytes).map_err(read_error)?;
    Kubeconfig::parse_file(&text, path)
}

/// UTF-8, or UTF-16 little endian with a byte order mark (as kube reads a file).
fn decode_text(bytes: &[u8]) -> io::Result<Zeroizing<String>> {
    let invalid = |reason: &'static str| io::Error::new(io::ErrorKind::InvalidData, reason);
    match bytes {
        [0xFF, 0xFE, rest @ ..] => {
            let units: Vec<u16> = rest
                .chunks(2)
                .map(|pair| u16::from_le_bytes([pair[0], *pair.get(1).unwrap_or(&0)]))
                .collect();
            String::from_utf16(&units)
                .map(Zeroizing::new)
                .map_err(|_| invalid("not valid UTF-16"))
        }
        _ => std::str::from_utf8(bytes)
            .map(|text| Zeroizing::new(text.to_owned()))
            .map_err(|_| invalid("not valid UTF-8")),
    }
}

#[cfg(test)]
#[path = "kubeconfig_folder_tests.rs"]
mod kubeconfig_folder_tests;
