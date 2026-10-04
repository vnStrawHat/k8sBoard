//! Steady-state pod demand of a workload and the advisory check against namespace quotas
//! (spec 0041). Pure functions over numbers; nothing here logs, and the derived `Debug` of the
//! demand types holds numbers only.
//!
//! `ponytail:` steady state only. Surge pods, `spec.overhead`, LimitRange defaults, and scoped
//! quotas are ignored; the check is advice, never a gate.

use serde_json::Value;

use crate::object_yaml::ObjectKind;
use crate::quantity::{ByteAmount, CpuAmount};
use crate::resource_quota::ResourceQuotaSummary;

/// Steady-state pod resources of a workload, in base units (nanocores, bytes, pods).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WorkloadDemand {
    pub pods: u64,
    pub requests_cpu: u64,
    pub requests_memory: u64,
    pub limits_cpu: u64,
    pub limits_memory: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DemandChange {
    pub before: WorkloadDemand,
    pub after: WorkloadDemand,
}

/// Declaration order is the order shortfalls are listed in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum QuotaResource {
    Pods,
    RequestsCpu,
    RequestsMemory,
    LimitsCpu,
    LimitsMemory,
}

impl QuotaResource {
    /// The quota item name, as written in a `ResourceQuota`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Pods => "pods",
            Self::RequestsCpu => "requests.cpu",
            Self::RequestsMemory => "requests.memory",
            Self::LimitsCpu => "limits.cpu",
            Self::LimitsMemory => "limits.memory",
        }
    }

    fn of_item(name: &str) -> Option<Self> {
        match name {
            "pods" | "count/pods" => Some(Self::Pods),
            "requests.cpu" | "cpu" => Some(Self::RequestsCpu),
            "requests.memory" | "memory" => Some(Self::RequestsMemory),
            "limits.cpu" => Some(Self::LimitsCpu),
            "limits.memory" => Some(Self::LimitsMemory),
            _ => None,
        }
    }

    /// Base units: nanocores for CPU, bytes for memory, a plain count for pods.
    fn parse(self, quantity: &str) -> Option<u64> {
        match self {
            Self::Pods | Self::RequestsMemory | Self::LimitsMemory => {
                ByteAmount::parse(quantity).map(ByteAmount::bytes)
            }
            Self::RequestsCpu | Self::LimitsCpu => {
                CpuAmount::parse(quantity).map(CpuAmount::nanocores)
            }
        }
    }
}

impl WorkloadDemand {
    fn of(self, resource: QuotaResource) -> u64 {
        match resource {
            QuotaResource::Pods => self.pods,
            QuotaResource::RequestsCpu => self.requests_cpu,
            QuotaResource::RequestsMemory => self.requests_memory,
            QuotaResource::LimitsCpu => self.limits_cpu,
            QuotaResource::LimitsMemory => self.limits_memory,
        }
    }

    /// Component-wise combination of two per-pod footprints.
    fn combine(self, other: Self, join: fn(u64, u64) -> u64) -> Self {
        Self {
            pods: join(self.pods, other.pods),
            requests_cpu: join(self.requests_cpu, other.requests_cpu),
            requests_memory: join(self.requests_memory, other.requests_memory),
            limits_cpu: join(self.limits_cpu, other.limits_cpu),
            limits_memory: join(self.limits_memory, other.limits_memory),
        }
    }

    /// The footprint of `count` pods, each of this footprint; `pods` becomes `count`.
    fn times(self, count: u64) -> Self {
        let scaled = self.combine(
            Self {
                pods: 0,
                requests_cpu: count,
                requests_memory: count,
                limits_cpu: count,
                limits_memory: count,
            },
            u64::saturating_mul,
        );
        Self {
            pods: count,
            ..scaled
        }
    }
}

