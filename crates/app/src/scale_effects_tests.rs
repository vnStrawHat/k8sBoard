use cluster::{ClaimTemplate, PodDisruptionBudgetSummary, Selector, WorkloadCondition};

use super::*;
use crate::workload_actions::workload_actions_tests::stateful_set;

fn claim_template(name: &str) -> ClaimTemplate {
    ClaimTemplate {
        name: name.to_owned(),
        storage: Some("1Gi".to_owned()),
        storage_class: None,
        access_modes: vec!["ReadWriteOnce".to_owned()],
    }
}

/// A StatefulSet `catalog-db` of 3 pods with one claim template `data` and the selector `app=db`.
fn set(retention: Option<&str>) -> KindObject {
    let mut set = stateful_set("catalog-db", "RollingUpdate");
    set.claim_templates = vec![claim_template("data")];
    set.claim_retention = retention.map(str::to_owned);
    set.selector = vec!["app=db".to_owned()];
    KindObject::StatefulSet(set)
}

fn budget(
    name: &str,
    min_available: Option<&str>,
    max_unavailable: Option<&str>,
    selector: &str,
) -> KindObject {
    KindObject::PodDisruptionBudget(PodDisruptionBudgetSummary {
        namespace: "team-a".to_owned(),
        name: name.to_owned(),
        created_at: None,
        labels: Vec::new(),
        min_available: min_available.map(str::to_owned),
        max_unavailable: max_unavailable.map(str::to_owned),
        selector: Selector::of_labels(&[selector.to_owned()]),
        current_healthy: 3,
        desired_healthy: 2,
        expected_pods: 3,
        disruptions_allowed: 1,
        unhealthy_pod_eviction_policy: None,
        conditions: Vec::<WorkloadCondition>::new(),
        is_status_stale: false,
    })
}

fn texts(lines: Vec<SharedString>) -> Vec<String> {
    lines.iter().map(ToString::to_string).collect()
}

#[test]
fn a_scale_down_names_the_claims_it_keeps() {
    let effects = ScaleEffects::of(&set(None));
    assert_eq!(
        texts(effects.lines(3, 2)),
        ["Keeps the PersistentVolumeClaims of the removed pods (data-catalog-db-2)"]
    );
    assert_eq!(
        texts(effects.lines(3, 0)),
        [
            "Keeps the PersistentVolumeClaims of the removed pods (data-catalog-db-0, data-catalog-db-1, data-catalog-db-2)"
        ]
    );
}

#[test]
fn many_removed_claims_are_counted() {
    let mut big = stateful_set("db", "RollingUpdate");
    big.desired = 6;
    big.claim_templates = vec![claim_template("data")];
    let effects = ScaleEffects::of(&KindObject::StatefulSet(big));
    assert_eq!(
        texts(effects.lines(6, 1)),
        [
            "Keeps the PersistentVolumeClaims of the removed pods (data-db-1, data-db-2, data-db-3 and 2 more)"
        ]
    );
}

#[test]
fn a_retention_policy_that_deletes_on_scale_says_so() {
    let retention = "whenDeleted Retain · whenScaled Delete";
    let effects = ScaleEffects::of(&set(Some(retention)));
    assert_eq!(
        texts(effects.lines(3, 2)),
        ["Deletes the PersistentVolumeClaims of the removed pods (data-catalog-db-2)"]
    );
    let keeps = ScaleEffects::of(&set(Some("whenDeleted Delete · whenScaled Retain")));
    assert!(texts(keeps.lines(3, 2))[0].starts_with("Keeps"));
}

#[test]
fn scaling_up_or_staying_says_nothing_about_claims() {
    let effects = ScaleEffects::of(&set(None));
    assert!(effects.lines(3, 3).is_empty());
    assert!(effects.lines(3, 5).is_empty());
}

#[test]
fn a_set_without_claim_templates_has_no_claim_line() {
    let plain = KindObject::StatefulSet(stateful_set("plain", "RollingUpdate"));
    assert!(ScaleEffects::of(&plain).lines(3, 1).is_empty());
}

#[test]
fn a_budget_that_blocks_at_the_new_count_is_named() {
    let budgets = [budget("catalog-db", Some("2"), None, "app=db")];
    let effects = ScaleEffects::of(&set(None)).with_budgets("team-a", &budgets);
    // 3 pods, minAvailable 2: one eviction is allowed now; at 2 and at 1 none is.
    assert_eq!(
        texts(effects.lines(3, 1))[1],
        "PDB catalog-db minAvailable 2: evictions will be blocked at 1 replica"
    );
    assert_eq!(
        texts(effects.lines(3, 2))[1],
        "PDB catalog-db minAvailable 2: evictions will be blocked at 2 replicas"
    );
}

#[test]
fn a_budget_that_still_allows_an_eviction_stays_quiet() {
    let budgets = [budget("catalog-db", Some("2"), None, "app=db")];
    let mut big = stateful_set("catalog-db", "RollingUpdate");
    big.selector = vec!["app=db".to_owned()];
    let effects = ScaleEffects::of(&KindObject::StatefulSet(big)).with_budgets("team-a", &budgets);
    assert!(effects.lines(6, 4).is_empty());
}

#[test]
fn percentages_round_up_like_the_budget_controller() {
    // 50% of 3 is 2 (rounded up), so 1 may be evicted; 50% of 1 is 1, so none may.
    let budgets = [budget("half", Some("50%"), None, "app=db")];
    let effects = ScaleEffects::of(&set(None)).with_budgets("team-a", &budgets);
    assert!(effects.lines(4, 3).iter().all(|line| !line.contains("PDB")));
    assert_eq!(
        texts(effects.lines(3, 1))[1],
        "PDB half minAvailable 50%: evictions will be blocked at 1 replica"
    );
}

#[test]
fn max_unavailable_zero_blocks_every_count() {
    let budgets = [budget("frozen", None, Some("0"), "app=db")];
    let effects = ScaleEffects::of(&set(None)).with_budgets("team-a", &budgets);
    assert_eq!(
        texts(effects.lines(3, 2))[1],
        "PDB frozen maxUnavailable 0: evictions will be blocked at 2 replicas"
    );
}

#[test]
fn scaling_to_zero_has_no_eviction_to_block() {
    let budgets = [budget("catalog-db", Some("2"), None, "app=db")];
    let effects = ScaleEffects::of(&set(None)).with_budgets("team-a", &budgets);
    assert!(effects.lines(3, 0).iter().all(|line| !line.contains("PDB")));
}

#[test]
fn only_budgets_of_the_namespace_that_select_the_workload_count() {
    let budgets = [
        budget("other-app", Some("2"), None, "app=web"),
        budget("catalog-db", Some("2"), None, "app=db"),
    ];
    let effects = ScaleEffects::of(&set(None)).with_budgets("team-a", &budgets);
    let lines = texts(effects.lines(3, 1));
    assert_eq!(lines.iter().filter(|line| line.contains("PDB")).count(), 1);
    assert!(lines[1].contains("catalog-db"));
    let elsewhere = ScaleEffects::of(&set(None)).with_budgets("team-b", &budgets);
    assert!(
        elsewhere
            .lines(3, 1)
            .iter()
            .all(|line| !line.contains("PDB"))
    );
}
