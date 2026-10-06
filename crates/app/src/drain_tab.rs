//! The drain tab of the dock (spec 0034 step 3b, W6 note 5): what a running or finished drain is
//! doing, per node and per pod, with Cancel while it runs and Uncordon / Close after. It only
//! shows the `DrainRun`; the driver (`drain_driver`) feeds it and sends the requests.

use std::time::{Duration, Instant};

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::progress::Progress;
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    Render, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, WeakEntity,
    Window, div, px,
};

use crate::app_shell::AppShell;
use crate::audit_log::AuditIdentity;
use crate::cluster_registry::ClusterRef;
use crate::drain_run::{BlockingBudget, DrainRun, NodeState, RunEnd, StatusLine};
use crate::drawer::link_style;
use crate::resource_kind::ResourceKind;
use crate::status_tone::{StatusTone, tone_color};
use crate::table_selection::{ClusterObject, ResourceKey};

pub(crate) struct DrainTab {
    shell: WeakEntity<AppShell>,
    cluster: ClusterRef,
    /// The display name of the cluster, for the texts that outlive its session.
    cluster_name: SharedString,
    run: DrainRun,
    identity: AuditIdentity,
    /// The dialog's note, written on every summary line.
    note: Option<String>,
    /// Redraws the tab when the cluster's nodes change, so a node uncordoned from here or from
    /// the Nodes table reads as such.
    _nodes_watch: Option<Subscription>,
    started: Instant,
    /// A screenshot fixture pretends the run is this old; a real run has none.
    age: Duration,
    /// A fixture's clock stands still, so its countdowns read the same on every capture.
    #[cfg(feature = "screenshot")]
    is_frozen: bool,
}

/// What a tab is opened on.
pub(crate) struct DrainTabInputs {
    pub(crate) shell: WeakEntity<AppShell>,
    pub(crate) cluster: ClusterRef,
    pub(crate) cluster_name: SharedString,
    pub(crate) run: DrainRun,
    pub(crate) identity: AuditIdentity,
    pub(crate) note: Option<String>,
}

impl DrainTab {
    pub(crate) fn new(inputs: DrainTabInputs) -> Self {
        Self {
            shell: inputs.shell,
            cluster: inputs.cluster,
            cluster_name: inputs.cluster_name,
            run: inputs.run,
            identity: inputs.identity,
            note: inputs.note,
            _nodes_watch: None,
            started: Instant::now(),
            age: Duration::ZERO,
            #[cfg(feature = "screenshot")]
            is_frozen: false,
        }
    }

