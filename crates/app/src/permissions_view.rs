//! The Check permissions dialog: what You (the API server's answer, SelfSubjectRulesReview) or
//! a service account, user, or group (client-side from RBAC objects) can do, as a verbs-by-resource
//! table, plus a one-line "can … ?" question.

use cluster::{
    AccessDecision, AccessRequest, EffectiveRule, Identity, RbacSnapshot, RequestTarget,
    RulesReview,
};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::select::{Select, SelectState};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{
    ActiveTheme as _, IndexPath, Sizable as _, StyledExt as _, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _,
    Subscription, Task, WeakEntity, Window, div, px,
};

use crate::access_bindings::{binding_key, binding_text, role_key, role_text, subject_text};
use crate::access_query::{QueryError, SubjectQuery, parse_request, parse_subject};
use crate::app_shell::AppShell;
use crate::cluster_runtime::ClusterRuntime;
use crate::cluster_session::{ClusterSession, RbacState, error_text};
use crate::permission_table::{PermissionTable, TABLE_VERBS, VerbCell, permission_table};
use crate::status_tone::{StatusTone, tone_color};
use crate::table_selection::ResourceKey;
use crate::who_can_view::{
    ALL_NAMESPACES, RBAC_CAVEATS, clock_text, coverage_notes, namespace_options,
};

const RESULT_MAX_HEIGHT: f32 = 300.;
const VERB_SLOT: f32 = 40.;
const OTHER_SLOT: f32 = 80.;
const VERB_HEADS: [&str; 8] = [
    "get", "list", "watch", "create", "update", "patch", "delete", "delcol",
];
const YOU_CAVEATS: &str = "The API server's own answer for your credentials. Webhook authorizers can leave it incomplete.";
const USER_CAVEAT: &str = " Only bindings to this user name and system:authenticated count.";
const ANONYMOUS_CAVEAT: &str = " Only bindings to this user name and system:unauthenticated count.";
const MASTERS_NOTE: &str = "Members of group system:masters are always allowed: RBAC is bypassed, so there is no rules table.";

/// A request that runs on the cluster runtime. Replacing the state drops the task, which aborts
/// the request; closing the dialog does the same.
pub(crate) enum RequestState<T> {
    Idle,
    Loading { _task: Task<()> },
    Ready(T),
    Failed(String),
}

/// What a table that is not ready shows.
#[derive(Debug, PartialEq, Eq)]
enum Waiting {
    Nothing,
    Muted(&'static str),
    Bad(String),
    /// Another subject: the RBAC snapshot's own state decides.
    Snapshot,
}

fn waiting_line<T>(table: &RequestState<T>, is_you: bool) -> Waiting {
    if let RequestState::Failed(message) = table {
        return Waiting::Bad(message.clone());
    }
    if !is_you {
        return Waiting::Snapshot;
    }
    match table {
        RequestState::Loading { .. } => Waiting::Muted("Asking the API server…"),
        _ => Waiting::Nothing,
    }
}

/// One answer line: Yes or No, with the reason or the grant behind it.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Answer {
    tone: StatusTone,
    text: String,
}

/// A binding that gives the subject some of the rules shown.
#[derive(Clone, Debug, PartialEq, Eq)]
struct GrantedBy {
    binding: ResourceKey,
    binding_text: String,
    role: Option<ResourceKey>,
    role_text: String,
    via: String,
}

struct ShownTable {
    table: PermissionTable,
    granted_by: Vec<GrantedBy>,
    /// `No permissions in {namespace}.` / `… cluster-wide`.
    scope_text: String,
    source: String,
    /// When the RBAC snapshot was listed: set for the client-side evaluation, which can refresh.
    listed_at: Option<jiff::Timestamp>,
    warnings: Vec<SharedString>,
    caveats: String,
    is_masters: bool,
}

struct Checked {
    subject: SubjectQuery,
    /// What the table covers: always set for You, `None` is cluster-wide grants for the others.
    namespace: Option<String>,
}

