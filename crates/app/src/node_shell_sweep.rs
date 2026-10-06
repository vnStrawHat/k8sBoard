//! The leftover sweep (spec 0037 decision 18): when a cluster first goes Live, one read-only list
//! finds node shell pods of other runs, in any phase. A notice offers a review; nothing is deleted
//! until the user clicks `Delete selected`, and every delete goes through the cleanup path with its
//! own audit line.
//!
//! A child of `app_shell`, like `write_flow`: every step names the cluster whose first Live
//! started it and takes its connection, scope, and guard from that cluster's own slot.

use cluster::{
    AccessCheck, NamespaceScope, NodeShellLeftover, ObjectKind, ObjectRef, WriteOperation,
    WriteRequest,
};
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    AppContext as _, Context, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    SharedString, StatefulInteractiveElement as _, Styled as _, WeakEntity, Window, div, px,
};

use super::AppShell;
use super::node_shell_run_history::PastRuns;
use super::write_flow::{CleanupAudit, NodeShellCleanup};
use crate::age::format_age;
use crate::cluster_registry::ClusterRef;
use crate::cluster_runtime::ClusterRuntime;
use crate::cluster_session::AccessState;
use crate::settings::AppSettings;
use crate::write_guard::{ClusterGuard, WriteLock};

const DIALOG_WIDTH: f32 = 560.;
const LIST_MAX_HEIGHT: f32 = 240.;
/// Shown in the warning color on the review dialog.
pub(crate) const RUNNING_WARNING: &str =
    "Running pods may belong to another k8sBoard window or user.";

/// `1 leftover node shell pod`, `3 leftover node shell pods`.
pub(crate) fn notice_text(count: usize) -> String {
    match count {
        1 => "1 leftover node shell pod".to_owned(),
        count => format!("{count} leftover node shell pods"),
    }
}

/// Why the delete may not go now, `None` when it may: the cluster is unlocked and `delete pods` is
/// allowed. The sweep has no other gate; the user's click is its confirmation.
pub(crate) fn sweep_block(guard: &ClusterGuard<'_>) -> Option<SharedString> {
    match guard.access {
        AccessState::Checking { .. } => return Some("Checking permissions…".into()),
        AccessState::Unknown => return Some("Permissions could not be checked".into()),
        AccessState::Known(report) if !report.is_allowed(AccessCheck::DeletePods) => {
            return Some(format!("Not permitted: {}", AccessCheck::DeletePods).into());
        }
        AccessState::Known(_) => {}
    }
    (guard.lock == WriteLock::Locked)
        .then(|| format!("{} is read-only", guard.display_name()).into())
}

/// The scope a sweep lists: the view's scope, plus the namespace the node shell pods go to when the
/// view does not already cover it (`All` does).
pub(crate) fn sweep_scope(
    scope: &NamespaceScope,
    node_shell_namespace: Option<&str>,
) -> NamespaceScope {
    if matches!(scope, NamespaceScope::All) {
        return NamespaceScope::All;
    }
    NamespaceScope::of_namespaces(
        scope
            .namespaces()
            .iter()
            .cloned()
            .chain(node_shell_namespace.map(str::to_owned)),
    )
}

/// Whether a sweep may list: a cluster whose review says no is not asked. While the review is
/// unknown or still running the list goes out, and a refusal says nothing.
fn may_list_pods(access: &AccessState) -> bool {
    match access {
        AccessState::Known(report) => report.is_allowed(AccessCheck::ListPods),
        AccessState::Checking { .. } | AccessState::Unknown => true,
    }
}

/// Finished pods are checked by default, and so are the pods of an earlier run of this settings
/// folder (the user's own); any other running pod may be someone else's.
pub(crate) fn is_checked_by_default(leftover: &NodeShellLeftover, runs: &PastRuns) -> bool {
    leftover.phase.is_finished() || runs.owns(leftover)
}

/// The delete of one listed pod, or `None` when its name or uid does not fit the write path.
fn delete_request(leftover: &NodeShellLeftover) -> Option<WriteRequest> {
    let target = ObjectRef::new(
        ObjectKind::Pod,
        Some(leftover.namespace.clone()),
        leftover.name.clone(),
    )?;
    WriteRequest::new(
        target,
        WriteOperation::DeleteNodeShellPod {
            uid: leftover.uid.clone(),
        },
    )
}

