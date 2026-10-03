use futures::Stream;
use k8s_openapi::api::policy::v1::PodDisruptionBudget;
use kube::Api;

use crate::connection::{ClusterConnection, ClusterError};
use crate::namespace::NamespaceScope;
use crate::pod_status::non_negative;
use crate::resource_watch::{WatchUpdate, summary_watch};
use crate::selector::Selector;
use crate::workload::{WorkloadCondition, condition, int_or_string_text, label_terms};

const DISRUPTION_ALLOWED: &str = "DisruptionAllowed";
const SYNC_FAILED: &str = "SyncFailed";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PodDisruptionBudgetSummary {
    pub namespace: String,
    pub name: String,
    pub created_at: Option<jiff::Timestamp>,
    /// `key=value` terms in key order.
    pub labels: Vec<String>,
    /// A number or a percentage, as written.
    pub min_available: Option<String>,
    pub max_unavailable: Option<String>,
    /// `None` selects no pods; an empty selector selects every pod of the namespace.
    pub selector: Option<Selector>,
    pub current_healthy: u32,
    pub desired_healthy: u32,
    pub expected_pods: u32,
    pub disruptions_allowed: u32,
    pub unhealthy_pod_eviction_policy: Option<String>,
    pub conditions: Vec<WorkloadCondition>,
    /// `status.observedGeneration` is below `metadata.generation`: the counts describe an
    /// older spec. False when either generation is missing.
    pub is_status_stale: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisruptionState {
    NoPods,
    Allowed(u32),
    Blocked(BlockCause),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockCause {
    /// The disruption controller could not compute the budget.
    SyncFailed,
    UnhealthyPods,
    /// The budget needs every pod.
    NoRoom,
}

impl PodDisruptionBudgetSummary {
    /// Whether an eviction of a selected pod is allowed now. The first matching rule wins.
    pub fn disruption_state(&self) -> DisruptionState {
        // A budget the controller cannot compute refuses every eviction.
        let is_sync_failed = self.conditions.iter().any(|item| {
            item.name == DISRUPTION_ALLOWED
                && !item.is_true
                && item.reason.as_deref() == Some(SYNC_FAILED)
        });
        if is_sync_failed {
            return DisruptionState::Blocked(BlockCause::SyncFailed);
        }
        if self.expected_pods == 0 {
            return DisruptionState::NoPods;
        }
        if self.disruptions_allowed > 0 {
            return DisruptionState::Allowed(self.disruptions_allowed);
        }
        if self.current_healthy < self.expected_pods {
            return DisruptionState::Blocked(BlockCause::UnhealthyPods);
        }
        DisruptionState::Blocked(BlockCause::NoRoom)
    }
}

impl ClusterConnection {
    /// Lists every pod disruption budget of the cluster, one-shot, ordered by (namespace, name).
    /// The drain preview reads it instead of the session watches, which can be namespace-scoped.
    pub async fn list_pod_disruption_budgets(
        &self,
    ) -> Result<Vec<PodDisruptionBudgetSummary>, ClusterError> {
        let api = Api::<PodDisruptionBudget>::all(self.client().clone());
        let budgets = self.list_all(api, "listing pod disruption budgets").await?;
        let mut summaries: Vec<_> = budgets.iter().map(pod_disruption_budget_summary).collect();
        summaries.sort_by(|left, right| {
            (&left.namespace, &left.name).cmp(&(&right.namespace, &right.name))
        });
        Ok(summaries)
    }

    /// Watches pod disruption budgets (`policy/v1`) in `scope`. Yields batched snapshots
    /// ordered by (namespace, name).
    pub fn watch_pod_disruption_budgets(
        &self,
        scope: NamespaceScope,
    ) -> impl Stream<Item = WatchUpdate<PodDisruptionBudgetSummary>> + Send + 'static {
        summary_watch(
            self,
            self.scoped_apis(&scope),
            "watching pod disruption budgets",
            pod_disruption_budget_summary,
        )
    }
}

pub(crate) fn pod_disruption_budget_summary(
    budget: &PodDisruptionBudget,
) -> PodDisruptionBudgetSummary {
    let spec = budget.spec.as_ref();
    let status = budget.status.as_ref();
    let is_status_stale = match (
        budget.metadata.generation,
        status.and_then(|status| status.observed_generation),
    ) {
        (Some(generation), Some(observed)) => observed < generation,
        _ => false,
    };
    PodDisruptionBudgetSummary {
        namespace: budget.metadata.namespace.clone().unwrap_or_default(),
        name: budget.metadata.name.clone().unwrap_or_default(),
        created_at: budget
            .metadata
            .creation_timestamp
            .as_ref()
            .map(|time| time.0),
        labels: label_terms(&budget.metadata),
        min_available: spec
            .and_then(|spec| spec.min_available.as_ref())
            .map(int_or_string_text),
        max_unavailable: spec
            .and_then(|spec| spec.max_unavailable.as_ref())
            .map(int_or_string_text),
        selector: spec
            .and_then(|spec| spec.selector.as_ref())
            .map(Selector::of),
        current_healthy: status.map_or(0, |status| non_negative(status.current_healthy)),
        desired_healthy: status.map_or(0, |status| non_negative(status.desired_healthy)),
        expected_pods: status.map_or(0, |status| non_negative(status.expected_pods)),
        disruptions_allowed: status.map_or(0, |status| non_negative(status.disruptions_allowed)),
        unhealthy_pod_eviction_policy: spec
            .and_then(|spec| spec.unhealthy_pod_eviction_policy.clone())
            .filter(|policy| !policy.is_empty()),
        conditions: status
            .into_iter()
            .flat_map(|status| status.conditions.iter().flatten())
            .map(|item| {
                condition(
                    &item.type_,
                    &item.status,
                    Some(&item.reason),
                    Some(&item.message),
                )
            })
            .collect(),
        is_status_stale,
    }
}

#[cfg(test)]
mod tests {
    use k8s_openapi::api::policy::v1::{PodDisruptionBudgetSpec, PodDisruptionBudgetStatus};
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::{
        Condition, LabelSelector, ObjectMeta, Time,
    };

    use super::*;

    fn budget(
        spec: PodDisruptionBudgetSpec,
        status: PodDisruptionBudgetStatus,
    ) -> PodDisruptionBudget {
        PodDisruptionBudget {
            spec: Some(spec),
            status: Some(status),
            ..Default::default()
        }
    }

    fn summary_with(
        expected: i32,
        healthy: i32,
        allowed: i32,
        conditions: Vec<Condition>,
    ) -> PodDisruptionBudgetSummary {
        pod_disruption_budget_summary(&budget(
            PodDisruptionBudgetSpec::default(),
            PodDisruptionBudgetStatus {
                expected_pods: expected,
                current_healthy: healthy,
                desired_healthy: expected,
                disruptions_allowed: allowed,
                conditions: Some(conditions),
                ..Default::default()
            },
        ))
    }

    fn disruption_allowed(is_true: bool, reason: &str) -> Condition {
        Condition {
            type_: DISRUPTION_ALLOWED.to_owned(),
            status: if is_true { "True" } else { "False" }.to_owned(),
            reason: reason.to_owned(),
            message: "msg".to_owned(),
            last_transition_time: Time(jiff::Timestamp::UNIX_EPOCH),
            observed_generation: None,
        }
    }

    #[test]
    fn null_selector_is_none_and_empty_selects_everything() {
        let none = pod_disruption_budget_summary(&budget(
            PodDisruptionBudgetSpec::default(),
            PodDisruptionBudgetStatus::default(),
        ));
        assert_eq!(none.selector, None);
        let empty = pod_disruption_budget_summary(&budget(
            PodDisruptionBudgetSpec {
                selector: Some(LabelSelector::default()),
                ..Default::default()
            },
            PodDisruptionBudgetStatus::default(),
        ));
        assert!(
            empty
                .selector
                .is_some_and(|selector| selector.selects_everything())
        );
    }

    #[test]
    fn disruption_state_no_pods() {
        assert_eq!(
            summary_with(0, 0, 0, Vec::new()).disruption_state(),
            DisruptionState::NoPods
        );
    }

    #[test]
    fn disruption_state_allowed() {
        assert_eq!(
            summary_with(3, 3, 1, Vec::new()).disruption_state(),
            DisruptionState::Allowed(1)
        );
    }

    #[test]
    fn disruption_state_blocked_by_unhealthy_pods() {
        assert_eq!(
            summary_with(3, 2, 0, Vec::new()).disruption_state(),
            DisruptionState::Blocked(BlockCause::UnhealthyPods)
        );
    }

    #[test]
    fn disruption_state_blocked_without_room() {
        assert_eq!(
            summary_with(3, 3, 0, Vec::new()).disruption_state(),
            DisruptionState::Blocked(BlockCause::NoRoom)
        );
    }

    #[test]
    fn sync_failed_wins_over_no_pods() {
        let summary = summary_with(0, 0, 0, vec![disruption_allowed(false, SYNC_FAILED)]);
        assert_eq!(
            summary.disruption_state(),
            DisruptionState::Blocked(BlockCause::SyncFailed)
        );
    }

    #[test]
    fn stale_status_from_observed_generation() {
        let with_generations = |generation, observed| {
            pod_disruption_budget_summary(&PodDisruptionBudget {
                metadata: ObjectMeta {
                    generation,
                    ..Default::default()
                },
                status: Some(PodDisruptionBudgetStatus {
                    observed_generation: observed,
                    ..Default::default()
                }),
                ..Default::default()
            })
            .is_status_stale
        };
        assert!(with_generations(Some(3), Some(2)));
        assert!(!with_generations(Some(3), Some(3)));
        assert!(!with_generations(Some(3), None));
        assert!(!with_generations(None, Some(2)));
    }

    #[tokio::test]
    async fn list_reads_every_namespace_sorted() {
        use crate::fake_api::FakeApi;
        use crate::object_write::WritePolicy;

        let item = |namespace: &str, name: &str| {
            serde_json::json!({
                "metadata": {"name": name, "namespace": namespace},
                "spec": {"minAvailable": 1},
            })
        };
        let body = serde_json::json!({
            "apiVersion": "policy/v1", "kind": "PodDisruptionBudgetList", "metadata": {},
            "items": [item("web", "b"), item("api", "z"), item("api", "a")],
        })
        .to_string();
        let (connection, api) =
            FakeApi::connection(WritePolicy::Blocked, move |_| (200, body.clone()));
        let budgets = connection
            .list_pod_disruption_budgets()
            .await
            .expect("the list goes through");
        let names: Vec<_> = budgets
            .iter()
            .map(|budget| (budget.namespace.as_str(), budget.name.as_str()))
            .collect();
        assert_eq!(names, [("api", "a"), ("api", "z"), ("web", "b")]);
        let requests = api.requests();
        assert_eq!(requests[0].method, "GET");
        assert_eq!(requests[0].path, "/apis/policy/v1/poddisruptionbudgets");
    }
}
