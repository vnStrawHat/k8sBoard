//! The Clusters page in a headless window. Every run uses a config dir under the temp dir and
//! the test platform's clipboard: nothing here touches the real settings or clipboard.

use std::path::{Path, PathBuf};

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::dialog::{Cancel, Confirm};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, ClipboardItem, TestAppContext, WindowOptions};

use super::*;
use crate::cluster_catalog::CatalogHandle;
use crate::cluster_form::RowOrigin;
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
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| ClustersPage::new(catalog, window, cx))
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
    click_ok(window, cx);
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

/// A click on the dialog's own OK button, which must be on screen for a user to reach it.
fn click_ok(window: AnyWindowHandle, cx: &mut TestAppContext) {
    cx.update_window(window, |_, window, cx| {
        assert!(
            window.try_find("ok").is_some(),
            "the dialog shows no OK button"
        );
        window.click("ok", cx);
    })
    .expect("the window is open");
    cx.run_until_parked();
    render(window, cx);
}

fn context_names(page: &Entity<ClustersPage>, cx: &TestAppContext) -> Vec<String> {
    page.read_with(cx, |page, cx| {
        page.rows(cx)
            .into_iter()
            .map(|row| row.cluster.context)
            .collect()
    })
}

#[gpui_kit::test]
fn paste_through_both_dialogs_lists_the_new_cluster(cx: &mut TestAppContext) {
    let (dir, window, page) = paste_setup("both-dialogs", cx);
    start_paste_dialog(window, &page, cx);
    click_ok(window, cx);
    render(window, cx);
    assert!(has_dialog(window, cx), "the preview is open");
    assert!(paste_text_is_set(&page, cx));
    click_ok(window, cx);
    render(window, cx);
    assert_eq!(context_names(&page, cx), ["pasted-ctx"]);
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

#[gpui_kit::test]
fn safety_toggle_stores_allow_node_shell(cx: &mut TestAppContext) {
    let (dir, window, page) = two_cluster_setup("node-shell", cx);
    let target = page
        .read_with(cx, |page, _| page.selected.clone())
        .expect("a selected cluster");
    let allowed = |cx: &mut TestAppContext| {
        page.read_with(cx, |page, cx| {
            page.rows(cx)
                .into_iter()
                .find(|row| row.cluster == target)
                .map(|row| row.profile.allow_node_shell)
        })
    };
    // prod-a is a Production guess: off until the switch says otherwise.
    assert_eq!(allowed(cx), Some(false));
    cx.update(|cx| set_allow_node_shell(&target, Some(true), cx));
    render(window, cx);
    assert_eq!(allowed(cx), Some(true));
    let stored = cx.read(|cx| AppSettings::get(cx).registry.clusters.clone());
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].allow_node_shell, Some(true));
    // Back to the environment's answer: the entry is dropped with its last override.
    cx.update(|cx| set_allow_node_shell(&target, None, cx));
    assert_eq!(allowed(cx), Some(false));
    assert!(cx.read(|cx| AppSettings::get(cx).registry.clusters.is_empty()));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_node_shell_hint_is_the_wireframe_text() {
    assert_eq!(
        NODE_SHELL_HINT,
        "Creates a privileged debug pod on the node. Off by default for production."
    );
}

// ---- Spec 0043 step 3: search, order ----

fn row_contexts(page: &Entity<ClustersPage>, cx: &TestAppContext) -> Vec<String> {
    page.read_with(cx, |page, cx| {
        page.rows(cx)
            .into_iter()
            .map(|row| row.cluster.context)
            .collect()
    })
}

/// Sets the search text. The box lives in the Settings header, which this window does not draw,
/// so the text is set on the state instead of typed.
fn type_into_search(
    window: AnyWindowHandle,
    page: &Entity<ClustersPage>,
    text: &str,
    cx: &mut TestAppContext,
) {
    cx.update_window(window, |_, window, cx| {
        let search = page.read(cx).search_input();
        search.update(cx, |state, cx| state.set_value(text.to_owned(), window, cx));
    })
    .expect("the window is open");
    cx.run_until_parked();
}

