use serde_json::{Value, json};

use super::*;
use crate::resource_quota::QuotaItem;

const GI: u64 = 1 << 30;

fn deployment(replicas: Option<u64>, template_spec: Value) -> Value {
    let mut spec = json!({"template": {"spec": template_spec}});
    if let Some(replicas) = replicas {
        spec["replicas"] = json!(replicas);
    }
    json!({"kind": "Deployment", "spec": spec})
}

fn container(requests: Value, limits: Value) -> Value {
    json!({"name": "c", "resources": {"requests": requests, "limits": limits}})
}

fn demand_of(object: &Value) -> WorkloadDemand {
    workload_demand(ObjectKind::Deployment, object).expect("a deployment has demand")
}

#[test]
fn demand_counts_replicas_default_one() {
    let template = json!({"containers": [container(json!({"cpu": "250m"}), json!({}))]});
    let single = demand_of(&deployment(None, template.clone()));
    assert_eq!((single.pods, single.requests_cpu), (1, 250_000_000));
    let triple = demand_of(&deployment(Some(3), template));
    assert_eq!((triple.pods, triple.requests_cpu), (3, 750_000_000));
}

#[test]
fn demand_sums_containers_and_sidecars() {
    let mut sidecar = container(json!({"cpu": "100m", "memory": "64Mi"}), json!({}));
    sidecar["restartPolicy"] = json!("Always");
    let object = deployment(
        Some(1),
        json!({
            "initContainers": [sidecar],
            "containers": [
                container(json!({"cpu": "200m", "memory": "128Mi"}), json!({})),
                container(json!({"cpu": "300m", "memory": "128Mi"}), json!({})),
            ],
        }),
    );
    let demand = demand_of(&object);
    assert_eq!(demand.requests_cpu, 600_000_000);
    assert_eq!(demand.requests_memory, 320 * (1 << 20));
}

#[test]
fn demand_takes_a_larger_init_container() {
    let object = deployment(
        Some(2),
        json!({
            "initContainers": [container(json!({"memory": "2Gi"}), json!({}))],
            "containers": [container(json!({"memory": "512Mi"}), json!({}))],
        }),
    );
    assert_eq!(demand_of(&object).requests_memory, 2 * 2 * GI);
}

#[test]
fn demand_adds_earlier_sidecars_to_a_regular_init() {
    let mut sidecar = container(json!({"memory": "1Gi"}), json!({}));
    sidecar["restartPolicy"] = json!("Always");
    let object = deployment(
        Some(1),
        json!({
            "initContainers": [sidecar, container(json!({"memory": "2Gi"}), json!({}))],
            "containers": [container(json!({"memory": "512Mi"}), json!({}))],
        }),
    );
    assert_eq!(demand_of(&object).requests_memory, 3 * GI);
}

#[test]
fn demand_uses_desired_scheduled_for_daemon_sets() {
    let object = json!({
        "kind": "DaemonSet",
        "spec": {"template": {"spec": {"containers": [container(json!({"cpu": "1"}), json!({}))]}}},
        "status": {"desiredNumberScheduled": 4},
    });
    let demand = workload_demand(ObjectKind::DaemonSet, &object).expect("a daemon set");
    assert_eq!((demand.pods, demand.requests_cpu), (4, 4_000_000_000));
    let unscheduled = json!({"spec": object["spec"]});
    let demand = workload_demand(ObjectKind::DaemonSet, &unscheduled).expect("a daemon set");
    assert_eq!(demand.pods, 0);
}

#[test]
fn demand_skips_unparsable_quantities() {
    let object = deployment(
        Some(1),
        json!({"containers": [container(json!({"cpu": "lots", "memory": "1Gi"}), json!({}))]}),
    );
    let demand = demand_of(&object);
    assert_eq!((demand.requests_cpu, demand.requests_memory), (0, GI));
}

#[test]
fn demand_request_defaults_to_limit() {
    let object = deployment(
        Some(1),
        json!({"containers": [container(json!({}), json!({"memory": "1Gi"}))]}),
    );
    let demand = demand_of(&object);
    assert_eq!((demand.requests_memory, demand.limits_memory), (GI, GI));
    let neither = deployment(
        Some(1),
        json!({"containers": [container(json!({}), json!({}))]}),
    );
    assert_eq!(demand_of(&neither).requests_memory, 0);
}

#[test]
fn demand_is_none_for_other_kinds() {
    let object = json!({"spec": {}});
    assert_eq!(workload_demand(ObjectKind::ConfigMap, &object), None);
    assert_eq!(workload_demand(ObjectKind::Pod, &object), None);
}

fn quota(
    name: &str,
    items: &[(&str, &str, Option<&str>)],
    scopes: &[&str],
) -> ResourceQuotaSummary {
    ResourceQuotaSummary {
        namespace: "shop".to_owned(),
        name: name.to_owned(),
        created_at: None,
        labels: Vec::new(),
        items: items
            .iter()
            .map(|(resource, hard, used)| QuotaItem {
                resource: (*resource).to_owned(),
                hard: (*hard).to_owned(),
                used: used.map(str::to_owned),
            })
            .collect(),
        scopes: scopes.iter().map(|scope| (*scope).to_owned()).collect(),
    }
}

fn growth(pods: u64, cpu: u64, memory: u64) -> DemandChange {
    DemandChange {
        before: WorkloadDemand::default(),
        after: WorkloadDemand {
            pods,
            requests_cpu: cpu,
            requests_memory: memory,
            ..WorkloadDemand::default()
        },
    }
}

