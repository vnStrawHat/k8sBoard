//! What a file of a watched folder may and may not start (spec 0043). The folder is a place anyone
//! can drop a kubeconfig in, so a session starts from it only as the exact cluster the user picked
//! last time; otherwise the shell asks for a pick. The fixture server is a closed local port, so
//! nothing leaves the machine, and no real watcher runs: tests send folder events themselves.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use gpui_kit::base::Root;
use gpui_kit::{
    Bounds, Point, TestAppContext, WindowBounds, WindowHandle, WindowOptions, px, size,
};

use super::app_shell_tests::render;
use super::*;
use crate::cluster_catalog::{CatalogHandle, FolderEvent};
use crate::cluster_registry::{ClusterRef, ClusterRegistry};
use crate::cluster_runtime::ClusterRuntime;
use crate::kubeconfig_folder::{FileStamp, FolderFile, RESCAN_DEBOUNCE};
use crate::launch_options::{LaunchRequest, kubeconfig_chain, parse_launch_options};
use crate::settings::Settings;
use crate::settings_store::{LoadedSettings, WriteMode};

static SERIAL: AtomicU64 = AtomicU64::new(0);

/// A folder under the temp dir, unique per test.
fn watched_folder(name: &str) -> PathBuf {
    let serial = SERIAL.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "k8sboard-0043-start-{name}-{}-{serial}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create the folder");
    dir
}

/// A kubeconfig whose only context is `context`, which is also its `current-context`.
fn drop_file(folder: &Path, name: &str, context: &str) -> PathBuf {
    let text = format!(
        "apiVersion: v1\nkind: Config\ncurrent-context: {context}\nclusters:\n  - name: c\n    cluster: {{ server: \"https://127.0.0.1:1\" }}\ncontexts:\n  - name: {context}\n    context: {{ cluster: c }}\n"
    );
    let path = folder.join(name);
    std::fs::write(&path, text).expect("write the file");
    path
}

/// The shell over an empty chain, with `folder` watched and `last_used` saved.
type Opened = (
    tokio::runtime::Runtime,
    WindowHandle<Root>,
    Entity<AppShell>,
);

fn open_shell(
    folder: &Path,
    last_used: Option<ClusterRef>,
    extra: &[&str],
    cx: &mut TestAppContext,
) -> Opened {
    open_shell_over(folder, &[], last_used, extra, cx)
}

/// `chain` is the `KUBECONFIG` list. There is never a `--kubeconfig`: with explicit files only a
/// `last_used` among them counts (0025), and the watched folder would never be picked.
fn open_shell_over(
    folder: &Path,
    chain: &[PathBuf],
    last_used: Option<ClusterRef>,
    extra: &[&str],
    cx: &mut TestAppContext,
) -> Opened {
    let args = extra.iter().map(|arg| (*arg).to_owned());
    let Ok(LaunchRequest::Run(options)) = parse_launch_options(args) else {
        panic!("the launch flags are valid");
    };
    // A session connects on a real tokio runtime, which the caller keeps alive for the test.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .expect("a tokio runtime");
    let handle = runtime.handle().clone();
    cx.executor().allow_parking();
    cx.update(|cx| cx.set_global(ClusterRuntime::new(handle)));
    let (window, shell) = cx.update(|cx| {
        gpui_kit::init(cx);
        crate::keymap::bind_keys(cx);
        crate::cluster_switcher::bind_keys(cx);
        AppSettings::install(
            LoadedSettings {
                settings: Settings {
                    registry: ClusterRegistry {
                        kubeconfig_folders: vec![folder.to_path_buf()],
                        last_used,
                        ..ClusterRegistry::default()
                    },
                    ..Settings::default()
                },
                writes: WriteMode::Disabled,
                notice: None,
            },
            cx,
        );
        let env = (!chain.is_empty()).then(|| std::env::join_paths(chain).expect("paths join"));
        let chain = kubeconfig_chain(options.kubeconfig.clone(), env, None);
        CatalogHandle::install(chain, cx);
        let bounds = Bounds {
            origin: Point::default(),
            size: size(px(1320.), px(900.)),
        };
        let (window, shell) = gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            cx,
            |window, cx| cx.new(|cx| AppShell::new(*options, window, cx)),
        )
        .expect("open the test window");
        (window.downcast::<Root>().expect("a Root window"), shell)
    });
    (runtime, window, shell)
}

