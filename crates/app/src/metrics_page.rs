//! The Metrics page of the Settings window (spec 0048): where the Monitor tab reads its 7- and 30-day
//! history and Topology reads traffic. The page lists the Prometheus-compatible services the active
//! cluster runs, takes one by hand, tests it, and saves it for the cluster. It reaches the cluster
//! only through the `ActiveConnection` the shell publishes and stores no credential.

use cluster::{
    ClusterError, MetricsCandidate, MetricsError, MetricsScheme, MetricsSource, MetricsSourceError,
    MetricsSourceFields, SourceCheck, metrics_candidates,
};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::radio::{Radio, RadioGroup};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Sizable as _, StyledExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render,
    SharedString, Styled as _, Subscription, Task, WeakEntity, Window, div, px,
};

use crate::active_session::ActiveConnection;
use crate::cluster_form::edit_entry;
use crate::cluster_metrics::SourceState;
use crate::cluster_registry::ClusterRef;
use crate::cluster_runtime::ClusterRuntime;
use crate::cluster_session::{ClusterSession, error_text};
use crate::settings::AppSettings;
use crate::status_tone::{StatusTone, tone_color};
use crate::usage_format::group_digits;

#[cfg(test)]
#[path = "metrics_page_tests.rs"]
mod metrics_page_tests;

const LABEL_WIDTH: f32 = 110.;
const SHORT_FIELD_WIDTH: f32 = 90.;
const FIELD_WIDTH: f32 = 170.;
const METRICS_SERVER_ONLY: &str = "metrics-server only";
const METRICS_SERVER_DETAIL: &str = "CPU and memory sampled by k8sBoard while it runs; 24 hours";
const PAGE_INTRO: &str = "Where the Monitor tab reads 7- and 30-day history and Topology reads traffic. k8sBoard reaches it through the API server service proxy with your kubeconfig credentials and stores no credential.";
const NO_CLUSTER_TEXT: &str = "Connect to a cluster to choose its metrics source.";

/// Marks the launch as `--screen settings-metrics-fixture`: the page shows fixed data.#[cfg(feature = "screenshot")]pub(crate) struct MetricsFixture;#[cfg(feature = "screenshot")]impl gpui_kit::Global for MetricsFixture {}
/// Marks the launch as `--screen settings-metrics-fixture`: the page shows fixed data.
#[cfg(feature = "screenshot")]
pub(crate) struct MetricsFixture;

#[cfg(feature = "screenshot")]
impl gpui_kit::Global for MetricsFixture {}

/// What the radio list has selected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Choice {
    ServerOnly,
    Candidate(usize),
    Other,
}

enum Detection {
    Idle,
    Running { _task: Task<()> },
    Done(Vec<MetricsCandidate>),
    Failed(String),
}

enum TestResult {
    Idle,
    Running { _task: Task<()> },
    Done(Result<SourceCheck, MetricsError>),
}

/// The four text fields of `Other service` and their messages, in form order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OtherField {
    Namespace,
    Service,
    Port,
    Prefix,
}

impl OtherField {
    const ALL: [Self; 4] = [Self::Namespace, Self::Service, Self::Port, Self::Prefix];

    fn error(self) -> MetricsSourceError {
        match self {
            Self::Namespace => MetricsSourceError::Namespace,
            Self::Service => MetricsSourceError::Service,
            Self::Port => MetricsSourceError::Port,
            Self::Prefix => MetricsSourceError::Prefix,
        }
    }
}

/// The fields of `fields` that do not validate. Each is judged alone: the others are replaced by
/// values that pass, so one message does not hide another.
fn invalid_fields(fields: &MetricsSourceFields) -> Vec<OtherField> {
    let valid = MetricsSourceFields {
        namespace: "a".to_owned(),
        service: "a".to_owned(),
        port: "80".to_owned(),
        scheme: fields.scheme,
        prefix: String::new(),
    };
    OtherField::ALL
        .into_iter()
        .filter(|field| {
            let mut probe = valid.clone();
            match field {
                OtherField::Namespace => probe.namespace.clone_from(&fields.namespace),
                OtherField::Service => probe.service.clone_from(&fields.service),
                OtherField::Port => probe.port.clone_from(&fields.port),
                OtherField::Prefix => probe.prefix.clone_from(&fields.prefix),
            }
            MetricsSource::new(&probe).err() == Some(field.error())
        })
        .collect()
}

