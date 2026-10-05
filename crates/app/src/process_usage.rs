//! The CPU, memory and Kubernetes network use of k8sBoard itself, for the right end of the
//! status bar (spec 0054). It follows OneTerm's resource and network items: one sample a second,
//! the heavy part off the UI thread, a tooltip table that keeps updating while it is open.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, h_flex};
use gpui_kit::{
    AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    Render, StatefulInteractiveElement as _, Styled as _, Task, WeakEntity, Window, div,
};
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

use crate::app_shell::AppShell;
use crate::status_bar::{separator_color, status_separator};
use crate::status_tooltip::{Section, details_table};
use crate::usage_format::Measure;

const SAMPLE_INTERVAL: Duration = Duration::from_secs(1);

/// What `sysinfo` calls the memory of a process, named as the OS does.
const RESIDENT_NAME: &str = if cfg!(windows) {
    "Working set"
} else {
    "Resident (RSS)"
};
const VIRTUAL_NAME: &str = if cfg!(windows) {
    "Commit (private bytes)"
} else {
    "Virtual size"
};

/// Bytes the cluster client has sent and received, read from its counters at one instant.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct TrafficTotals {
    pub(crate) received: u64,
    pub(crate) sent: u64,
}

/// Bytes per second over the last sampling interval.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct NetworkRate {
    pub(crate) received: f64,
    pub(crate) sent: f64,
}

/// The rate between two readings of the counters. A counter that went down belongs to another
/// connection (the cluster was switched), so that direction reads 0 instead of a huge number.
pub(crate) fn network_rate(
    before: TrafficTotals,
    after: TrafficTotals,
    elapsed: Duration,
) -> NetworkRate {
    let seconds = elapsed.as_secs_f64();
    if seconds <= 0.0 {
        return NetworkRate::default();
    }
    let per_second = |before: u64, after: u64| after.saturating_sub(before) as f64 / seconds;
    NetworkRate {
        received: per_second(before.received, after.received),
        sent: per_second(before.sent, after.sent),
    }
}

/// `sysinfo` reports 100% for one busy core; the status bar shows the share of the whole machine,
/// as Task Manager does.
pub(crate) fn machine_cpu_percent(per_core_percent: f32, cores: usize) -> f32 {
    (per_core_percent / cores.max(1) as f32).clamp(0.0, 100.0)
}

/// One reading of this process.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ProcessReading {
    pub(crate) cpu_percent: f32,
    pub(crate) cores: usize,
    /// The working set on Windows, the resident set elsewhere.
    pub(crate) memory: u64,
    /// The commit on Windows, the virtual size elsewhere.
    pub(crate) virtual_memory: u64,
    /// Only Linux lists the tasks of a process.
    pub(crate) threads: Option<usize>,
    pub(crate) uptime: Duration,
}

/// The latest reading of everything the status bar shows.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Usage {
    pub(crate) process: Option<ProcessReading>,
    pub(crate) rate: NetworkRate,
    /// `None` while no cluster is open.
    pub(crate) traffic: Option<TrafficTotals>,
}

/// Folds successive readings into the latest [`Usage`].
#[derive(Default)]
pub(crate) struct UsageState {
    latest: Option<Usage>,
    previous_traffic: Option<(TrafficTotals, Instant)>,
}

impl UsageState {
    pub(crate) fn record(
        &mut self,
        process: Option<ProcessReading>,
        traffic: Option<TrafficTotals>,
        now: Instant,
    ) {
        let rate = match (self.previous_traffic, traffic) {
            (Some((before, at)), Some(after)) => {
                network_rate(before, after, now.saturating_duration_since(at))
            }
            _ => NetworkRate::default(),
        };
        self.previous_traffic = traffic.map(|totals| (totals, now));
        self.latest = Some(Usage {
            process,
            rate,
            traffic,
        });
    }

    pub(crate) fn latest(&self) -> Option<&Usage> {
        self.latest.as_ref()
    }
}

/// Reads this process through `sysinfo`, which keeps the previous reading for its CPU delta.
#[derive(Clone)]
struct ProcessSampler {
    pid: Pid,
    system: Arc<Mutex<System>>,
}

impl ProcessSampler {
    /// `None` on a platform that cannot name its own process.
    fn new() -> Option<Self> {
        let pid = sysinfo::get_current_pid().ok()?;
        Some(Self {
            pid,
            system: Arc::new(Mutex::new(System::new())),
        })
    }

    /// Blocking: meant for the background executor.
    fn sample(&self) -> Option<ProcessReading> {
        let mut system = self
            .system
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[self.pid]),
            ProcessRefreshKind::new().with_cpu().with_memory(),
        );
        let process = system.process(self.pid)?;
        let cores = std::thread::available_parallelism().map_or(1, usize::from);
        Some(ProcessReading {
            cpu_percent: machine_cpu_percent(process.cpu_usage(), cores),
            cores,
            memory: process.memory(),
            virtual_memory: process.virtual_memory(),
            threads: process.tasks().map(|tasks| tasks.len()),
            uptime: Duration::from_secs(process.run_time()),
        })
    }
}

/// `CPU 3.1%  MEM 142.0 MB`.
pub(crate) fn resource_text(process: &ProcessReading) -> String {
    format!(
        "CPU {:.1}%  MEM {}",
        process.cpu_percent,
        format_memory(process.memory)
    )
}

/// `↓ 1.2 KB/s  ↑ 300 B/s`.
pub(crate) fn network_text(rate: NetworkRate) -> String {
    format!(
        "↓ {}  ↑ {}",
        Measure::Rate.format(rate.received),
        Measure::Rate.format(rate.sent)
    )
}

