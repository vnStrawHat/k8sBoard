//! The settings file on disk: where it lives, how it loads and recovers, and how it is written.
//! No GPUI here; `settings.rs` owns the in-app global.

use std::ffi::OsString;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use crate::settings::{SETTINGS_VERSION, Settings};

pub(crate) const CONFIG_DIR_ENV: &str = "K8SBOARD_CONFIG_DIR";
const SETTINGS_FILE: &str = "settings.json";
const BACKUP_FILE: &str = "settings.json.bak";
const TEMP_FILE: &str = "settings.json.tmp";
const CONFIG_FOLDER: &str = "k8sboard";
const RENAME_RETRY_DELAY: Duration = Duration::from_millis(50);

/// `flag` > a non-empty `env_value` > `fallback`; the result is made absolute so registry keys
/// and later comparisons do not depend on the working directory.
pub(crate) fn config_dir(
    flag: Option<PathBuf>,
    env_value: Option<OsString>,
    fallback: Option<PathBuf>,
) -> Option<PathBuf> {
    let chosen = flag
        .or_else(|| {
            env_value
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
        })
        .or(fallback)?;
    Some(std::path::absolute(&chosen).unwrap_or(chosen))
}

/// Debug builds keep settings inside the workspace (`<workspace>/.tmp/config`), so a dev or
/// agent run never touches the real config folder; release builds use the OS config dir.
pub(crate) fn default_config_dir() -> Option<PathBuf> {
    if cfg!(debug_assertions) {
        // `crates/app` is two levels below the workspace root.
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().nth(2)?;
        return Some(workspace.join(".tmp").join("config"));
    }
    dirs::config_dir().map(|dir| dir.join(CONFIG_FOLDER))
}

#[derive(Debug)]
pub(crate) struct LoadedSettings {
    pub(crate) settings: Settings,
    pub(crate) writes: WriteMode,
    pub(crate) notice: Option<SettingsNotice>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum WriteMode {
    /// Writes go to this config dir.
    Enabled(PathBuf),
    Disabled,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SettingsNotice {
    Reset { backup: PathBuf },
    NewerVersion { version: u32 },
    Unreadable { path: PathBuf, kind: io::ErrorKind },
    NoConfigDir,
    WriteFailed { path: PathBuf, kind: io::ErrorKind },
}

impl SettingsNotice {
    /// The text of the persistent banner under the title bar; only a reset has one, because it
    /// silently drops the environments and locks the user set.
    pub(crate) fn banner_text(&self) -> Option<String> {
        let Self::Reset { backup } = self else {
            return None;
        };
        Some(format!(
            "Settings were reset; environments and locks are back to defaults. Old file: {}",
            backup.display()
        ))
    }
}

impl fmt::Display for SettingsNotice {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Reset { backup } => write!(
                formatter,
                "Settings were unreadable and were reset; the old file is {}",
                backup.display()
            ),
            Self::NewerVersion { version } => write!(
                formatter,
                "Settings were saved by a newer k8sBoard (version {version}); changes are not saved this session"
            ),
            Self::Unreadable { path, kind } => write!(
                formatter,
                "Cannot read {} ({kind}); changes are not saved this session",
                path.display()
            ),
            Self::NoConfigDir => {
                formatter.write_str("No config folder found; settings are not saved")
            }
            Self::WriteFailed { path, kind } => {
                write!(
                    formatter,
                    "Cannot save settings to {} ({kind})",
                    path.display()
                )
            }
        }
    }
}

impl LoadedSettings {
    fn defaults(writes: WriteMode, notice: Option<SettingsNotice>) -> Self {
        Self {
            settings: Settings::default(),
            writes,
            notice,
        }
    }

    /// A screenshot run reads the seeded file but never changes it, so shots stay repeatable. The lab
    /// build really writes to the kind lab, so it keeps the config folder, which holds the audit log.
    pub(crate) fn for_screenshot(mut self, is_lab_build: bool) -> Self {
        if !is_lab_build {
            self.writes = WriteMode::Disabled;
        }
        self
    }

    /// No config folder could be located: defaults, nothing is saved.
    pub(crate) fn without_config_dir() -> Self {
        Self::defaults(WriteMode::Disabled, Some(SettingsNotice::NoConfigDir))
    }
}

/// The result of reading the file's bytes.
enum Parsed {
    Current(Settings),
    Newer { settings: Settings, version: u32 },
    Corrupt,
}

