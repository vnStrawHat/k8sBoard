use cluster::{
    ByteAmount, ContainerKind, ContainerResource, ContainerState, ContainerSummary, CpuAmount,
    NodeMetrics, NodeReadiness, NodeResource, NodeScheduling, NodeStatus, NodeSystemInfo,
    PodStatus, PvcUsage, ReadyCount, ResourceUsage, StatusReason,
};

use crate::kind_row::KindObject;

use super::*;

const GI: u64 = 1 << 30;

fn node(name: &str, cpu: &str, memory: &str, pods: &str) -> NodeSummary {
    let resource = |name: &str, allocatable: &str| NodeResource {
        name: name.to_owned(),
        capacity: None,
        allocatable: Some(allocatable.to_owned()),
    };
    NodeSummary {
        name: name.to_owned(),
        status: NodeStatus {
            readiness: NodeReadiness::Ready,
            scheduling: NodeScheduling::Enabled,
        },
        roles: Vec::new(),
        taints: Vec::new(),
        kubelet_version: "v1.29.5".to_owned(),
        internal_ip: None,
        created_at: None,
        conditions: Vec::new(),
        addresses: Vec::new(),
        system: NodeSystemInfo::default(),
        resources: vec![
            resource("cpu", cpu),
            resource("memory", memory),
            resource("pods", pods),
        ],
        labels: Vec::new(),
    }
}

fn container(kind: ContainerKind, cpu: &str, memory: &str) -> ContainerSummary {
    let request = |name: &str, request: &str| ContainerResource {
        name: name.to_owned(),
        request: Some(request.to_owned()),
        limit: None,
    };
    ContainerSummary {
        terminal: cluster::ContainerTerminal::None,
        name: "c".to_owned(),
        image: "img".to_owned(),
        kind,
        state: ContainerState::NotReported,
        is_ready: true,
        restart_count: 0,
        last_termination: None,
        image_digest: None,
        pull_policy: None,
        is_started: None,
        ports: Vec::new(),
        resources: vec![request("cpu", cpu), request("memory", memory)],
        probes: cluster::ContainerProbes::default(),
        env: Vec::new(),
        env_from: Vec::new(),
        mounts: Vec::new(),
    }
}

fn pod(name: &str, node: &str, status: StatusReason, cpu: &str, memory: &str) -> PodSummary {
    PodSummary {
        annotations: cluster::AnnotationTerms::default(),
        is_finished: false,
        namespace: "ns".to_owned(),
        name: name.to_owned(),
        status: PodStatus::Reason(status),
        ready: ReadyCount { ready: 1, total: 1 },
        restarts: 0,
        node_name: Some(node.to_owned()),
        created_at: None,
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
        containers: vec![container(ContainerKind::Main, cpu, memory)],
    }
}

fn sample(name: &str, millicores: u64, gibibytes: u64) -> NodeMetrics {
    NodeMetrics {
        name: name.to_owned(),
        sampled_at: None,
        usage: ResourceUsage {
            cpu: CpuAmount::from_nanocores(millicores * 1_000_000),
            memory: ByteAmount::from_bytes(gibibytes * GI),
        },
    }
}

fn history(samples: &[NodeMetrics]) -> NodeUsageHistory {
    let mut history = NodeUsageHistory::default();
    history.record(jiff::Timestamp::UNIX_EPOCH, samples);
    history
}

fn inputs<'a>(
    nodes: &'a [NodeSummary],
    pods: Option<&'a [PodSummary]>,
    usage: Option<&'a NodeUsageHistory>,
) -> CapacityInputs<'a> {
    CapacityInputs {
        nodes,
        pods: pods.map_or(FromPods::NeedsAllNamespaces, FromPods::Known),
        node_usage: usage,
        volumes: VolumeFeed::Live(totals(0, 0, 0)),
    }
}

fn totals(used: u64, capacity: u64, claims: usize) -> VolumeTotals {
    VolumeTotals {
        used,
        capacity,
        claims,
        shared_claims: 0,
        is_sharing_unchecked: false,
        limited: None,
    }
}

fn label(row: &CapacityRow) -> String {
    row.label_parts()
        .into_iter()
        .map(|part| part.text)
        .collect()
}