pub(crate) struct PermissionsView {
    session: WeakEntity<ClusterSession>,
    shell: WeakEntity<AppShell>,
    subject: Entity<InputState>,
    namespace: Entity<SelectState<Vec<String>>>,
    ask: Entity<InputState>,
    /// The namespaces the Select lists, without the cluster-wide option.
    listed: Vec<String>,
    /// Whether the Select offers `All namespaces`: not for You, who is reviewed in one namespace.
    has_cluster_wide_option: bool,
    checked: Option<Result<Checked, QueryError>>,
    table: RequestState<ShownTable>,
    question: Option<Result<AccessRequest, QueryError>>,
    answer: RequestState<Answer>,
    link_count: usize,
    _subscriptions: Vec<Subscription>,
}

/// The Select options: `All namespaces` (cluster-wide grants) is for other subjects only, since
/// You is reviewed in one namespace.
fn permission_namespace_options(
    listed: Vec<String>,
    wanted: Option<&str>,
    is_you: bool,
) -> Vec<String> {
    let mut names = namespace_options(listed, wanted);
    if is_you {
        names.remove(0);
    }
    names
}

/// The namespaces You can pick: the session's namespaces list, or the scope's own namespaces when
/// that list is not available (a user who may not list namespaces).
fn you_namespaces(listed: Option<&[String]>, scope: &[String]) -> Vec<String> {
    match listed {
        Some(listed) => listed.to_vec(),
        None => scope.to_vec(),
    }
}

/// `Incomplete: rules shown are granted; others may be missing ({error})`.
fn incomplete_note(review: &RulesReview) -> Option<String> {
    if !review.is_incomplete {
        return None;
    }
    let detail = match &review.evaluation_error {
        Some(error) => format!(" ({error})"),
        None => String::new(),
    };
    Some(format!(
        "Incomplete: rules shown are granted; others may be missing{detail}"
    ))
}

fn you_answer(decision: &AccessDecision) -> Answer {
    match decision {
        AccessDecision::Allowed => Answer {
            tone: StatusTone::Ok,
            text: "Yes".to_owned(),
        },
        AccessDecision::Denied {
            reason: Some(reason),
        } => Answer {
            tone: StatusTone::Bad,
            text: format!("No: {reason}"),
        },
        AccessDecision::Denied { reason: None } => Answer {
            tone: StatusTone::Bad,
            text: "No".to_owned(),
        },
    }
}

/// `first`: the binding and role text of the first grant.
fn other_answer(first: Option<(&str, &str)>) -> Answer {
    match first {
        Some((binding, role)) => Answer {
            tone: StatusTone::Ok,
            text: format!("Yes · via {binding} → {role}"),
        },
        None => Answer {
            tone: StatusTone::Bad,
            text: "No RBAC binding grants this.".to_owned(),
        },
    }
}

fn masters_answer() -> Answer {
    Answer {
        tone: StatusTone::Ok,
        text: "Yes · group system:masters is always allowed (bypasses RBAC)".to_owned(),
    }
}

/// One row per contributing binding, in rule order.
fn granted_by(rules: &[EffectiveRule<'_>]) -> Vec<GrantedBy> {
    let mut rows: Vec<GrantedBy> = Vec::new();
    for effective in rules {
        let binding = binding_text(effective.binding);
        if rows.iter().any(|row| row.binding_text == binding) {
            continue;
        }
        rows.push(GrantedBy {
            binding: binding_key(effective.binding),
            binding_text: binding,
            role: role_key(effective.binding),
            role_text: role_text(&effective.binding.role),
            via: subject_text(effective.subject),
        });
    }
    rows
}

/// The client-side table of `identity` in `namespace` (`None`: ClusterRoleBindings only).
fn other_table(
    snapshot: &RbacSnapshot,
    listed_at: jiff::Timestamp,
    subject: &SubjectQuery,
    identity: &Identity,
    namespace: Option<&str>,
) -> ShownTable {
    let rules = snapshot.rules_of(identity, namespace);
    let mut caveats = RBAC_CAVEATS.to_owned();
    caveats.push_str(user_caveat(subject));
    ShownTable {
        table: permission_table(rules.iter().map(|effective| effective.rule)),
        granted_by: granted_by(&rules),
        scope_text: scope_text(namespace),
        source: format!(
            "Computed from RBAC objects listed at {}",
            clock_text(listed_at)
        ),
        listed_at: Some(listed_at),
        warnings: coverage_notes(&snapshot.coverage, namespace),
        caveats,
        is_masters: false,
    }
}

/// What a user query leaves out: its other groups. The anonymous user is unauthenticated.
fn user_caveat(subject: &SubjectQuery) -> &'static str {
    match subject {
        SubjectQuery::Other { text, .. } if text == "user system:anonymous" => ANONYMOUS_CAVEAT,
        SubjectQuery::Other { text, .. } if text.starts_with("user ") => USER_CAVEAT,
        _ => "",
    }
}

