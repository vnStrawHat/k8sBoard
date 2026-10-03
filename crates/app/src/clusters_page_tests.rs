//! The Clusters page in a headless window. Every run uses a config dir under the temp dir and
//! the test platform's clipboard: nothing here touches the real settings or clipboard.

use std::path::{Path, PathBuf};

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::dialog::{Cancel, Confirm};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, ClipboardItem, TestAppContext, WindowOptions};

use super::*;
use crate::cluster_catalog::CatalogHandle;
use crate::settings::Settings;
use crate::settings_store::{LoadedSettings, WriteMode};

const TOKEN: &str = "fixture-token-value";

fn temp_dir(name: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("k8sboard-0025-page-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn kubeconfig_text(contexts: &[&str]) -> String {
    let mut text = format!(
        "apiVersion: v1\nkind: Config\nclusters:\n  - name: c\n    cluster: {{ server: 'https://127.0.0.1:1' }}\nusers:\n  - name: u\n    user: {{ token: {TOKEN} }}\ncontexts:\n"
    );
    for name in contexts {
        text.push_str(&format!(
            "  - name: {name}\n    context: {{ cluster: c, user: u }}\n"
        ));
    }
    text
}

fn write_kubeconfig(dir: &Path, name: &str, contexts: &[&str]) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, kubeconfig_text(contexts)).expect("write fixture");
    path
}

/// Installs the globals the page reads. `config_dir` `None` keeps writes off.
fn install(config_dir: Option<&Path>, chain: &[PathBuf], cx: &mut TestAppContext) {
    let chain = chain.to_vec();
    let writes = config_dir.map_or(WriteMode::Disabled, |dir| {
        WriteMode::Enabled(dir.to_path_buf())
    });
    cx.update(|cx| {
        gpui_kit::init(cx);
        // The dialog entrance animation would need frames to settle.
        cx.set_reduce_motion(true);
        AppSettings::install(
            LoadedSettings {
                settings: Settings::default(),
                writes,
                notice: None,
            },
            cx,
        );
        CatalogHandle::install(chain, cx);
    });
    cx.run_until_parked();
}

/// The page as the root view of a window (inside the kit `Root`, which hosts dialogs).
fn open_page(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<ClustersPage>) {
    cx.update(|cx| {
        let catalog = CatalogHandle::of(cx);
        gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| {
            cx.new(|cx| ClustersPage::new(catalog, cx))
        })
        .expect("open the test window")
    })
}

fn render(window: AnyWindowHandle, cx: &mut TestAppContext) {
    cx.update_window(window, |_, window, cx| window.render_frame(cx))
        .expect("the window is open");
    cx.run_until_parked();
}

// ---- The Clusters page ----

fn two_cluster_setup(
    name: &str,
    cx: &mut TestAppContext,
) -> (PathBuf, AnyWindowHandle, Entity<ClustersPage>) {
    let dir = temp_dir(name);
    let file = write_kubeconfig(&dir, "chain.yaml", &["prod-a", "prod-b"]);
    install(None, std::slice::from_ref(&file), cx);
    let (window, page) = open_page(cx);
    render(window, cx);
    (dir, window, page)
}

fn selected(page: &Entity<ClustersPage>, cx: &TestAppContext) -> Option<String> {
    page.read_with(cx, |page, _| {
        page.selected
            .as_ref()
            .map(|cluster| cluster.context.clone())
    })
}

