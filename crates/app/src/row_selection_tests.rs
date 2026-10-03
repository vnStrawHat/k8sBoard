use super::*;

fn labels(screen: Screen) -> Vec<&'static str> {
    bulk_actions(screen).iter().map(|item| item.label).collect()
}

#[test]
fn bulk_actions_follow_the_wireframe() {
    assert_eq!(labels(Screen::Nodes), ["Cordon", "Uncordon", "Drain…"]);
    assert_eq!(
        labels(Screen::Kind(ResourceKind::Deployments)),
        ["Scale…", "Restart", "Roll back…"]
    );
    assert_eq!(
        labels(Screen::Kind(ResourceKind::StatefulSets)),
        ["Scale…", "Restart"]
    );
    assert_eq!(labels(Screen::Kind(ResourceKind::DaemonSets)), ["Restart"]);
    assert_eq!(labels(Screen::Kind(ResourceKind::Jobs)), ["Re-run"]);
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
        action_of(Screen::Kind(ResourceKind::DaemonSets), "Restart"),
        Some(ResourceAction::RestartRollout(ObjectKind::DaemonSet))
    );
    assert_eq!(
        action_of(Screen::Kind(ResourceKind::Jobs), "Re-run"),
        Some(ResourceAction::RerunJob)
    );
    assert_eq!(
        action_of(Screen::Kind(ResourceKind::CronJobs), "Suspend"),
        Some(ResourceAction::SuspendCronJob)
    );
    // The Nodes buttons belong to 0034: they carry no action yet.
    assert_eq!(action_of(Screen::Nodes, "Cordon"), None);
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
