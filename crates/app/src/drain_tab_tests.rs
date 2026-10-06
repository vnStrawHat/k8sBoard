use std::collections::HashSet;
use std::path::PathBuf;

use gpui_kit::WeakEntity;

use super::*;
use crate::app_shell::write_flow::{DryRunState, TypedMatch, confirmed};
use crate::drain_plan::DrainOptions;
use crate::drain_run::{NextStep, NodeOutcome, RunInput};

fn tab_over(nodes: &[&str]) -> DrainTab {
    let proof = confirmed(&DryRunState::NotSupported, TypedMatch::NotNeeded, 1)
        .expect("a satisfied confirm step");
    let run = DrainRun::new(RunInput {
        nodes: nodes.iter().map(|node| (*node).to_owned()).collect(),
        to_cordon: Vec::new(),
        options: DrainOptions::default(),
        confirmed: proof,
        generation: 1,
        checked: HashSet::new(),
    });
    let cluster = ClusterRef {
        kubeconfig: PathBuf::from("/home/me/.kube/config"),
        context: "stg-ctx".to_owned(),
    };
    DrainTab::new(DrainTabInputs {
        shell: WeakEntity::new_invalid(),
        cluster,
        cluster_name: "stg-b".into(),
        run,
        identity: stub_identity(),
        note: Some("night shift".to_owned()),
    })
}

fn stub_identity() -> AuditIdentity {
    let access = crate::cluster_session::AccessState::Unknown;
    let guard = crate::write_guard::test_guard(
        &access,
        crate::write_guard::WriteLock::Unlocked,
        "stg-b",
        crate::environment::Environment::STAGING,
    );
    AuditIdentity::of(&guard)
}

#[test]
fn the_label_names_one_node_or_the_count() {
    assert_eq!(tab_over(&["wk-04"]).label(), "Drain wk-04");
    assert_eq!(tab_over(&["a", "b", "c"]).label(), "Drain 3 nodes");
}

#[test]
fn a_running_tab_is_pinned_and_warns() {
    let tab = tab_over(&["wk-04"]);
    assert!(tab.is_running());
    assert_eq!(tab.tone(), StatusTone::Warn);
    assert_eq!(tab.note(), Some("night shift"));
    assert_eq!(tab.cluster_name().as_ref(), "stg-b");
}

#[test]
fn the_dot_follows_how_the_run_ended() {
    let mut drained = tab_over(&["wk-04"]);
    drained.run_mut().on_read(Ok(Vec::new()), Duration::ZERO);
    drained.run_mut().on_node_done(NodeOutcome::Drained);
    assert!(!drained.is_running());
    assert_eq!(drained.tone(), StatusTone::Ok);

    let mut stuck = tab_over(&["wk-04"]);
    stuck.run_mut().on_read(Ok(Vec::new()), Duration::ZERO);
    stuck.run_mut().on_node_done(NodeOutcome::Stuck {
        reason: "Timed out".into(),
    });
    assert_eq!(stuck.tone(), StatusTone::Bad);

    let mut cancelled = tab_over(&["wk-04"]);
    cancelled.run_mut().cancel();
    assert_eq!(cancelled.tone(), StatusTone::Done);
    assert_eq!(
        cancelled.run().next_step(Duration::ZERO),
        NextStep::Finished
    );
}

#[test]
fn the_run_clock_starts_at_zero_and_only_moves_forward() {
    let tab = tab_over(&["wk-04"]);
    let first = tab.now();
    assert!(first < Duration::from_secs(5));
    assert!(tab.now() >= first);
}

#[test]
fn drain_again_offers_the_nodes_a_stuck_or_stopped_run_left_undrained() {
    let mut running = tab_over(&["a", "b"]);
    assert!(running.nodes_to_drain_again().is_empty());

    // Node a drains, node b gets stuck: only b is offered again.
    running.run_mut().on_read(Ok(Vec::new()), Duration::ZERO);
    running.run_mut().on_node_done(NodeOutcome::Drained);
    running.run_mut().on_read(Ok(Vec::new()), Duration::ZERO);
    running.run_mut().on_node_done(NodeOutcome::Stuck {
        reason: "Timed out".into(),
    });
    assert_eq!(running.nodes_to_drain_again(), ["b"]);

    let mut stopped = tab_over(&["wk-04"]);
    stopped.run_mut().stop("the app quit");
    assert_eq!(stopped.nodes_to_drain_again(), ["wk-04"]);

    let mut cancelled = tab_over(&["wk-04"]);
    cancelled.run_mut().cancel();
    assert!(cancelled.nodes_to_drain_again().is_empty());

    let mut drained = tab_over(&["wk-04"]);
    drained.run_mut().on_read(Ok(Vec::new()), Duration::ZERO);
    drained.run_mut().on_node_done(NodeOutcome::Drained);
    assert!(drained.nodes_to_drain_again().is_empty());
}