fn layers(row: &CapacityRow) -> &Layers {
    match row {
        CapacityRow::Cpu(layers) | CapacityRow::Memory(layers) => layers,
        other => panic!("not a compute row: {other:?}"),
    }
}

fn two_nodes() -> Vec<NodeSummary> {
    vec![node("a", "4", "16Gi", "110"), node("b", "4", "16Gi", "110")]
}

#[test]
fn cpu_and_memory_sum_used_requested_allocatable() {
    let nodes = two_nodes();
    let pods = [
        pod("p1", "a", StatusReason::Running, "500m", "1Gi"),
        pod("p2", "b", StatusReason::Running, "1", "2Gi"),
    ];
    let usage = history(&[sample("a", 1_000, 4), sample("b", 500, 2)]);
    let rows = cluster_capacity(&inputs(&nodes, Some(&pods), Some(&usage)));
    let cpu = layers(&rows[0]);
    assert_eq!(cpu.used, Some(1.5));
    assert_eq!(cpu.requested, FromPods::Known(1.5));
    assert_eq!(cpu.allocatable, 8.);
    let memory = layers(&rows[1]);
    assert_eq!(memory.used, Some(6. * GI as f64));
    assert_eq!(memory.requested, FromPods::Known(3. * GI as f64));
    assert_eq!(memory.allocatable, 32. * GI as f64);
}

#[test]
fn requests_are_none_without_all_scope_pods() {
    let nodes = two_nodes();
    let rows = cluster_capacity(&inputs(&nodes, None, None));
    assert_eq!(layers(&rows[0]).requested, FromPods::NeedsAllNamespaces);
}

#[test]
fn finished_pods_do_not_request() {
    let nodes = two_nodes();
    let pods = [
        pod("done", "a", StatusReason::Completed, "2", "1Gi"),
        pod("live", "a", StatusReason::Running, "1", "1Gi"),
    ];
    let rows = cluster_capacity(&inputs(&nodes, Some(&pods), None));
    assert_eq!(layers(&rows[0]).requested, FromPods::Known(1.));
}

#[test]
fn used_notes_unsampled_nodes() {
    let nodes = two_nodes();
    let usage = history(&[sample("a", 1_000, 4)]);
    let rows = cluster_capacity(&inputs(&nodes, None, Some(&usage)));
    assert_eq!(layers(&rows[0]).unsampled_nodes, 1);
    let note = rows[0].note().unwrap();
    assert!(note.starts_with("used from 1 of 2 nodes"), "{note}");
}

#[test]
fn used_is_none_without_node_feed() {
    let nodes = two_nodes();
    let rows = cluster_capacity(&inputs(&nodes, None, None));
    assert_eq!(layers(&rows[0]).used, None);
    assert_eq!(layers(&rows[0]).unsampled_nodes, 0);
    let empty = history(&[]);
    let rows = cluster_capacity(&inputs(&nodes, None, Some(&empty)));
    assert_eq!(layers(&rows[0]).used, None);
}

#[test]
fn pods_row_counts_pods_that_take_room() {
    let nodes = two_nodes();
    let pods = [
        pod("p1", "a", StatusReason::Running, "1", "1Gi"),
        pod("p2", "b", StatusReason::Running, "1", "1Gi"),
        pod("done", "b", StatusReason::Succeeded, "1", "1Gi"),
    ];
    let rows = cluster_capacity(&inputs(&nodes, Some(&pods), None));
    assert_eq!(
        rows[2],
        CapacityRow::Pods {
            taking_room: FromPods::Known(2),
            allocatable: 220
        }
    );
}

#[test]
fn pods_row_without_all_scope_shows_capacity_only() {
    let nodes = two_nodes();
    let rows = cluster_capacity(&inputs(&nodes, None, None));
    assert_eq!(rows[2].note(), None);
    assert_eq!(label(&rows[2]), "220 capacity");
}

fn usage(claim: &str, used: Option<u64>, capacity: Option<u64>) -> PvcUsage {
    PvcUsage {
        namespace: "ns".to_owned(),
        claim: claim.to_owned(),
        sampled_at: None,
        used: used.map(ByteAmount::from_bytes),
        capacity: capacity.map(ByteAmount::from_bytes),
        available: None,
        inodes_used: None,
        inodes: None,
    }
}