impl AppShell {
    /// The first Live of `cluster`: lists the node shell pods of other runs on that cluster's own
    /// connection, over its own scope. An answer of none, or an error (no right to list, a network
    /// failure), says nothing: this is a convenience, not a gate.
    pub(super) fn sweep_leftovers(&mut self, cluster: &ClusterRef, cx: &mut Context<Self>) {
        let (Some(connection), Some(live)) =
            (self.connection_of(cluster, cx), self.live_of(cluster, cx))
        else {
            return;
        };
        if !may_list_pods(&live.access) {
            return;
        }
        // The namespace the node shell pods are created in is listed too: a view scoped elsewhere
        // would otherwise never see them.
        let node_shell_namespace = self
            .guard_for(cluster, cx)
            .map(|guard| guard.profile.node_shell_namespace.clone());
        let scope = sweep_scope(&live.scope, node_shell_namespace.as_deref());
        let instance = self.run_id.clone();
        let runtime = cx.global::<ClusterRuntime>().clone();
        let cluster = cluster.clone();
        cx.spawn(async move |this, cx| {
            let listed = runtime
                .spawn(async move { connection.node_shell_leftovers(&scope, &instance).await })
                .await;
            let Ok(Ok(found)) = listed else {
                return;
            };
            if found.is_empty() {
                return;
            }
            let _ = this.update(cx, |shell, cx| {
                shell.show_leftover_notice(cluster, found, cx)
            });
        })
        .detach();
    }

    /// `{n} leftover node shell pods` with `Review…`. It never deletes by itself. The answer of a
    /// sweep can land after a switch: a notice for a cluster that is no longer open is dropped,
    /// because its review would find no guard anyway.
    fn show_leftover_notice(
        &mut self,
        cluster: ClusterRef,
        found: Vec<NodeShellLeftover>,
        cx: &mut Context<Self>,
    ) {
        if self.session_of(&cluster).is_none() {
            return;
        }
        #[cfg(test)]
        self.sweep_notices.push((cluster.clone(), found.len()));
        let text = notice_text(found.len());
        let (handle, shell) = (self.window, cx.weak_entity());
        cx.defer(move |cx| {
            let _ = cx.update_window(handle, |_, window, cx| {
                let notification = Notification::warning(text).action(move |_, _, cx| {
                    let (shell, cluster, found) = (shell.clone(), cluster.clone(), found.clone());
                    Button::new("review-leftovers")
                        .label("Review…")
                        .small()
                        .on_click(cx.listener(move |notification, _, window, cx| {
                            notification.dismiss(window, cx);
                            let (shell, cluster, found) =
                                (shell.clone(), cluster.clone(), found.clone());
                            let _ = shell.update(cx, |shell, cx| {
                                shell.open_leftover_review(cluster, found, window, cx);
                            });
                        }))
                });
                window.push_notification(notification, cx);
            });
        });
    }

    /// The review dialog: the pods, a checkbox each, and `Delete selected`.
    pub(crate) fn open_leftover_review(
        &mut self,
        cluster: ClusterRef,
        found: Vec<NodeShellLeftover>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let runs = AppSettings::config_dir(cx)
            .map(PastRuns::load)
            .unwrap_or_default();
        let review = LeftoverReview {
            shell: cx.weak_entity(),
            cluster,
            rows: review_rows(found, &runs),
            #[cfg(feature = "screenshot")]
            is_fixture: false,
        };
        Self::open_review_dialog(review, window, cx);
    }

    fn open_review_dialog(review: LeftoverReview, window: &mut Window, cx: &mut Context<Self>) {
        let review = cx.new(|_| review);
        window.open_dialog(cx, move |dialog, _, _| {
            dialog
                .title("Leftover node shell pods")
                .w(px(DIALOG_WIDTH))
                .child(review.clone())
        });
    }