/// Binary units: `812 B`, `1.5 KB`, `142.0 MB`, `1.50 GB`.
pub(crate) fn format_memory(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}

/// `12.4 s` under a minute, `4m 05s` under an hour, `3h 07m` from there.
pub(crate) fn format_uptime(duration: Duration) -> String {
    let seconds = duration.as_secs();
    if seconds < 60 {
        format!("{:.1} s", duration.as_secs_f64())
    } else if seconds < 3600 {
        format!("{}m {:02}s", seconds / 60, seconds % 60)
    } else {
        format!("{}h {:02}m", seconds / 3600, seconds % 3600 / 60)
    }
}

/// Which item a tooltip belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UsageItem {
    Network,
    Resources,
}

/// The tooltip table of the network item, in a fixed order.
pub(crate) fn network_sections(usage: &Usage) -> Vec<Section> {
    let total = |pick: fn(&TrafficTotals) -> u64| {
        usage
            .traffic
            .as_ref()
            .map_or_else(|| "—".to_owned(), |totals| format_memory(pick(totals)))
    };
    vec![
        Section {
            title: "Kubernetes API traffic",
            rows: vec![
                ("Received", Measure::Rate.format(usage.rate.received)),
                ("Sent", Measure::Rate.format(usage.rate.sent)),
            ],
        },
        Section {
            title: "Since the cluster connected",
            rows: vec![
                ("Received", total(|totals| totals.received)),
                ("Sent", total(|totals| totals.sent)),
            ],
        },
    ]
}

/// The tooltip table of the CPU and memory item, in a fixed order.
pub(crate) fn resource_sections(process: &ProcessReading) -> Vec<Section> {
    vec![
        Section {
            title: "Memory",
            rows: vec![
                (RESIDENT_NAME, format_memory(process.memory)),
                (VIRTUAL_NAME, format_memory(process.virtual_memory)),
            ],
        },
        Section {
            title: "CPU",
            rows: vec![
                (
                    "Usage",
                    format!(
                        "{:.1}% of {} logical cores",
                        process.cpu_percent, process.cores
                    ),
                ),
                (
                    "Threads",
                    process
                        .threads
                        .map_or_else(|| "n/a".to_owned(), |threads| threads.to_string()),
                ),
                ("Uptime", format_uptime(process.uptime)),
            ],
        },
    ]
}

/// The right end of the status bar: network use, a separator, CPU and memory. A task samples
/// once a second; dropping the entity (the window closed) ends it.
pub(crate) struct ProcessUsage {
    state: UsageState,
    _sampling: Task<()>,
}

impl ProcessUsage {
    pub(crate) fn new(shell: WeakEntity<AppShell>, cx: &mut Context<Self>) -> Self {
        let sampler = ProcessSampler::new();
        let sampling = cx.spawn(async move |this, cx| {
            loop {
                let process = match &sampler {
                    Some(sampler) => {
                        let sampler = sampler.clone();
                        cx.background_executor()
                            .spawn(async move { sampler.sample() })
                            .await
                    }
                    None => None,
                };
                let updated = this.update(cx, |usage, cx| {
                    let traffic = shell.upgrade().and_then(|shell| shell.read(cx).traffic(cx));
                    usage.state.record(process, traffic, Instant::now());
                    cx.notify();
                });
                if updated.is_err() {
                    return;
                }
                cx.background_executor().timer(SAMPLE_INTERVAL).await;
            }
        });
        Self {
            state: UsageState::default(),
            _sampling: sampling,
        }
    }

    fn sections(&self, item: UsageItem) -> Vec<Section> {
        let Some(usage) = self.state.latest() else {
            return Vec::new();
        };
        match item {
            UsageItem::Network => network_sections(usage),
            UsageItem::Resources => usage
                .process
                .as_ref()
                .map(resource_sections)
                .unwrap_or_default(),
        }
    }
}

impl Render for ProcessUsage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(usage) = self.state.latest() else {
            return div().into_any_element();
        };
        let source = cx.entity();
        let item = |id: &'static str, text: String, item: UsageItem| {
            let source = source.clone();
            div().id(id).child(text).tooltip(move |window, cx| {
                let content = cx.new(|cx| UsageTooltip::new(source.clone(), item, cx));
                Tooltip::element(move |_, _| content.clone()).build(window, cx)
            })
        };
        let mut items = h_flex().gap_2().items_center().child(item(
            "status-network",
            network_text(usage.rate),
            UsageItem::Network,
        ));
        if let Some(process) = &usage.process {
            items = items
                .child(status_separator(separator_color(cx)))
                .child(item(
                    "status-resources",
                    resource_text(process),
                    UsageItem::Resources,
                ));
        }
        items.into_any_element()
    }
}

/// A live view of one tooltip table: the kit builds a tooltip once when it first shows, so a
/// table baked into it would freeze. This entity re-renders on every sample instead.
struct UsageTooltip {
    source: Entity<ProcessUsage>,
    item: UsageItem,
}

impl UsageTooltip {
    fn new(source: Entity<ProcessUsage>, item: UsageItem, cx: &mut Context<Self>) -> Self {
        cx.observe(&source, |_, _, cx| cx.notify()).detach();
        Self { source, item }
    }
}

impl Render for UsageTooltip {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let sections = self.source.read(cx).sections(self.item);
        details_table(sections, cx.theme().muted_foreground)
    }
}

#[cfg(test)]
#[path = "process_usage_tests.rs"]
mod process_usage_tests;