fn claim_object(name: &str, capacity: &str) -> KindObject {
    KindObject::PersistentVolumeClaim(cluster::PersistentVolumeClaimSummary {
        namespace: "ns".to_owned(),
        name: name.to_owned(),
        created_at: None,
        labels: Vec::new(),
        phase: "Bound".to_owned(),
        is_terminating: false,
        volume: None,
        capacity: Some(capacity.to_owned()),
        requested: None,
        access_modes: Vec::new(),
        storage_class: None,
        volume_mode: None,
        conditions: Vec::new(),
    })
}

#[test]
fn volumes_sum_claims_with_capacity() {
    let usages = [
        usage("a", Some(2 * GI), Some(10 * GI)),
        usage("b", None, Some(5 * GI)),
        usage("c", Some(GI), None),
    ];
    let claims = [claim_object("a", "10Gi"), claim_object("b", "5Gi")];
    let totals = volume_totals(usages.iter(), Some(&claims));
    assert_eq!(
        totals,
        VolumeTotals {
            used: 2 * GI,
            capacity: 15 * GI,
            claims: 2,
            shared_claims: 0,
            is_sharing_unchecked: false,
            limited: None
        }
    );
}

#[test]
fn volumes_skip_shared_filesystem_claims() {
    // The kubelet reports a 500 GiB node disk for a claim of 10 GiB: it is not the claim's volume.
    let usages = [
        usage("own", Some(2 * GI), Some(10 * GI)),
        usage("hostpath", Some(200 * GI), Some(500 * GI)),
    ];
    let claims = [
        claim_object("own", "10Gi"),
        claim_object("hostpath", "10Gi"),
    ];
    let summed = volume_totals(usages.iter(), Some(&claims));
    assert_eq!((summed.used, summed.capacity), (2 * GI, 10 * GI));
    assert_eq!((summed.claims, summed.shared_claims), (1, 1));
    let note = CapacityRow::Volumes(VolumeFeed::Live(summed)).note();
    assert_eq!(note.as_deref(), Some("1 claim shares a node disk"));
    let mut several = totals(GI, 10 * GI, 3);
    several.shared_claims = 3;
    assert_eq!(
        CapacityRow::Volumes(VolumeFeed::Live(several))
            .note()
            .as_deref(),
        Some("3 claims share node disks")
    );
}

#[test]
fn volumes_note_unchecked_sharing_without_claim_list() {
    let usages = [usage("a", Some(GI), Some(10 * GI))];
    let totals = volume_totals(usages.iter(), None);
    assert_eq!(totals.claims, 1);
    assert_eq!(
        CapacityRow::Volumes(VolumeFeed::Live(totals))
            .note()
            .as_deref(),
        Some("Totals may include node filesystems")
    );
}

#[test]
fn volumes_row_without_feed_shows_dash_and_reason() {
    let nodes = two_nodes();
    let mut input = inputs(&nodes, None, None);
    input.volumes = VolumeFeed::Unavailable {
        reason: Some("denied".to_owned()),
    };
    let rows = cluster_capacity(&input);
    let volumes = rows.last().unwrap();
    assert_eq!(volumes.name(), "Volumes");
    assert_eq!(label(volumes), "—");
    assert_eq!(volumes.unavailable_reason(), Some("denied"));
    assert_eq!(volumes.ratios(), (None, None));
}

#[test]
fn pods_loading_shows_dash_without_scope_note() {
    let nodes = two_nodes();
    let mut input = inputs(&nodes, None, None);
    input.pods = FromPods::Pending;
    let rows = cluster_capacity(&input);
    assert_eq!(layers(&rows[0]).requested, FromPods::Pending);
    assert_eq!(label(&rows[0]), "— used · — req · 8 cores");
    assert_eq!(rows[0].note(), None);
    assert!(!rows[0].has_requested());
    assert_eq!(label(&rows[2]), "— running / 220 capacity");
    assert_eq!(rows[2].note(), None);
}

#[test]
fn volumes_omitted_without_claims() {
    let nodes = two_nodes();
    let mut input = inputs(&nodes, None, None);
    input.volumes = VolumeFeed::Live(totals(0, 0, 0));
    let rows = cluster_capacity(&input);
    assert!(rows.iter().all(|row| row.name() != "Volumes"));
}