/// The initial choice from the saved source: a candidate it equals, else `Other service`.
fn initial_choice(saved: Option<&MetricsSourceFields>, candidates: &[MetricsCandidate]) -> Choice {
    let Some(saved) = saved else {
        return Choice::ServerOnly;
    };
    candidates
        .iter()
        .position(|candidate| candidate.fields == *saved)
        .map_or(Choice::Other, Choice::Candidate)
}

/// The Test result line and its tone.
fn test_line(result: &Result<SourceCheck, MetricsError>) -> (String, StatusTone) {
    match result {
        Ok(check) if check.cpu_series == 0 => (
            "Reachable, but it has no container_cpu_usage_seconds_total series".to_owned(),
            StatusTone::Warn,
        ),
        Ok(check) => (
            format!(
                "Reachable · {} ms · {} CPU series",
                check.latency.as_millis(),
                group_digits(usize::try_from(check.cpu_series).unwrap_or(usize::MAX))
            ),
            StatusTone::Ok,
        ),
        Err(error) => (error.to_string(), StatusTone::Bad),
    }
}

/// The `Saved:` line: the stored source and what the session made of it. `None` when nothing is
/// saved.
fn saved_line(saved: Option<&MetricsSourceFields>, state: Option<&SourceState>) -> Option<String> {
    let saved = saved?;
    let Ok(stored) = MetricsSource::new(saved) else {
        return Some("Saved: invalid entry in settings".to_owned());
    };
    // The state names the source it checked, which can briefly lag the stored one.
    let (source, shown) = match state {
        Some(SourceState::Checking { source, .. }) => (source, "Checking…".to_owned()),
        Some(SourceState::Ready { source, check }) if check.cpu_series == 0 => {
            (source, "Ready · no container CPU series".to_owned())
        }
        Some(SourceState::Ready { source, .. }) => (source, "Ready".to_owned()),
        Some(SourceState::Failed { source, error }) => (source, format!("Failed: {error}")),
        Some(SourceState::Invalid) => (&stored, "Invalid entry in settings".to_owned()),
        Some(SourceState::None) | None => return Some(format!("Saved: {}", stored.display())),
    };
    Some(format!("Saved: {} · {shown}", source.display()))
}

fn candidate_detail(candidate: &MetricsCandidate) -> String {
    MetricsSource::new(&candidate.fields).map_or_else(|_| String::new(), |source| source.display())
}

pub(crate) struct MetricsPage {
    /// The cluster the state below belongs to; `None` without an open one.
    cluster: Option<ClusterRef>,
    label: String,
    session: Option<WeakEntity<ClusterSession>>,
    detection: Detection,
    choice: Choice,
    /// The user picked a row, so the saved source no longer pre-selects one.
    has_chosen: bool,
    namespace: Entity<InputState>,
    service: Entity<InputState>,
    port: Entity<InputState>,
    prefix: Entity<InputState>,
    scheme: MetricsScheme,
    test: TestResult,
    /// A screenshot fixture: nothing is read or sent.
    is_fixture: bool,
    _session_observer: Option<Subscription>,
    _observers: Vec<Subscription>,
}

