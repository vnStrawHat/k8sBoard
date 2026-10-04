use std::path::Path;
use std::time::Duration;

use gpui_kit::{AppContext as _, ClipboardItem, TestAppContext};

use super::*;
use crate::cluster_registry::ClusterRegistry;
use crate::kubeconfig_folder::{FolderStatus, RESCAN_DEBOUNCE, RESCAN_MAX_WAIT};
use crate::settings::{Settings, ThemePreference};
use crate::settings_store::{LoadedSettings, WriteMode};

/// A fresh temp dir per test; the contents hold no credentials.
fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("k8sboard-0025-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn write_kubeconfig(dir: &Path, name: &str, context: &str) -> PathBuf {
    let text = format!(
        "apiVersion: v1\nkind: Config\nclusters:\n  - name: c\n    cluster: {{ server: 'https://127.0.0.1:1' }}\ncontexts:\n  - name: {context}\n    context: {{ cluster: c }}\n"
    );
    let path = dir.join(name);
    std::fs::write(&path, text).expect("write kubeconfig fixture");
    path
}

fn install_settings(registered: &[PathBuf], cx: &mut TestAppContext) {
    cx.update(|cx| {
        AppSettings::install(
            LoadedSettings {
                settings: Settings {
                    registry: ClusterRegistry {
                        kubeconfigs: registered.to_vec(),
                        ..ClusterRegistry::default()
                    },
                    ..Settings::default()
                },
                writes: WriteMode::Disabled,
                notice: None,
            },
            cx,
        );
    });
}

fn set_registered(registered: &[PathBuf], cx: &mut TestAppContext) {
    let registered = registered.to_vec();
    cx.update(|cx| AppSettings::update(cx, |settings| settings.registry.kubeconfigs = registered));
    cx.run_until_parked();
}

fn open_catalog(chain: &[PathBuf], cx: &mut TestAppContext) -> Entity<ClusterCatalog> {
    let chain = chain.to_vec();
    let catalog = cx.new(|cx| ClusterCatalog::new(chain, cx));
    cx.run_until_parked();
    catalog
}

fn context_names(catalog: &Entity<ClusterCatalog>, cx: &TestAppContext) -> Vec<String> {
    catalog.read_with(cx, |catalog, _| {
        catalog
            .kubeconfigs()
            .flat_map(|kubeconfig| kubeconfig.contexts())
            .map(|context| context.name.clone())
            .collect()
    })
}