    /// `Delete selected`: each listed pod is deleted through the cleanup path, with the uid it was
    /// listed with. The gate is read again here, because the dialog may have stood open.
    pub(crate) fn delete_leftovers(
        &mut self,
        cluster: &ClusterRef,
        selected: &[NodeShellLeftover],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (Some(guard), Some(connection)) =
            (self.guard_for(cluster, cx), self.connection_of(cluster, cx))
        else {
            self.notify_window(window, cx, format!("{} is not open", cluster.context));
            return;
        };
        if let Some(reason) = sweep_block(&guard) {
            let text = format!("Delete selected is unavailable: {reason}");
            drop(guard);
            self.notify_window(window, cx, text);
            return;
        }
        let audit = CleanupAudit::of(&guard);
        drop(guard);
        let cleanups: Vec<NodeShellCleanup> = selected
            .iter()
            .filter_map(delete_request)
            .filter_map(|request| NodeShellCleanup::new(connection.clone(), request, audit.clone()))
            .collect();
        let count = cleanups.len();
        for cleanup in cleanups {
            self.begin_cleanup(cleanup, cx);
        }
        let noun = if count == 1 { "pod" } else { "pods" };
        self.notify_window(window, cx, format!("Deleting {count} node shell {noun}…"));
    }

    fn notify_window(&self, window: &mut Window, cx: &mut Context<Self>, text: String) {
        super::write_flow::notify(window, cx, text);
    }
}

struct ReviewRow {
    leftover: NodeShellLeftover,
    is_checked: bool,
    /// `left by your session, quit at 14:41` for a pod of an earlier run of this settings folder.
    note: Option<String>,
}

fn review_rows(found: Vec<NodeShellLeftover>, runs: &PastRuns) -> Vec<ReviewRow> {
    let zone = jiff::tz::TimeZone::system();
    found
        .into_iter()
        .map(|leftover| ReviewRow {
            is_checked: is_checked_by_default(&leftover, runs),
            note: runs.note(&leftover, &zone),
            leftover,
        })
        .collect()
}

/// The body of the review dialog.
struct LeftoverReview {
    shell: WeakEntity<AppShell>,
    cluster: ClusterRef,
    rows: Vec<ReviewRow>,
    /// A screenshot fixture has no cluster behind it, so the gate is not read.
    #[cfg(feature = "screenshot")]
    is_fixture: bool,
}

impl LeftoverReview {
    fn selected(&self) -> Vec<NodeShellLeftover> {
        self.rows
            .iter()
            .filter(|row| row.is_checked)
            .map(|row| row.leftover.clone())
            .collect()
    }

    /// Why `Delete selected` is off, read from the cluster's own guard now.
    fn block(&self, cx: &gpui_kit::App) -> Option<SharedString> {
        #[cfg(feature = "screenshot")]
        if self.is_fixture {
            return None;
        }
        let shell = self.shell.upgrade()?;
        let shell = shell.read(cx);
        match shell.guard_for(&self.cluster, cx) {
            Some(guard) => sweep_block(&guard),
            None => Some("The cluster is not open".into()),
        }
    }
}

impl Render for LeftoverReview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (warning, muted) = (theme.warning, theme.muted_foreground);
        let mono = theme.mono_font_family.clone();
        let now = jiff::Timestamp::now();
        let block = self.block(cx);
        let is_off = block.is_some() || !self.rows.iter().any(|row| row.is_checked);
        let rows = self.rows.iter().enumerate().map(|(index, row)| {
            let leftover = &row.leftover;
            let detail = format!(
                "{} · {} · {}",
                leftover.node.as_deref().unwrap_or("—"),
                phase_text(leftover),
                format_age(leftover.created_at, now)
            );
            let line = h_flex()
                .gap_2()
                .items_center()
                .child(
                    Checkbox::new(("leftover-check", index))
                        .checked(row.is_checked)
                        .on_click(cx.listener(move |review, checked: &bool, _, cx| {
                            if let Some(row) = review.rows.get_mut(index) {
                                row.is_checked = *checked;
                            }
                            cx.notify();
                        })),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_sm()
                        .font_family(mono.clone())
                        .child(format!("{}/{}", leftover.namespace, leftover.name)),
                )
                .child(div().text_xs().text_color(muted).child(detail));
            // Indented under the name, past the checkbox.
            let note = row
                .note
                .as_ref()
                .map(|note| div().pl_6().text_xs().text_color(muted).child(note.clone()));
            v_flex().child(line).children(note)
        });
        let weak = cx.weak_entity();
        v_flex()
            .w_full()
            .gap_3()
            .child(div().text_sm().text_color(warning).child(RUNNING_WARNING))
            .child(
                v_flex()
                    .id("leftover-rows")
                    .gap_1()
                    .max_h(px(LIST_MAX_HEIGHT))
                    .overflow_y_scroll()
                    .children(rows),
            )
            .children(block.map(|reason| div().text_xs().text_color(muted).child(reason)))
            .child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .justify_end()
                    .child(
                        Button::new("leftover-cancel")
                            .label("Cancel")
                            .small()
                            .outline()
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("leftover-delete")
                            .label("Delete selected")
                            .small()
                            .danger()
                            .disabled(is_off)
                            .on_click(move |_, window, cx| {
                                let _ = weak.update(cx, |review, cx| review.delete(window, cx));
                            }),
                    ),
            )
    }
}