    /// Redraws the tab whenever `session` changes: its node list says which nodes are cordoned.
    pub(crate) fn watch_nodes<T: 'static>(&mut self, session: &Entity<T>, cx: &mut Context<Self>) {
        self._nodes_watch = Some(cx.observe(session, |_, _, cx| cx.notify()));
    }

    /// A tab for `--screen drain-progress`: the run is `age` old and its clock stands still.
    #[cfg(feature = "screenshot")]
    pub(crate) fn frozen_at(mut self, age: Duration) -> Self {
        self.age = age;
        self.is_frozen = true;
        self
    }

    /// The time since the run started, which the state machine counts in.
    pub(crate) fn now(&self) -> Duration {
        #[cfg(feature = "screenshot")]
        if self.is_frozen {
            return self.age;
        }
        self.started.elapsed() + self.age
    }

    pub(crate) fn run(&self) -> &DrainRun {
        &self.run
    }

    pub(crate) fn run_mut(&mut self) -> &mut DrainRun {
        &mut self.run
    }

    pub(crate) fn identity(&self) -> &AuditIdentity {
        &self.identity
    }

    /// The audit line of the commit in the air, for a quit: its outcome is unknown.
    pub(crate) fn in_flight_entry(&self) -> Option<crate::audit_log::AuditEntry> {
        let step = self.run.in_flight()?;
        crate::audit_log::drain_in_flight_entry(
            &self.identity,
            step,
            self.run.options(),
            self.note(),
        )
    }

    pub(crate) fn note(&self) -> Option<&str> {
        self.note.as_deref()
    }

    pub(crate) fn cluster(&self) -> &ClusterRef {
        &self.cluster
    }

    pub(crate) fn cluster_name(&self) -> &SharedString {
        &self.cluster_name
    }

    /// `Drain wk-04` or `Drain 3 nodes`, the tab label.
    pub(crate) fn label(&self) -> String {
        let nodes = self.run.node_states();
        match nodes.as_slice() {
            [(only, _)] => format!("Drain {only}"),
            nodes => format!("Drain {} nodes", nodes.len()),
        }
    }

    /// The dot of the tab: running reads as a warning, a clean drain green, a stuck one red.
    pub(crate) fn tone(&self) -> StatusTone {
        match self.run.end() {
            None => StatusTone::Warn,
            Some(RunEnd::Finished) => {
                let is_stuck = self
                    .run
                    .node_states()
                    .iter()
                    .any(|(_, state)| matches!(state, NodeState::Stuck(_)));
                if is_stuck {
                    StatusTone::Bad
                } else if self.run.pending_replacements() > 0 {
                    StatusTone::Warn
                } else {
                    StatusTone::Ok
                }
            }
            Some(RunEnd::Cancelled | RunEnd::Stopped(_)) => StatusTone::Done,
        }
    }

    /// The tab cannot be closed while the run is going (`Cancel the drain first`).
    pub(crate) fn is_running(&self) -> bool {
        self.run.is_running()
    }

    pub(crate) fn cancel(&mut self, cx: &mut Context<Self>) {
        self.run.cancel();
        cx.notify();
    }

    fn state_tone(state: &NodeState) -> StatusTone {
        match state {
            NodeState::Waiting | NodeState::Cancelled | NodeState::Stopped => StatusTone::Done,
            NodeState::Cordoning | NodeState::Evicting { .. } => StatusTone::Warn,
            NodeState::Drained => StatusTone::Ok,
            NodeState::Stuck(_) => StatusTone::Bad,
        }
    }

    fn render_nodes(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let chips = self.run.node_states().into_iter().map(|(node, state)| {
            h_flex()
                .gap_1p5()
                .items_center()
                .px_2()
                .py_0p5()
                .rounded_md()
                .border_1()
                .border_color(theme.border)
                .text_xs()
                .child(
                    div()
                        .font_family(theme.mono_font_family.clone())
                        .child(node),
                )
                .child(
                    div()
                        .text_color(tone_color(Self::state_tone(&state), cx))
                        .child(state.text()),
                )
        });
        h_flex().gap_2().flex_wrap().children(chips)
    }

    /// The nodes Drain again… reopens the dialog on, when the run ended short of drained: stuck,
    /// failed, or stopped by the app. A cancel is the user's own choice, so it offers nothing.
    fn nodes_to_drain_again(&self) -> Vec<String> {
        let is_short =
            self.tone() == StatusTone::Bad || matches!(self.run.end(), Some(RunEnd::Stopped(_)));
        if !is_short {
            return Vec::new();
        }
        self.run
            .node_states()
            .into_iter()
            .filter(|(_, state)| *state != NodeState::Drained)
            .map(|(node, _)| node)
            .collect()
    }

    /// The header: the line, with the budgets that block the drain as links to them.
    fn render_status(&self, line: StatusLine, cx: &Context<Self>) -> impl IntoElement {
        if line.blockers.is_empty() {
            return div().text_sm().child(format!("{}{}", line.lead, line.tail));
        }
        let last = line.blockers.len() - 1;
        let links = line
            .blockers
            .into_iter()
            .enumerate()
            .map(|(index, budget)| {
                let BlockingBudget { namespace, name } = budget;
                let key = ResourceKey::Kind {
                    kind: ResourceKind::PodDisruptionBudgets,
                    namespace: Some(namespace.clone()),
                    name: name.clone(),
                };
                let object = ClusterObject::new(self.cluster.clone(), key);
                let shell = self.shell.clone();
                let shown: SharedString = name.into();
                let separator = if index == last { " " } else { ", " };
                h_flex()
                    .child(
                        link_style(div().id(("drain-blocker", index)), &shown, cx)
                            .on_click(move |_, _, cx| {
                                let object = object.clone();
                                let _ =
                                    shell.update(cx, |shell, cx| shell.reveal_object(object, cx));
                            })
                            .child(shown.clone()),
                    )
                    .child(separator)
                    .into_any_element()
            });
        // One flowing line: every word is its own item, so the line breaks between words as plain
        // text does, and the budget names stay links.
        let words = |text: String| -> Vec<AnyElement> {
            text.split_inclusive(' ')
                .map(|word| div().child(word.to_owned()).into_any_element())
                .collect()
        };
        let mut flow = words(format!("{} · blocked by ", line.lead));
        flow.extend(links);
        flow.extend(words(line.tail.trim_start().to_owned()));
        div().text_sm().child(h_flex().flex_wrap().children(flow))
    }

    /// The nodes of this drain the cluster reports schedulable now.
    fn uncordoned(&self, cx: &App) -> Vec<String> {
        let candidates = self.run.cordoned_nodes(&[]);
        self.shell
            .upgrade()
            .map(|shell| {
                shell
                    .read(cx)
                    .schedulable_nodes(&self.cluster, &candidates, cx)
            })
            .unwrap_or_default()
    }

    fn render_buttons(&self, cx: &mut Context<Self>) -> impl IntoElement {
        if self.is_running() {
            return h_flex().gap_2().child(
                Button::new("drain-tab-cancel")
                    .label("Cancel")
                    .small()
                    .danger()
                    .outline()
                    .on_click(cx.listener(|tab, _, _, cx| tab.cancel(cx))),
            );
        }
        // The nodes the cluster still reports cordoned: once they are schedulable the button goes.
        let nodes = self.run.cordoned_nodes(&self.uncordoned(cx));
        let (shell, cluster) = (self.shell.clone(), self.cluster.clone());
        let uncordon = (!nodes.is_empty()).then(|| {
            let label = match nodes.len() {
                1 => "Uncordon 1 node".to_owned(),
                count => format!("Uncordon {count} nodes"),
            };
            Button::new("drain-tab-uncordon")
                .label(label)
                .small()
                .outline()
                .on_click(move |_, window, cx| {
                    let (cluster, nodes) = (cluster.clone(), nodes.clone());
                    let _ = shell.update(cx, |shell, cx| {
                        shell.uncordon_drained(&cluster, &nodes, window, cx);
                    });
                })
        });
        let again_nodes = self.nodes_to_drain_again();
        let options = *self.run.options();
        let again = (!again_nodes.is_empty()).then(|| {
            let (shell, cluster) = (self.shell.clone(), self.cluster.clone());
            Button::new("drain-tab-again")
                .label("Drain again…")
                .small()
                .outline()
                .on_click(move |_, window, cx| {
                    let _ = shell.update(cx, |shell, cx| {
                        shell.start_drain_with(&cluster, &again_nodes, options, window, cx);
                    });
                })
        });
        let (shell, handle) = (self.shell.clone(), cx.entity());
        h_flex().gap_2().children(again).children(uncordon).child(
            Button::new("drain-tab-close")
                .label("Close")
                .small()
                .outline()
                .on_click(move |_, _, cx| {
                    let handle = handle.clone();
                    let _ = shell.update(cx, |shell, cx| shell.close_drain_tab(&handle, cx));
                }),
        )
    }
}

