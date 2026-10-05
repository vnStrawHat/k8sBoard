//! Add cluster: the dialogs that import a kubeconfig file by path or paste one from the
//! clipboard. A child module of `clusters_page.rs`, so it reaches the page's private state.
//!
//! The clipboard text is never rendered. It lives in a local, then in `paste_text`, then moves
//! into the catalog's write task.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use cluster::Kubeconfig;
use gpui_kit::component::dialog::DialogButtonProps;
use gpui_kit::component::{ActiveTheme as _, StyledExt as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, Context, IntoElement, ParentElement as _, PathPromptOptions, SharedString,
    Styled as _, Window, div, px,
};

use super::ClustersPage;
use crate::cluster_catalog::{CatalogHandle, PasteStatus};
use crate::environment::environment_badge;
use crate::kubeconfig_import::{
    ImportError, ImportPreview, ImportSource, NO_CONFIG_DIR_MESSAGE, check_clipboard_text,
    check_new_file, import_preview, pasted_file_path,
};
use crate::secret_clipboard::ClipboardMark;
use crate::settings::AppSettings;

const DIALOG_WIDTH: f32 = 560.;

impl ClustersPage {
    /// Why pasting is off, if it is: the file needs a config folder to live in.
    pub(crate) fn paste_blocked_reason(cx: &App) -> Option<&'static str> {
        if AppSettings::config_dir(cx).is_none() {
            return Some(NO_CONFIG_DIR_MESSAGE);
        }
        let is_saving = *CatalogHandle::of(cx).read(cx).paste_status() == PasteStatus::Saving;
        is_saving.then_some("Saving the pasted kubeconfig…")
    }

    /// `Ctrl O`: the file picker, then the preview of the picked file.
    pub(crate) fn import_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Import kubeconfig".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = picked.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let _ = this.update_in(cx, |page, window, cx| page.preview_file(path, window, cx));
        })
        .detach();
    }

    /// Checks `path`, loads it off the UI thread, and opens its preview.
    pub(crate) fn preview_file(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let registered = AppSettings::get(cx).registry.kubeconfigs.clone();
        let is_chain_source = self.catalog.read(cx).is_chain_source(&path);
        if let Err(error) = check_new_file(&path, &registered, is_chain_source) {
            show_import_error(&error, window, cx);
            return;
        }
        cx.spawn_in(window, async move |this, cx| {
            let load_path = path.clone();
            let loaded = cx
                .background_executor()
                .spawn(async move { Kubeconfig::load(std::slice::from_ref(&load_path)) })
                .await;
            let _ = this.update_in(cx, |page, window, cx| match loaded {
                Ok(loaded) => {
                    page.show_preview(
                        ImportSource::File(path),
                        loaded.kubeconfig,
                        None,
                        window,
                        cx,
                    );
                }
                Err(error) => show_import_error(&ImportError::Load(error), window, cx),
            });
        })
        .detach();
    }

    /// "Paste kubeconfig YAML…": says where the text comes from, then reads the clipboard.
    pub(crate) fn start_paste(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let page = cx.weak_entity();
        window.open_alert_dialog(cx, move |dialog, _, _| {
            let page = page.clone();
            dialog
                .title("Paste kubeconfig YAML")
                .width(px(DIALOG_WIDTH))
                .description(
                    "k8sBoard reads the kubeconfig from the clipboard. Its content is never shown.",
                )
                .button_props(
                    DialogButtonProps::default()
                        .ok_text("Read clipboard")
                        .show_cancel(true),
                )
                .on_ok(move |_, window, cx| {
                    // After the close: the kit pops the last dialog when it closes this one, which
                    // would be the error dialog of an empty clipboard.
                    let page = page.clone();
                    window.defer(cx, move |window, cx| {
                        let _ = page.update(cx, |page, cx| page.read_clipboard(window, cx));
                    });
                    true
                })
        });
    }

    /// Reads the clipboard text, parses it off the UI thread, and opens the preview. Every error
    /// drops the text.
    pub(crate) fn read_clipboard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(config_dir) = AppSettings::config_dir(cx).map(Path::to_path_buf) else {
            show_import_error(&ImportError::NoConfigDir, window, cx);
            return;
        };
        let text = cx.read_from_clipboard().and_then(|item| item.text());
        let text = match check_clipboard_text(text.as_deref()) {
            Ok(text) => text.to_owned(),
            Err(error) => {
                show_import_error(&error, window, cx);
                return;
            }
        };
        cx.spawn_in(window, async move |this, cx| {
            let parsed = cx
                .background_executor()
                .spawn(async move {
                    let kubeconfig = Kubeconfig::parse(&text, Path::new("clipboard"))
                        .map_err(|_| ImportError::InvalidClipboard)?;
                    // Checks which names exist, so it stays off the UI thread.
                    let first = kubeconfig
                        .contexts()
                        .first()
                        .map(|context| context.name.as_str());
                    let target = pasted_file_path(&config_dir, first);
                    Ok::<_, ImportError>((kubeconfig, text, target))
                })
                .await;
            let _ = this.update_in(cx, |page, window, cx| match parsed {
                Ok((kubeconfig, text, target)) => {
                    page.show_preview(
                        ImportSource::Pasted { target },
                        kubeconfig,
                        Some(text),
                        window,
                        cx,
                    );
                }
                Err(error) => show_import_error(&error, window, cx),
            });
        })
        .detach();
    }

    /// Opens the preview of `candidate`. `pasted` is the clipboard text; it is kept only when the
    /// preview opens.
    fn show_preview(
        &mut self,
        source: ImportSource,
        candidate: Kubeconfig,
        pasted: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let rows = self.rows(cx);
        let preview = {
            let catalog = self.catalog.read(cx);
            let standalone: Vec<&Kubeconfig> = catalog
                .standalone_kubeconfigs()
                .map(|kubeconfig| kubeconfig.as_ref())
                .collect();
            import_preview(
                source,
                &candidate,
                &rows,
                catalog.chain().map(|chain| chain.as_ref()),
                &standalone,
            )
        };
        let preview = match preview {
            Ok(preview) => Rc::new(preview),
            Err(error) => {
                show_import_error(&error, window, cx);
                return;
            }
        };
        self.paste_text = pasted;
        let page = cx.weak_entity();
        window.open_alert_dialog(cx, move |dialog, _, cx| {
            let (ok_page, close_page, ok_preview) = (page.clone(), page.clone(), preview.clone());
            let (title, ok_text) = match preview.source {
                ImportSource::File(_) => ("Import kubeconfig", "Add"),
                ImportSource::Pasted { .. } => ("Paste kubeconfig", "Save and add"),
            };
            dialog
                .title(title)
                .width(px(DIALOG_WIDTH))
                .child(preview_body(&preview, cx))
                .button_props(
                    DialogButtonProps::default()
                        .ok_text(ok_text)
                        .show_cancel(true),
                )
                .on_ok(move |_, _, cx| {
                    let _ = ok_page.update(cx, |page, cx| page.confirm_import(&ok_preview, cx));
                    true
                })
                // Runs after OK, Cancel, Esc, and an overlay click alike.
                .on_close(move |_, _, cx| {
                    let _ = close_page.update(cx, |page, _| page.paste_text = None);
                })
        });
    }

    /// Add (file: only the path is stored) or Save and add (paste: the text moves to the catalog,
    /// which writes, loads, and registers it).
    fn confirm_import(&mut self, preview: &ImportPreview, cx: &mut Context<Self>) {
        match &preview.source {
            ImportSource::File(path) => {
                self.pending_select = Some(path.clone());
                let path = path.clone();
                AppSettings::update(cx, |settings| settings.registry.kubeconfigs.push(path));
            }
            ImportSource::Pasted { .. } => {
                let Some(text) = self.paste_text.take() else {
                    return;
                };
                let mark = ClipboardMark::of(&text);
                let first_context = preview.contexts.first().map(|context| context.name.clone());
                self.catalog.update(cx, |catalog, cx| {
                    catalog.add_pasted(text, first_context, mark, cx);
                });
            }
        }
    }
}