fn settle(window: WindowHandle<Root>, cx: &mut TestAppContext) {
    cx.run_until_parked();
    render(window, cx);
    cx.run_until_parked();
}

/// Tells the catalog the folder changed and lets the debounce pass.
fn folder_changed(folder: &Path, window: WindowHandle<Root>, cx: &mut TestAppContext) {
    let sender = cx
        .read(CatalogHandle::of)
        .read_with(cx, |catalog, _| catalog.folder_events_for_test());
    sender
        .unbounded_send(FolderEvent {
            folder: folder.to_path_buf(),
        })
        .expect("the catalog listens");
    cx.run_until_parked();
    cx.executor()
        .advance_clock(RESCAN_DEBOUNCE + Duration::from_millis(1));
    settle(window, cx);
}

fn active_context(shell: &Entity<AppShell>, cx: &TestAppContext) -> Option<String> {
    shell.read_with(cx, |shell, _| {
        shell.active.as_ref().map(|summary| summary.name.clone())
    })
}

fn needs_pick(shell: &Entity<AppShell>, cx: &TestAppContext) -> bool {
    shell.read_with(cx, |shell, _| shell.needs_pick)
}

fn switcher_is_open(shell: &Entity<AppShell>, cx: &TestAppContext) -> bool {
    shell.read_with(cx, |shell, _| shell.switcher.is_open())
}

/// What the last scan would record of `path`: the stamp `last_used` keeps.
fn stamp_of(path: &Path) -> FileStamp {
    let metadata = std::fs::metadata(path).expect("the file exists");
    FileStamp::of(&FolderFile {
        path: path.to_path_buf(),
        len: metadata.len(),
        modified: metadata.modified().ok(),
    })
}

fn save_stamp(stamp: Option<FileStamp>, cx: &mut TestAppContext) {
    cx.update(|cx| AppSettings::update(cx, |settings| settings.registry.last_used_stamp = stamp));
}

fn cluster_in(file: &Path, context: &str) -> ClusterRef {
    ClusterRef {
        kubeconfig: file.to_path_buf(),
        context: context.to_owned(),
    }
}

#[gpui_kit::test]
fn folder_file_is_never_the_fallback_start(cx: &mut TestAppContext) {
    let folder = watched_folder("fallback");
    drop_file(&folder, "dropped.yaml", "dropped");
    let (_runtime, window, shell) = open_shell(&folder, None, &[], cx);
    settle(window, cx);
    // Its `current-context` and its being the only file start nothing; the shell asks instead.
    assert_eq!(active_context(&shell, cx), None);
    assert!(needs_pick(&shell, cx));
    assert!(switcher_is_open(&shell, cx));
    assert!(
        shell
            .read_with(cx, |shell, _| shell.context_error.clone())
            .is_none()
    );
    let _ = std::fs::remove_dir_all(&folder);
}

#[gpui_kit::test]
fn a_context_named_on_the_command_line_does_not_reach_a_folder_file(cx: &mut TestAppContext) {
    let folder = watched_folder("requested");
    let file = drop_file(&folder, "dropped.yaml", "dropped");
    // Even when the saved choice is that very cluster: `--context` decides, and it finds nothing
    // among the files that may start on their own.
    let (_runtime, window, shell) = open_shell(
        &folder,
        Some(cluster_in(&file, "dropped")),
        &["--context", "dropped"],
        cx,
    );
    settle(window, cx);
    assert_eq!(active_context(&shell, cx), None);
    assert!(needs_pick(&shell, cx));
    let _ = std::fs::remove_dir_all(&folder);
}

#[gpui_kit::test]
fn a_file_that_appears_later_does_not_start_either(cx: &mut TestAppContext) {
    let folder = watched_folder("appears");
    let (_runtime, window, shell) = open_shell(&folder, None, &[], cx);
    settle(window, cx);
    assert_eq!(active_context(&shell, cx), None);
    drop_file(&folder, "late.yaml", "late");
    folder_changed(&folder, window, cx);
    assert_eq!(active_context(&shell, cx), None);
    assert!(needs_pick(&shell, cx));
    let _ = std::fs::remove_dir_all(&folder);
}

#[gpui_kit::test]
fn folder_file_starts_when_it_is_last_used(cx: &mut TestAppContext) {
    let folder = watched_folder("last-used");
    let file = drop_file(&folder, "dropped.yaml", "dropped");
    drop_file(&folder, "other.yaml", "other");
    let (_runtime, window, shell) =
        open_shell(&folder, Some(cluster_in(&file, "dropped")), &[], cx);
    save_stamp(Some(stamp_of(&file)), cx);
    settle(window, cx);
    assert_eq!(active_context(&shell, cx).as_deref(), Some("dropped"));
    assert!(!needs_pick(&shell, cx));
    let _ = std::fs::remove_dir_all(&folder);
}