#[test]
fn check_not_affected_when_nothing_grows() {
    let quotas = [quota("q", &[("pods", "10", Some("9"))], &[])];
    let same = DemandChange {
        before: growth(2, 0, 0).after,
        after: growth(2, 0, 0).after,
    };
    assert_eq!(quota_check(&same, &quotas), QuotaCheck::NotAffected);
    let smaller = DemandChange {
        before: growth(4, 0, 0).after,
        after: growth(2, 0, 0).after,
    };
    assert_eq!(quota_check(&smaller, &quotas), QuotaCheck::NotAffected);
}

#[test]
fn check_fits_reports_smallest_fraction() {
    // CPU: 2 of 40 cores left after the change (5 %); memory: 20Gi of 64Gi (31 %).
    let quotas = [quota(
        "compute",
        &[
            ("requests.cpu", "40", Some("37")),
            ("requests.memory", "64Gi", Some("40Gi")),
        ],
        &[],
    )];
    let change = growth(0, 1_000_000_000, 4 * GI);
    assert_eq!(
        quota_check(&change, &quotas),
        QuotaCheck::Fits {
            quota: "compute".to_owned(),
            resource: QuotaResource::RequestsCpu,
            left: 2_000_000_000,
            hard: 40_000_000_000,
        }
    );
}

#[test]
fn check_fits_compares_fractions_not_raw_numbers() {
    // Pods: 5 left of 10 (50 %) is the smaller raw number; memory: 100Gi left of 10Ti (about 1 %)
    // is the larger raw number but the tighter fraction.
    let quotas = [quota(
        "q",
        &[
            ("pods", "10", Some("4")),
            ("requests.memory", "10Ti", Some("9.9Ti")),
        ],
        &[],
    )];
    let change = growth(1, 0, GI);
    let QuotaCheck::Fits { resource, .. } = quota_check(&change, &quotas) else {
        panic!("expected Fits");
    };
    assert_eq!(resource, QuotaResource::RequestsMemory);
}

#[test]
fn check_exceeds_lists_every_shortfall() {
    let quotas = [quota(
        "compute",
        &[
            ("requests.memory", "8Gi", Some("7Gi")),
            ("pods", "10", Some("9")),
            ("requests.cpu", "40", Some("0")),
        ],
        &[],
    )];
    let change = growth(3, 1_000_000_000, 2 * GI);
    assert_eq!(
        quota_check(&change, &quotas),
        QuotaCheck::Exceeds(vec![
            QuotaShortfall {
                quota: "compute".to_owned(),
                resource: QuotaResource::Pods,
                needed: 3,
                left: 1,
            },
            QuotaShortfall {
                quota: "compute".to_owned(),
                resource: QuotaResource::RequestsMemory,
                needed: 2 * GI,
                left: GI,
            },
        ])
    );
}

#[test]
fn check_treats_a_zero_hard_as_a_shortfall() {
    let quotas = [quota("q", &[("pods", "0", Some("0"))], &[])];
    assert!(matches!(
        quota_check(&growth(1, 0, 0), &quotas),
        QuotaCheck::Exceeds(_)
    ));
}

#[test]
fn check_reads_cpu_and_memory_aliases() {
    let quotas = [quota(
        "q",
        &[
            ("cpu", "10", Some("0")),
            ("memory", "10Gi", Some("0")),
            ("count/pods", "100", Some("0")),
        ],
        &[],
    )];
    let QuotaCheck::Fits { resource, .. } = quota_check(&growth(1, 1_000_000_000, GI), &quotas)
    else {
        panic!("expected Fits");
    };
    // 99 / 100 vs 9 / 10 vs 9.0 / 10: the memory and CPU aliases are the tightest (90 %).
    assert_eq!(resource, QuotaResource::RequestsCpu);
    assert_eq!(
        QuotaResource::of_item("memory"),
        Some(QuotaResource::RequestsMemory)
    );
    assert_eq!(
        QuotaResource::of_item("count/pods"),
        Some(QuotaResource::Pods)
    );
}

#[test]
fn check_skips_scoped_and_unsynced_quotas() {
    let quotas = [
        quota("scoped", &[("pods", "1", Some("1"))], &["BestEffort"]),
        quota("unsynced", &[("pods", "1", None)], &[]),
        quota("garbled", &[("pods", "many", Some("0"))], &[]),
        quota("other", &[("services", "1", Some("1"))], &[]),
    ];
    assert_eq!(
        quota_check(&growth(1, 0, 0), &quotas),
        QuotaCheck::NotAffected
    );
}

#[test]
fn demand_types_debug_holds_numbers_only() {
    let text = format!("{:?}", growth(2, 3, 4));
    assert_eq!(
        text,
        "DemandChange { before: WorkloadDemand { pods: 0, requests_cpu: 0, requests_memory: 0, \
         limits_cpu: 0, limits_memory: 0 }, after: WorkloadDemand { pods: 2, requests_cpu: 3, \
         requests_memory: 4, limits_cpu: 0, limits_memory: 0 } }"
    );
}

#[test]
fn demand_change_grows_when_any_resource_does() {
    assert!(growth(1, 0, 0).grows());
    assert!(growth(0, 0, 1).grows());
    let same = DemandChange {
        before: growth(2, 3, 4).after,
        after: growth(2, 3, 4).after,
    };
    assert!(!same.grows());
    let smaller = DemandChange {
        before: growth(2, 3, 4).after,
        after: growth(1, 3, 4).after,
    };
    assert!(!smaller.grows());
}