impl Render for DrainTab {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, mono) = (theme.muted_foreground, theme.mono_font_family.clone());
        let now = self.now();
        let (gone, total) = self.run.progress();
        let percent = (gone * 100).checked_div(total).unwrap_or(0);
        let rows: Vec<_> = self
            .run
            .pod_rows(now)
            .into_iter()
            .map(|row| {
                h_flex()
                    .gap_3()
                    .justify_between()
                    .items_start()
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_xs()
                            .font_family(mono.clone())
                            .child(row.pod),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .max_w(px(520.))
                            .text_xs()
                            .text_right()
                            .text_color(tone_color(row.tone, cx))
                            .child(row.text),
                    )
            })
            .collect();
        v_flex()
            .size_full()
            .min_h_0()
            .gap_2()
            .p_3()
            .child(
                h_flex()
                    .gap_3()
                    .justify_between()
                    .items_start()
                    .child(
                        v_flex()
                            .gap_1()
                            .min_w_0()
                            .child(
                                self.render_status(
                                    self.run.status_line(now, &self.uncordoned(cx)),
                                    cx,
                                ),
                            )
                            .child(self.render_nodes(cx)),
                    )
                    .child(self.render_buttons(cx)),
            )
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .child(Progress::new("drain-tab-progress").value(percent as f32)),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_xs()
                            .text_color(muted)
                            .child(format!("{gone} of {total} gone")),
                    ),
            )
            .children(self.run.poll_error().map(|text| {
                div()
                    .text_xs()
                    .text_color(tone_color(StatusTone::Warn, cx))
                    .child(format!("Could not refresh pods: {text}"))
            }))
            .child(
                v_flex()
                    .id("drain-tab-pods")
                    .flex_1()
                    .min_h_0()
                    .gap_1()
                    .overflow_y_scroll()
                    .children(rows),
            )
    }
}

#[cfg(test)]
#[path = "drain_tab_tests.rs"]
mod drain_tab_tests;