fn show_import_error(error: &ImportError, window: &mut Window, cx: &mut App) {
    let message: SharedString = error.to_string().into();
    window.open_alert_dialog(cx, move |alert, _, _| {
        alert
            .title("Cannot add the kubeconfig")
            .description(message.clone())
    });
}

/// The contexts, where the file goes, and the collision warnings.
fn preview_body(preview: &ImportPreview, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let (muted, warning) = (theme.muted_foreground, theme.warning);
    let mono = theme.mono_font_family.clone();
    let target = match &preview.source {
        ImportSource::File(path) => format!("Adds {}", path.display()),
        ImportSource::Pasted { target } => format!("Saves to {}", target.display()),
    };
    let contexts = preview.contexts.iter().map(|context| {
        h_flex()
            .gap_2()
            .items_center()
            .child(environment_badge(context.environment, cx))
            .child(
                div()
                    .text_sm()
                    .font_family(mono.clone())
                    .child(context.name.clone()),
            )
            .child(div().text_xs().text_color(muted).child(format!(
                "{} · {}",
                context.server.as_deref().unwrap_or("—"),
                context.auth
            )))
    });
    v_flex()
        .gap_2()
        .child(div().text_sm().font_semibold().child(format!(
            "{} in this kubeconfig",
            count_label(preview.contexts.len())
        )))
        .children(contexts)
        .child(div().text_sm().text_color(muted).child(target))
        .children(preview.collisions.iter().map(|collision| {
            div()
                .text_sm()
                .text_color(warning)
                .child(collision.to_string())
        }))
        .into_any_element()
}

fn count_label(count: usize) -> String {
    match count {
        1 => "1 context".to_owned(),
        count => format!("{count} contexts"),
    }
}