fn scope_text(namespace: Option<&str>) -> String {
    match namespace {
        Some(namespace) => format!("in {namespace}"),
        None => "cluster-wide".to_owned(),
    }
}

fn masters_table() -> ShownTable {
    ShownTable {
        table: PermissionTable::default(),
        granted_by: Vec::new(),
        scope_text: String::new(),
        source: "System group, not an RBAC grant".to_owned(),
        listed_at: None,
        warnings: Vec::new(),
        caveats: RBAC_CAVEATS.to_owned(),
        is_masters: true,
    }
}

fn you_table(namespace: &str, review: &RulesReview) -> ShownTable {
    ShownTable {
        table: permission_table(&review.rules),
        granted_by: Vec::new(),
        scope_text: scope_text(Some(namespace)),
        source: format!("From SelfSubjectRulesReview for {namespace}"),
        listed_at: None,
        warnings: incomplete_note(review)
            .map(SharedString::from)
            .into_iter()
            .collect(),
        caveats: YOU_CAVEATS.to_owned(),
        is_masters: false,
    }
}

impl PermissionsView {
    /// `subject`: the text to prefill (`None` is You); `namespace`: the preselected one.
    /// `check_now` and `ask` run at once.
    pub(crate) fn new(
        shell: WeakEntity<AppShell>,
        session: &Entity<ClusterSession>,
        subject: Option<String>,
        namespace: Option<String>,
        check_now: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut listed = Vec::new();
        let mut default_namespace = None;
        if let Some(live) = session.read(cx).live() {
            default_namespace = Some(live.default_namespace().to_owned());
            let ready: Option<Vec<String>> = live
                .namespaces
                .ready_items()
                .map(|items| items.iter().map(|item| item.name.clone()).collect());
            listed = you_namespaces(ready.as_deref(), live.scope.namespaces());
        }
        let is_you = subject
            .as_deref()
            .is_none_or(|text| matches!(parse_subject(text), Ok(SubjectQuery::You)));
        // You is reviewed in one namespace, the context default unless one was asked for.
        let wanted = match is_you {
            true => namespace.clone().or_else(|| default_namespace.clone()),
            false => namespace.clone(),
        };
        let names = permission_namespace_options(listed.clone(), wanted.as_deref(), is_you);
        let selected = wanted
            .as_deref()
            .and_then(|wanted| names.iter().position(|name| name == wanted))
            .unwrap_or(0);
        let namespace = cx.new(|cx| {
            SelectState::new(names, Some(IndexPath::default().row(selected)), window, cx)
                .searchable(true)
        });
        let subject_input = cx.new(|cx| {
            let mut input = InputState::new(window, cx)
                .placeholder("You — or sa ns/name, user name, group name");
            if let Some(subject) = &subject {
                input.set_value(subject.clone(), window, cx);
            }
            input
        });
        let ask = cx.new(|cx| {
            InputState::new(window, cx).placeholder("list pods, get secrets.apps, delete /metrics")
        });
        let subscriptions = vec![
            cx.subscribe_in(
                &subject_input,
                window,
                |view, _, event, window, cx| match event {
                    InputEvent::PressEnter { .. } => view.check(cx),
                    InputEvent::Change => view.sync_namespace_options(window, cx),
                    _ => {}
                },
            ),
            cx.subscribe_in(&ask, window, |view, _, event, _, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    view.ask(cx);
                }
            }),
            cx.observe(session, |view, _, cx| view.evaluate(cx)),
        ];
        let mut view = Self {
            session: session.downgrade(),
            shell,
            subject: subject_input,
            namespace,
            ask,
            listed,
            has_cluster_wide_option: !is_you,
            checked: None,
            table: RequestState::Idle,
            question: None,
            answer: RequestState::Idle,
            link_count: 0,
            _subscriptions: subscriptions,
        };
        if check_now {
            view.check(cx);
        }
        view
    }

    /// Whether an answer is still on its way: the capture waits on it.
    #[cfg(feature = "screenshot")]
    pub(crate) fn is_pending(&self, cx: &gpui_kit::App) -> bool {
        let is_loading = matches!(self.table, RequestState::Loading { .. })
            || matches!(self.answer, RequestState::Loading { .. });
        if is_loading {
            return true;
        }
        let waits_for_snapshot = matches!(
            self.checked,
            Some(Ok(Checked {
                subject: SubjectQuery::Other { .. },
                ..
            }))
        ) && matches!(self.table, RequestState::Idle);
        waits_for_snapshot
            && self
                .session
                .upgrade()
                .and_then(|session| {
                    session.read(cx).live().map(|live| {
                        matches!(live.rbac, RbacState::Idle | RbacState::Loading { .. })
                    })
                })
                .unwrap_or(false)
    }

    /// Rebuilds the namespace options when the subject switches between You and another subject.
    /// The pick stays when the new list has it; You falls back to the context default.
    fn sync_namespace_options(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let is_you = matches!(
            parse_subject(&self.subject.read(cx).value()),
            Ok(SubjectQuery::You)
        );
        if self.has_cluster_wide_option != is_you {
            return;
        }
        self.has_cluster_wide_option = !is_you;
        let picked = self.picked_namespace(cx);
        let default = self.connection_default_namespace(cx);
        let wanted = match is_you {
            true => picked.or(default),
            false => picked,
        };
        let names = permission_namespace_options(self.listed.clone(), wanted.as_deref(), is_you);
        let row = wanted
            .as_deref()
            .and_then(|wanted| names.iter().position(|name| name == wanted))
            .unwrap_or(0);
        self.namespace.update(cx, |state, cx| {
            state.set_items(names, window, cx);
            state.set_selected_index(Some(IndexPath::default().row(row)), window, cx);
        });
    }

    fn connection_default_namespace(&self, cx: &Context<Self>) -> Option<String> {
        let session = self.session.upgrade()?;
        let live = session.read(cx).live()?;
        Some(live.default_namespace().to_owned())
    }

    fn picked_namespace(&self, cx: &Context<Self>) -> Option<String> {
        let picked = self.namespace.read(cx).selected_value()?;
        (picked != ALL_NAMESPACES).then(|| picked.clone())
    }

    fn parse_subject(&self, cx: &Context<Self>) -> Result<Checked, QueryError> {
        let subject = parse_subject(&self.subject.read(cx).value())?;
        let picked = self.picked_namespace(cx);
        let namespace = match &subject {
            SubjectQuery::You => picked.or_else(|| {
                let session = self.session.upgrade()?;
                let live = session.read(cx).live()?;
                Some(live.default_namespace().to_owned())
            }),
            SubjectQuery::Other { .. } => picked,
        };
        Ok(Checked { subject, namespace })
    }

    fn check(&mut self, cx: &mut Context<Self>) {
        self.table = RequestState::Idle;
        // An answer belongs to the subject it was asked for.
        self.answer = RequestState::Idle;
        let checked = self.parse_subject(cx);
        let Ok(ready) = &checked else {
            self.checked = Some(checked);
            cx.notify();
            return;
        };
        let (is_you, namespace) = (ready.subject == SubjectQuery::You, ready.namespace.clone());
        let is_masters = ready.subject.is_masters();
        self.checked = Some(checked);
        if is_masters {
            self.table = RequestState::Ready(masters_table());
        } else if is_you {
            self.table = match namespace {
                Some(namespace) => self.start_rules_review(namespace, cx),
                None => RequestState::Failed("Not connected".to_owned()),
            };
        } else {
            self.request_snapshot(cx);
        }
        self.evaluate(cx);
        cx.notify();
    }

    fn request_snapshot(&self, cx: &mut Context<Self>) {
        if let Some(session) = self.session.upgrade() {
            session.update(cx, |session, cx| session.request_rbac(cx));
        }
    }

    /// Asks the API server on the runtime; the answer arrives through `finish_rules`.
    fn start_rules_review(
        &mut self,
        namespace: String,
        cx: &mut Context<Self>,
    ) -> RequestState<ShownTable> {
        let Some(connection) = self.connection(cx) else {
            return RequestState::Failed("Not connected".to_owned());
        };
        let runtime = cx.global::<ClusterRuntime>().clone();
        let asked = namespace.clone();
        let reviewing = runtime.spawn(async move { connection.review_rules(&asked).await });
        let task = cx.spawn(async move |this, cx| {
            let result = reviewing.await;
            let _ = this.update(cx, |view, cx| view.finish_rules(&namespace, result, cx));
        });
        RequestState::Loading { _task: task }
    }

    fn finish_rules(
        &mut self,
        namespace: &str,
        result: Result<Result<RulesReview, cluster::ClusterError>, tokio::task::JoinError>,
        cx: &mut Context<Self>,
    ) {
        self.table = match result {
            Ok(Ok(review)) => RequestState::Ready(you_table(namespace, &review)),
            Ok(Err(error)) => RequestState::Failed(error_text(&error)),
            Err(_) => RequestState::Failed("the permission review stopped unexpectedly".to_owned()),
        };
        cx.notify();
    }

    fn connection(&self, cx: &Context<Self>) -> Option<cluster::ClusterConnection> {
        let session = self.session.upgrade()?;
        let live = session.read(cx).live()?;
        Some(live.connection().clone())
    }

    /// Asks "can the subject …?". A subject that changed since the last Check is checked first.
    fn ask(&mut self, cx: &mut Context<Self>) {
        let is_current = match (&self.checked, self.parse_subject(cx)) {
            (Some(Ok(checked)), Ok(now)) => {
                checked.subject == now.subject && checked.namespace == now.namespace
            }
            _ => false,
        };
        if !is_current {
            self.check(cx);
        }
        self.answer = RequestState::Idle;
        let namespace = match &self.checked {
            Some(Ok(checked)) => checked.namespace.clone(),
            _ => None,
        };
        let text = self.ask.read(cx).value();
        self.question =
            Some(parse_request(&text, namespace.as_deref()).map(|parsed| parsed.request));
        if let (Some(Ok(request)), Some(Ok(checked))) = (&self.question, &self.checked) {
            if checked.subject.is_masters() {
                self.answer = RequestState::Ready(masters_answer());
            } else if checked.subject == SubjectQuery::You {
                self.answer = self.start_request_review(request.clone(), cx);
            } else {
                self.request_snapshot(cx);
            }
        }
        self.evaluate(cx);
        cx.notify();
    }

    fn start_request_review(
        &mut self,
        request: AccessRequest,
        cx: &mut Context<Self>,
    ) -> RequestState<Answer> {
        let Some(connection) = self.connection(cx) else {
            return RequestState::Failed("Not connected".to_owned());
        };
        let runtime = cx.global::<ClusterRuntime>().clone();
        let reviewing = runtime.spawn(async move { connection.review_request(&request).await });
        let task = cx.spawn(async move |this, cx| {
            let result = reviewing.await;
            let _ = this.update(cx, |view, cx| {
                view.answer = match result {
                    Ok(Ok(decision)) => RequestState::Ready(you_answer(&decision)),
                    Ok(Err(error)) => RequestState::Failed(error_text(&error)),
                    Err(_) => RequestState::Failed(
                        "the permission review stopped unexpectedly".to_owned(),
                    ),
                };
                cx.notify();
            });
        });
        RequestState::Loading { _task: task }
    }

    /// Evaluates what waits for the RBAC snapshot, once it is ready.
    // ponytail: rules_of and decide run on the GPUI thread, O(bindings x rules); move them to the
    // runtime if a Check stalls a frame.
    fn evaluate(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.session.upgrade() else {
            return;
        };
        let Some(live) = session.read(cx).live() else {
            return;
        };
        let RbacState::Ready {
            snapshot,
            listed_at,
        } = &live.rbac
        else {
            return;
        };
        let Some(Ok(checked)) = &self.checked else {
            return;
        };
        let SubjectQuery::Other { identity, .. } = &checked.subject else {
            return;
        };
        if checked.subject.is_masters() {
            return;
        }
        let mut changed = false;
        if matches!(self.table, RequestState::Idle) {
            let table = other_table(
                snapshot,
                *listed_at,
                &checked.subject,
                identity,
                checked.namespace.as_deref(),
            );
            self.table = RequestState::Ready(table);
            changed = true;
        }
        if let (RequestState::Idle, Some(Ok(request))) = (&self.answer, &self.question) {
            let grants = snapshot.decide(identity, request);
            let first = grants
                .first()
                .map(|grant| (binding_text(grant.binding), role_text(&grant.binding.role)));
            let first = first
                .as_ref()
                .map(|(binding, role)| (binding.as_str(), role.as_str()));
            self.answer = RequestState::Ready(other_answer(first));
            changed = true;
        }
        if changed {
            cx.notify();
        }
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        self.table = RequestState::Idle;
        self.answer = RequestState::Idle;
        if let Some(session) = self.session.upgrade() {
            // A scope change leaves the state Idle, where a refresh does nothing: ask first.
            session.update(cx, |session, cx| {
                session.request_rbac(cx);
                session.refresh_rbac(cx);
            });
        }
        cx.notify();
    }

    fn reveal(&mut self, key: ResourceKey, window: &mut Window, cx: &mut Context<Self>) {
        window.close_dialog(cx);
        let _ = self.shell.update(cx, |shell, cx| shell.reveal(key, cx));
    }

    fn link(&mut self, text: &str, key: ResourceKey, cx: &mut Context<Self>) -> AnyElement {
        self.link_count += 1;
        let theme = cx.theme();
        div()
            .id(("permissions-link", self.link_count))
            .cursor_pointer()
            .font_family(theme.mono_font_family.clone())
            .text_color(theme.link)
            .underline()
            .on_click(cx.listener(move |view, _, window, cx| {
                view.reveal(key.clone(), window, cx);
            }))
            .child(text.to_owned())
            .into_any_element()
    }

    fn muted(&self, text: impl Into<SharedString>, cx: &Context<Self>) -> AnyElement {
        div()
            .text_sm()
            .text_color(cx.theme().muted_foreground)
            .child(text.into())
            .into_any_element()
    }

    fn toned(
        &self,
        text: impl Into<SharedString>,
        tone: StatusTone,
        cx: &Context<Self>,
    ) -> AnyElement {
        div()
            .text_color(tone_color(tone, cx))
            .child(text.into())
            .into_any_element()
    }

    /// The muted or Bad line of a state that has nothing to show yet.
    fn render_waiting(
        &mut self,
        table: &RequestState<ShownTable>,
        is_you: bool,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        match waiting_line(table, is_you) {
            Waiting::Nothing => return None,
            Waiting::Muted(text) => return Some(self.muted(text, cx)),
            Waiting::Bad(message) => return Some(self.toned(message, StatusTone::Bad, cx)),
            Waiting::Snapshot => {}
        }
        let session = self.session.upgrade();
        let Some(live) = session.as_ref().and_then(|session| session.read(cx).live()) else {
            return Some(self.muted("Not connected", cx));
        };
        match &live.rbac {
            RbacState::Idle | RbacState::Loading { .. } => {
                Some(self.muted("Listing RBAC objects…", cx))
            }
            RbacState::Failed(message) => Some(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(self.toned(message.clone(), StatusTone::Bad, cx))
                    .child(
                        Button::new("permissions-retry")
                            .small()
                            .outline()
                            .label("Retry")
                            .on_click(cx.listener(|view, _, _, cx| view.refresh(cx))),
                    )
                    .into_any_element(),
            ),
            RbacState::Ready { .. } => None,
        }
    }

    fn render_question(&mut self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut rows = vec![
            h_flex()
                .gap_2()
                .items_center()
                .child(self.muted("Can", cx))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(Input::new(&self.ask).small()),
                )
                .child(self.muted("?", cx))
                .child(
                    Button::new("permissions-ask")
                        .small()
                        .outline()
                        .label("Ask")
                        .on_click(cx.listener(|view, _, _, cx| view.ask(cx))),
                )
                .into_any_element(),
        ];
        match &self.question {
            Some(Err(error)) => rows.push(self.toned(error.text(), StatusTone::Bad, cx)),
            Some(Ok(request)) => {
                let target = question_text(request);
                match &self.answer {
                    RequestState::Ready(answer) => rows.push(
                        div()
                            .font_semibold()
                            .text_color(tone_color(answer.tone, cx))
                            .child(format!("{target}: {}", answer.text))
                            .into_any_element(),
                    ),
                    RequestState::Loading { .. } => {
                        rows.push(self.muted("Asking the API server…", cx));
                    }
                    RequestState::Failed(message) => {
                        rows.push(self.toned(message.clone(), StatusTone::Bad, cx));
                    }
                    RequestState::Idle => {}
                }
            }
            None => {}
        }
        rows
    }

    fn render_table(&mut self, shown: &ShownTable, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut body = Vec::new();
        if shown.is_masters {
            body.push(self.toned(MASTERS_NOTE, StatusTone::Ok, cx));
        } else {
            body.push(self.table_grid(shown, cx));
        }
        if !shown.granted_by.is_empty() {
            body.push(
                div()
                    .pt_1()
                    .font_semibold()
                    .child("Granted by")
                    .into_any_element(),
            );
            for row in &shown.granted_by {
                body.push(self.render_granted_by(row, cx));
            }
        }
        for warning in &shown.warnings {
            body.push(self.toned(warning.clone(), StatusTone::Warn, cx));
        }
        let mut source = h_flex()
            .gap_2()
            .items_center()
            .child(self.muted(shown.source.clone(), cx));
        if shown.listed_at.is_some() {
            source = source.child(
                Button::new("permissions-refresh")
                    .small()
                    .outline()
                    .label("Refresh")
                    .on_click(cx.listener(|view, _, _, cx| view.refresh(cx))),
            );
        }
        body.push(source.into_any_element());
        body.push(self.muted(shown.caveats.clone(), cx));
        body
    }

    fn render_granted_by(&mut self, row: &GrantedBy, cx: &mut Context<Self>) -> AnyElement {
        let binding = self.link(&row.binding_text, row.binding.clone(), cx);
        let role = match &row.role {
            Some(key) => self.link(&row.role_text, key.clone(), cx),
            None => div()
                .font_family(cx.theme().mono_font_family.clone())
                .child(row.role_text.clone())
                .into_any_element(),
        };
        v_flex()
            .gap_0p5()
            .text_sm()
            .child(
                h_flex()
                    .gap_1()
                    .flex_wrap()
                    .child(binding)
                    .child(h_flex().gap_1().child(self.muted("→", cx)).child(role)),
            )
            .child(
                div()
                    .pl_4()
                    .child(self.muted(format!("via {}", row.via), cx)),
            )
            .into_any_element()
    }

    /// Header, rows, URL rows, and the cut-off note, in one scroll box.
    fn table_grid(&mut self, shown: &ShownTable, cx: &mut Context<Self>) -> AnyElement {
        let table = &shown.table;
        if table.rows.is_empty() && table.url_rows.is_empty() {
            return self.muted(format!("No permissions {}.", shown.scope_text), cx);
        }
        let theme = cx.theme();
        let mono = theme.mono_font_family.clone();
        let verb_heads = VERB_HEADS.iter().map(|head| {
            div()
                .w(px(VERB_SLOT))
                .flex_none()
                .text_center()
                .child(*head)
                .into_any_element()
        });
        let header = h_flex()
            .gap_1()
            .text_xs()
            .text_color(theme.muted_foreground)
            .child(div().flex_1().min_w_0().child("Resource"))
            .children(verb_heads)
            .child(div().w(px(OTHER_SLOT)).flex_none().child("Other"));
        let mut rows: Vec<AnyElement> = vec![header.into_any_element()];
        for (ix, row) in table.rows.iter().enumerate() {
            let resource = v_flex()
                .flex_1()
                .min_w_0()
                .font_family(mono.clone())
                .child(div().truncate().child(row.resource.clone()))
                .when(shows_group(row), |resource| {
                    resource.child(
                        div()
                            .truncate()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(format!(".{}", row.group)),
                    )
                });
            let cells: Vec<AnyElement> = row
                .cells
                .iter()
                .zip(TABLE_VERBS)
                .enumerate()
                .map(|(column, (cell, verb))| {
                    verb_cell(ix * 8 + column, verb, cell, row.is_everything, cx)
                })
                .collect();
            let other = row.other_text();
            rows.push(
                h_flex()
                    .gap_1()
                    .text_sm()
                    .child(resource)
                    .children(cells)
                    .child(div().w(px(OTHER_SLOT)).flex_none().truncate().child(other))
                    .into_any_element(),
            );
        }
        for row in &table.url_rows {
            rows.push(
                h_flex()
                    .gap_1()
                    .text_sm()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .font_family(mono.clone())
                            .child(row.url.clone()),
                    )
                    .child(div().truncate().child(row.verbs.join(", ")))
                    .into_any_element(),
            );
        }
        if table.hidden_rows > 0 {
            rows.push(self.muted(format!("… {} more rows", table.hidden_rows), cx));
        }
        v_flex()
            .max_h(px(RESULT_MAX_HEIGHT))
            .gap_0p5()
            .children(rows)
            .overflow_y_scrollbar()
            .into_any_element()
    }
}

