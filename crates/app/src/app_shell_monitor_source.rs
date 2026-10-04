//! Starting and landing the metrics source query of the open Monitor (spec 0048). The query belongs
//! to the shown Monitor: it starts when a Monitor tab shows for a subject the source can answer,
//! refreshes while the tab stays visible, and is dropped (its requests aborted) with a hidden tab,
//! another subject, scope, or range, or a source that is no longer ready.

use std::time::Instant;

use cluster::{
    ClusterConnection, MetricsError, NamespaceScope, RangeSpec, UsageMetric, UsageTarget,
};
use futures::StreamExt as _;
use gpui_kit::{App, Context};

use super::AppShell;
use crate::cluster_runtime::ClusterRuntime;
use crate::drawer::{ContainerTab, DrawerTab, MonitorRange};
use crate::history_rings::Resolution;
use crate::monitor_data::{MonitorInput, read_series};
use crate::monitor_source::{
    SourceFetch, SourceKey, SourceResult, SourceView, refresh_after, source_metrics,
    source_monitor_data, source_target,
};
#[cfg(feature = "screenshot")]
use crate::table_selection::ResourceKey;
use cluster::UsageSeries as SourceSeries;

/// How many of a view's queries run at once.
const FETCH_CONCURRENCY: usize = 3;

type Answers = Vec<(UsageMetric, Result<SourceSeries, MetricsError>)>;

/// What the shown Monitor would ask the source.
struct Wanted {
    key: SourceKey,
    target: UsageTarget,
    connection: ClusterConnection,
}

impl AppShell {
    /// Keeps `drawer.monitor.source` for what the Monitor shows. It runs inside `render`, before the
    /// views are built, so a new fetch is seen in the same frame. It starts a query when the key
    /// changed or the answer is due, and only assigns otherwise.
    pub(super) fn sync_monitor_source(&mut self, cx: &mut Context<Self>) {
        #[cfg(feature = "screenshot")]
        if self.is_monitor_source_fixture {
            self.sync_source_fixture(cx);
            return;
        }
        let Some(wanted) = self.wanted_source(cx) else {
            self.drawer.monitor.source = None;
            // A range that needs a source is not offered without one.
            if self.shows_monitor() && self.drawer.monitor.range.is_long() {
                self.drawer.monitor.range = MonitorRange::Hours24;
            }
            return;
        };
        let older = match self.drawer.monitor.source.take() {
            Some(fetch) if fetch.key == wanted.key => {
                if !fetch.is_due() {
                    self.drawer.monitor.source = Some(fetch);
                    return;
                }
                Some(fetch)
            }
            // A new key drops the old fetch, and its requests with it.
            Some(_) | None => None,
        };
        self.start_source_fetch(wanted, older, cx);
    }

    fn wanted_source(&self, cx: &App) -> Option<Wanted> {
        if !self.shows_monitor() {
            return None;
        }
        let subject = self.drawer_subject()?.clone();
        let live = self.subject_live(cx)?;
        let (source, check) = live.metrics.source.ready()?;
        if check.cpu_series == 0 {
            return None;
        }
        let is_container_tab = self.drawer.tab == DrawerTab::Containers
            && self.drawer.container_tab == ContainerTab::Monitor;
        let container = self.monitor_container(&subject.key, live, is_container_tab);
        let monitor_subject = Self::monitor_subject(&subject.key, container.as_deref(), live)?;
        let scope = self.drawer.monitor.scope.clone();
        let target = source_target(&monitor_subject, &scope)?;
        Some(Wanted {
            key: SourceKey {
                subject,
                container,
                scope,
                range: self.drawer.monitor.range,
                source: source.clone(),
            },
            target,
            connection: live.connection().clone(),
        })
    }

    fn start_source_fetch(
        &mut self,
        wanted: Wanted,
        older: Option<SourceFetch>,
        cx: &mut Context<Self>,
    ) {
        let Ok(spec) = RangeSpec::ending_at(jiff::Timestamp::now(), wanted.key.range.duration())
        else {
            return;
        };
        let runtime = cx.global::<ClusterRuntime>().clone();
        let Wanted {
            key,
            target,
            connection,
        } = wanted;
        let source = key.source.clone();
        let metrics = source_metrics(&target);
        let fetching = runtime.spawn(async move {
            futures::stream::iter(metrics.iter().copied())
                .map(|metric| {
                    let (connection, source, target) = (&connection, &source, &target);
                    async move {
                        (
                            metric,
                            connection.usage_range(source, target, metric, &spec).await,
                        )
                    }
                })
                .buffered(FETCH_CONCURRENCY)
                .collect::<Vec<_>>()
                .await
        });
        let landing = key.clone();
        let task = cx.spawn(async move |this, cx| {
            let answers = fetching.await;
            let _ = this.update(cx, |shell, cx| {
                shell.finish_source_fetch(&landing, (spec.end(), spec.step()), answers, cx);
            });
        });
        let mut fetch = older.unwrap_or_else(|| SourceFetch::new(key));
        fetch.started = Instant::now();
        fetch._task = Some(task);
        fetch._refresh = None;
        self.drawer.monitor.source = Some(fetch);
    }