#[gpui_kit::test]
fn first_row_is_selected_by_default(cx: &mut TestAppContext) {
    let (dir, window, page) = two_cluster_setup("default-selection", cx);
    render(window, cx);
    assert_eq!(selected(&page, cx).as_deref(), Some("prod-a"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn selection_change_recreates_inputs(cx: &mut TestAppContext) {
    let (dir, window, page) = two_cluster_setup("recreate", cx);
    let before = page.read_with(cx, |page, _| {
        page.form.as_ref().map(|form| form.name.entity_id())
    });
    assert!(before.is_some());
    let second = page.read_with(cx, |page, cx| page.rows(cx)[1].cluster.clone());
    page.update(cx, |page, cx| page.select(second, cx));
    render(window, cx);
    let after = page.read_with(cx, |page, _| {
        page.form.as_ref().map(|form| form.name.entity_id())
    });
    assert!(after.is_some());
    assert_ne!(before, after);
    assert_eq!(selected(&page, cx).as_deref(), Some("prod-b"));
    let _ = std::fs::remove_dir_all(&dir);
}

fn type_into_name(
    window: AnyWindowHandle,
    page: &Entity<ClustersPage>,
    text: &str,
    cx: &mut TestAppContext,
) {
    cx.update_window(window, |_, window, cx| {
        let name = page.read(cx).form.as_ref().expect("a form").name.clone();
        name.update(cx, |state, cx| state.focus(window, cx));
        window.input(text, cx);
    })
    .expect("the window is open");
    cx.run_until_parked();
}

fn type_into_namespace(
    window: AnyWindowHandle,
    page: &Entity<ClustersPage>,
    text: &str,
    cx: &mut TestAppContext,
) {
    cx.update_window(window, |_, window, cx| {
        let namespace = page
            .read(cx)
            .form
            .as_ref()
            .expect("a form")
            .namespace
            .clone();
        namespace.update(cx, |state, cx| state.focus(window, cx));
        window.input(text, cx);
    })
    .expect("the window is open");
    cx.run_until_parked();
}

fn stored_names(cx: &TestAppContext) -> Vec<Option<String>> {
    cx.read(|cx| {
        AppSettings::get(cx)
            .registry
            .clusters
            .iter()
            .map(|entry| entry.display_name.clone())
            .collect()
    })
}

#[gpui_kit::test]
fn editing_display_name_saves_it_and_relabels_the_row(cx: &mut TestAppContext) {
    let (dir, window, page) = two_cluster_setup("rename", cx);
    type_into_name(window, &page, "Alpha", cx);
    assert_eq!(stored_names(cx), [Some("Alpha".to_owned())]);
    render(window, cx);
    let labels = page.read_with(cx, |page, cx| {
        page.rows(cx)
            .into_iter()
            .map(|row| row.label)
            .collect::<Vec<_>>()
    });
    assert!(labels.contains(&"Alpha".to_owned()), "{labels:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn duplicate_display_name_is_not_saved(cx: &mut TestAppContext) {
    let (dir, window, page) = two_cluster_setup("duplicate", cx);
    type_into_name(window, &page, "prod-b", cx);
    assert!(stored_names(cx).is_empty());
    let message = page.read_with(cx, |page, _| {
        page.form
            .as_ref()
            .and_then(|form| form.name_error.as_ref())
            .map(|error| error.0.to_string())
    });
    assert_eq!(
        message.as_deref(),
        Some("Another cluster is already shown as 'prod-b'.")
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn invalid_namespace_is_not_saved(cx: &mut TestAppContext) {
    let (dir, window, page) = two_cluster_setup("namespace", cx);
    type_into_namespace(window, &page, "Bad_Name", cx);
    let saved = cx.read(|cx| AppSettings::get(cx).registry.clusters.clone());
    assert!(saved.is_empty(), "{saved:?}");
    let has_error = page.read_with(cx, |page, _| {
        page.form
            .as_ref()
            .is_some_and(|form| form.namespace_error.is_some())
    });
    assert!(has_error);
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn valid_namespace_is_saved(cx: &mut TestAppContext) {
    let (dir, window, page) = two_cluster_setup("namespace-ok", cx);
    type_into_namespace(window, &page, "kube-system", cx);
    let saved = cx.read(|cx| {
        AppSettings::get(cx)
            .registry
            .clusters
            .first()
            .and_then(|entry| entry.default_namespace.clone())
    });
    assert_eq!(saved.as_deref(), Some("kube-system"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn reset_drops_the_entry_and_clears_the_form(cx: &mut TestAppContext) {
    let (dir, window, page) = two_cluster_setup("reset", cx);
    type_into_name(window, &page, "Alpha", cx);
    page.update(cx, |page, cx| page.reset_selected(cx));
    render(window, cx);
    assert!(cx.read(|cx| AppSettings::get(cx).registry.clusters.is_empty()));
    let text = page.read_with(cx, |page, cx| {
        page.form
            .as_ref()
            .map(|form| form.name.read(cx).value().to_string())
    });
    assert_eq!(text.as_deref(), Some(""));
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn description_counts_clusters_and_files(cx: &mut TestAppContext) {
    let (dir, _window, page) = two_cluster_setup("description", cx);
    let text = page.read_with(cx, |page, cx| page.description(cx));
    assert_eq!(text, "2 clusters · 1 kubeconfig file");
    let _ = std::fs::remove_dir_all(&dir);
}

// ---- Add cluster: import and paste ----

fn press_confirm(window: AnyWindowHandle, cx: &mut TestAppContext) {
    cx.update_window(window, |_, window, cx| {
        window.dispatch_action(Box::new(Confirm { secondary: false }), cx);
    })
    .expect("the window is open");
    cx.run_until_parked();
}

fn press_cancel(window: AnyWindowHandle, cx: &mut TestAppContext) {
    cx.update_window(window, |_, window, cx| {
        window.dispatch_action(Box::new(Cancel), cx);
    })
    .expect("the window is open");
    cx.run_until_parked();
}

fn read_clipboard(window: AnyWindowHandle, page: &Entity<ClustersPage>, cx: &mut TestAppContext) {
    cx.update_window(window, |_, window, cx| {
        page.update(cx, |page, cx| page.read_clipboard(window, cx));
    })
    .expect("the window is open");
    cx.run_until_parked();
    render(window, cx);
}

fn paste_text_is_set(page: &Entity<ClustersPage>, cx: &TestAppContext) -> bool {
    page.read_with(cx, |page, _| page.paste_text.is_some())
}

fn paste_setup(
    name: &str,
    cx: &mut TestAppContext,
) -> (PathBuf, AnyWindowHandle, Entity<ClustersPage>) {
    let dir = temp_dir(name);
    install(Some(&dir), &[], cx);
    let (window, page) = open_page(cx);
    render(window, cx);
    cx.write_to_clipboard(ClipboardItem::new_string(kubeconfig_text(&["pasted-ctx"])));
    (dir, window, page)
}

fn has_dialog(window: AnyWindowHandle, cx: &mut TestAppContext) -> bool {
    cx.update_window(window, |_, window, cx| window.has_active_dialog(cx))
        .expect("the window is open")
}

#[gpui_kit::test]
fn paste_cancel_drops_text(cx: &mut TestAppContext) {
    let (dir, window, page) = paste_setup("cancel", cx);
    read_clipboard(window, &page, cx);
    assert!(paste_text_is_set(&page, cx));
    press_cancel(window, cx);
    assert!(!paste_text_is_set(&page, cx));
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn paste_escape_drops_text(cx: &mut TestAppContext) {
    let (dir, window, page) = paste_setup("escape", cx);
    read_clipboard(window, &page, cx);
    assert!(paste_text_is_set(&page, cx));
    cx.update_window(window, |_, window, cx| window.press("escape", cx))
        .expect("the window is open");
    cx.run_until_parked();
    // Esc ran `on_close`, which is what drops the text.
    assert!(!paste_text_is_set(&page, cx));
    assert!(!has_dialog(window, cx));
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn paste_preview_error_drops_text(cx: &mut TestAppContext) {
    let (dir, window, page) = paste_setup("error", cx);
    cx.write_to_clipboard(ClipboardItem::new_string(format!(
        "token: {TOKEN}\n  : [broken"
    )));
    read_clipboard(window, &page, cx);
    assert!(!paste_text_is_set(&page, cx));
    assert!(has_dialog(window, cx), "the error is shown in a dialog");
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn paste_without_clipboard_text_shows_an_error(cx: &mut TestAppContext) {
    let (dir, window, page) = paste_setup("no-text", cx);
    cx.write_to_clipboard(ClipboardItem::new_string(String::new()));
    read_clipboard(window, &page, cx);
    assert!(!paste_text_is_set(&page, cx));
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn paste_imports_and_clears_matching_clipboard(cx: &mut TestAppContext) {
    let (dir, window, page) = paste_setup("import", cx);
    read_clipboard(window, &page, cx);
    press_confirm(window, cx);
    // The text moved out of the page into the catalog's write.
    assert!(!paste_text_is_set(&page, cx));
    let files = cx.read(|cx| AppSettings::get(cx).registry.kubeconfigs.clone());
    assert_eq!(files.len(), 1, "{files:?}");
    assert!(files[0].starts_with(dir.join("kubeconfigs")), "{files:?}");
    let on_clipboard = cx.read_from_clipboard().and_then(|item| item.text());
    assert!(
        on_clipboard.is_none_or(|text| !text.contains(TOKEN)),
        "the pasted text must leave the clipboard"
    );
    // The new file's first row becomes the selection.
    render(window, cx);
    assert_eq!(selected(&page, cx).as_deref(), Some("pasted-ctx"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn paste_survives_settings_window_close(cx: &mut TestAppContext) {
    let (dir, window, page) = paste_setup("close-while-saving", cx);
    read_clipboard(window, &page, cx);
    cx.update_window(window, |_, window, cx| {
        window.dispatch_action(Box::new(Confirm { secondary: false }), cx);
    })
    .expect("the window is open");
    // No parking in between: the write is still in flight when the window goes.
    cx.update_window(window, |_, window, _| window.remove_window())
        .expect("the window is open");
    cx.run_until_parked();
    let files = cx.read(|cx| AppSettings::get(cx).registry.kubeconfigs.clone());
    assert_eq!(files.len(), 1, "{files:?}");
    assert!(files[0].exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn paste_is_blocked_without_a_config_dir(cx: &mut TestAppContext) {
    install(None, &[], cx);
    let reason = cx.read(ClustersPage::paste_blocked_reason);
    assert_eq!(
        reason,
        Some("Settings are not saved this session, so a pasted kubeconfig cannot be stored.")
    );
    let dir = temp_dir("blocked");
    install(Some(&dir), &[], cx);
    assert_eq!(cx.read(ClustersPage::paste_blocked_reason), None);
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn import_file_adds_only_the_path(cx: &mut TestAppContext) {
    let dir = temp_dir("import-file");
    let config = dir.join("config");
    install(Some(&config), &[], cx);
    let (window, page) = open_page(cx);
    render(window, cx);
    let file = write_kubeconfig(&dir, "extra.yaml", &["imported-ctx"]);
    cx.update_window(window, |_, window, cx| {
        page.update(cx, |page, cx| page.preview_file(file.clone(), window, cx));
    })
    .expect("the window is open");
    cx.run_until_parked();
    render(window, cx);
    assert!(has_dialog(window, cx));
    press_confirm(window, cx);
    assert_eq!(
        cx.read(|cx| AppSettings::get(cx).registry.kubeconfigs.clone()),
        std::slice::from_ref(&file)
    );
    // The file is not copied: the config folder has no kubeconfigs folder.
    assert!(!config.join("kubeconfigs").exists());
    render(window, cx);
    assert_eq!(selected(&page, cx).as_deref(), Some("imported-ctx"));
    cx.run_until_parked();
    let saved = std::fs::read_to_string(config.join("settings.json")).expect("settings saved");
    assert!(!saved.contains(TOKEN), "{saved}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn import_of_an_added_file_is_rejected(cx: &mut TestAppContext) {
    let dir = temp_dir("import-twice");
    let file = write_kubeconfig(&dir, "extra.yaml", &["imported-ctx"]);
    install(None, &[], cx);
    cx.update(|cx| {
        AppSettings::update(cx, |settings| {
            settings.registry.kubeconfigs.push(file.clone())
        });
    });
    let (window, page) = open_page(cx);
    render(window, cx);
    cx.update_window(window, |_, window, cx| {
        page.update(cx, |page, cx| page.preview_file(file.clone(), window, cx));
    })
    .expect("the window is open");
    cx.run_until_parked();
    render(window, cx);
    // The only dialog is the error; confirming it adds nothing.
    assert!(has_dialog(window, cx));
    press_confirm(window, cx);
    assert_eq!(
        cx.read(|cx| AppSettings::get(cx).registry.kubeconfigs.len()),
        1
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn remove_dialog_removes_a_registry_file(cx: &mut TestAppContext) {
    let dir = temp_dir("remove-dialog");
    let extra = write_kubeconfig(&dir, "extra.yaml", &["extra-ctx"]);
    install(None, &[], cx);
    cx.update(|cx| {
        AppSettings::update(cx, |settings| {
            settings.registry.kubeconfigs.push(extra.clone())
        });
    });
    cx.run_until_parked();
    let (window, page) = open_page(cx);
    render(window, cx);
    let row = page.read_with(cx, |page, cx| page.rows(cx).remove(0));
    assert_eq!(row.origin, RowOrigin::Registry);
    cx.update_window(window, |_, window, cx| {
        page.update(cx, |page, cx| page.confirm_remove(&row, window, cx));
    })
    .expect("the window is open");
    render(window, cx);
    assert!(has_dialog(window, cx));
    press_confirm(window, cx);
    assert!(cx.read(|cx| AppSettings::get(cx).registry.kubeconfigs.is_empty()));
    assert!(extra.exists(), "the user's own file stays");
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn catalog_entity_is_shared_by_the_global(cx: &mut TestAppContext) {
    install(None, &[], cx);
    let first: Entity<ClusterCatalog> = cx.read(CatalogHandle::of);
    let second: Entity<ClusterCatalog> = cx.read(CatalogHandle::of);
    assert_eq!(first.entity_id(), second.entity_id());
}

fn start_paste_dialog(
    window: AnyWindowHandle,
    page: &Entity<ClustersPage>,
    cx: &mut TestAppContext,
) {
    cx.update_window(window, |_, window, cx| {
        page.update(cx, |page, cx| page.start_paste(window, cx));
    })
    .expect("the window is open");
    render(window, cx);
}

#[gpui_kit::test]
fn read_clipboard_error_stays_open_after_the_paste_dialog_closes(cx: &mut TestAppContext) {
    let (dir, window, page) = paste_setup("empty-clipboard", cx);
    cx.write_to_clipboard(ClipboardItem::new_string(String::new()));
    start_paste_dialog(window, &page, cx);
    assert!(has_dialog(window, cx));
    press_confirm(window, cx);
    render(window, cx);
    // The paste dialog closed and the error dialog is the one left open; closing the paste
    // dialog must not pop it.
    assert!(has_dialog(window, cx));
    press_confirm(window, cx);
    assert!(!has_dialog(window, cx));
    assert!(!paste_text_is_set(&page, cx));
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn read_clipboard_without_a_config_dir_says_why(cx: &mut TestAppContext) {
    install(None, &[], cx);
    let (window, page) = open_page(cx);
    render(window, cx);
    cx.write_to_clipboard(ClipboardItem::new_string(kubeconfig_text(&["pasted-ctx"])));
    read_clipboard(window, &page, cx);
    assert!(!paste_text_is_set(&page, cx));
    assert!(has_dialog(window, cx));
}

#[gpui_kit::test]
fn paste_is_blocked_while_a_paste_is_saving(cx: &mut TestAppContext) {
    let dir = temp_dir("saving");
    install(Some(&dir), &[], cx);
    assert_eq!(cx.read(ClustersPage::paste_blocked_reason), None);
    let catalog = cx.read(CatalogHandle::of);
    let mark = crate::secret_clipboard::ClipboardMark::of("x");
    catalog.update(cx, |catalog, cx| {
        catalog.add_pasted(kubeconfig_text(&["c"]), None, mark, cx);
    });
    // Not parked: the write has not finished.
    assert_eq!(
        cx.read(ClustersPage::paste_blocked_reason),
        Some("Saving the pasted kubeconfig…")
    );
    cx.run_until_parked();
    assert_eq!(cx.read(ClustersPage::paste_blocked_reason), None);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn confirm_labels_name_the_tiers() {
    assert_eq!(
        confirm_label(ConfirmMode::TypeName),
        "Typing the cluster name"
    );
    assert_eq!(confirm_label(ConfirmMode::Click), "Clicking Confirm");
}
