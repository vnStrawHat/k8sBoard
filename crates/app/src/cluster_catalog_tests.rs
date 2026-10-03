use std::path::Path;

use gpui_kit::{AppContext as _, ClipboardItem, TestAppContext};

use super::*;
use crate::cluster_registry::ClusterRegistry;
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
