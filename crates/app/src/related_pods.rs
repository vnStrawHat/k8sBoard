//! The pods section of a drawer: the pods a workload or a node runs.

use cluster::{
    ClaimTemplate, DisruptionState, NamespaceScope, PodDisruptionBudgetSummary, PodSummary,
    VolumeSource,
};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, div, px,
};

use crate::app_shell::AppShell;
use crate::cluster_session::{LiveCluster, namespaces_label};
use crate::drawer::{link_name, named_object_text, open_link, section_title};
use crate::kind_diagnosis::first_main_termination;
use crate::kind_row::{
    DAEMON_SET_KIND, JOB_KIND, KindObject, PodOwner, STATEFUL_SET_KIND, owns_pod,
};
use crate::resource_kind::ResourceKind;
use crate::status_tone::{StatusTone, pod_status_label, tone_color, toned_text};
use crate::table_selection::ResourceKey;
use crate::workload_rows::sort_by_ordinal;

/// Bounds the render cost of a workload with very many pods.
const MAX_RELATED_PODS: usize = 50;

/// The pods of `owner`, read from the live pods list at render time so they stay current.
/// A click opens the pod on the Pods screen. A node lists the pods of the current scope that run on it.
pub(crate) fn pods_section(
    owner: &PodOwner,
    object: &KindObject,
    live: &LiveCluster,
    cx: &Context<AppShell>,
) -> AnyElement {
    let mut pods: Vec<&PodSummary> = live
        .pods
        .items()
        .iter()
        .filter(|pod| owns_pod(owner, pod))
        .collect();
    let detail = pod_row_detail(owner, object);
    sort_pods(&mut pods, owner, detail);
    let heading = detail.heading();
    let (title, note) = if live.pods.is_loading() {
        (heading.to_owned(), Some("Loading pods…"))
    } else if live.pods.failure().is_some() {
        (heading.to_owned(), Some("Pods are unavailable"))
    } else {
        (
            format!("{heading} {}", pods.len()),
            pods.is_empty().then_some("No pods"),
        )
    };
    let hidden = pods.len().saturating_sub(MAX_RELATED_PODS);
    // Only a node's pods are read for a drain; the budgets come from the always-on PDB feed.
    let budgets: Vec<&PodDisruptionBudgetSummary> = match owner {
        PodOwner::Node { .. } => live
            .issue_feeds
            .condition(ResourceKind::PodDisruptionBudgets)
            .and_then(|feed| feed.list.ready_items())
            .into_iter()
            .flatten()
            .filter_map(|object| match object {
                KindObject::PodDisruptionBudget(budget) => Some(budget),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    };
    let theme = cx.theme();
    v_flex()
        .child(section_title(title, cx))
        .children(scope_note(owner, &live.scope).map(|text| {
            div()
                .pb_1()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child(text)
        }))
        .children(note.map(|note| {
            div()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child(note)
        }))
        .children(
            pods.iter()
                .take(MAX_RELATED_PODS)
                .enumerate()
                .map(|(index, pod)| related_pod_row(index, pod, detail, &budgets, cx)),
        )
        .children((hidden > 0).then(|| {
            div()
                .px_2()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child(format!("+{hidden} more"))
        }))
        .into_any_element()
}

/// The pods watch follows the namespace picker, so a node lists only the pods of that namespace.
fn scope_note(owner: &PodOwner, scope: &NamespaceScope) -> Option<String> {
    match (owner, scope) {
        (PodOwner::Node { .. }, NamespaceScope::Named(namespace)) => {
            Some(format!("Only pods in {namespace} are listed"))
        }
        (PodOwner::Node { .. }, NamespaceScope::Several(names)) => Some(format!(
            "Only pods in {} are listed",
            namespaces_label(names)
        )),
        _ => None,
    }
}

/// What a related-pod row shows after the pod name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PodRowDetail<'a> {
    StatusOnly,
    StatusAndNode,
    /// A node runs pods of any namespace, so the namespace is shown.
    NamespaceAndStatus,
    /// StatefulSet pods: the claims each pod mounts, from the set's templates.
    StatusAndClaims(&'a [ClaimTemplate]),
    /// Job pods are its attempts: the exit code of the first main container.
    Attempt,
}

impl PodRowDetail<'_> {
    /// The section title before the count.
    fn heading(self) -> &'static str {
        match self {
            Self::StatusAndClaims(_) => "Pods by ordinal",
            Self::Attempt => "Attempts",
            Self::StatusOnly | Self::StatusAndNode | Self::NamespaceAndStatus => "Pods",
        }
    }
}