/// Blocking: called from `main` before the first frame.
pub(crate) fn load_settings(dir: &Path) -> LoadedSettings {
    let path = settings_path(dir);
    let enabled = || WriteMode::Enabled(dir.to_path_buf());
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return LoadedSettings::defaults(enabled(), None);
        }
        Err(error) => return unreadable(path, error.kind()),
    };
    match parse_settings(&bytes) {
        Parsed::Current(settings) => LoadedSettings {
            settings,
            writes: enabled(),
            notice: None,
        },
        Parsed::Newer { settings, version } => LoadedSettings {
            settings,
            writes: WriteMode::Disabled,
            notice: Some(SettingsNotice::NewerVersion { version }),
        },
        Parsed::Corrupt => {
            let backup = dir.join(BACKUP_FILE);
            // `rename` replaces an older backup on every OS we ship.
            match std::fs::rename(&path, &backup) {
                Ok(()) => {
                    LoadedSettings::defaults(enabled(), Some(SettingsNotice::Reset { backup }))
                }
                Err(error) => unreadable(path, error.kind()),
            }
        }
    }
}

fn unreadable(path: PathBuf, kind: io::ErrorKind) -> LoadedSettings {
    tracing::warn!(path = %path.display(), ?kind, "cannot read the settings file");
    LoadedSettings::defaults(
        WriteMode::Disabled,
        Some(SettingsNotice::Unreadable { path, kind }),
    )
}

/// Two passes, so the version is known even when the rest does not fit the model. Only the error
/// class and position are traced: serde messages can quote values.
fn parse_settings(bytes: &[u8]) -> Parsed {
    let value = match serde_json::from_slice::<serde_json::Value>(bytes) {
        Ok(value) => value,
        Err(error) => return corrupt(&error),
    };
    let version = value
        .get("version")
        .and_then(serde_json::Value::as_u64)
        .and_then(|version| u32::try_from(version).ok())
        .filter(|version| *version > 0);
    let Some(version) = version else {
        tracing::warn!("settings file has no usable version");
        return Parsed::Corrupt;
    };
    match serde_json::from_value::<Settings>(value) {
        Ok(settings) if version <= SETTINGS_VERSION => Parsed::Current(settings),
        Ok(settings) => Parsed::Newer { settings, version },
        // A newer file this version cannot read is left alone, not reset.
        Err(_) if version > SETTINGS_VERSION => Parsed::Newer {
            settings: Settings::default(),
            version,
        },
        Err(error) => corrupt(&error),
    }
}

fn corrupt(error: &serde_json::Error) -> Parsed {
    tracing::warn!(
        class = ?error.classify(),
        line = error.line(),
        column = error.column(),
        "settings file is corrupt"
    );
    Parsed::Corrupt
}

/// Serializes writes and remembers the newest generation on disk, so an older snapshot never
/// replaces a newer one (the quit flush against a background write still in flight).
#[derive(Default)]
pub(crate) struct WriteGate {
    written: Mutex<u64>,
}

/// Pretty JSON with a trailing newline. `None` only if `serde_json` fails, which these types
/// cannot cause; the warning carries no content.
pub(crate) fn serialize_settings(settings: &Settings) -> Option<Vec<u8>> {
    match serde_json::to_vec_pretty(settings) {
        Ok(mut bytes) => {
            bytes.push(b'\n');
            Some(bytes)
        }
        Err(_) => {
            tracing::warn!("cannot serialize settings");
            None
        }
    }
}

/// Under the gate: skips when `generation` is not newer than the last write, creates `dir`,
/// writes the temp file, syncs it, and renames it over `settings.json`. Blocking: call it from
/// the background executor, or from the main thread only for the quit flush.
pub(crate) fn write_settings(
    dir: &Path,
    bytes: &[u8],
    generation: u64,
    gate: &WriteGate,
) -> io::Result<()> {
    let mut written = gate.written.lock().unwrap_or_else(PoisonError::into_inner);
    if generation <= *written {
        return Ok(());
    }
    std::fs::create_dir_all(dir)?;
    let temp = dir.join(TEMP_FILE);
    let result = write_temp_then_rename(&temp, &settings_path(dir), bytes);
    match &result {
        Ok(()) => *written = generation,
        Err(_) => {
            let _ = std::fs::remove_file(&temp);
        }
    }
    result
}

fn write_temp_then_rename(temp: &Path, target: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = std::fs::File::create(temp)?;
    io::Write::write_all(&mut file, bytes)?;
    file.sync_all()?;
    drop(file);
    // ponytail: one retry covers transient Windows sharing violations (antivirus, indexer, editor); add backoff if WriteFailed shows up in practice
    std::fs::rename(temp, target).or_else(|_| {
        std::thread::sleep(RENAME_RETRY_DELAY);
        std::fs::rename(temp, target)
    })
}

/// The settings file inside `dir`.
pub(crate) fn settings_path(dir: &Path) -> PathBuf {
    dir.join(SETTINGS_FILE)
}

#[cfg(test)]
#[path = "settings_store_tests.rs"]
mod settings_store_tests;