#[gpui_kit::test]
fn a_folder_file_that_changed_since_the_pick_asks_instead_of_starting(cx: &mut TestAppContext) {
    let folder = watched_folder("changed");
    let file = drop_file(&folder, "dropped.yaml", "dropped");
    // The stamp is of the file as it was when the user picked it.
    let picked = stamp_of(&file);
    drop_file(&folder, "dropped.yaml", "dropped-and-then-rewritten");
    let (_runtime, window, shell) = open_shell(
        &folder,
        Some(cluster_in(&file, "dropped-and-then-rewritten")),
        &[],
        cx,
    );
    save_stamp(Some(picked), cx);
    settle(window, cx);
    assert_eq!(active_context(&shell, cx), None);
    assert!(needs_pick(&shell, cx));
    assert!(switcher_is_open(&shell, cx));
    let _ = std::fs::remove_dir_all(&folder);
}

#[gpui_kit::test]
fn a_folder_file_without_a_saved_stamp_asks_instead_of_starting(cx: &mut TestAppContext) {
    let folder = watched_folder("no-stamp");
    let file = drop_file(&folder, "dropped.yaml", "dropped");
    let (_runtime, window, shell) =
        open_shell(&folder, Some(cluster_in(&file, "dropped")), &[], cx);
    settle(window, cx);
    assert_eq!(active_context(&shell, cx), None);
    assert!(needs_pick(&shell, cx));
    let _ = std::fs::remove_dir_all(&folder);
}

#[gpui_kit::test]
fn a_chain_file_does_not_start_before_the_picked_folder_file_has_loaded(cx: &mut TestAppContext) {
    let folder = watched_folder("chain-race");
    let chain_dir = watched_folder("chain-race-chain");
    let chain = drop_file(&chain_dir, "chain.yaml", "from-chain");
    let file = drop_file(&folder, "dropped.yaml", "dropped");
    let (_runtime, window, shell) = open_shell_over(
        &folder,
        std::slice::from_ref(&chain),
        Some(cluster_in(&file, "dropped")),
        &[],
        cx,
    );
    save_stamp(Some(stamp_of(&file)), cx);
    settle(window, cx);
    // The chain loads first and has a current-context; the picked file is still loading then,
    // and the start waits for it instead of taking the chain's cluster.
    assert_eq!(active_context(&shell, cx).as_deref(), Some("dropped"));
    let _ = std::fs::remove_dir_all(&chain_dir);
    let _ = std::fs::remove_dir_all(&folder);
}

#[gpui_kit::test]
fn the_picked_cluster_starts_when_its_file_appears_later(cx: &mut TestAppContext) {
    let folder = watched_folder("last-used-late");
    let file = folder.join("late.yaml");
    let (_runtime, window, shell) = open_shell(&folder, Some(cluster_in(&file, "late")), &[], cx);
    settle(window, cx);
    assert_eq!(active_context(&shell, cx), None);
    let file = drop_file(&folder, "late.yaml", "late");
    save_stamp(Some(stamp_of(&file)), cx);
    folder_changed(&folder, window, cx);
    assert_eq!(active_context(&shell, cx).as_deref(), Some("late"));
    let _ = std::fs::remove_dir_all(&folder);
}

#[gpui_kit::test]
fn the_pick_prompt_names_the_switcher(cx: &mut TestAppContext) {
    assert_eq!(
        super::workspace::NO_CLUSTER_SELECTED,
        "No cluster selected. Pick one in the switcher."
    );
    let folder = watched_folder("prompt");
    drop_file(&folder, "dropped.yaml", "dropped");
    let (_runtime, window, shell) = open_shell(&folder, None, &[], cx);
    settle(window, cx);
    // Picking a row from the switcher starts it, and the prompt goes.
    let target = cluster_in(&folder.join("dropped.yaml"), "dropped");
    shell.update(cx, |shell, cx| shell.switch_cluster(&target, cx));
    settle(window, cx);
    assert_eq!(active_context(&shell, cx).as_deref(), Some("dropped"));
    assert!(!needs_pick(&shell, cx));
    let _ = std::fs::remove_dir_all(&folder);
}