impl LeftoverReview {
    fn delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (shell, cluster, selected) =
            (self.shell.clone(), self.cluster.clone(), self.selected());
        window.close_dialog(cx);
        window.defer(cx, move |window, cx| {
            let _ = shell.update(cx, |shell, cx| {
                shell.delete_leftovers(&cluster, &selected, window, cx);
            });
        });
    }
}

fn phase_text(leftover: &NodeShellLeftover) -> &'static str {
    use cluster::LeftoverPhase;
    match leftover.phase {
        LeftoverPhase::Pending => "Pending",
        LeftoverPhase::Running => "Running",
        LeftoverPhase::Succeeded => "Succeeded",
        LeftoverPhase::Failed => "Failed",
        LeftoverPhase::Unknown => "Unknown",
    }
}

#[cfg(feature = "screenshot")]
impl AppShell {
    /// `--screen leftover-sweep-fixture`: the review dialog over four fixed pods of a fixed cluster,
    /// two finished (checked), one running that an earlier run of this settings folder left
    /// (checked, with its note), and one pending of an unknown run (unchecked). It reads no gate and
    /// `Delete selected` finds no cluster, so it can never delete a pod.
    pub(super) fn open_leftover_fixture(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        use cluster::LeftoverPhase;

        let now = jiff::Timestamp::now();
        let quit_at = now
            .checked_sub(jiff::SignedDuration::from_mins(10))
            .unwrap_or(now);
        let runs = PastRuns::parse(&format!("fixtureprev quit {quit_at}\n"));
        let row = |name: &str, node: &str, phase: LeftoverPhase, minutes: i64, run: &str| {
            NodeShellLeftover {
                namespace: "kube-system".to_owned(),
                name: name.to_owned(),
                uid: format!("uid-{name}"),
                node: Some(node.to_owned()),
                instance: Some(run.to_owned()),
                phase,
                created_at: now
                    .checked_sub(jiff::SignedDuration::from_mins(minutes))
                    .ok(),
            }
        };
        let found = vec![
            row(
                "k8sboard-node-shell-wk-03-x7k2q",
                "wk-03",
                LeftoverPhase::Succeeded,
                190,
                "otherrun01",
            ),
            row(
                "k8sboard-node-shell-wk-01-m4d9z",
                "wk-01",
                LeftoverPhase::Failed,
                75,
                "otherrun01",
            ),
            row(
                "k8sboard-node-shell-wk-02-q8r5t",
                "wk-02",
                LeftoverPhase::Running,
                12,
                "fixtureprev",
            ),
            row(
                "k8sboard-node-shell-wk-04-h2j6c",
                "wk-04",
                LeftoverPhase::Pending,
                3,
                "otherrun02",
            ),
        ];
        let review = LeftoverReview {
            shell: cx.weak_entity(),
            cluster: ClusterRef {
                kubeconfig: std::path::PathBuf::from("fixture.yaml"),
                context: crate::screenshot::SHELL_FIXTURE_CLUSTER.to_owned(),
            },
            rows: review_rows(found, &runs),
            is_fixture: true,
        };
        Self::open_review_dialog(review, window, cx);
    }
}

#[cfg(test)]
#[path = "node_shell_sweep_tests.rs"]
mod node_shell_sweep_tests;
