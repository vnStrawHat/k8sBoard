use cluster::ConditionStatus;

use super::*;

fn at(seconds: i64) -> jiff::Timestamp {
    jiff::Timestamp::from_second(seconds).expect("valid timestamp")
}

fn system(os: &str, arch: &str, image: &str) -> NodeSystemInfo {
    NodeSystemInfo {
        operating_system: os.to_owned(),
        architecture: arch.to_owned(),
        os_image: image.to_owned(),
        kernel_version: String::new(),
        container_runtime: String::new(),
    }
}

#[test]
fn os_text_leaves_out_empty_parts_and_their_separators() {
    assert_eq!(
        os_text(&system("linux", "amd64", "Ubuntu 22.04.4 LTS")).as_deref(),
        Some("linux/amd64 · Ubuntu 22.04.4 LTS")
    );
    assert_eq!(os_text(&system("linux", "", "")).as_deref(), Some("linux"));
    assert_eq!(
        os_text(&system("", "", "Flatcar")).as_deref(),
        Some("Flatcar")
    );
    assert_eq!(os_text(&system("", "", "")), None);
}

#[test]
fn condition_detail_joins_reason_and_age() {
    let condition = |reason: Option<&str>, changed: Option<i64>| NodeCondition {
        name: "Ready".to_owned(),
        status: ConditionStatus::True,
        reason: reason.map(str::to_owned),
        message: None,
        changed_at: changed.map(at),
    };
    let now = at(7_200);
    assert_eq!(
        condition_detail(&condition(Some("KubeletReady"), Some(0)), now),
        "KubeletReady · since 2h"
    );
    assert_eq!(
        condition_detail(&condition(Some("KubeletReady"), None), now),
        "KubeletReady"
    );
    assert_eq!(condition_detail(&condition(None, Some(0)), now), "since 2h");
    assert_eq!(condition_detail(&condition(None, None), now), "");
}

fn node_with(allocatable: &[(&str, &str)]) -> NodeSummary {
    NodeSummary {
        name: "wk-1".to_owned(),
        status: cluster::NodeStatus {
            readiness: cluster::NodeReadiness::Ready,
            scheduling: cluster::NodeScheduling::Enabled,
        },
        roles: Vec::new(),
        taints: Vec::new(),
        kubelet_version: "v1.29.5".to_owned(),
        internal_ip: None,
        created_at: None,
        conditions: Vec::new(),
        addresses: Vec::new(),
        system: NodeSystemInfo::default(),
        resources: allocatable
            .iter()
            .map(|(name, value)| cluster::NodeResource {
                name: (*name).to_owned(),
                capacity: None,
                allocatable: Some((*value).to_owned()),
            })
            .collect(),
        labels: Vec::new(),
    }
}

fn usage(cores: f64, gibibytes: u64) -> ResourceUsage {
    ResourceUsage {
        cpu: CpuAmount::from_nanocores((cores * 1e9) as u64),
        memory: cluster::ByteAmount::from_bytes(gibibytes << 30),
    }
}

#[test]
fn allocatable_rows_show_used_of_allocatable_with_bars() {
    let node = node_with(&[("cpu", "15800m"), ("memory", "61Gi"), ("pods", "110")]);
    let pods = NodePods {
        cpu_request: CpuAmount::from_nanocores(7_900_000_000),
        memory_request: cluster::ByteAmount::from_bytes(30 << 30),
        count: 55,
    };
    let rows = allocatable_rows(&node, Some(usage(9.8, 43)), Some(&pods));
    let values: Vec<_> = rows
        .iter()
        .map(|row| (row.label, row.value.as_str()))
        .collect();
    assert_eq!(
        values,
        [
            ("CPU", "9.8 / 15.8 cores"),
            ("Memory", "43 / 61Gi"),
            ("Pods", "55 / 110")
        ]
    );
    let cpu_bar = rows[0].bar.as_ref().expect("a CPU bar");
    assert!((cpu_bar.fill - 0.620).abs() < 0.001, "{}", cpu_bar.fill);
    assert_eq!(cpu_bar.marker, Some(0.5));
    assert_eq!(rows[2].bar.as_ref().expect("a pods bar").marker, None);
}

#[test]
fn allocatable_rows_without_the_all_scope_leave_out_requests_and_pods() {
    let node = node_with(&[("cpu", "4"), ("memory", "8Gi")]);
    let rows = allocatable_rows(&node, Some(usage(1.0, 2)), None);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].bar.as_ref().expect("a bar").marker, None);
}

#[test]
fn allocatable_rows_are_dashes_without_a_sample_or_allocatable() {
    let node = node_with(&[("cpu", "4")]);
    let rows = allocatable_rows(&node, None, None);
    assert!(rows.iter().all(|row| row.value == "—" && row.bar.is_none()));
    let no_memory = allocatable_rows(&node, Some(usage(1.0, 2)), None);
    assert_eq!(no_memory[0].value, "1 / 4 cores");
    assert_eq!(no_memory[1].value, "—");
}

#[test]
fn pod_count_row_needs_a_limit_for_a_bar() {
    assert_eq!(pod_count_row(7, None).value, "7");
    assert_eq!(pod_count_row(7, None).bar, None);
    assert_eq!(pod_count_row(7, Some(0)).bar, None);
    assert!(pod_count_row(7, Some(14)).bar.is_some());
}