/// The muted `.group` under the resource. The core group has none, and a `*` resource in group
/// `*` is already everything, so a lone `.*` would only add noise.
fn shows_group(row: &crate::permission_table::PermissionRow) -> bool {
    if row.group.is_empty() {
        return false;
    }
    row.resource != "*" || row.group != "*"
}

/// `get pods in shop`: the request in words.
fn question_text(request: &AccessRequest) -> String {
    match &request.target {
        RequestTarget::NonResource { path } => format!("{} {path}", request.verb),
        RequestTarget::Resource(resource) => {
            let mut target = resource.resource.clone();
            if !resource.group.is_empty() {
                target = format!("{target}.{}", resource.group);
            }
            if let Some(subresource) = &resource.subresource {
                target = format!("{target}/{subresource}");
            }
            if let Some(name) = &resource.name {
                target = format!("{target} {name}");
            }
            match &resource.namespace {
                Some(namespace) => format!("{} {target} in {namespace}", request.verb),
                None => format!("{} {target}", request.verb),
            }
        }
    }
}

/// `✓` for every object, `names` (with a tooltip) for named ones, `all` on a wildcard row.
fn verb_cell(
    id: usize,
    verb: &str,
    cell: &VerbCell,
    is_everything: bool,
    cx: &Context<PermissionsView>,
) -> AnyElement {
    let slot = div().w(px(VERB_SLOT)).flex_none().text_center();
    match cell {
        VerbCell::Empty => slot.into_any_element(),
        VerbCell::All if is_everything => slot
            .text_color(tone_color(StatusTone::Warn, cx))
            .child("all")
            .into_any_element(),
        VerbCell::All => slot.child("✓").into_any_element(),
        VerbCell::Names(names) => {
            let tooltip = SharedString::from(format!("{verb}: {}", names.join(", ")));
            slot.id(("permission-names", id))
                .text_color(cx.theme().muted_foreground)
                .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
                .child("names")
                .into_any_element()
        }
    }
}

