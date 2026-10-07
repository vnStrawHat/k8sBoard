use super::*;

#[test]
fn a_taint_and_a_selector_cause_read_in_one_short_line() {
    let message = "0/3 nodes are available: 1 node(s) had untolerated taint {node-role.kubernetes.io/control-plane: }, 2 node(s) didn't match Pod's node affinity/selector. preemption: 0/3 nodes are available: 3 Preemption is not helpful for scheduling.";
    assert_eq!(scheduler_summary(message), "0/3 nodes: 1 taint, 2 selector");
}

#[test]
fn a_volume_conflict_and_a_taint_are_told_apart() {
    let message = "0/3 nodes are available: 1 node(s) had volume node affinity conflict, 2 node(s) had untolerated taint {workload: data}, {a: b}.";
    assert_eq!(scheduler_summary(message), "0/3 nodes: 1 volume, 2 taint");
}

#[test]
fn insufficient_resources_name_the_resource() {
    let message = "0/3 nodes are available: 1 Insufficient cpu, 1 Insufficient memory, 1 node(s) were unschedulable.";
    assert_eq!(
        scheduler_summary(message),
        "0/3 nodes: 1 cpu, 1 memory, 1 cordoned"
    );
}

#[test]
fn a_cause_the_summary_does_not_know_keeps_its_own_words() {
    let message = "0/2 nodes are available: 2 node(s) had a surprise.";
    assert_eq!(
        scheduler_summary(message),
        "0/2 nodes: 2 node(s) had a surprise"
    );
}

#[test]
fn a_message_of_another_shape_is_left_as_it_is() {
    assert_eq!(scheduler_summary("not scheduled yet"), "not scheduled yet");
}