/// How the pods of `owner` are listed. A DaemonSet runs one pod per node, so the node tells its
/// pods apart; a StatefulSet shows the claims of each ordinal, and a Job its attempts.
fn pod_row_detail<'a>(owner: &PodOwner, object: &'a KindObject) -> PodRowDetail<'a> {
    match (owner, object) {
        (PodOwner::Controller { kind, .. }, KindObject::StatefulSet(set))
            if *kind == STATEFUL_SET_KIND =>
        {
            PodRowDetail::StatusAndClaims(&set.claim_templates)
        }
        (PodOwner::Controller { kind, .. }, _) if *kind == DAEMON_SET_KIND => {
            PodRowDetail::StatusAndNode
        }
        (PodOwner::Controller { kind, .. }, _) if *kind == JOB_KIND => PodRowDetail::Attempt,
        (PodOwner::Node { .. }, _) => PodRowDetail::NamespaceAndStatus,
        _ => PodRowDetail::StatusOnly,
    }
}

/// StatefulSet pods read best in ordinal order and Job attempts newest first; the others keep
/// the snapshot order.
fn sort_pods(pods: &mut [&PodSummary], owner: &PodOwner, detail: PodRowDetail) {
    match (detail, owner) {
        (PodRowDetail::StatusAndClaims(_), PodOwner::Controller { name, .. }) => {
            sort_by_ordinal(pods, name);
        }
        (PodRowDetail::Attempt, _) => {
            // A pod without a creation time is the oldest.
            pods.sort_by_key(|pod| std::cmp::Reverse(pod.created_at));
        }
        _ => {}
    }
}

/// `{claim} {storage}` for each claim template whose PVC (`{template}-{pod}`) the pod mounts.
fn pod_claims(pod: &PodSummary, templates: &[ClaimTemplate]) -> Vec<String> {
    templates
        .iter()
        .filter(|template| {
            let claim = format!("{}-{}", template.name, pod.name);
            pod.containers
                .iter()
                .flat_map(|container| &container.mounts)
                .any(|mount| {
                    matches!(&mount.source, VolumeSource::PersistentVolumeClaim { claim: mounted } if *mounted == claim)
                })
        })
        .map(|template| match &template.storage {
            Some(storage) => format!("{} {storage}", template.name),
            None => template.name.clone(),
        })
        .collect()
}

/// `exit {code}` of the pod's first main container, when it has ended.
fn exit_text(pod: &PodSummary) -> Option<String> {
    first_main_termination(pod).map(|termination| format!("exit {}", termination.exit_code))
}

/// What a drain would meet on a pod of a node, shown as a small tag after its status.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DrainTag {
    DaemonSet,
    EmptyDir,
    /// A budget that selects the pod allows no disruption now.
    ZeroDisruptions,
    NoController,
}

impl DrainTag {
    fn label(self) -> &'static str {
        match self {
            Self::DaemonSet => "DS",
            Self::EmptyDir => "emptyDir",
            Self::ZeroDisruptions => "PDB 0",
            Self::NoController => "no controller",
        }
    }

    fn hint(self) -> &'static str {
        match self {
            Self::DaemonSet => "DaemonSet pod: a drain skips it only with Ignore DaemonSet pods",
            Self::EmptyDir => "Uses an emptyDir: a drain needs Delete emptyDir data",
            Self::ZeroDisruptions => "A PodDisruptionBudget allows 0 disruptions: the drain waits",
            Self::NoController => "No controller recreates it: a drain needs Force unmanaged pods",
        }
    }
}