impl MetricsPage {
    /// The page of the Settings window; the fixture under `--screen settings-metrics-fixture`.
    pub(crate) fn open(window: &mut Window, cx: &mut Context<Self>) -> Self {
        #[cfg(feature = "screenshot")]
        if cx.has_global::<MetricsFixture>() {
            return Self::fixture(window, cx);
        }
        Self::new(window, cx)
    }

    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut page = Self::empty(window, cx);
        page.follow_connection(window, cx);
        page
    }

    /// The `settings-metrics-fixture` screen: no cluster, a fixed candidate list, `Other service`
    /// filled, and a passed Test.
    #[cfg(any(feature = "screenshot", test))]
    pub(crate) fn fixture(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut page = Self::empty(window, cx);
        page.is_fixture = true;
        page.label = "readonly@Monitor".to_owned();
        page.cluster = Some(ClusterRef {
            kubeconfig: std::path::PathBuf::from("fixture.yaml"),
            context: "readonly@Monitor".to_owned(),
        });
        let candidate = |flavor, service: &str, port: &str, prefix: &str| MetricsCandidate {
            flavor,
            fields: MetricsSourceFields {
                namespace: "monitoring".to_owned(),
                service: service.to_owned(),
                port: port.to_owned(),
                scheme: MetricsScheme::Http,
                prefix: prefix.to_owned(),
            },
        };
        page.detection = Detection::Done(vec![
            candidate(
                cluster::MetricsFlavor::VictoriaMetricsCluster,
                "vmselect-vm-victoria-metrics-k8s-stack",
                "8481",
                "/select/0/prometheus",
            ),
            candidate(
                cluster::MetricsFlavor::VictoriaMetricsQuery,
                "vmquery-vm-victoria-metrics-k8s-stack",
                "8481",
                "/select/0/prometheus",
            ),
            candidate(
                cluster::MetricsFlavor::Prometheus,
                "prometheus-operated",
                "9090",
                "",
            ),
        ]);
        page.choice = Choice::Other;
        page.set_inputs(
            &MetricsSourceFields {
                namespace: "monitoring".to_owned(),
                service: "thanos-query".to_owned(),
                port: "10902".to_owned(),
                scheme: MetricsScheme::Http,
                prefix: String::new(),
            },
            window,
            cx,
        );
        page.test = TestResult::Done(Ok(SourceCheck {
            latency: std::time::Duration::from_millis(35),
            cpu_series: 1_234,
        }));
        page
    }

    fn empty(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = |placeholder: &'static str, window: &mut Window, cx: &mut Context<Self>| {
            cx.new(|cx| InputState::new(window, cx).placeholder(placeholder))
        };
        let namespace = input("namespace", window, cx);
        let service = input("service", window, cx);
        let port = input("port", window, cx);
        let prefix = input("path prefix", window, cx);
        let mut observers =
            vec![
                cx.observe_global_in::<ActiveConnection>(window, |page, window, cx| {
                    if !page.is_fixture {
                        page.follow_connection(window, cx);
                    }
                }),
            ];
        observers.push(cx.observe_global::<AppSettings>(|_, cx| cx.notify()));
        for input in [&namespace, &service, &port, &prefix] {
            observers.push(cx.subscribe(input, |page, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    page.on_other_changed(cx);
                }
            }));
        }
        Self {
            cluster: None,
            label: String::new(),
            session: None,
            detection: Detection::Idle,
            choice: Choice::ServerOnly,
            has_chosen: false,
            namespace,
            service,
            port,
            prefix,
            scheme: MetricsScheme::Http,
            test: TestResult::Idle,
            is_fixture: false,
            _session_observer: None,
            _observers: observers,
        }
    }

    /// The published connection changed: a new cluster resets the page and detects again; the same
    /// cluster after a reconnect only keeps the observer of its session current.
    fn follow_connection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let published = cx.try_global::<ActiveConnection>().map(|active| {
            (
                active.cluster.clone(),
                active.label.clone(),
                active.session.clone(),
            )
        });
        let Some((cluster, label, session)) = published else {
            self.cluster = None;
            self.session = None;
            self._session_observer = None;
            self.detection = Detection::Idle;
            self.test = TestResult::Idle;
            cx.notify();
            return;
        };
        self.label = label;
        self.session = Some(session.clone());
        self._session_observer = session
            .upgrade()
            .map(|session| cx.observe(&session, |_, _, cx| cx.notify()));
        if self.cluster.as_ref() == Some(&cluster) {
            cx.notify();
            return;
        }
        self.cluster = Some(cluster);
        self.has_chosen = false;
        self.test = TestResult::Idle;
        let saved = self.saved_fields(cx);
        self.choice = initial_choice(saved.as_ref(), &[]);
        if let Some(saved) = &saved {
            self.set_inputs(saved, window, cx);
        }
        self.start_detection(cx);
        cx.notify();
    }

    fn set_inputs(
        &mut self,
        fields: &MetricsSourceFields,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let set = |input: &Entity<InputState>, text: &str, window: &mut Window, cx: &mut App| {
            input.update(cx, |state, cx| state.set_value(text.to_owned(), window, cx));
        };
        set(&self.namespace, &fields.namespace, window, cx);
        set(&self.service, &fields.service, window, cx);
        set(&self.port, &fields.port, window, cx);
        set(&self.prefix, &fields.prefix, window, cx);
        self.scheme = fields.scheme;
    }

    /// What Settings stores for the open cluster.
    fn saved_fields(&self, cx: &App) -> Option<MetricsSourceFields> {
        let cluster = self.cluster.as_ref()?;
        AppSettings::get(cx)
            .registry
            .clusters
            .iter()
            .find(|entry| entry.cluster == *cluster)
            .and_then(|entry| entry.metrics.clone())
    }

    fn start_detection(&mut self, cx: &mut Context<Self>) {
        if self.is_fixture {
            return;
        }
        let Some(connection) = cx
            .try_global::<ActiveConnection>()
            .map(|active| active.connection.clone())
        else {
            return;
        };
        let runtime = cx.global::<ClusterRuntime>().clone();
        let listing = runtime.spawn(async move { connection.list_all_services().await });
        let task = cx.spawn(async move |this, cx| {
            let result = listing.await;
            let _ = this.update(cx, |page, cx| page.finish_detection(result, cx));
        });
        self.detection = Detection::Running { _task: task };
        cx.notify();
    }

    fn finish_detection(
        &mut self,
        result: Result<Result<Vec<cluster::ServiceSummary>, ClusterError>, tokio::task::JoinError>,
        cx: &mut Context<Self>,
    ) {
        self.detection = match result {
            Ok(Ok(services)) => Detection::Done(metrics_candidates(&services)),
            Ok(Err(error)) => Detection::Failed(error_text(&error)),
            Err(_) => Detection::Failed("the service listing stopped unexpectedly".to_owned()),
        };
        if !self.has_chosen {
            let saved = self.saved_fields(cx);
            let candidates = self.candidates();
            self.choice = initial_choice(saved.as_ref(), candidates);
        }
        cx.notify();
    }

    fn candidates(&self) -> &[MetricsCandidate] {
        match &self.detection {
            Detection::Done(candidates) => candidates,
            Detection::Idle | Detection::Running { .. } | Detection::Failed(_) => &[],
        }
    }

    fn on_other_changed(&mut self, cx: &mut Context<Self>) {
        // Typing in a field means the user means `Other service`.
        self.choice = Choice::Other;
        self.has_chosen = true;
        self.test = TestResult::Idle;
        cx.notify();
    }

    fn choose(&mut self, index: usize, cx: &mut Context<Self>) {
        let candidates = self.candidates().len();
        self.choice = match index {
            0 => Choice::ServerOnly,
            index if index <= candidates => Choice::Candidate(index - 1),
            _ => Choice::Other,
        };
        self.has_chosen = true;
        self.test = TestResult::Idle;
        cx.notify();
    }

    /// The text fields as stored fields (not yet validated).
    fn other_fields(&self, cx: &App) -> MetricsSourceFields {
        MetricsSourceFields {
            namespace: self.namespace.read(cx).value().trim().to_owned(),
            service: self.service.read(cx).value().trim().to_owned(),
            port: self.port.read(cx).value().trim().to_owned(),
            scheme: self.scheme,
            prefix: self.prefix.read(cx).value().trim().to_owned(),
        }
    }

    /// The selected choice as what would be saved; `Err` while `Other service` does not validate.
    fn selected(&self, cx: &App) -> Result<Option<MetricsSource>, MetricsSourceError> {
        match self.choice {
            Choice::ServerOnly => Ok(None),
            Choice::Candidate(index) => match self.candidates().get(index) {
                Some(candidate) => MetricsSource::new(&candidate.fields).map(Some),
                None => Ok(None),
            },
            Choice::Other => MetricsSource::new(&self.other_fields(cx)).map(Some),
        }
    }

    fn start_test(&mut self, cx: &mut Context<Self>) {
        if self.is_fixture {
            return;
        }
        let Ok(Some(source)) = self.selected(cx) else {
            return;
        };
        let Some(connection) = cx
            .try_global::<ActiveConnection>()
            .map(|active| active.connection.clone())
        else {
            return;
        };
        let runtime = cx.global::<ClusterRuntime>().clone();
        let checking = runtime.spawn(async move { connection.check_metrics_source(&source).await });
        let task = cx.spawn(async move |this, cx| {
            let result = checking.await.unwrap_or_else(|_| {
                Err(MetricsError::Unexpected(
                    "the test stopped unexpectedly".to_owned(),
                ))
            });
            let _ = this.update(cx, |page, cx| {
                page.test = TestResult::Done(result);
                cx.notify();
            });
        });
        self.test = TestResult::Running { _task: task };
        cx.notify();
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        let (Some(cluster), Ok(selected)) = (self.cluster.clone(), self.selected(cx)) else {
            return;
        };
        let stored = selected.map(|source| source.fields());
        AppSettings::update(cx, |settings| {
            edit_entry(&mut settings.registry, &cluster, |entry| {
                entry.metrics = stored;
            });
        });
        cx.notify();
    }

    fn pick_scheme(&mut self, scheme: MetricsScheme, cx: &mut Context<Self>) {
        self.scheme = scheme;
        self.on_other_changed(cx);
    }

    /// Whether the page shows what a screenshot waits for: a fixture, or a connected cluster whose
    /// detection ended.
    #[cfg(any(feature = "screenshot", test))]
    pub(crate) fn is_settled(&self) -> bool {
        self.is_fixture
            || (self.cluster.is_some()
                && matches!(self.detection, Detection::Done(_) | Detection::Failed(_)))
    }
}