    fn finish_source_fetch(
        &mut self,
        key: &SourceKey,
        (end, step): (jiff::Timestamp, std::time::Duration),
        answers: Result<Answers, tokio::task::JoinError>,
        cx: &mut Context<Self>,
    ) {
        // A fetch of another key, or none, was dropped: its answer is not wanted.
        if self
            .drawer
            .monitor
            .source
            .as_ref()
            .is_none_or(|fetch| fetch.key != *key)
        {
            return;
        }
        let metrics = answers.unwrap_or_default();
        let view = self.source_view_of(key, (end, step), metrics, None, cx);
        let Some(fetch) = self.drawer.monitor.source.as_mut() else {
            return;
        };
        fetch._task = None;
        fetch.started = Instant::now();
        match (view, &fetch.view) {
            // A refresh that failed keeps the older answer on screen, with a line saying so.
            (Some(SourceView::Fallback(reason)), Some(SourceView::Charts { .. })) => {
                fetch.last_failure = Some(reason);
            }
            (Some(view), _) => {
                fetch.view = Some(view);
                fetch.last_failure = None;
            }
            // The subject left the lists meanwhile.
            (None, _) => {}
        }
        let wait = refresh_after(key.range);
        fetch._refresh = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(wait).await;
            let _ = this.update(cx, |_, cx| cx.notify());
        }));
        cx.notify();
    }

    /// The charts and rows of an answer, from the lists as they are now.
    fn source_view_of(
        &self,
        key: &SourceKey,
        (end, step): (jiff::Timestamp, std::time::Duration),
        metrics: Answers,
        oom: Option<Vec<jiff::Timestamp>>,
        cx: &App,
    ) -> Option<SourceView> {
        let live = self.subject_live(cx)?;
        let subject = Self::monitor_subject(&key.subject.key, key.container.as_deref(), live)?;
        let input = MonitorInput {
            subject,
            scope: &key.scope,
            range: key.range,
            pods: live.pods.items(),
            pod_history: &live.metrics.pods.history,
            node_history: &live.metrics.nodes.history,
            kubelet: &live.metrics.kubelet,
            nodes: live.nodes.items(),
            is_all_namespaces: live.scope == NamespaceScope::All,
        };
        let oom = oom.unwrap_or_else(|| read_series(&input, &key.scope, Resolution::Coarse).0.oom);
        Some(source_monitor_data(
            &input,
            &SourceResult {
                end,
                step,
                oom,
                metrics,
            },
        ))
    }

    /// The `pod-monitor-source-fixture` screen: the real drawer of a pod, with synthetic source
    /// data on the 30d range and no request to any source.
    #[cfg(feature = "screenshot")]
    fn sync_source_fixture(&mut self, cx: &mut Context<Self>) {
        if !self.shows_monitor() || self.drawer.monitor.source.is_some() {
            return;
        }
        let Some(subject) = self.drawer_subject().cloned() else {
            return;
        };
        if !matches!(subject.key, ResourceKey::Pod { .. }) {
            return;
        }
        let Some(live) = self.subject_live(cx) else {
            return;
        };
        let Some(pod) = live.pods.items().iter().find(|pod| subject.key.is_pod(pod)) else {
            return;
        };
        let _ = pod;
        let Some(source) = crate::monitor_source::fixture_source() else {
            return;
        };
        self.drawer.monitor.range = MonitorRange::Days30;
        let key = SourceKey {
            subject,
            container: None,
            scope: self.drawer.monitor.scope.clone(),
            range: MonitorRange::Days30,
            source,
        };
        let Ok(spec) =
            RangeSpec::ending_at(jiff::Timestamp::now(), MonitorRange::Days30.duration())
        else {
            return;
        };
        let metrics = crate::monitor_source::fixture_answers(&spec);
        let oom = vec![crate::monitor_source::fixture_oom(&spec)];
        let view = self.source_view_of(&key, (spec.end(), spec.step()), metrics, Some(oom), cx);
        if let Some(view) = view {
            let mut fetch = SourceFetch::new(key);
            fetch.view = Some(view);
            self.drawer.monitor.source = Some(fetch);
        }
    }
}