/// The tags of `pod` for a drain of its node, from the same facts `drain_plan` reads. A finished
/// pod is evicted whatever it is, so it has none.
fn drain_tags(pod: &PodSummary, budgets: &[&PodDisruptionBudgetSummary]) -> Vec<DrainTag> {
    if pod.is_finished {
        return Vec::new();
    }
    let mut tags = Vec::new();
    match &pod.controller {
        Some(controller) if controller.kind == DAEMON_SET_KIND => tags.push(DrainTag::DaemonSet),
        Some(_) => {}
        None => tags.push(DrainTag::NoController),
    }
    let has_empty_dir = pod
        .containers
        .iter()
        .flat_map(|container| &container.mounts)
        .any(|mount| mount.source == VolumeSource::EmptyDir);
    if has_empty_dir {
        tags.push(DrainTag::EmptyDir);
    }
    let is_blocked = budgets.iter().any(|budget| {
        budget.namespace == pod.namespace
            && budget
                .selector
                .as_ref()
                .is_some_and(|selector| selector.matches(&pod.labels))
            && matches!(budget.disruption_state(), DisruptionState::Blocked(_))
    });
    if is_blocked {
        tags.push(DrainTag::ZeroDisruptions);
    }
    tags
}

/// One small tag of a pod row; `PDB 0` is the warning one, the rest are plain facts.
fn drain_tag(tag: DrainTag, cx: &Context<AppShell>) -> AnyElement {
    let theme = cx.theme();
    let color = match tag {
        DrainTag::ZeroDisruptions => tone_color(StatusTone::Warn, cx),
        _ => theme.muted_foreground,
    };
    div()
        .id(tag.label())
        .flex_shrink_0()
        .px_1p5()
        .rounded(theme.radius)
        .border_1()
        .border_color(theme.border)
        .text_xs()
        .text_color(color)
        .tooltip(move |window, cx| Tooltip::new(tag.hint()).build(window, cx))
        .child(tag.label())
        .into_any_element()
}