#[gpui_kit::test]
fn alt_arrows_move_the_selected_cluster(cx: &mut TestAppContext) {
    let (dir, window, page) = two_cluster_setup("alt-arrows", cx);
    assert_eq!(row_contexts(&page, cx), ["prod-a", "prod-b"]);
    page.update(cx, |page, cx| page.step_selected(MoveStep::Down, cx));
    render(window, cx);
    assert_eq!(row_contexts(&page, cx), ["prod-b", "prod-a"]);
    assert_eq!(selected(&page, cx).as_deref(), Some("prod-a"));
    page.update(cx, |page, cx| page.step_selected(MoveStep::Up, cx));
    assert_eq!(row_contexts(&page, cx), ["prod-a", "prod-b"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn the_order_is_fixed_while_searching(cx: &mut TestAppContext) {
    let (dir, window, page) = two_cluster_setup("order-search", cx);
    type_into_search(window, &page, "prod", cx);
    render(window, cx);
    assert!(page.read_with(cx, |page, cx| page.is_searching(cx)));
    page.update(cx, |page, cx| page.step_selected(MoveStep::Down, cx));
    assert!(cx.read(|cx| AppSettings::get(cx).registry.clusters.is_empty()));
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn search_leaves_the_selection_and_its_form(cx: &mut TestAppContext) {
    let (dir, window, page) = two_cluster_setup("search-selection", cx);
    assert_eq!(selected(&page, cx).as_deref(), Some("prod-a"));
    // Only prod-b matches, so the selected row is filtered out of the list.
    type_into_search(window, &page, "prod-b", cx);
    render(window, cx);
    assert_eq!(selected(&page, cx).as_deref(), Some("prod-a"));
    assert!(page.read_with(cx, |page, _| page.form.is_some()));
    // The header count stays the full count.
    assert_eq!(
        page.read_with(cx, |page, cx| page.description(cx)),
        "2 clusters · 1 kubeconfig file"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn dropping_a_row_inside_its_group_reorders_and_across_groups_does_nothing(
    cx: &mut TestAppContext,
) {
    let (dir, window, page) = two_cluster_setup("drop", cx);
    let rows = page.read_with(cx, |page, cx| page.rows(cx));
    let (first, second) = (rows[0].cluster.clone(), rows[1].cluster.clone());
    let dragged = DraggedCluster {
        cluster: first.clone(),
        group_title: "Production",
        label: "prod-a".into(),
    };
    page.update(cx, |page, cx| page.drop_cluster(&dragged, &second, cx));
    render(window, cx);
    assert_eq!(row_contexts(&page, cx), ["prod-b", "prod-a"]);
    // A drag that started in another group does not move anything here.
    let before = cx.read(|cx| AppSettings::get(cx).registry.clone());
    let stranger = DraggedCluster {
        cluster: first,
        group_title: "Staging",
        label: "prod-a".into(),
    };
    page.update(cx, |page, cx| page.drop_cluster(&stranger, &second, cx));
    assert_eq!(cx.read(|cx| AppSettings::get(cx).registry.clone()), before);
    let _ = std::fs::remove_dir_all(&dir);
}

// ---- Spec 0043 step 4: the Proxy control ----

fn stored_proxy_of(cx: &TestAppContext) -> Option<ClusterProxy> {
    cx.read(|cx| {
        AppSettings::get(cx)
            .registry
            .clusters
            .first()
            .and_then(|entry| entry.proxy.clone())
    })
}

/// Types into the proxy input, which only draws once Custom URL is picked.
fn type_into_proxy(
    window: AnyWindowHandle,
    page: &Entity<ClustersPage>,
    text: &str,
    cx: &mut TestAppContext,
) {
    cx.update_window(window, |_, window, cx| {
        let input = page
            .read(cx)
            .form
            .as_ref()
            .expect("a form")
            .proxy_url
            .clone();
        input.update(cx, |state, cx| state.focus(window, cx));
        window.input(text, cx);
    })
    .expect("the window is open");
    cx.run_until_parked();
}

fn emit_on_proxy_input(page: &Entity<ClustersPage>, event: InputEvent, cx: &mut TestAppContext) {
    let input = page.read_with(cx, |page, _| {
        page.form.as_ref().expect("a form").proxy_url.clone()
    });
    input.update(cx, |_, cx| cx.emit(event));
    cx.run_until_parked();
}

fn proxy_error_of(page: &Entity<ClustersPage>, cx: &TestAppContext) -> Option<String> {
    page.read_with(cx, |page, _| {
        page.form
            .as_ref()
            .and_then(|form| form.proxy_error.as_ref())
            .map(|error| error.0.to_string())
    })
}

#[gpui_kit::test]
fn proxy_input_commits_on_enter_or_blur_only(cx: &mut TestAppContext) {
    let (dir, window, page) = two_cluster_setup("proxy-commit", cx);
    page.update(cx, |page, cx| page.pick_proxy_mode(ProxyMode::Custom, cx));
    render(window, cx);
    type_into_proxy(window, &page, "http://p:3128", cx);
    // Typing stores nothing.
    assert_eq!(stored_proxy_of(cx), None);
    let text = page.read_with(cx, |page, cx| {
        page.form
            .as_ref()
            .expect("a form")
            .proxy_url
            .read(cx)
            .value()
    });
    assert_eq!(text, "http://p:3128");
    emit_on_proxy_input(
        &page,
        InputEvent::PressEnter {
            secondary: false,
            shift: false,
        },
        cx,
    );
    assert_eq!(
        stored_proxy_of(cx),
        Some(ClusterProxy::Url("http://p:3128".to_owned()))
    );
    // Blur commits the next edit the same way.
    cx.update_window(window, |_, window, cx| {
        let input = page
            .read(cx)
            .form
            .as_ref()
            .expect("a form")
            .proxy_url
            .clone();
        input.update(cx, |state, cx| {
            state.set_value("socks5://q:1080".to_owned(), window, cx);
        });
    })
    .expect("the window is open");
    assert_eq!(
        stored_proxy_of(cx),
        Some(ClusterProxy::Url("http://p:3128".to_owned()))
    );
    emit_on_proxy_input(&page, InputEvent::Blur, cx);
    assert_eq!(
        stored_proxy_of(cx),
        Some(ClusterProxy::Url("socks5://q:1080".to_owned()))
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn a_proxy_with_credentials_is_refused_with_the_message(cx: &mut TestAppContext) {
    let (dir, window, page) = two_cluster_setup("proxy-credentials", cx);
    page.update(cx, |page, cx| page.pick_proxy_mode(ProxyMode::Custom, cx));
    render(window, cx);
    type_into_proxy(window, &page, "http://user:pw@p:3128", cx);
    emit_on_proxy_input(&page, InputEvent::Blur, cx);
    assert_eq!(stored_proxy_of(cx), None);
    let message = proxy_error_of(&page, cx).expect("a message");
    assert!(
        message.starts_with("Leave out the user name and password"),
        "{message}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn picking_none_or_the_kubeconfig_stores_at_once(cx: &mut TestAppContext) {
    let (dir, window, page) = two_cluster_setup("proxy-pick", cx);
    page.update(cx, |page, cx| page.pick_proxy_mode(ProxyMode::Direct, cx));
    render(window, cx);
    assert_eq!(stored_proxy_of(cx), Some(ClusterProxy::Direct));
    // Custom stores nothing by itself: the stored choice still applies.
    page.update(cx, |page, cx| page.pick_proxy_mode(ProxyMode::Custom, cx));
    assert_eq!(stored_proxy_of(cx), Some(ClusterProxy::Direct));
    page.update(cx, |page, cx| {
        page.pick_proxy_mode(ProxyMode::FromKubeconfig, cx);
    });
    assert!(cx.read(|cx| AppSettings::get(cx).registry.clusters.is_empty()));
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn proxy_input_never_shows_an_unparsable_value(cx: &mut TestAppContext) {
    let (dir, window, page) = two_cluster_setup("proxy-prefill", cx);
    let target = page
        .read_with(cx, |page, _| page.selected.clone())
        .expect("a selected cluster");
    cx.update(|cx| {
        AppSettings::update(cx, |settings| {
            edit_entry(&mut settings.registry, &target, |entry| {
                entry.proxy = Some(ClusterProxy::Url("http://u:p@x".to_owned()));
            });
        });
    });
    render(window, cx);
    page.update(cx, |page, _| page.forget_form_and_test());
    render(window, cx);
    let shown = page.read_with(cx, |page, cx| {
        page.form
            .as_ref()
            .expect("a form")
            .proxy_url
            .read(cx)
            .value()
    });
    assert_eq!(shown, "");
    let _ = std::fs::remove_dir_all(&dir);
}

// ---- The Metrics section ----

fn metrics_fields() -> MetricsSourceFields {
    MetricsSourceFields {
        namespace: "monitoring".to_owned(),
        service: "vmselect".to_owned(),
        port: "8481".to_owned(),
        scheme: cluster::MetricsScheme::Http,
        prefix: "/select/0/prometheus".to_owned(),
    }
}

#[test]
fn source_button_names_metrics_server_or_the_saved_service() {
    assert_eq!(metrics_source_label(None), "metrics-server only");
    assert_eq!(
        metrics_source_label(Some(&StoredMetrics::Fields(metrics_fields()))),
        "Prometheus-compatible · monitoring/vmselect:8481"
    );
}

#[gpui_kit::test]
fn form_dropdown_clears_the_source(cx: &mut TestAppContext) {
    let (dir, _window, _page) = two_cluster_setup("metrics-clear", cx);
    let cluster = ClusterRef {
        kubeconfig: dir.join("chain.yaml"),
        context: "prod-a".to_owned(),
    };
    let stored = |cx: &mut TestAppContext| {
        cx.read(|cx| {
            AppSettings::get(cx)
                .registry
                .clusters
                .iter()
                .find(|entry| entry.cluster == cluster)
                .and_then(|entry| entry.metrics.clone())
        })
    };
    cx.update(|cx| set_metrics_source(&cluster, Some(metrics_fields()), cx));
    assert_eq!(stored(cx), Some(StoredMetrics::Fields(metrics_fields())));
    cx.update(|cx| set_metrics_source(&cluster, None, cx));
    assert_eq!(stored(cx), None);
    assert!(cx.read(|cx| AppSettings::get(cx).registry.clusters.is_empty()));
    let _ = std::fs::remove_dir_all(&dir);
}