impl Render for PermissionsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.link_count = 0;
        let mut body: Vec<AnyElement> = Vec::new();
        match &self.checked {
            Some(Err(error)) => body.push(self.toned(error.text(), StatusTone::Bad, cx)),
            Some(Ok(checked)) => {
                let is_you = checked.subject == SubjectQuery::You;
                // Taken out so the render helpers can borrow `self` mutably, and read here.
                let table = std::mem::replace(&mut self.table, RequestState::Idle);
                match &table {
                    RequestState::Ready(shown) => body.extend(self.render_table(shown, cx)),
                    _ => body.extend(self.render_waiting(&table, is_you, cx)),
                }
                self.table = table;
            }
            None => {}
        }
        let question = self.render_question(cx);
        v_flex()
            .gap_3()
            .w_full()
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(Input::new(&self.subject).small()),
                    )
                    .child(
                        div()
                            .w(px(240.))
                            .child(Select::new(&self.namespace).small()),
                    )
                    .child(
                        Button::new("permissions-check")
                            .small()
                            .primary()
                            .label("Check")
                            .on_click(cx.listener(|view, _, _, cx| view.check(cx))),
                    ),
            )
            .children(question)
            .children(body)
    }
}

#[cfg(test)]
#[path = "permissions_view_tests.rs"]
mod permissions_view_tests;