fn related_pod_row(
    index: usize,
    pod: &PodSummary,
    detail: PodRowDetail,
    budgets: &[&PodDisruptionBudgetSummary],
    cx: &Context<AppShell>,
) -> AnyElement {
    let theme = cx.theme();
    // Only a node's pods are tagged: they are what a drain acts on.
    let tags = match detail {
        PodRowDetail::NamespaceAndStatus => drain_tags(pod, budgets),
        _ => Vec::new(),
    };
    let key = ResourceKey::of_pod(pod);
    // The text after the status: the claims of an ordinal, or the exit code of an attempt.
    let extra = match detail {
        PodRowDetail::StatusAndClaims(templates) => {
            Some(pod_claims(pod, templates).join(", ")).filter(|text| !text.is_empty())
        }
        PodRowDetail::Attempt => exit_text(pod),
        PodRowDetail::StatusOnly
        | PodRowDetail::StatusAndNode
        | PodRowDetail::NamespaceAndStatus => None,
    };
    let hover_bg = theme.muted;
    // A node's pods come from every namespace, so their links name it.
    let pod_link = link_name(
        index,
        named_object_text(
            &pod.name,
            &key,
            matches!(detail, PodRowDetail::NamespaceAndStatus),
        ),
        cx,
    );
    h_flex()
        .id(("related-pod", index))
        .gap_2()
        .items_center()
        .py_1()
        .rounded(theme.radius)
        .text_sm()
        .cursor_pointer()
        .hover(move |style| style.bg(hover_bg))
        .on_click(cx.listener(move |shell, _, window, cx| {
            open_link(shell, key.clone(), window, cx);
        }))
        .child(
            h_flex()
                .flex_1()
                .min_w_0()
                .items_center()
                .overflow_hidden()
                .font_family(theme.mono_font_family.clone())
                .child(pod_link),
        )
        .child(toned_text(pod_status_label(pod), cx))
        .children(tags.into_iter().map(|tag| drain_tag(tag, cx)))
        .children(matches!(detail, PodRowDetail::StatusAndNode).then(|| {
            div()
                .flex_shrink_0()
                .text_color(theme.muted_foreground)
                .child(pod.node_name.clone().unwrap_or_default())
        }))
        .children(extra.map(|text| {
            div()
                .flex_shrink_0()
                .max_w(px(160.))
                .truncate()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(text)
        }))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node() -> PodOwner {
        PodOwner::Node { name: "n1".into() }
    }

    #[test]
    fn scope_note_only_for_a_node_in_a_named_namespace() {
        let named = NamespaceScope::Named("ns".into());
        assert_eq!(
            scope_note(&node(), &named).as_deref(),
            Some("Only pods in ns are listed")
        );
        assert_eq!(scope_note(&node(), &NamespaceScope::All), None);
        let controller = PodOwner::Controller {
            kind: DAEMON_SET_KIND,
            name: "d".into(),
            namespace: "ns".into(),
        };
        assert_eq!(scope_note(&controller, &named), None);
    }

    use cluster::{
        ContainerKind, ContainerProbes, ContainerState, ContainerSummary, MountEntry, PodStatus,
        ReadyCount, StatefulSetSummary, StatusReason, Termination,
    };

    fn pod(name: &str, created: Option<i64>) -> PodSummary {
        PodSummary {
            annotations: cluster::AnnotationTerms::default(),
            is_finished: false,
            namespace: "ns".to_owned(),
            name: name.to_owned(),
            status: PodStatus::Reason(StatusReason::Running),
            ready: ReadyCount { ready: 1, total: 1 },
            restarts: 0,
            node_name: None,
            created_at: created
                .map(|seconds| jiff::Timestamp::from_second(seconds).expect("valid timestamp")),
            pod_ip: None,
            qos_class: None,
            service_account: None,
            controller: None,
            conditions: Vec::new(),
            status_message: None,
            labels: Vec::new(),
            host_network: false,
            image_pull_secrets: Vec::new(),
            node_selector: Vec::new(),
            node_affinity: Vec::new(),
            containers: Vec::new(),
        }
    }

    fn container(
        kind: ContainerKind,
        state: ContainerState,
        mounts: Vec<MountEntry>,
    ) -> ContainerSummary {
        ContainerSummary {
            terminal: cluster::ContainerTerminal::None,
            name: "main".to_owned(),
            image: "app:1".to_owned(),
            kind,
            state,
            is_ready: true,
            restart_count: 0,
            last_termination: None,
            image_digest: None,
            pull_policy: None,
            is_started: None,
            ports: Vec::new(),
            resources: Vec::new(),
            probes: ContainerProbes::default(),
            env: Vec::new(),
            env_from: Vec::new(),
            mounts,
        }
    }

    fn running() -> ContainerState {
        ContainerState::Running { started_at: None }
    }

    fn mount_of(claim: &str) -> MountEntry {
        MountEntry {
            path: "/data".to_owned(),
            volume: "data".to_owned(),
            source: VolumeSource::PersistentVolumeClaim {
                claim: claim.to_owned(),
            },
            is_read_only: false,
            sub_path: None,
        }
    }

    fn template(name: &str, storage: Option<&str>) -> ClaimTemplate {
        ClaimTemplate {
            name: name.to_owned(),
            storage: storage.map(str::to_owned),
            storage_class: None,
            access_modes: Vec::new(),
        }
    }

    fn controller(kind: &'static str) -> PodOwner {
        PodOwner::Controller {
            namespace: "ns".to_owned(),
            kind,
            name: "web".to_owned(),
        }
    }

    fn stateful_set(templates: Vec<ClaimTemplate>) -> KindObject {
        KindObject::StatefulSet(StatefulSetSummary {
            annotations: cluster::AnnotationTerms::default(),
            namespace: "ns".to_owned(),
            name: "web".to_owned(),
            created_at: None,
            labels: Vec::new(),
            desired: 1,
            ready: 1,
            current: 1,
            updated: 1,
            service_name: None,
            update_strategy: String::new(),
            pod_management_policy: String::new(),
            selector: Vec::new(),
            containers: Vec::new(),
            claim_templates: templates,
            claim_retention: None,
        })
    }

    #[test]
    fn pod_row_detail_per_owner() {
        let templates = vec![template("data", Some("10Gi"))];
        let set = stateful_set(templates.clone());
        assert_eq!(
            pod_row_detail(&controller(STATEFUL_SET_KIND), &set),
            PodRowDetail::StatusAndClaims(&templates)
        );
        assert_eq!(
            pod_row_detail(&controller(DAEMON_SET_KIND), &KindObject::Plain),
            PodRowDetail::StatusAndNode
        );
        assert_eq!(
            pod_row_detail(&controller(JOB_KIND), &KindObject::Plain),
            PodRowDetail::Attempt
        );
        assert_eq!(
            pod_row_detail(&node(), &KindObject::Plain),
            PodRowDetail::NamespaceAndStatus
        );
        let deployment = PodOwner::Deployment {
            namespace: "ns".to_owned(),
            name: "web".to_owned(),
        };
        assert_eq!(
            pod_row_detail(&deployment, &KindObject::Plain),
            PodRowDetail::StatusOnly
        );
        assert_eq!(
            PodRowDetail::StatusAndClaims(&templates).heading(),
            "Pods by ordinal"
        );
        assert_eq!(PodRowDetail::Attempt.heading(), "Attempts");
        assert_eq!(PodRowDetail::StatusAndNode.heading(), "Pods");
    }

    #[test]
    fn ordinal_detail_lists_claims_of_the_pod() {
        let templates = [template("data", Some("10Gi")), template("logs", None)];
        let mut web0 = pod("web-0", None);
        web0.containers = vec![container(
            ContainerKind::Main,
            running(),
            vec![
                mount_of("data-web-0"),
                mount_of("logs-web-0"),
                mount_of("data-web-1"),
            ],
        )];
        assert_eq!(pod_claims(&web0, &templates), ["data 10Gi", "logs"]);
        // A claim of another pod, or no mounts at all, lists nothing.
        let mut web1 = pod("web-1", None);
        web1.containers = vec![container(
            ContainerKind::Main,
            running(),
            vec![mount_of("data-web-0")],
        )];
        assert!(pod_claims(&web1, &templates).is_empty());
        assert!(pod_claims(&pod("web-2", None), &templates).is_empty());
        // Ordinal order, with a pod without an ordinal last.
        let owner = controller(STATEFUL_SET_KIND);
        let (a, b, c) = (
            pod("web-10", None),
            pod("web-abc", None),
            pod("web-2", None),
        );
        let mut pods = vec![&a, &b, &c];
        sort_pods(&mut pods, &owner, PodRowDetail::StatusAndClaims(&templates));
        let names: Vec<&str> = pods.iter().map(|pod| pod.name.as_str()).collect();
        assert_eq!(names, ["web-2", "web-10", "web-abc"]);
    }

    #[test]
    fn attempts_newest_first_with_exit_code() {
        let owner = controller(JOB_KIND);
        let (old, new, unstamped) = (
            pod("a-old", Some(100)),
            pod("a-new", Some(200)),
            pod("a-none", None),
        );
        let mut pods = vec![&old, &unstamped, &new];
        sort_pods(&mut pods, &owner, PodRowDetail::Attempt);
        let names: Vec<&str> = pods.iter().map(|pod| pod.name.as_str()).collect();
        assert_eq!(names, ["a-new", "a-old", "a-none"]);
        // The exit code is the first main container, its current state before its last one.
        let termination = |exit_code| Termination {
            reason: None,
            exit_code,
            signal: None,
            started_at: None,
            finished_at: None,
        };
        let mut failed = pod("a-failed", None);
        failed.containers = vec![
            container(
                ContainerKind::Init,
                ContainerState::Terminated(termination(9)),
                Vec::new(),
            ),
            container(
                ContainerKind::Main,
                ContainerState::Terminated(termination(137)),
                Vec::new(),
            ),
        ];
        assert_eq!(exit_text(&failed).as_deref(), Some("exit 137"));
        let mut restarted = pod("a-restarted", None);
        let mut main = container(ContainerKind::Main, running(), Vec::new());
        main.last_termination = Some(termination(1));
        restarted.containers = vec![main];
        assert_eq!(exit_text(&restarted).as_deref(), Some("exit 1"));
        assert_eq!(exit_text(&pod("a-running", None)), None);
    }

    fn owned_by(kind: &str, mut pod: PodSummary) -> PodSummary {
        pod.controller = Some(cluster::ControllerRef {
            kind: kind.to_owned(),
            name: "owner".to_owned(),
        });
        pod
    }

    fn budget(allowed: u32, healthy: u32) -> PodDisruptionBudgetSummary {
        PodDisruptionBudgetSummary {
            namespace: "ns".to_owned(),
            name: "api-pdb".to_owned(),
            created_at: None,
            labels: Vec::new(),
            min_available: None,
            max_unavailable: None,
            selector: cluster::Selector::of_labels(&["app=api".to_owned()]),
            current_healthy: healthy,
            desired_healthy: 2,
            expected_pods: 2,
            disruptions_allowed: allowed,
            unhealthy_pod_eviction_policy: None,
            conditions: Vec::new(),
            is_status_stale: false,
        }
    }

    #[test]
    fn a_replica_set_pod_with_nothing_special_has_no_tag() {
        let pod = owned_by("ReplicaSet", pod("web-1", None));
        assert_eq!(drain_tags(&pod, &[]), Vec::new());
    }

    #[test]
    fn a_daemon_set_pod_is_tagged_ds() {
        let pod = owned_by(DAEMON_SET_KIND, pod("agent", None));
        assert_eq!(drain_tags(&pod, &[]), [DrainTag::DaemonSet]);
        assert_eq!(DrainTag::DaemonSet.label(), "DS");
    }

    #[test]
    fn a_pod_mounting_an_empty_dir_is_tagged() {
        let mut pod = owned_by("ReplicaSet", pod("web-1", None));
        let mut scratch = mount_of("unused");
        scratch.source = VolumeSource::EmptyDir;
        pod.containers = vec![container(ContainerKind::Main, running(), vec![scratch])];
        assert_eq!(drain_tags(&pod, &[]), [DrainTag::EmptyDir]);
        assert_eq!(DrainTag::EmptyDir.label(), "emptyDir");
    }

    #[test]
    fn a_pod_without_a_controller_is_tagged() {
        assert_eq!(
            drain_tags(&pod("naked", None), &[]),
            [DrainTag::NoController]
        );
        assert_eq!(DrainTag::NoController.label(), "no controller");
    }

    #[test]
    fn a_pod_under_a_budget_that_allows_none_is_tagged_pdb_0() {
        let mut pod = owned_by("ReplicaSet", pod("api-1", None));
        pod.labels = vec!["app=api".to_owned()];
        let full = budget(0, 2);
        assert_eq!(drain_tags(&pod, &[&full]), [DrainTag::ZeroDisruptions]);
        assert_eq!(DrainTag::ZeroDisruptions.label(), "PDB 0");
        // A budget with room, one of another namespace, and one of other labels do not count.
        let roomy = budget(1, 2);
        let mut elsewhere = budget(0, 2);
        elsewhere.namespace = "other".to_owned();
        let mut unrelated = budget(0, 2);
        unrelated.selector = cluster::Selector::of_labels(&["app=db".to_owned()]);
        assert_eq!(
            drain_tags(&pod, &[&roomy, &elsewhere, &unrelated]),
            Vec::new()
        );
    }

    #[test]
    fn a_finished_pod_has_no_tag() {
        let mut pod = pod("job-1", None);
        pod.is_finished = true;
        assert_eq!(drain_tags(&pod, &[]), Vec::new());
    }
}