impl DemandChange {
    /// Whether any resource, or the pod count, is larger after the change.
    pub fn grows(&self) -> bool {
        [
            QuotaResource::Pods,
            QuotaResource::RequestsCpu,
            QuotaResource::RequestsMemory,
            QuotaResource::LimitsCpu,
            QuotaResource::LimitsMemory,
        ]
        .into_iter()
        .any(|resource| self.after.of(resource) > self.before.of(resource))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QuotaCheck {
    /// Nothing grows, or no unscoped quota limits a resource that grows: no line.
    NotAffected,
    /// The tightest item after the change: the smallest `left / hard` among the growing
    /// resources (a fraction, so nanocores, bytes, and pod counts compare fairly).
    Fits {
        quota: String,
        resource: QuotaResource,
        left: u64,
        hard: u64,
    },
    /// Every item the change exceeds.
    Exceeds(Vec<QuotaShortfall>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuotaShortfall {
    pub quota: String,
    pub resource: QuotaResource,
    pub needed: u64,
    pub left: u64,
}

/// `None` for a kind without pods to count (only Deployment, StatefulSet, and DaemonSet count).
/// A DaemonSet reads `status.desiredNumberScheduled`, so `object` must still have its status.
pub(crate) fn workload_demand(kind: ObjectKind, object: &Value) -> Option<WorkloadDemand> {
    let pods = match kind {
        ObjectKind::Deployment | ObjectKind::StatefulSet => object
            .pointer("/spec/replicas")
            .and_then(Value::as_u64)
            .unwrap_or(1),
        ObjectKind::DaemonSet => object
            .pointer("/status/desiredNumberScheduled")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        _ => return None,
    };
    let per_pod = object
        .pointer("/spec/template/spec")
        .map_or_else(WorkloadDemand::default, pod_footprint);
    Some(per_pod.times(pods))
}

/// `max(sum(containers) + sum(sidecars), max(each regular init + the sidecars started before it))`
/// per resource, with `pods` left at zero. A sidecar is an init container with
/// `restartPolicy: Always`.
fn pod_footprint(pod_spec: &Value) -> WorkloadDemand {
    let mut sidecars = WorkloadDemand::default();
    let mut peak_init = WorkloadDemand::default();
    for init in containers(pod_spec, "initContainers") {
        let own = container_footprint(init);
        if init.get("restartPolicy").and_then(Value::as_str) == Some("Always") {
            sidecars = sidecars.combine(own, u64::saturating_add);
        } else {
            let during = sidecars.combine(own, u64::saturating_add);
            peak_init = peak_init.combine(during, u64::max);
        }
    }
    let running = containers(pod_spec, "containers")
        .map(container_footprint)
        .fold(sidecars, |sum, own| sum.combine(own, u64::saturating_add));
    running.combine(peak_init, u64::max)
}

fn containers<'a>(pod_spec: &'a Value, key: &str) -> impl Iterator<Item = &'a Value> {
    pod_spec
        .get(key)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
}

fn container_footprint(container: &Value) -> WorkloadDemand {
    WorkloadDemand {
        pods: 0,
        requests_cpu: amount(container, "requests", "cpu", QuotaResource::RequestsCpu),
        requests_memory: amount(
            container,
            "requests",
            "memory",
            QuotaResource::RequestsMemory,
        ),
        limits_cpu: amount(container, "limits", "cpu", QuotaResource::LimitsCpu),
        limits_memory: amount(container, "limits", "memory", QuotaResource::LimitsMemory),
    }
}

/// A missing request uses the limit, the Kubernetes rule for a pod nothing has defaulted. The
/// API server applies it to pods, not to templates, so it is applied here.
fn amount(container: &Value, section: &str, name: &str, resource: QuotaResource) -> u64 {
    let quantity = |section: &str| {
        container
            .pointer(&format!("/resources/{section}/{name}"))
            .and_then(Value::as_str)
    };
    let text = quantity(section).or_else(|| {
        if section == "requests" {
            quantity("limits")
        } else {
            None
        }
    });
    text.and_then(|text| resource.parse(text)).unwrap_or(0)
}

/// The headroom of `quotas` against the growth in `change`. Only unscoped quotas count; an item
/// without `used` (not synced) or with an unparsable quantity is skipped.
pub fn quota_check(change: &DemandChange, quotas: &[ResourceQuotaSummary]) -> QuotaCheck {
    let mut shortfalls = Vec::new();
    let mut tightest: Option<(&str, QuotaResource, u64, u64)> = None;
    for quota in quotas.iter().filter(|quota| quota.scopes.is_empty()) {
        for item in &quota.items {
            let Some(resource) = QuotaResource::of_item(&item.resource) else {
                continue;
            };
            let growth = change
                .after
                .of(resource)
                .saturating_sub(change.before.of(resource));
            if growth == 0 {
                continue;
            }
            let (Some(hard), Some(used)) = (
                resource.parse(&item.hard),
                item.used.as_deref().and_then(|used| resource.parse(used)),
            ) else {
                continue;
            };
            let left = hard.saturating_sub(used);
            if growth > left {
                shortfalls.push(QuotaShortfall {
                    quota: quota.name.clone(),
                    resource,
                    needed: growth,
                    left,
                });
                continue;
            }
            let left = left - growth;
            // `left / hard < other_left / other_hard`, cross-multiplied in `u128` so no float
            // rounding decides which item is the tightest.
            let is_tighter = tightest.is_none_or(|(_, _, other_left, other_hard)| {
                u128::from(left) * u128::from(other_hard)
                    < u128::from(other_left) * u128::from(hard)
            });
            if is_tighter {
                tightest = Some((&quota.name, resource, left, hard));
            }
        }
    }
    if !shortfalls.is_empty() {
        shortfalls.sort_by(|a, b| (&a.quota, a.resource).cmp(&(&b.quota, b.resource)));
        return QuotaCheck::Exceeds(shortfalls);
    }
    match tightest {
        Some((quota, resource, left, hard)) => QuotaCheck::Fits {
            quota: quota.to_owned(),
            resource,
            left,
            hard,
        },
        None => QuotaCheck::NotAffected,
    }
}

#[cfg(test)]
#[path = "quota_demand_tests.rs"]
mod quota_demand_tests;
