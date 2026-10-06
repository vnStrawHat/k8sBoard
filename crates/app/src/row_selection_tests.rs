use super::*;

fn labels(screen: Screen) -> Vec<&'static str> {
    bulk_actions(screen).iter().map(|item| item.label).collect()
}

#[test]
fn bulk_actions_follow_the_wireframe() {
    assert_eq!(labels(Screen::Nodes), ["Cordon", "Uncordon", "Drain…"]);
    assert_eq!(
        labels(Screen::Kind(ResourceKind::Deployments)),
        ["Scale…", "Restart rollout", "Roll back…"]
    );
    assert_eq!(
        labels(Screen::Kind(ResourceKind::StatefulSets)),
        ["Scale…", "Restart rollout"]
    );
    assert_eq!(
        labels(Screen::Kind(ResourceKind::DaemonSets)),
        ["Restart rollout"]
    );
    assert_eq!(labels(Screen::Kind(ResourceKind::Jobs)), ["Re-run job"]);
    assert_eq!(
        labels(Screen::Kind(ResourceKind::CronJobs)),
        ["Trigger now", "Suspend"]
    );
}

#[test]
fn bulk_actions_follow_the_wireframe_with_their_actions() {
    let action_of = |screen: Screen, label: &str| {
        bulk_actions(screen)
            .iter()
            .find(|item| item.label == label)
            .and_then(|item| item.action)
    };
    let deployments = Screen::Kind(ResourceKind::Deployments);
    assert_eq!(
        action_of(deployments, "Scale…"),
        Some(ResourceAction::Scale(ObjectKind::Deployment))
    );
    assert_eq!(
        action_of(Screen::Kind(ResourceKind::StatefulSets), "Scale…"),
        Some(ResourceAction::Scale(ObjectKind::StatefulSet))
    );
    assert_eq!(
        action_of(Screen::Kind(ResourceKind::DaemonSets), "Restart rollout"),
        Some(ResourceAction::RestartRollout(ObjectKind::DaemonSet))
    );
    assert_eq!(
        action_of(Screen::Kind(ResourceKind::Jobs), "Re-run job"),
        Some(ResourceAction::RerunJob)
    );
    assert_eq!(
        action_of(Screen::Kind(ResourceKind::CronJobs), "Suspend"),
        Some(ResourceAction::SuspendCronJob)
    );
    // The Nodes buttons are spec 0034's.
    assert_eq!(
        action_of(Screen::Nodes, "Cordon"),
        Some(ResourceAction::Cordon)
    );
    assert_eq!(
        action_of(Screen::Nodes, "Uncordon"),
        Some(ResourceAction::Uncordon)
    );
    assert_eq!(
        action_of(Screen::Nodes, "Drain…"),
        Some(ResourceAction::Drain)
    );
}

#[test]
fn screens_without_bulk_actions_show_only_the_count() {
    assert!(bulk_actions(Screen::Pods).is_empty());
    for kind in [
        ResourceKind::Events,
        ResourceKind::Services,
        ResourceKind::ReplicaSets,
        ResourceKind::ConfigMaps,
    ] {
        assert!(bulk_actions(Screen::Kind(kind)).is_empty(), "{kind:?}");
    }
}

#[test]
fn the_unticked_notice_counts_the_hidden_rows() {
    assert_eq!(
        unticked_notice_text(3),
        "3 ticked rows hidden by the filter were unticked"
    );
    assert_eq!(
        unticked_notice_text(1),
        "1 ticked row hidden by the filter was unticked"
    );
}