#[gpui_kit::test]
fn catalog_lists_chain_before_standalone(cx: &mut TestAppContext) {
    let dir = temp_dir("order");
    let chain = write_kubeconfig(&dir, "chain.yaml", "from-chain");
    let extra = write_kubeconfig(&dir, "extra.yaml", "from-file");
    install_settings(&[extra], cx);
    let catalog = open_catalog(&[chain], cx);
    assert_eq!(context_names(&catalog, cx), ["from-chain", "from-file"]);
    assert!(catalog.read_with(cx, |catalog, _| !catalog.is_loading()));
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn catalog_is_loading_until_the_background_load_finishes(cx: &mut TestAppContext) {
    let dir = temp_dir("loading");
    let chain = write_kubeconfig(&dir, "chain.yaml", "from-chain");
    install_settings(&[], cx);
    let catalog = cx.new(|cx| ClusterCatalog::new(vec![chain], cx));
    assert!(catalog.read_with(cx, |catalog, _| catalog.is_loading()));
    cx.run_until_parked();
    assert!(catalog.read_with(cx, |catalog, _| !catalog.is_loading()));
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn added_registry_file_is_loaded(cx: &mut TestAppContext) {
    let dir = temp_dir("added");
    let chain = write_kubeconfig(&dir, "chain.yaml", "from-chain");
    let extra = write_kubeconfig(&dir, "extra.yaml", "from-file");
    install_settings(&[], cx);
    let catalog = open_catalog(&[chain], cx);
    assert_eq!(context_names(&catalog, cx), ["from-chain"]);
    set_registered(&[extra], cx);
    assert_eq!(context_names(&catalog, cx), ["from-chain", "from-file"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn removed_registry_file_is_dropped(cx: &mut TestAppContext) {
    let dir = temp_dir("removed");
    let chain = write_kubeconfig(&dir, "chain.yaml", "from-chain");
    let extra = write_kubeconfig(&dir, "extra.yaml", "from-file");
    install_settings(std::slice::from_ref(&extra), cx);
    let catalog = open_catalog(&[chain], cx);
    assert_eq!(context_names(&catalog, cx), ["from-chain", "from-file"]);
    set_registered(&[], cx);
    assert_eq!(context_names(&catalog, cx), ["from-chain"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn chain_is_not_reloaded_on_registry_change(cx: &mut TestAppContext) {
    let dir = temp_dir("chain-once");
    let chain = write_kubeconfig(&dir, "chain.yaml", "from-chain");
    let extra = write_kubeconfig(&dir, "extra.yaml", "from-file");
    install_settings(&[], cx);
    let catalog = open_catalog(std::slice::from_ref(&chain), cx);
    // A reload would pick up this edit.
    write_kubeconfig(&dir, "chain.yaml", "edited");
    set_registered(&[extra], cx);
    assert_eq!(context_names(&catalog, cx), ["from-chain", "from-file"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn unrelated_settings_change_keeps_the_loaded_files(cx: &mut TestAppContext) {
    let dir = temp_dir("unrelated");
    let extra = write_kubeconfig(&dir, "extra.yaml", "from-file");
    install_settings(&[extra], cx);
    let catalog = open_catalog(&[], cx);
    let before = catalog.read_with(cx, |catalog, _| catalog.kubeconfigs().next().cloned());
    cx.update(|cx| AppSettings::update(cx, |settings| settings.theme = ThemePreference::Dark));
    cx.run_until_parked();
    let after = catalog.read_with(cx, |catalog, _| catalog.kubeconfigs().next().cloned());
    let (Some(before), Some(after)) = (before, after) else {
        panic!("the file stays loaded");
    };
    assert!(Arc::ptr_eq(&before, &after));
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn failed_file_becomes_a_notice(cx: &mut TestAppContext) {
    let dir = temp_dir("failed");
    let chain = write_kubeconfig(&dir, "chain.yaml", "from-chain");
    let missing = dir.join("gone.yaml");
    install_settings(&[missing], cx);
    let catalog = open_catalog(&[chain], cx);
    assert_eq!(context_names(&catalog, cx), ["from-chain"]);
    catalog.read_with(cx, |catalog, _| {
        let [CatalogNotice::Skipped { error, .. }] = catalog.notices() else {
            panic!("one notice expected, got {:?}", catalog.notices());
        };
        assert!(error.contains("gone.yaml"), "{error}");
        assert!(
            catalog.notices()[0]
                .to_string()
                .starts_with("Skipped kubeconfig: ")
        );
    });
    catalog.update(cx, |catalog, cx| catalog.clear_notices(cx));
    assert!(catalog.read_with(cx, |catalog, _| catalog.notices().is_empty()));
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn skipped_chain_file_is_noted_and_the_rest_loads(cx: &mut TestAppContext) {
    let dir = temp_dir("skipped-chain");
    let chain = [
        write_kubeconfig(&dir, "a.yaml", "one"),
        dir.join("missing.yaml"),
    ];
    install_settings(&[], cx);
    let catalog = open_catalog(&chain, cx);
    assert_eq!(context_names(&catalog, cx), ["one"]);
    assert_eq!(
        catalog.read_with(cx, |catalog, _| catalog.notices().len()),
        1
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn nothing_loaded_reports_the_first_error(cx: &mut TestAppContext) {
    install_settings(&[], cx);
    let catalog = open_catalog(&[PathBuf::from("definitely-missing-kubeconfig.yaml")], cx);
    catalog.read_with(cx, |catalog, _| {
        assert_eq!(catalog.kubeconfigs().count(), 0);
        assert!(!catalog.is_loading());
        assert!(
            catalog
                .failure_text()
                .contains("definitely-missing-kubeconfig.yaml"),
            "{}",
            catalog.failure_text()
        );
    });
}

#[gpui_kit::test]
fn empty_catalog_says_no_kubeconfig_was_found(cx: &mut TestAppContext) {
    install_settings(&[], cx);
    let catalog = open_catalog(&[], cx);
    catalog.read_with(cx, |catalog, _| {
        assert!(!catalog.is_loading());
        assert!(catalog.failure_text().starts_with("no kubeconfig found"));
    });
}

// ---- Paths, pasted files, and removal ----

#[test]
fn same_path_text_table() {
    use PathStyle::{Unix, Windows};
    assert!(same_path_text(
        "C:\x5cA\x5cb.yaml",
        "c:/a/./B.yaml",
        Windows
    ));
    assert!(!same_path_text(
        "C:\x5cA\x5cb.yaml",
        "C:\x5cA\x5cc.yaml",
        Windows
    ));
    assert!(!same_path_text("/a/B", "/a/b", Unix));
    assert!(same_path_text("/a//b/./c", "/a/b/c", Unix));
    // A backslash is no separator on Unix: the whole text is one part.
    assert!(!same_path_text("a\x5cb", "a/b", Unix));
}

fn install_writable_settings(dir: &Path, registered: &[PathBuf], cx: &mut TestAppContext) {
    cx.update(|cx| {
        AppSettings::install(
            LoadedSettings {
                settings: Settings {
                    registry: ClusterRegistry {
                        kubeconfigs: registered.to_vec(),
                        ..ClusterRegistry::default()
                    },
                    ..Settings::default()
                },
                writes: WriteMode::Enabled(dir.to_path_buf()),
                notice: None,
            },
            cx,
        );
    });
}

const PASTED_YAML: &str = "apiVersion: v1\nkind: Config\nclusters:\n  - name: c\n    cluster: { server: 'https://127.0.0.1:1' }\nusers:\n  - name: u\n    user: { token: fixture-token-value }\ncontexts:\n  - name: pasted-ctx\n    context: { cluster: c, user: u }\n";

fn add_pasted(catalog: &Entity<ClusterCatalog>, text: &str, cx: &mut TestAppContext) {
    let mark = ClipboardMark::of(text);
    let text = text.to_owned();
    catalog.update(cx, |catalog, cx| {
        catalog.add_pasted(text, Some("pasted-ctx".to_owned()), mark, cx);
    });
    cx.run_until_parked();
}

fn registered(cx: &TestAppContext) -> Vec<PathBuf> {
    cx.read(|cx| AppSettings::get(cx).registry.kubeconfigs.clone())
}

fn clipboard_text(cx: &TestAppContext) -> Option<String> {
    cx.read_from_clipboard().and_then(|item| item.text())
}

#[gpui_kit::test]
fn paste_writes_loads_then_registers(cx: &mut TestAppContext) {
    let dir = temp_dir("paste");
    install_writable_settings(&dir, &[], cx);
    let catalog = open_catalog(&[], cx);
    add_pasted(&catalog, PASTED_YAML, cx);
    let files = registered(cx);
    assert_eq!(files.len(), 1);
    assert!(is_app_owned(&files[0], &dir), "{files:?}");
    assert_eq!(
        std::fs::read_to_string(&files[0]).expect("file"),
        PASTED_YAML
    );
    assert_eq!(context_names(&catalog, cx), ["pasted-ctx"]);
    catalog.read_with(cx, |catalog, _| {
        assert_eq!(
            catalog.paste_status(),
            &PasteStatus::Added(files[0].clone())
        );
    });
    catalog.update(cx, |catalog, cx| catalog.reset_paste_status(cx));
    catalog.read_with(cx, |catalog, _| {
        assert_eq!(catalog.paste_status(), &PasteStatus::Idle);
    });
    cx.run_until_parked();
    let saved = std::fs::read_to_string(dir.join("settings.json")).expect("settings saved");
    assert!(!saved.contains("fixture-token-value"), "{saved}");
    assert!(saved.contains("kubeconfigs"), "{saved}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn failed_write_registers_nothing_and_keeps_clipboard(cx: &mut TestAppContext) {
    let dir = temp_dir("paste-fail");
    // The config dir is a file, so the folder under it cannot be created.
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::write(&dir, "not a folder").expect("file in place of the folder");
    install_writable_settings(&dir, &[], cx);
    let catalog = open_catalog(&[], cx);
    cx.write_to_clipboard(ClipboardItem::new_string(PASTED_YAML.to_owned()));
    add_pasted(&catalog, PASTED_YAML, cx);
    assert!(registered(cx).is_empty());
    assert_eq!(clipboard_text(cx).as_deref(), Some(PASTED_YAML));
    catalog.read_with(cx, |catalog, _| {
        assert_eq!(catalog.paste_status(), &PasteStatus::Failed);
        assert!(matches!(catalog.notices(), [CatalogNotice::SaveFailed(_)]));
    });
    let _ = std::fs::remove_file(&dir);
}

#[gpui_kit::test]
fn paste_clears_the_matching_clipboard(cx: &mut TestAppContext) {
    let dir = temp_dir("paste-clear");
    install_writable_settings(&dir, &[], cx);
    let catalog = open_catalog(&[], cx);
    cx.write_to_clipboard(ClipboardItem::new_string(PASTED_YAML.to_owned()));
    add_pasted(&catalog, PASTED_YAML, cx);
    assert_ne!(clipboard_text(cx).as_deref(), Some(PASTED_YAML));
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn paste_keeps_an_unrelated_clipboard(cx: &mut TestAppContext) {
    let dir = temp_dir("paste-keep");
    install_writable_settings(&dir, &[], cx);
    let catalog = open_catalog(&[], cx);
    cx.write_to_clipboard(ClipboardItem::new_string("something else".to_owned()));
    add_pasted(&catalog, PASTED_YAML, cx);
    assert_eq!(clipboard_text(cx).as_deref(), Some("something else"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn unregistered_pasted_file_raises_notice(cx: &mut TestAppContext) {
    let dir = temp_dir("unregistered");
    let folder = dir.join(PASTED_DIR);
    std::fs::create_dir_all(&folder).expect("folder");
    let orphan = write_kubeconfig(&folder, "orphan.yaml", "orphan-ctx");
    let listed = write_kubeconfig(&folder, "listed.yaml", "listed-ctx");
    install_writable_settings(&dir, std::slice::from_ref(&listed), cx);
    let catalog = open_catalog(&[], cx);
    catalog.read_with(cx, |catalog, _| {
        assert_eq!(
            catalog.notices(),
            [CatalogNotice::Unregistered(orphan.clone())]
        );
        assert_eq!(
            catalog.notices()[0].to_string(),
            "Pasted file orphan.yaml is not registered"
        );
    });
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn remove_unregistered_deletes_the_file(cx: &mut TestAppContext) {
    let dir = temp_dir("remove-orphan");
    let folder = dir.join(PASTED_DIR);
    std::fs::create_dir_all(&folder).expect("folder");
    let orphan = write_kubeconfig(&folder, "orphan.yaml", "orphan-ctx");
    install_writable_settings(&dir, &[], cx);
    let catalog = open_catalog(&[], cx);
    catalog.update(cx, |catalog, cx| {
        catalog.remove_kubeconfig(orphan.clone(), cx);
    });
    cx.run_until_parked();
    assert!(!orphan.exists());
    catalog.read_with(cx, |catalog, _| assert!(catalog.notices().is_empty()));
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn remove_app_owned_deletes_before_dropping_entries(cx: &mut TestAppContext) {
    let dir = temp_dir("remove-owned");
    let folder = dir.join(PASTED_DIR);
    std::fs::create_dir_all(&folder).expect("folder");
    let owned = write_kubeconfig(&folder, "owned.yaml", "owned-ctx");
    install_writable_settings(&dir, std::slice::from_ref(&owned), cx);
    let catalog = open_catalog(&[], cx);
    assert_eq!(context_names(&catalog, cx), ["owned-ctx"]);
    catalog.update(cx, |catalog, cx| {
        catalog.remove_kubeconfig(owned.clone(), cx);
    });
    // Nothing changed yet: the delete runs in the background first.
    assert_eq!(registered(cx), std::slice::from_ref(&owned));
    cx.run_until_parked();
    assert!(!owned.exists());
    assert!(registered(cx).is_empty());
    assert!(context_names(&catalog, cx).is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn failed_delete_keeps_the_file_listed(cx: &mut TestAppContext) {
    let dir = temp_dir("remove-fail");
    // A directory with a file name: `remove_file` fails on every OS.
    let stuck = dir.join(PASTED_DIR).join("stuck.yaml");
    std::fs::create_dir_all(&stuck).expect("directory in place of the file");
    install_writable_settings(&dir, std::slice::from_ref(&stuck), cx);
    let catalog = open_catalog(&[], cx);
    catalog.update(cx, |catalog, cx| {
        catalog.remove_kubeconfig(stuck.clone(), cx);
    });
    cx.run_until_parked();
    assert_eq!(registered(cx), std::slice::from_ref(&stuck));
    catalog.read_with(cx, |catalog, _| {
        assert!(
            catalog
                .notices()
                .contains(&CatalogNotice::DeleteFailed(stuck.clone()))
        );
    });
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn remove_of_a_user_file_edits_the_registry_only(cx: &mut TestAppContext) {
    let dir = temp_dir("remove-user");
    let extra = write_kubeconfig(&dir, "extra.yaml", "from-file");
    install_writable_settings(&dir.join("config"), std::slice::from_ref(&extra), cx);
    let catalog = open_catalog(&[], cx);
    catalog.update(cx, |catalog, cx| {
        catalog.remove_kubeconfig(extra.clone(), cx);
    });
    cx.run_until_parked();
    assert!(extra.exists(), "the user's own file is never deleted");
    assert!(registered(cx).is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn failed_file_is_noted_once_per_listing(cx: &mut TestAppContext) {
    let dir = temp_dir("dedupe");
    let missing = dir.join("gone.yaml");
    install_settings(&[], cx);
    let catalog = open_catalog(&[], cx);
    let count = |cx: &TestAppContext| catalog.read_with(cx, |catalog, _| catalog.notices().len());
    set_registered(std::slice::from_ref(&missing), cx);
    assert_eq!(count(cx), 1);
    // A listing that names the file twice still loads it once.
    set_registered(&[missing.clone(), missing.clone()], cx);
    assert_eq!(count(cx), 1);
    set_registered(&[], cx);
    assert_eq!(count(cx), 0);
    set_registered(&[missing], cx);
    assert_eq!(count(cx), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn notice_of_a_removed_file_is_dropped(cx: &mut TestAppContext) {
    let dir = temp_dir("stale-notice");
    let missing = dir.join("gone.yaml");
    install_settings(std::slice::from_ref(&missing), cx);
    let catalog = open_catalog(&[], cx);
    catalog.read_with(cx, |catalog, _| assert_eq!(catalog.notices().len(), 1));
    set_registered(&[], cx);
    catalog.read_with(cx, |catalog, _| assert!(catalog.notices().is_empty()));
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn user_file_in_the_pasted_folder_is_not_offered_for_deletion(cx: &mut TestAppContext) {
    let dir = temp_dir("user-file");
    let folder = dir.join(PASTED_DIR);
    std::fs::create_dir_all(&folder).expect("folder");
    let mine = write_kubeconfig(&folder, "My Config.yaml", "mine");
    install_writable_settings(&dir, &[], cx);
    let catalog = open_catalog(&[], cx);
    catalog.read_with(cx, |catalog, _| assert!(catalog.notices().is_empty()));
    // Even asked to remove it, only the registry is edited: the file stays.
    catalog.update(cx, |catalog, cx| {
        catalog.remove_kubeconfig(mine.clone(), cx);
    });
    cx.run_until_parked();
    assert!(mine.exists());
    let _ = std::fs::remove_dir_all(&dir);
}

// ---- Spec 0043 step 5: watched folders ----
//
// No real watcher runs here: a test sends `FolderEvent`s through `folder_events_for_test` and moves
// the GPUI fake clock, so the debounce is exact and nothing waits on the wall clock.

fn install_folder_settings(folders: &[PathBuf], registered: &[PathBuf], cx: &mut TestAppContext) {
    cx.update(|cx| {
        AppSettings::install(
            LoadedSettings {
                settings: Settings {
                    registry: ClusterRegistry {
                        kubeconfigs: registered.to_vec(),
                        kubeconfig_folders: folders.to_vec(),
                        ..ClusterRegistry::default()
                    },
                    ..Settings::default()
                },
                writes: WriteMode::Disabled,
                notice: None,
            },
            cx,
        );
    });
}

fn send_event(folder: &Path, catalog: &Entity<ClusterCatalog>, cx: &mut TestAppContext) {
    let sender = catalog.read_with(cx, |catalog, _| catalog.folder_events_for_test());
    sender
        .unbounded_send(FolderEvent {
            folder: folder.to_path_buf(),
        })
        .expect("the catalog listens");
    // The task has taken the event, so its timers exist before the clock moves.
    cx.run_until_parked();
}

fn advance(by: Duration, cx: &mut TestAppContext) {
    cx.executor().advance_clock(by);
    cx.run_until_parked();
}

/// Sends an event for `folder` and lets the debounce pass: one rescan.
fn rescan_after_debounce(folder: &Path, catalog: &Entity<ClusterCatalog>, cx: &mut TestAppContext) {
    send_event(folder, catalog, cx);
    advance(RESCAN_DEBOUNCE, cx);
}

fn rescans(catalog: &Entity<ClusterCatalog>, cx: &TestAppContext) -> usize {
    catalog.read_with(cx, |catalog, _| catalog.folder_rescans())
}

fn notice_texts(catalog: &Entity<ClusterCatalog>, cx: &TestAppContext) -> Vec<String> {
    catalog.read_with(cx, |catalog, _| {
        catalog.notices().iter().map(ToString::to_string).collect()
    })
}

#[gpui_kit::test]
fn folder_file_rows_come_and_go(cx: &mut TestAppContext) {
    let dir = temp_dir("folder-rows");
    write_kubeconfig(&dir, "a.yaml", "from-a");
    install_folder_settings(std::slice::from_ref(&dir), &[], cx);
    let catalog = open_catalog(&[], cx);
    assert_eq!(context_names(&catalog, cx), ["from-a"]);
    write_kubeconfig(&dir, "b.yaml", "from-b");
    // Nothing happens before the debounce has passed.
    send_event(&dir, &catalog, cx);
    advance(Duration::from_millis(499), cx);
    assert_eq!(context_names(&catalog, cx), ["from-a"]);
    advance(Duration::from_millis(1), cx);
    assert_eq!(context_names(&catalog, cx), ["from-a", "from-b"]);
    std::fs::remove_file(dir.join("b.yaml")).expect("delete a file");
    rescan_after_debounce(&dir, &catalog, cx);
    assert_eq!(context_names(&catalog, cx), ["from-a"]);
    // The app only reads: the folder holds what the test left there.
    assert!(dir.join("a.yaml").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn burst_of_events_rescans_once(cx: &mut TestAppContext) {
    let dir = temp_dir("folder-burst");
    write_kubeconfig(&dir, "a.yaml", "from-a");
    install_folder_settings(std::slice::from_ref(&dir), &[], cx);
    let catalog = open_catalog(&[], cx);
    for _ in 0..5 {
        send_event(&dir, &catalog, cx);
        advance(Duration::from_millis(100), cx);
    }
    // 100 ms after the last event: still waiting. 500 ms after it: one rescan.
    assert_eq!(rescans(&catalog, cx), 0);
    advance(RESCAN_DEBOUNCE - Duration::from_millis(101), cx);
    assert_eq!(rescans(&catalog, cx), 0);
    advance(Duration::from_millis(1), cx);
    assert_eq!(rescans(&catalog, cx), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn endless_events_rescan_every_two_seconds(cx: &mut TestAppContext) {
    let dir = temp_dir("folder-endless");
    write_kubeconfig(&dir, "a.yaml", "from-a");
    install_folder_settings(std::slice::from_ref(&dir), &[], cx);
    let catalog = open_catalog(&[], cx);
    // An event every 100 ms for 5 s, as a log file written in a folder would cause.
    for _ in 0..50 {
        send_event(&dir, &catalog, cx);
        advance(Duration::from_millis(100), cx);
    }
    let during = rescans(&catalog, &*cx);
    assert!(
        (2..=3).contains(&during),
        "about one rescan per {RESCAN_MAX_WAIT:?} while events keep coming, got {during}"
    );
    // The stream ends: one more rescan after the debounce.
    advance(RESCAN_DEBOUNCE, cx);
    assert_eq!(rescans(&catalog, cx), during + 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn broken_rewrite_keeps_the_last_good_file(cx: &mut TestAppContext) {
    let dir = temp_dir("folder-broken");
    let file = write_kubeconfig(&dir, "a.yaml", "good");
    install_folder_settings(std::slice::from_ref(&dir), &[], cx);
    let catalog = open_catalog(&[], cx);
    assert_eq!(context_names(&catalog, cx), ["good"]);
    // An editor saved half a file.
    std::fs::write(&file, "clusters: [token: s3cr3t-value").expect("rewrite");
    rescan_after_debounce(&dir, &catalog, cx);
    assert_eq!(context_names(&catalog, cx), ["good"]);
    let notices = notice_texts(&catalog, cx);
    assert!(
        notices
            .iter()
            .any(|text| text.starts_with("Skipped kubeconfig:")),
        "{notices:?}"
    );
    assert!(
        notices.iter().all(|text| !text.contains("s3cr3t")),
        "{notices:?}"
    );
    // The next good version replaces it and the notice goes.
    write_kubeconfig(&dir, "a.yaml", "better-name");
    rescan_after_debounce(&dir, &catalog, cx);
    assert_eq!(context_names(&catalog, cx), ["better-name"]);
    assert!(notice_texts(&catalog, cx).is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn missing_folder_raises_a_notice(cx: &mut TestAppContext) {
    let dir = temp_dir("folder-missing");
    write_kubeconfig(&dir, "a.yaml", "from-a");
    install_folder_settings(std::slice::from_ref(&dir), &[], cx);
    let catalog = open_catalog(&[], cx);
    assert_eq!(context_names(&catalog, cx), ["from-a"]);
    std::fs::remove_dir_all(&dir).expect("remove the folder");
    rescan_after_debounce(&dir, &catalog, cx);
    assert!(context_names(&catalog, cx).is_empty());
    let notices = notice_texts(&catalog, cx);
    let expected_start = format!("Watched folder {} is missing (", dir.display());
    assert!(
        notices.iter().any(|text| text.starts_with(&expected_start)
            && text.ends_with("); checked again at the next start")),
        "{notices:?}"
    );
    let status = catalog.read_with(cx, |catalog, _| catalog.folder_summaries()[0].status);
    assert_eq!(status, FolderStatus::Missing);
    // The folder stays in the settings.
    let folders = cx.read(|cx| AppSettings::get(cx).registry.kubeconfig_folders.clone());
    assert_eq!(folders, [dir]);
}

#[gpui_kit::test]
fn a_folder_that_is_missing_at_start_raises_the_notice_at_once(cx: &mut TestAppContext) {
    let dir = temp_dir("folder-missing-at-start");
    std::fs::remove_dir_all(&dir).expect("remove the folder");
    install_folder_settings(std::slice::from_ref(&dir), &[], cx);
    let catalog = open_catalog(&[], cx);
    assert!(
        notice_texts(&catalog, cx)
            .iter()
            .any(|text| text.contains("is missing"))
    );
    assert!(catalog.read_with(cx, |catalog, _| !catalog.is_loading()));
}

#[gpui_kit::test]
fn stop_watching_keeps_the_files(cx: &mut TestAppContext) {
    let dir = temp_dir("folder-stop");
    write_kubeconfig(&dir, "a.yaml", "from-a");
    install_folder_settings(std::slice::from_ref(&dir), &[], cx);
    let catalog = open_catalog(&[], cx);
    assert_eq!(context_names(&catalog, cx), ["from-a"]);
    cx.update(|cx| {
        AppSettings::update(cx, |settings| {
            crate::cluster_form::stop_watching_folder(&mut settings.registry, &dir);
        });
    });
    cx.run_until_parked();
    assert!(context_names(&catalog, cx).is_empty());
    assert!(cx.read(|cx| AppSettings::get(cx).registry.kubeconfig_folders.is_empty()));
    assert!(dir.join("a.yaml").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn adding_a_folder_lists_its_files(cx: &mut TestAppContext) {
    let dir = temp_dir("folder-add");
    write_kubeconfig(&dir, "a.yaml", "from-a");
    install_folder_settings(&[], &[], cx);
    let catalog = open_catalog(&[], cx);
    assert!(context_names(&catalog, cx).is_empty());
    cx.update(|cx| {
        AppSettings::update(cx, |settings| {
            assert!(crate::cluster_form::add_watched_folder(
                &mut settings.registry,
                dir.clone()
            ));
            // The same folder twice is one watch.
            assert!(!crate::cluster_form::add_watched_folder(
                &mut settings.registry,
                dir.clone()
            ));
        });
    });
    cx.run_until_parked();
    assert_eq!(context_names(&catalog, cx), ["from-a"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn a_folder_file_is_never_a_start_candidate(cx: &mut TestAppContext) {
    let dir = temp_dir("folder-start");
    let watched = dir.join("watched");
    std::fs::create_dir_all(&watched).expect("folder");
    let chain = write_kubeconfig(&dir, "chain.yaml", "from-chain");
    let registered = write_kubeconfig(&dir, "reg.yaml", "from-registry");
    write_kubeconfig(&watched, "dropped.yaml", "from-folder");
    install_folder_settings(std::slice::from_ref(&watched), &[registered], cx);
    let catalog = open_catalog(&[chain], cx);
    let listed = context_names(&catalog, cx);
    assert_eq!(listed, ["from-chain", "from-registry", "from-folder"]);
    let started: Vec<String> = catalog.read_with(cx, |catalog, _| {
        catalog
            .start_kubeconfigs()
            .flat_map(|kubeconfig| kubeconfig.contexts())
            .map(|context| context.name.clone())
            .collect()
    });
    assert_eq!(started, ["from-chain", "from-registry"]);
    let is_folder_source = |name: &str, cx: &TestAppContext| {
        catalog.read_with(cx, |catalog, _| {
            catalog.is_folder_source(&watched.join(name))
        })
    };
    assert!(is_folder_source("dropped.yaml", cx));
    assert!(!catalog.read_with(cx, |catalog, _| {
        catalog.is_folder_source(&dir.join("reg.yaml"))
    }));
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn a_folder_file_the_registry_names_loads_once(cx: &mut TestAppContext) {
    let dir = temp_dir("folder-dup");
    let file = write_kubeconfig(&dir, "a.yaml", "from-a");
    install_folder_settings(std::slice::from_ref(&dir), &[file], cx);
    let catalog = open_catalog(&[], cx);
    assert_eq!(context_names(&catalog, cx), ["from-a"]);
    // It loads as the registry's file, so it is not a folder row.
    assert!(!catalog.read_with(cx, |catalog, _| {
        catalog.is_folder_source(&dir.join("a.yaml"))
    }));
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn the_folder_line_counts_what_it_found(cx: &mut TestAppContext) {
    let dir = temp_dir("folder-line");
    for index in 0..52 {
        write_kubeconfig(
            &dir,
            &format!("f{index:02}.yaml"),
            &format!("ctx-{index:02}"),
        );
    }
    std::fs::write(dir.join("f00.yaml"), "not a kubeconfig: [").expect("replace one");
    install_folder_settings(std::slice::from_ref(&dir), &[], cx);
    let catalog = open_catalog(&[], cx);
    let summaries = catalog.read_with(cx, |catalog, _| catalog.folder_summaries());
    let [summary] = summaries.as_slice() else {
        panic!("one watched folder");
    };
    assert_eq!(summary.kubeconfigs, 49);
    assert_eq!(summary.not_kubeconfigs, 1);
    assert_eq!(summary.not_loaded, 2);
    assert_eq!(
        summary.line_text(),
        format!(
            "Watching {} · 49 kubeconfig files, 1 not a kubeconfig, 2 more files not loaded",
            dir.display()
        )
    );
    let _ = std::fs::remove_dir_all(&dir);
}