#[test]
fn volumes_note_limited_polling() {
    let mut limited = totals(GI, 10 * GI, 3);
    limited.limited = Some("10 of 42 nodes polled".to_owned());
    let row = CapacityRow::Volumes(VolumeFeed::Live(limited));
    assert_eq!(
        row.note().as_deref(),
        Some("Volume usage: 10 of 42 nodes polled.")
    );
    assert_eq!(
        CapacityRow::Volumes(VolumeFeed::Live(totals(GI, 10 * GI, 3))).note(),
        None
    );
}

#[test]
fn zero_allocatable_omits_row() {
    let nodes = [node("a", "0", "0", "0")];
    assert!(cluster_capacity(&inputs(&nodes, None, None)).is_empty());
}

#[test]
fn compute_label_prints_a_unit_on_every_figure() {
    let nodes = [node("a", "168", "16Gi", "110")];
    let pods = [pod("p", "a", StatusReason::Running, "131", "1Gi")];
    let usage = history(&[sample("a", 104_000, 1)]);
    let rows = cluster_capacity(&inputs(&nodes, Some(&pods), Some(&usage)));
    assert_eq!(
        label(&rows[0]),
        "104 cores used · 131 cores req · 168 cores"
    );
}

#[test]
fn memory_label_prints_a_unit_on_every_figure() {
    let nodes = [node("a", "168", "101Gi", "110")];
    let pods = [pod("p", "a", StatusReason::Running, "1", "30Gi")];
    let usage = history(&[sample("a", 1_000, 65)]);
    let rows = cluster_capacity(&inputs(&nodes, Some(&pods), Some(&usage)));
    assert_eq!(label(&rows[1]), "65Gi used · 30Gi req · 101Gi");
}

#[test]
fn label_drops_req_without_requests() {
    let nodes = [node("a", "168", "16Gi", "110")];
    let usage = history(&[sample("a", 104_000, 1)]);
    let rows = cluster_capacity(&inputs(&nodes, None, Some(&usage)));
    assert_eq!(label(&rows[0]), "104 cores used · 168 cores");
    let rows = cluster_capacity(&inputs(&nodes, None, None));
    assert_eq!(label(&rows[0]), "— used · 168 cores");
}

#[test]
fn pods_label_groups_digits() {
    let row = CapacityRow::Pods {
        taking_room: FromPods::Known(1_284),
        allocatable: 4_620,
    };
    assert_eq!(label(&row), "1,284 running / 4,620 capacity");
}

#[test]
fn volumes_label_shares_unit() {
    let row = CapacityRow::Volumes(VolumeFeed::Live(totals(2 * GI, 10 * GI, 3)));
    assert_eq!(label(&row), "2 / 10Gi · 3 PVCs");
}

#[test]
fn compute_rows_name_their_ceilings() {
    let nodes = two_nodes();
    let rows = cluster_capacity(&inputs(&nodes, None, None));
    assert!(rows[0].ceiling().unwrap().contains("Init-container"));
    assert!(rows[1].ceiling().unwrap().contains("Init-container"));
    let pods_ceiling = rows[2].ceiling().unwrap();
    assert!(pods_ceiling.contains("NotReady") && !pods_ceiling.contains("Init-container"));
}

#[test]
fn pods_on_unlisted_nodes_do_not_count() {
    let nodes = two_nodes();
    let pods = [
        pod("p1", "a", StatusReason::Running, "1", "1Gi"),
        pod("gone", "removed", StatusReason::Running, "4", "1Gi"),
    ];
    let rows = cluster_capacity(&inputs(&nodes, Some(&pods), None));
    assert_eq!(layers(&rows[0]).requested, FromPods::Known(1.));
    assert_eq!(
        rows[2],
        CapacityRow::Pods {
            taking_room: FromPods::Known(1),
            allocatable: 220
        }
    );
}

#[test]
fn volumes_row_stays_when_every_claim_shares_a_disk() {
    let nodes = two_nodes();
    let mut input = inputs(&nodes, None, None);
    let mut shared = totals(0, 0, 0);
    shared.shared_claims = 4;
    input.volumes = VolumeFeed::Live(shared);
    let rows = cluster_capacity(&input);
    let volumes = rows.last().unwrap();
    assert_eq!(volumes.name(), "Volumes");
    assert_eq!(label(volumes), "—");
    assert_eq!(volumes.note().as_deref(), Some("4 claims share node disks"));
    assert_eq!(volumes.ratios(), (None, None));
}
