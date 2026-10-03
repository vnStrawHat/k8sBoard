//! The Who can… dialog: which subjects an RBAC binding gives `verb resource`, grouped with the
//! binding and role that grant it. It evaluates the session's RBAC snapshot on the GPUI thread.

use cluster::{
    AccessRequest, BindingSummary, BroadGroup, Grant, GrantNames, NamespaceCoverage, RbacCoverage,
    RbacSnapshot, RequestTarget, Subject, SubjectKind,
};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::select::{Select, SelectState};
use gpui_kit::component::{
    ActiveTheme as _, IndexPath, Sizable as _, StyledExt as _, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::{
    AnyElement, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _,
    Subscription, WeakEntity, Window, div, px,
};

use crate::access_bindings::{binding_key, binding_text, role_key, role_text, subject_text};
use crate::access_query::{ParsedRequest, QueryError, QueryHint, parse_request};
use crate::cluster_session::{ClusterSession, RbacState};
use crate::resource_kind::ResourceKind;
use crate::status_tone::{StatusTone, tone_color};
use crate::table_selection::{DialogOrigin, ResourceKey};

pub(crate) const ALL_NAMESPACES: &str = "All namespaces (cluster-wide grants)";
const MAX_LISTED_NAMES: usize = 5;
/// How `subject_text` spells the superuser group.
const MASTERS_SUBJECT: &str = "group system:masters";
/// The result list scrolls inside the dialog so the source line and caveats stay in view.
const RESULT_MAX_HEIGHT: f32 = 400.;
/// Past this many subjects the list outgrows its box, which shows no scrollbar until hovered.
const SCROLL_HINT_SUBJECTS: usize = 4;
pub(crate) const RBAC_CAVEATS: &str = "Not covered: other authorizers (Node, webhook), admission, impersonation, and groups assigned at sign-in. Bindings to missing roles grant nothing.";

/// One subject with every grant that reaches it.
#[derive(Clone, Debug, PartialEq, Eq)]
struct SubjectGroup {
    text: String,
    /// Bad for the groups that bind everyone, Warn for the service-account groups.
    tone: Option<StatusTone>,
    /// The row of a service account subject.
    account: Option<ResourceKey>,
    is_broad: bool,
    lines: Vec<GrantLine>,
}

/// `via {binding} → {role}` with where the binding applies.
#[derive(Clone, Debug, PartialEq, Eq)]
struct GrantLine {
    binding: ResourceKey,
    binding_text: String,
    role: Option<ResourceKey>,
    role_text: String,
    /// `cluster-wide` or `in {namespace}`.
    scope: String,
    /// ` · only a, b` for a grant limited to named objects.
    only: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct WhoCanGroups {
    full: Vec<SubjectGroup>,
    named_only: Vec<SubjectGroup>,
}

/// Groups grants by subject: broad groups first, then users, groups, and service accounts, each
/// run by name. Grants limited to named objects go to their own list.
fn who_can_groups(grants: &[Grant<'_>]) -> WhoCanGroups {
    let mut groups = WhoCanGroups::default();
    for grant in grants {
        let (list, only) = match &grant.names {
            GrantNames::Any => (&mut groups.full, None),
            GrantNames::Only(names) => (&mut groups.named_only, Some(only_text(names))),
        };
        add_grant(list, grant, only);
    }
    sort_subjects(&mut groups.full);
    sort_subjects(&mut groups.named_only);
    groups
}

fn add_grant(list: &mut Vec<SubjectGroup>, grant: &Grant<'_>, only: Option<String>) {
    let text = subject_text(grant.subject);
    let line = grant_line(grant.binding, only);
    match list.iter_mut().find(|group| group.text == text) {
        Some(group) => group.lines.push(line),
        None => list.push(SubjectGroup {
            tone: subject_tone(grant.subject),
            account: account_key(grant.subject),
            is_broad: grant.subject.broad_group().is_some(),
            text,
            lines: vec![line],
        }),
    }
}

fn grant_line(binding: &BindingSummary, only: Option<String>) -> GrantLine {
    GrantLine {
        binding: binding_key(binding),
        binding_text: binding_text(binding),
        role: role_key(binding),
        role_text: role_text(&binding.role),
        scope: match &binding.namespace {
            Some(namespace) => format!("in {namespace}"),
            None => "cluster-wide".to_owned(),
        },
        only,
    }
}

/// ` · only a, b`; at most five names, then `+{n}`.
fn only_text(names: &[String]) -> String {
    let shown = names
        .iter()
        .take(MAX_LISTED_NAMES)
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(", ");
    match names.len().checked_sub(MAX_LISTED_NAMES) {
        Some(rest) if rest > 0 => format!("only {shown}, +{rest}"),
        _ => format!("only {shown}"),
    }
}

fn subject_tone(subject: &Subject) -> Option<StatusTone> {
    match subject.broad_group()? {
        BroadGroup::Authenticated | BroadGroup::Unauthenticated => Some(StatusTone::Bad),
        BroadGroup::AllServiceAccounts | BroadGroup::NamespaceServiceAccounts(_) => {
            Some(StatusTone::Warn)
        }
    }
}

fn account_key(subject: &Subject) -> Option<ResourceKey> {
    if subject.kind != SubjectKind::ServiceAccount {
        return None;
    }
    Some(ResourceKey::Kind {
        kind: ResourceKind::ServiceAccounts,
        namespace: Some(subject.namespace.clone()?),
        name: subject.name.clone(),
    })
}

/// Broad groups, then users, groups, service accounts; each run by text.
fn sort_subjects(groups: &mut [SubjectGroup]) {
    let rank = |group: &SubjectGroup| {
        if group.is_broad {
            0
        } else if group.text.starts_with("user ") {
            1
        } else if group.text.starts_with("group ") {
            2
        } else {
            3
        }
    };
    groups.sort_by(|a, b| (rank(a), &a.text).cmp(&(rank(b), &b.text)));
}

/// `{n} subjects can {verb} {target}{ in {ns} | cluster-wide}`.
fn headline_text(request: &AccessRequest, subjects: usize) -> String {
    let count = match subjects {
        1 => "1 subject".to_owned(),
        n => format!("{n} subjects"),
    };
    let (target, scope) = match &request.target {
        RequestTarget::NonResource { path } => (path.clone(), "cluster-wide".to_owned()),
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
            let scope = match &resource.namespace {
                Some(namespace) => format!("in {namespace}"),
                None => "cluster-wide".to_owned(),
            };
            (target, scope)
        }
    };
    format!("{count} can {} {target} {scope}", request.verb)
}

/// `All namespaces`, then the listed ones, plus `wanted` when the list lacks it (the list may not
/// have loaded, or the account may not list namespaces), so a preselected namespace is never
/// silently replaced by All.
pub(crate) fn namespace_options(listed: Vec<String>, wanted: Option<&str>) -> Vec<String> {
    let mut names = vec![ALL_NAMESPACES.to_owned()];
    names.extend(listed);
    if let Some(wanted) = wanted.filter(|wanted| !names.iter().any(|name| name == wanted)) {
        names.insert(1, wanted.to_owned());
    }
    names
}

/// One Warn line per gap in what the snapshot could list.
pub(crate) fn coverage_notes(
    coverage: &RbacCoverage,
    namespace: Option<&str>,
) -> Vec<SharedString> {
    let mut notes: Vec<SharedString> = Vec::new();
    if !coverage.cluster_bindings {
        notes.push("ClusterRoleBindings were not listed; only namespace grants are shown.".into());
    }
    if !coverage.cluster_roles {
        notes.push("ClusterRoles were not listed; grants through them are not shown.".into());
    }
    let listed = |coverage: &NamespaceCoverage| match coverage {
        NamespaceCoverage::AllNamespaces => None,
        NamespaceCoverage::Namespaces(listed) => Some(listed.len()),
    };
    let narrower = match (listed(&coverage.roles), listed(&coverage.role_bindings)) {
        (Some(roles), Some(bindings)) => Some(roles.min(bindings)),
        (count, None) | (None, count) => count,
    };
    if let Some(count) = narrower {
        let noun = if count == 1 {
            "namespace"
        } else {
            "namespaces"
        };
        notes.push(
            format!(
                "Roles and RoleBindings were listed in {count} {noun} only (not permitted cluster-wide)."
            )
            .into(),
        );
    }
    if let Some(namespace) = namespace.filter(|namespace| !coverage.covers(namespace)) {
        notes.push(
            format!(
                "RoleBindings of {namespace} were not listed; only cluster-wide grants are shown."
            )
            .into(),
        );
    }
    notes
}

/// The evaluated answer, kept as owned text so it outlives the snapshot borrow.
struct WhoCanResult {
    headline: String,
    has_broad_group: bool,
    /// The grants of `group system:masters`, shown on its fixed row instead of a second one.
    masters: Vec<GrantLine>,
    groups: WhoCanGroups,
    notes: Vec<SharedString>,
    listed_at: jiff::Timestamp,
}

struct Asked {
    request: AccessRequest,
    hint: Option<QueryHint>,
    result: Option<WhoCanResult>,
}

pub(crate) struct WhoCanView {
    session: WeakEntity<ClusterSession>,
    origin: DialogOrigin,
    query: Entity<InputState>,
    namespace: Entity<SelectState<Vec<String>>>,
    question: Option<Result<Asked, QueryError>>,
    link_count: usize,
    _subscriptions: Vec<Subscription>,
}

impl WhoCanView {
    /// `namespace`: the preselected one; `None` is cluster-wide. `query` is typed in and, when
    /// `check_now`, asked at once.
    pub(crate) fn new(
        origin: DialogOrigin,
        session: &Entity<ClusterSession>,
        query: Option<String>,
        namespace: Option<String>,
        check_now: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut listed = Vec::new();
        if let Some(live) = session.read(cx).live() {
            match live.namespaces.ready_items() {
                Some(namespaces) => listed.extend(namespaces.iter().map(|item| item.name.clone())),
                None => listed.extend(live.scope.namespaces().iter().cloned()),
            }
        }
        let names = namespace_options(listed, namespace.as_deref());
        let selected = namespace
            .as_deref()
            .and_then(|namespace| names.iter().position(|name| name == namespace))
            .unwrap_or(0);
        let namespace = cx.new(|cx| {
            SelectState::new(names, Some(IndexPath::default().row(selected)), window, cx)
                .searchable(true)
        });
        let placeholder = "verb resource[.group][/subresource] [name]   or   verb /url   (e.g. list pods.metrics.k8s.io)";
        let input = cx.new(|cx| {
            let mut input = InputState::new(window, cx).placeholder(placeholder);
            if let Some(query) = &query {
                input.set_value(query.clone(), window, cx);
            }
            input
        });
        let subscriptions = vec![
            cx.subscribe_in(&input, window, |view, _, event, _, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    view.check(cx);
                }
            }),
            cx.observe(session, |view, _, cx| view.evaluate(cx)),
        ];
        let mut view = Self {
            session: session.downgrade(),
            origin,
            query: input,
            namespace,
            question: None,
            link_count: 0,
            _subscriptions: subscriptions,
        };
        if check_now {
            view.check(cx);
        }
        view
    }

    /// Whether the answer is still on its way: the capture waits on it.
    #[cfg(feature = "screenshot")]
    pub(crate) fn is_pending(&self, cx: &gpui_kit::App) -> bool {
        let Some(Ok(asked)) = &self.question else {
            return false;
        };
        if asked.result.is_some() {
            return false;
        }
        let Some(session) = self.session.upgrade() else {
            return false;
        };
        session
            .read(cx)
            .live()
            .is_some_and(|live| awaits_listing(&live.rbac))
    }

    fn picked_namespace(&self, cx: &Context<Self>) -> Option<String> {
        let picked = self.namespace.read(cx).selected_value()?;
        (picked != ALL_NAMESPACES).then(|| picked.clone())
    }

    fn check(&mut self, cx: &mut Context<Self>) {
        let text = self.query.read(cx).value();
        let namespace = self.picked_namespace(cx);
        self.question = Some(parse_request(&text, namespace.as_deref()).map(
            |ParsedRequest { request, hint }| Asked {
                request,
                hint,
                result: None,
            },
        ));
        if let Some(session) = self.session.upgrade() {
            session.update(cx, |session, cx| session.request_rbac(cx));
        }
        self.evaluate(cx);
        cx.notify();
    }

    /// Evaluates once the snapshot is ready. The grants borrow the snapshot, so the answer is
    /// copied into owned text here.
    // ponytail: who_can runs on the GPUI thread, O(bindings x rules); move it to the runtime if a
    // Check stalls a frame.
    fn evaluate(&mut self, cx: &mut Context<Self>) {
        let Some(Ok(asked)) = &self.question else {
            return;
        };
        if asked.result.is_some() {
            return;
        }
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
        let result = evaluate_request(snapshot, *listed_at, &asked.request);
        if let Some(Ok(asked)) = &mut self.question {
            asked.result = Some(result);
        }
        cx.notify();
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        if let Some(Ok(asked)) = &mut self.question {
            asked.result = None;
        }
        if let Some(session) = self.session.upgrade() {
            // A scope change leaves the state Idle, where a refresh does nothing: ask first. The
            // two together start a listing from every state but Loading.
            session.update(cx, |session, cx| {
                session.request_rbac(cx);
                session.refresh_rbac(cx);
            });
        }
        cx.notify();
    }

    /// Closes the dialog and shows the target on its own screen.
    fn reveal(&mut self, key: ResourceKey, window: &mut Window, cx: &mut Context<Self>) {
        window.close_dialog(cx);
        self.origin.reveal(key, cx);
    }

    fn link(&mut self, text: &str, key: ResourceKey, cx: &mut Context<Self>) -> AnyElement {
        self.link_count += 1;
        let theme = cx.theme();
        div()
            .id(("who-can-link", self.link_count))
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

    fn mono(&self, text: &str, cx: &Context<Self>) -> AnyElement {
        div()
            .font_family(cx.theme().mono_font_family.clone())
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

    fn render_group(&mut self, group: &SubjectGroup, cx: &mut Context<Self>) -> AnyElement {
        let subject = match (&group.account, group.tone) {
            (Some(key), _) => self.link(&group.text, key.clone(), cx),
            (None, Some(tone)) => div()
                .font_family(cx.theme().mono_font_family.clone())
                .text_color(tone_color(tone, cx))
                .child(group.text.clone())
                .into_any_element(),
            (None, None) => self.mono(&group.text, cx),
        };
        let lines: Vec<AnyElement> = group
            .lines
            .iter()
            .map(|line| self.render_line(line, cx))
            .collect();
        v_flex()
            .gap_0p5()
            .py_1()
            .child(subject)
            .children(lines)
            .into_any_element()
    }

    fn render_line(&mut self, line: &GrantLine, cx: &mut Context<Self>) -> AnyElement {
        let binding = self.link(&line.binding_text, line.binding.clone(), cx);
        let role = match &line.role {
            Some(key) => self.link(&line.role_text, key.clone(), cx),
            None => self.mono(&line.role_text, cx),
        };
        let scope = match &line.only {
            Some(only) => format!(" · {} · {only}", line.scope),
            None => format!(" · {}", line.scope),
        };
        h_flex()
            .gap_1()
            .pl_4()
            .flex_wrap()
            .text_sm()
            .child(self.muted("via", cx))
            .child(binding)
            .child(h_flex().gap_1().child(self.muted("→", cx)).child(role))
            .child(self.muted(scope, cx))
            .into_any_element()
    }

    fn render_result(&mut self, result: &WhoCanResult, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut body = Vec::new();
        let headline = div().font_semibold().child(result.headline.clone());
        body.push(if result.has_broad_group {
            headline
                .text_color(tone_color(StatusTone::Warn, cx))
                .into_any_element()
        } else {
            headline.into_any_element()
        });
        let masters_lines: Vec<AnyElement> = result
            .masters
            .iter()
            .map(|line| self.render_line(line, cx))
            .collect();
        body.push(
            v_flex()
                .gap_0p5()
                .py_1()
                .child(
                    h_flex()
                        .gap_2()
                        .child(self.mono(MASTERS_SUBJECT, cx))
                        .child(self.muted("· always allowed (bypasses RBAC)", cx)),
                )
                .children(masters_lines)
                .into_any_element(),
        );
        for group in &result.groups.full {
            body.push(self.render_group(group, cx));
        }
        if result.groups.full.is_empty() {
            body.push(self.muted("No RBAC binding grants this.", cx));
        }
        if !result.groups.named_only.is_empty() {
            body.push(
                div()
                    .pt_2()
                    .font_semibold()
                    .child("Only for named objects")
                    .into_any_element(),
            );
            for group in &result.groups.named_only {
                body.push(self.render_group(group, cx));
            }
        }
        body
    }

    fn render_snapshot_state(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let session = self.session.upgrade();
        let Some(live) = session.as_ref().and_then(|session| session.read(cx).live()) else {
            return Some(self.muted("Not connected", cx));
        };
        match &live.rbac {
            RbacState::Idle | RbacState::Loading { .. } => {
                Some(self.muted("Listing RBAC objects…", cx))
            }
            RbacState::Failed(message) => {
                let message = message.clone();
                Some(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(self.toned(message, StatusTone::Bad, cx))
                        .child(
                            Button::new("who-can-retry")
                                .small()
                                .outline()
                                .label("Retry")
                                .on_click(cx.listener(|view, _, _, cx| view.refresh(cx))),
                        )
                        .into_any_element(),
                )
            }
            RbacState::Ready { .. } => None,
        }
    }
}

fn evaluate_request(
    snapshot: &RbacSnapshot,
    listed_at: jiff::Timestamp,
    request: &AccessRequest,
) -> WhoCanResult {
    let grants = snapshot.who_can(request);
    let mut groups = who_can_groups(&grants);
    let headline = headline_text(request, groups.full.len());
    let has_broad_group = groups.full.iter().any(|group| group.is_broad);
    let masters = take_masters(&mut groups);
    let namespace = match &request.target {
        RequestTarget::Resource(resource) => resource.namespace.as_deref(),
        RequestTarget::NonResource { .. } => None,
    };
    WhoCanResult {
        headline,
        has_broad_group,
        masters,
        notes: coverage_notes(&snapshot.coverage, namespace),
        groups,
        listed_at,
    }
}

/// The superuser group bypasses RBAC and has its own fixed row, so a binding to it moves there
/// (its grants are moot when named, since the group is always allowed).
fn take_masters(groups: &mut WhoCanGroups) -> Vec<GrantLine> {
    groups
        .named_only
        .retain(|group| group.text != MASTERS_SUBJECT);
    match groups
        .full
        .iter()
        .position(|group| group.text == MASTERS_SUBJECT)
    {
        Some(index) => groups.full.remove(index).lines,
        None => Vec::new(),
    }
}

/// A failed listing is the answer to show, so only Idle and Loading are still waiting.
#[cfg(any(feature = "screenshot", test))]
fn awaits_listing(state: &RbacState) -> bool {
    matches!(state, RbacState::Idle | RbacState::Loading { .. })
}

pub(crate) fn clock_text(time: jiff::Timestamp) -> String {
    time.to_zoned(jiff::tz::TimeZone::system())
        .strftime("%H:%M:%S")
        .to_string()
}

impl Render for WhoCanView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.link_count = 0;
        let mut body: Vec<AnyElement> = Vec::new();
        match self.question.take() {
            None => {}
            Some(Err(error)) => {
                body.push(self.toned(error.text(), StatusTone::Bad, cx));
                self.question = Some(Err(error));
            }
            Some(Ok(asked)) => {
                if let Some(hint) = asked.hint {
                    body.push(match hint {
                        // These two may turn a typo into an empty answer, so they stand out.
                        QueryHint::AssumedCoreGroup | QueryHint::UnknownSubresource => {
                            self.toned(hint_text(hint), StatusTone::Warn, cx)
                        }
                        QueryHint::ClusterScoped => self.muted(hint_text(hint), cx),
                    });
                }
                match &asked.result {
                    Some(result) => {
                        let rows = self.render_result(result, cx);
                        body.push(
                            v_flex()
                                .max_h(px(RESULT_MAX_HEIGHT))
                                .gap_1()
                                .children(rows)
                                .overflow_y_scrollbar()
                                .into_any_element(),
                        );
                        if result.groups.full.len() > SCROLL_HINT_SUBJECTS {
                            body.push(self.muted("Scroll the list to see every subject.", cx));
                        }
                        for note in &result.notes {
                            body.push(self.toned(note.clone(), StatusTone::Warn, cx));
                        }
                        body.push(
                            h_flex()
                                .gap_2()
                                .items_center()
                                .child(self.muted(
                                    format!(
                                        "Computed from RBAC objects listed at {}",
                                        clock_text(result.listed_at)
                                    ),
                                    cx,
                                ))
                                .child(
                                    Button::new("who-can-refresh")
                                        .small()
                                        .outline()
                                        .label("Refresh")
                                        .on_click(cx.listener(|view, _, _, cx| view.refresh(cx))),
                                )
                                .into_any_element(),
                        );
                    }
                    None => body.extend(self.render_snapshot_state(cx)),
                }
                self.question = Some(Ok(asked));
            }
        }
        body.push(self.muted(RBAC_CAVEATS, cx));
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
                            .child(Input::new(&self.query).small()),
                    )
                    .child(
                        div()
                            .w(px(240.))
                            .child(Select::new(&self.namespace).small()),
                    )
                    .child(
                        Button::new("who-can-check")
                            .small()
                            .primary()
                            .label("Check")
                            .on_click(cx.listener(|view, _, _, cx| view.check(cx))),
                    ),
            )
            .children(body)
    }
}

fn hint_text(hint: QueryHint) -> &'static str {
    match hint {
        QueryHint::AssumedCoreGroup => {
            "Unknown resource: assumed the core group; write resource.group"
        }
        QueryHint::ClusterScoped => "Cluster-scoped resource: namespace ignored",
        QueryHint::UnknownSubresource => {
            "Unknown subresource: check the spelling (for example log, exec, status, scale)"
        }
    }
}

#[cfg(test)]
#[path = "who_can_view_tests.rs"]
mod who_can_view_tests;