fn muted(text: impl Into<SharedString>, cx: &App) -> gpui_kit::Div {
    div()
        .text_sm()
        .text_color(cx.theme().muted_foreground)
        .child(text.into())
}

impl MetricsPage {
    fn render_choices(&self, cx: &mut Context<Self>) -> AnyElement {
        let mono = cx.theme().mono_font_family.clone();
        let muted_color = cx.theme().muted_foreground;
        let mut rows: Vec<(SharedString, String)> =
            vec![(METRICS_SERVER_ONLY.into(), METRICS_SERVER_DETAIL.to_owned())];
        rows.extend(self.candidates().iter().map(|candidate| {
            (
                SharedString::from(candidate.flavor.label()),
                candidate_detail(candidate),
            )
        }));
        rows.push(("Other service".into(), String::new()));
        let selected = match self.choice {
            Choice::ServerOnly => 0,
            Choice::Candidate(index) => index + 1,
            Choice::Other => rows.len() - 1,
        };
        RadioGroup::vertical("metrics-source")
            .selected_index(Some(selected))
            .on_change(cx.listener(|page, index: &usize, _, cx| page.choose(*index, cx)))
            .children(
                rows.into_iter()
                    .enumerate()
                    .map(|(index, (label, detail))| {
                        let is_mono = index != 0;
                        Radio::new(("metrics-choice", index)).label(label).child(
                            div()
                                .text_xs()
                                .text_color(muted_color)
                                .when(is_mono, |detail_row| detail_row.font_family(mono.clone()))
                                .child(detail),
                        )
                    }),
            )
            .into_any_element()
    }

