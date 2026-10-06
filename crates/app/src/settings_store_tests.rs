use std::ffi::OsString;

use super::*;
use crate::settings::ThemePreference;

/// A fresh temp dir per test; the contents hold no credentials.
fn temp_dir(name: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("k8sboard-0024-store-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn cleanup(dir: &Path) {
    let _ = std::fs::remove_dir_all(dir);
}

fn put(dir: &Path, text: &str) {
    std::fs::write(settings_path(dir), text).expect("write settings fixture");
}

fn assert_reset(loaded: &LoadedSettings, dir: &Path) {
    let backup = dir.join(BACKUP_FILE);
    assert_eq!(loaded.settings, Settings::default());
    assert_eq!(loaded.writes, WriteMode::Enabled(dir.to_path_buf()));
    assert_eq!(loaded.notice, Some(SettingsNotice::Reset { backup }));
    assert!(!settings_path(dir).exists(), "the corrupt file is moved");
}

#[test]
fn config_dir_prefers_flag_then_env_then_fallback() {
    let flag = std::env::temp_dir().join("flag");
    let env = std::env::temp_dir().join("env");
    let fallback = std::env::temp_dir().join("fallback");
    let pick = |flag: Option<&PathBuf>, env: Option<&PathBuf>| {
        config_dir(
            flag.cloned(),
            env.map(|path| path.clone().into_os_string()),
            Some(fallback.clone()),
        )
    };
    assert_eq!(pick(Some(&flag), Some(&env)), Some(flag.clone()));
    assert_eq!(pick(None, Some(&env)), Some(env));
    assert_eq!(pick(None, None), Some(fallback));
    assert_eq!(config_dir(None, None, None), None);
}

#[test]
fn empty_env_value_is_ignored() {
    let fallback = std::env::temp_dir().join("fallback");
    let chosen = config_dir(None, Some(OsString::new()), Some(fallback.clone()));
    assert_eq!(chosen, Some(fallback));
}

#[test]
fn relative_config_dir_becomes_absolute() {
    let chosen = config_dir(Some(PathBuf::from(".tmp/config")), None, None).expect("a dir");
    assert!(chosen.is_absolute(), "{}", chosen.display());
    assert!(chosen.ends_with(Path::new(".tmp").join("config")));
}

#[test]
fn debug_default_is_inside_the_workspace() {
    if !cfg!(debug_assertions) {
        return;
    }
    let dir = default_config_dir().expect("a workspace dir");
    assert!(
        dir.ends_with(Path::new(".tmp").join("config")),
        "{}",
        dir.display()
    );
    assert!(dir.is_absolute());
}

#[test]
fn missing_file_gives_defaults_without_notice() {
    let dir = temp_dir("missing");
    let loaded = load_settings(&dir);
    assert_eq!(loaded.settings, Settings::default());
    assert_eq!(loaded.writes, WriteMode::Enabled(dir.clone()));
    assert_eq!(loaded.notice, None);
    cleanup(&dir);
}

#[test]
fn valid_file_is_parsed() {
    let dir = temp_dir("valid");
    put(&dir, r#"{"version": 1, "theme": "dark"}"#);
    let loaded = load_settings(&dir);
    assert_eq!(loaded.settings.theme, ThemePreference::Dark);
    assert_eq!(loaded.notice, None);
    cleanup(&dir);
}

#[test]
fn invalid_json_is_backed_up_and_reset() {
    let dir = temp_dir("invalid-json");
    put(&dir, "{");
    let loaded = load_settings(&dir);
    assert_reset(&loaded, &dir);
    let backup = std::fs::read(dir.join(BACKUP_FILE)).expect("backup exists");
    assert_eq!(backup, b"{");
    cleanup(&dir);
}

#[test]
fn wrong_type_is_backed_up_and_reset() {
    let dir = temp_dir("wrong-type");
    put(&dir, r#"{"version": 1, "theme": 7}"#);
    assert_reset(&load_settings(&dir), &dir);
    cleanup(&dir);
}

#[test]
fn missing_version_is_backed_up_and_reset() {
    let dir = temp_dir("no-version");
    put(&dir, r#"{"theme": "dark"}"#);
    assert_reset(&load_settings(&dir), &dir);
    cleanup(&dir);
}

#[test]
fn zero_version_is_backed_up_and_reset() {
    let dir = temp_dir("zero-version");
    put(&dir, r#"{"version": 0}"#);
    assert_reset(&load_settings(&dir), &dir);
    cleanup(&dir);
}

#[test]
fn version_above_u32_is_backed_up_and_reset() {
    let dir = temp_dir("huge-version");
    put(&dir, r#"{"version": 4294967296}"#);
    assert_reset(&load_settings(&dir), &dir);
    cleanup(&dir);
}

#[test]
fn backup_replaces_an_older_backup() {
    let dir = temp_dir("backup-replace");
    std::fs::write(dir.join(BACKUP_FILE), "old").expect("old backup");
    put(&dir, "{");
    assert_reset(&load_settings(&dir), &dir);
    assert_eq!(std::fs::read(dir.join(BACKUP_FILE)).expect("backup"), b"{");
    cleanup(&dir);
}

#[test]
fn newer_version_loads_with_writes_disabled() {
    let dir = temp_dir("newer");
    put(
        &dir,
        r#"{"version": 2, "theme": "light", "future": {"a": 1}}"#,
    );
    let loaded = load_settings(&dir);
    assert_eq!(loaded.settings.theme, ThemePreference::Light);
    assert_eq!(loaded.writes, WriteMode::Disabled);
    assert_eq!(
        loaded.notice,
        Some(SettingsNotice::NewerVersion { version: 2 })
    );
    assert!(settings_path(&dir).exists(), "a newer file is never moved");
    cleanup(&dir);
}

#[test]
fn newer_version_that_does_not_fit_gives_defaults_and_keeps_the_file() {
    let dir = temp_dir("newer-misfit");
    put(&dir, r#"{"version": 2, "theme": {"name": "x"}}"#);
    let loaded = load_settings(&dir);
    assert_eq!(loaded.settings, Settings::default());
    assert_eq!(loaded.writes, WriteMode::Disabled);
    assert!(settings_path(&dir).exists());
    cleanup(&dir);
}

#[test]
fn unreadable_file_disables_writes() {
    let dir = temp_dir("unreadable");
    // A directory where the file should be: reading it fails with something other than NotFound.
    std::fs::create_dir(settings_path(&dir)).expect("directory in place of the file");
    let loaded = load_settings(&dir);
    assert_eq!(loaded.writes, WriteMode::Disabled);
    assert!(
        matches!(loaded.notice, Some(SettingsNotice::Unreadable { .. })),
        "{:?}",
        loaded.notice
    );
    cleanup(&dir);
}

#[test]
fn write_creates_the_dir_and_file() {
    let dir = temp_dir("write-create").join("nested");
    let gate = WriteGate::default();
    write_settings(&dir, b"one\n", 1, &gate).expect("write succeeds");
    assert_eq!(std::fs::read(settings_path(&dir)).expect("file"), b"one\n");
    cleanup(dir.parent().expect("parent"));
}

#[test]
fn write_replaces_existing_file_and_leaves_no_temp() {
    let dir = temp_dir("write-replace");
    let gate = WriteGate::default();
    write_settings(&dir, b"one\n", 1, &gate).expect("first write");
    write_settings(&dir, b"two\n", 2, &gate).expect("second write");
    assert_eq!(std::fs::read(settings_path(&dir)).expect("file"), b"two\n");
    assert!(!dir.join(TEMP_FILE).exists());
    cleanup(&dir);
}

#[test]
fn older_generation_never_overwrites_newer() {
    let dir = temp_dir("gate");
    let gate = WriteGate::default();
    write_settings(&dir, b"new\n", 5, &gate).expect("newer write");
    write_settings(&dir, b"old\n", 4, &gate).expect("older write is skipped");
    write_settings(&dir, b"same\n", 5, &gate).expect("same generation is skipped");
    assert_eq!(std::fs::read(settings_path(&dir)).expect("file"), b"new\n");
    cleanup(&dir);
}

#[test]
fn failed_write_removes_the_temp_and_keeps_the_generation() {
    let dir = temp_dir("write-fail");
    // A directory in place of the target: the rename fails.
    std::fs::create_dir(settings_path(&dir)).expect("directory in place of the file");
    let gate = WriteGate::default();
    assert!(write_settings(&dir, b"x\n", 1, &gate).is_err());
    assert!(!dir.join(TEMP_FILE).exists());
    // The failed generation is not recorded, so the same one can still be written.
    std::fs::remove_dir(settings_path(&dir)).expect("remove the blocking directory");
    write_settings(&dir, b"x\n", 1, &gate).expect("retry of the same generation");
    assert_eq!(std::fs::read(settings_path(&dir)).expect("file"), b"x\n");
    cleanup(&dir);
}

#[test]
fn serialized_settings_end_with_newline() {
    let bytes = serialize_settings(&Settings::default()).expect("serializes");
    assert_eq!(bytes.last(), Some(&b'\n'));
}

#[test]
fn notice_texts_name_the_cause() {
    let path = PathBuf::from("settings.json");
    let kind = io::ErrorKind::PermissionDenied;
    let texts = [
        SettingsNotice::Reset {
            backup: path.clone(),
        }
        .to_string(),
        SettingsNotice::NewerVersion { version: 3 }.to_string(),
        SettingsNotice::Unreadable {
            path: path.clone(),
            kind,
        }
        .to_string(),
        SettingsNotice::NoConfigDir.to_string(),
        SettingsNotice::WriteFailed { path, kind }.to_string(),
    ];
    assert!(
        texts[0].contains("the old file is settings.json"),
        "{}",
        texts[0]
    );
    assert!(texts[1].contains("version 3"), "{}", texts[1]);
    assert!(
        texts[2].starts_with("Cannot read settings.json"),
        "{}",
        texts[2]
    );
    assert_eq!(texts[3], "No config folder found; settings are not saved");
    assert!(
        texts[4].starts_with("Cannot save settings to settings.json"),
        "{}",
        texts[4]
    );
}

#[test]
fn only_a_reset_has_a_banner() {
    let backup = PathBuf::from("settings.json.bak");
    assert_eq!(
        SettingsNotice::Reset { backup }.banner_text().as_deref(),
        Some(
            "Settings were reset; environments and locks are back to defaults. \
             Old file: settings.json.bak"
        )
    );
    assert_eq!(SettingsNotice::NoConfigDir.banner_text(), None);
    assert_eq!(
        SettingsNotice::NewerVersion { version: 2 }.banner_text(),
        None
    );
}