    fn render_other(&self, cx: &mut Context<Self>) -> AnyElement {
        let fields = self.other_fields(cx);
        let invalid = invalid_fields(&fields);
        let danger = cx.theme().danger;
        let message = |field: OtherField, text: &str| {
            let is_shown =
                self.choice == Choice::Other && !text.is_empty() && invalid.contains(&field);
            is_shown.then(|| {
                div()
                    .text_xs()
                    .text_color(danger)
                    .child(field.error().to_string())
            })
        };
        let scheme = self.scheme;
        let page = cx.entity();
        let scheme_menu = Button::new("metrics-scheme")
            .small()
            .outline()
            .label(match scheme {
                MetricsScheme::Http => "http",
                MetricsScheme::Https => "https",
            })
            .dropdown_caret(true)
            .dropdown_menu(move |menu, _, _| {
                [
                    (MetricsScheme::Http, "http"),
                    (MetricsScheme::Https, "https"),
                ]
                .into_iter()
                .fold(menu, |menu, (choice, label)| {
                    let page = page.clone();
                    menu.item(
                        PopupMenuItem::new(label)
                            .checked(choice == scheme)
                            .on_click(move |_, _, cx| {
                                page.update(cx, |page, cx| page.pick_scheme(choice, cx));
                            }),
                    )
                })
            });
        v_flex()
            .gap_1()
            .pl(px(LABEL_WIDTH / 4.))
            .child(
                h_flex()
                    .gap_2()
                    .items_start()
                    .child(div().w(px(FIELD_WIDTH)).child(Input::new(&self.namespace)))
                    .child(div().w(px(FIELD_WIDTH)).child(Input::new(&self.service)))
                    .child(div().w(px(SHORT_FIELD_WIDTH)).child(Input::new(&self.port)))
                    .child(scheme_menu)
                    .child(div().w(px(FIELD_WIDTH)).child(Input::new(&self.prefix))),
            )
            .children(message(OtherField::Namespace, &fields.namespace))
            .children(message(OtherField::Service, &fields.service))
            .children(message(OtherField::Port, &fields.port))
            .children(message(OtherField::Prefix, &fields.prefix))
            .into_any_element()
    }

    fn render_detection_note(&self, cx: &App) -> Option<AnyElement> {
        match &self.detection {
            Detection::Running { .. } => Some(muted("Detecting…", cx).into_any_element()),
            Detection::Failed(reason) => Some(
                muted(
                    format!("Cannot list services: {reason}. Enter the service below."),
                    cx,
                )
                .into_any_element(),
            ),
            Detection::Done(candidates) if candidates.is_empty() => Some(
                muted("No metrics service found. Enter the service below.", cx).into_any_element(),
            ),
            Detection::Done(_) | Detection::Idle => None,
        }
    }

    fn render_actions(&self, cx: &mut Context<Self>) -> AnyElement {
        let selection = self.selected(cx);
        let can_run = matches!(selection, Ok(Some(_)));
        let can_save = selection.is_ok();
        let is_detecting = matches!(self.detection, Detection::Running { .. });
        let is_testing = matches!(self.test, TestResult::Running { .. });
        let result = match &self.test {
            TestResult::Idle => None,
            TestResult::Running { .. } => Some(muted("Testing…", cx).into_any_element()),
            TestResult::Done(outcome) => {
                let (text, tone) = test_line(outcome);
                Some(
                    div()
                        .text_sm()
                        .text_color(tone_color(tone, cx))
                        .child(text)
                        .into_any_element(),
                )
            }
        };
        h_flex()
            .gap_3()
            .items_center()
            .child(
                Button::new("metrics-detect")
                    .small()
                    .label("Detect again")
                    .disabled(is_detecting || self.is_fixture)
                    .on_click(cx.listener(|page, _, _, cx| page.start_detection(cx))),
            )
            .child(
                Button::new("metrics-test")
                    .small()
                    .label("Test")
                    .disabled(!can_run || is_testing)
                    .on_click(cx.listener(|page, _, _, cx| page.start_test(cx))),
            )
            .children(result)
            .child(div().flex_1())
            .child(
                Button::new("metrics-save")
                    .small()
                    .primary()
                    .label("Save")
                    .disabled(!can_save || self.is_fixture)
                    .on_click(cx.listener(|page, _, _, cx| page.save(cx))),
            )
            .into_any_element()
    }
}

impl Render for MetricsPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.cluster.is_none() {
            return muted(NO_CLUSTER_TEXT, cx).into_any_element();
        }
        let saved = self.saved_fields(cx);
        let session = self.session.as_ref().and_then(|session| session.upgrade());
        let live_state = session
            .as_ref()
            .and_then(|session| session.read(cx).live())
            .map(|live| &live.metrics.source);
        let saved_text = saved_line(saved.as_ref(), live_state);
        v_flex()
            .w_full()
            .gap_3()
            .child(
                div()
                    .text_sm()
                    .font_semibold()
                    .child(format!("Metrics · {}", self.label)),
            )
            .child(muted(PAGE_INTRO, cx))
            .child(self.render_choices(cx))
            .child(self.render_other(cx))
            .children(self.render_detection_note(cx))
            .child(self.render_actions(cx))
            .children(saved_text.map(|text| muted(text, cx)))
            .into_any_element()
    }
}
