use cluster::{ControllerRef, Selector, WorkloadCondition};

use super::*;

fn pod(namespace: &str, name: &str) -> DrainPod {
    DrainPod {
        namespace: namespace.to_owned(),
        name: name.to_owned(),
        uid: format!("uid-{name}"),
        labels: vec!["app=api".to_owned()],
        controller: Some(ControllerRef {
            kind: "ReplicaSet".to_owned(),
            name: "api-5d".to_owned(),
        }),
        is_mirror: false,
        has_empty_dir: false,
        is_finished: false,
        is_pending: false,
        is_terminating: false,
    }
}

fn daemon_set_pod(name: &str) -> DrainPod {
    DrainPod {
        controller: Some(ControllerRef {
            kind: "DaemonSet".to_owned(),
            name: "agent".to_owned(),
        }),
        ..pod("kube-system", name)
    }
}

/// A budget over `app=api` in `payments`: `allowed` evictions now out of `expected` pods.
fn budget(name: &str, expected: u32, healthy: u32, allowed: u32) -> PodDisruptionBudgetSummary {
    PodDisruptionBudgetSummary {
        namespace: "payments".to_owned(),
        name: name.to_owned(),
        created_at: None,
        labels: Vec::new(),
        min_available: None,
        max_unavailable: None,
        selector: Selector::of_labels(&["app=api".to_owned()]),
        current_healthy: healthy,
        desired_healthy: expected,
        expected_pods: expected,
        disruptions_allowed: allowed,
        unhealthy_pod_eviction_policy: None,
        conditions: Vec::<WorkloadCondition>::new(),
        is_status_stale: false,
    }
}

fn verdict_of(pod: &DrainPod, budgets: &[PodDisruptionBudgetSummary], rank: u32) -> PodVerdict {
    pod_verdict(pod, budgets, &DrainOptions::default(), rank)
}

#[test]
fn defaults_follow_the_wireframe() {
    let options = DrainOptions::default();
    assert!(options.ignore_daemon_sets);
    assert!(!options.delete_empty_dir && !options.force_unmanaged);
    assert_eq!(options.grace, GracePeriod::PodDefault);
    assert_eq!(options.timeout, Duration::from_secs(300));
}

#[test]
fn verdict_table() {
    let payments = pod("payments", "api-1");
    // 1. A mirror pod is skipped, whatever else it is.
    let mirror = DrainPod {
        is_mirror: true,
        is_terminating: true,
        ..daemon_set_pod("etcd")
    };
    assert_eq!(
        verdict_of(&mirror, &[], 0),
        PodVerdict::Skip(SkipReason::Mirror)
    );
    // 2. Terminating comes before finished.
    let terminating = DrainPod {
        is_terminating: true,
        is_finished: true,
        ..pod("payments", "api-2")
    };
    assert_eq!(verdict_of(&terminating, &[], 0), PodVerdict::Terminating);
    // 3. Finished comes before DaemonSet: a finished DaemonSet pod is evicted.
    let finished_agent = DrainPod {
        is_finished: true,
        ..daemon_set_pod("agent-1")
    };
    assert_eq!(
        verdict_of(&finished_agent, &[], 0),
        PodVerdict::Evict(Budget::None)
    );
    // 4. DaemonSet: skipped with the option, needed without it.
    assert_eq!(
        verdict_of(&daemon_set_pod("agent-2"), &[], 0),
        PodVerdict::Skip(SkipReason::DaemonSet)
    );
    let strict = DrainOptions {
        ignore_daemon_sets: false,
        ..DrainOptions::default()
    };
    assert_eq!(
        pod_verdict(&daemon_set_pod("agent-2"), &[], &strict, 0),
        PodVerdict::Needs(DrainOption::IgnoreDaemonSets)
    );
    // 5. No controller needs Force.
    let unmanaged = DrainPod {
        controller: None,
        ..pod("default", "debug-tools")
    };
    assert_eq!(
        verdict_of(&unmanaged, &[], 0),
        PodVerdict::Needs(DrainOption::ForceUnmanaged)
    );
    let forced = DrainOptions {
        force_unmanaged: true,
        ..DrainOptions::default()
    };
    assert_eq!(
        pod_verdict(&unmanaged, &[], &forced, 0),
        PodVerdict::Evict(Budget::None)
    );
    // 6. emptyDir needs its option; no controller is asked first.
    let scratch = DrainPod {
        has_empty_dir: true,
        ..payments.clone()
    };
    assert_eq!(
        verdict_of(&scratch, &[], 0),
        PodVerdict::Needs(DrainOption::DeleteEmptyDir)
    );
    let both = DrainPod {
        has_empty_dir: true,
        ..unmanaged
    };
    assert_eq!(
        verdict_of(&both, &[], 0),
        PodVerdict::Needs(DrainOption::ForceUnmanaged)
    );
    // 7. A pod without a budget is evicted.
    assert_eq!(
        verdict_of(&payments, &[], 1),
        PodVerdict::Evict(Budget::None)
    );
}

#[test]
fn pending_pods_skip_the_pdb_row() {
    let pending = DrainPod {
        is_pending: true,
        ..pod("payments", "api-3")
    };
    let blocked = budget("api-pdb", 2, 2, 0);
    assert_eq!(
        verdict_of(&pending, std::slice::from_ref(&blocked), 1),
        PodVerdict::Evict(Budget::None)
    );
    // Two budgets would refuse a running pod for good, but not a pending one.
    let twice = [blocked, budget("other-pdb", 2, 2, 1)];
    assert_eq!(
        verdict_of(&pending, &twice, 1),
        PodVerdict::Evict(Budget::None)
    );
}

#[test]
fn second_pod_of_a_one_allowed_budget_waits() {
    let budgets = [budget("api-pdb", 3, 3, 1)];
    let first = verdict_of(&pod("payments", "api-1"), &budgets, 1);
    let second = verdict_of(&pod("payments", "api-2"), &budgets, 2);
    assert_eq!(
        first,
        PodVerdict::Evict(Budget::Allows {
            name: "api-pdb".to_owned(),
            allowed: 1
        })
    );
    assert_eq!(
        second,
        PodVerdict::Evict(Budget::Waits {
            name: "api-pdb".to_owned(),
            allowed: 1
        })
    );
}

#[test]
fn blocked_budget_reuses_disruption_state() {
    let cases = [
        // Not enough healthy pods.
        (budget("api-pdb", 3, 2, 0), BlockCause::UnhealthyPods),
        // Every pod is needed.
        (budget("api-pdb", 2, 2, 0), BlockCause::NoRoom),
    ];
    for (budget, cause) in cases {
        let verdict = verdict_of(&pod("payments", "api-1"), &[budget], 1);
        assert_eq!(
            verdict,
            PodVerdict::Evict(Budget::Blocked {
                name: "api-pdb".to_owned(),
                cause
            })
        );
    }
    let mut failed = budget("api-pdb", 2, 2, 1);
    failed.conditions = vec![WorkloadCondition {
        name: "DisruptionAllowed".to_owned(),
        is_true: false,
        reason: Some("SyncFailed".to_owned()),
        message: None,
        last_transition: None,
    }];
    assert_eq!(
        verdict_of(&pod("payments", "api-1"), &[failed], 1),
        PodVerdict::Evict(Budget::Blocked {
            name: "api-pdb".to_owned(),
            cause: BlockCause::SyncFailed
        })
    );
}

#[test]
fn a_budget_of_another_namespace_or_without_pods_does_not_count() {
    let elsewhere = DrainPod {
        namespace: "data".to_owned(),
        ..pod("data", "kafka-0")
    };
    let budgets = [budget("api-pdb", 2, 2, 0)];
    assert_eq!(
        verdict_of(&elsewhere, &budgets, 1),
        PodVerdict::Evict(Budget::None)
    );
    let empty = [budget("api-pdb", 0, 0, 0)];
    assert_eq!(
        verdict_of(&pod("payments", "api-1"), &empty, 1),
        PodVerdict::Evict(Budget::None)
    );
}

#[test]
fn two_budgets_refuse_the_pod() {
    let budgets = [budget("a-pdb", 2, 2, 1), budget("b-pdb", 2, 2, 1)];
    let verdict = verdict_of(&pod("payments", "api-1"), &budgets, 1);
    assert_eq!(
        verdict,
        PodVerdict::Refused("Matches 2 PDBs; the API refuses to evict it".into())
    );
    let plan = node_plan(
        "wk-04",
        &[pod("payments", "api-1")],
        &budgets,
        &DrainOptions::default(),
    );
    assert_eq!(
        drain_blocker(&[plan]).as_deref(),
        Some("payments/api-1 cannot be evicted: Matches 2 PDBs; the API refuses to evict it")
    );
}

#[test]
fn node_plan_ranks_pods_by_budget_in_plan_order() {
    let pods = [
        pod("payments", "api-1"),
        daemon_set_pod("agent"),
        pod("payments", "api-2"),
        DrainPod {
            is_finished: true,
            ..pod("payments", "api-done")
        },
        pod("payments", "api-3"),
    ];
    let budgets = [budget("api-pdb", 4, 4, 2)];
    let plan = node_plan("wk-04", &pods, &budgets, &DrainOptions::default());
    let kinds: Vec<&str> = plan
        .pods
        .iter()
        .map(|planned| match &planned.verdict {
            PodVerdict::Evict(Budget::Allows { .. }) => "allows",
            PodVerdict::Evict(Budget::Waits { .. }) => "waits",
            PodVerdict::Evict(Budget::None) => "free",
            PodVerdict::Skip(_) => "skip",
            _ => "other",
        })
        .collect();
    // The finished pod and the skipped DaemonSet pod take no place in the budget.
    assert_eq!(kinds, ["allows", "skip", "allows", "free", "waits"]);
    assert_eq!(plan.evictions().count(), 4);
}

#[test]
fn unticked_option_blocks_drain() {
    let pods = [
        DrainPod {
            controller: None,
            ..pod("default", "debug-a")
        },
        DrainPod {
            controller: None,
            ..pod("default", "debug-b")
        },
        pod("payments", "api-1"),
    ];
    let plan = node_plan("wk-04", &pods, &[], &DrainOptions::default());
    assert_eq!(
        drain_blocker(std::slice::from_ref(&plan)).as_deref(),
        Some("2 pods need \"Force unmanaged pods\": tick it, or use Cordon only")
    );
    let forced = DrainOptions {
        force_unmanaged: true,
        ..DrainOptions::default()
    };
    let plan = node_plan("wk-04", &pods, &[], &forced);
    assert_eq!(drain_blocker(&[plan]), None);
    let one = node_plan("wk-04", &pods[..1], &[], &DrainOptions::default());
    assert_eq!(
        drain_blocker(&[one]).as_deref(),
        Some("1 pod needs \"Force unmanaged pods\": tick it, or use Cordon only")
    );
}

#[test]
fn option_counts_ignore_the_checkbox_state() {
    let pods = [
        daemon_set_pod("agent-1"),
        daemon_set_pod("agent-2"),
        DrainPod {
            has_empty_dir: true,
            ..pod("payments", "api-1")
        },
        DrainPod {
            controller: None,
            ..pod("default", "debug")
        },
        // A static pod is the kubelet's: it counts for nothing.
        DrainPod {
            is_mirror: true,
            controller: None,
            has_empty_dir: true,
            ..pod("kube-system", "etcd")
        },
    ];
    for options in [
        DrainOptions::default(),
        DrainOptions {
            ignore_daemon_sets: false,
            delete_empty_dir: true,
            force_unmanaged: true,
            ..DrainOptions::default()
        },
    ] {
        let plan = node_plan("wk-04", &pods, &[], &options);
        let counts = option_counts(&[plan]);
        assert_eq!(
            counts,
            OptionCounts {
                daemon_sets: 2,
                empty_dir: 1,
                unmanaged: 1
            }
        );
    }
    let counts = OptionCounts {
        daemon_sets: 6,
        empty_dir: 1,
        unmanaged: 2,
    };
    assert_eq!(
        option_hint(DrainOption::IgnoreDaemonSets, &counts),
        "6 pods stay on the node"
    );
    assert_eq!(
        option_hint(DrainOption::DeleteEmptyDir, &counts),
        "1 pod loses local scratch data"
    );
    assert_eq!(
        option_hint(DrainOption::ForceUnmanaged, &counts),
        "2 pods have no controller and will not come back"
    );
}

fn text_of(line: &PreviewLine) -> String {
    match line {
        PreviewLine::Node(node) => format!("[{node}]"),
        PreviewLine::Pod {
            namespace,
            name,
            result,
            ..
        } => format!("{namespace}/{name}: {result}"),
        PreviewLine::Skipped { text } => text.to_string(),
    }
}

#[test]
fn preview_puts_blocked_pods_first() {
    let pods = [
        pod("a", "free"),
        DrainPod {
            is_terminating: true,
            ..pod("a", "going")
        },
        pod("payments", "api-1"),
        pod("payments", "api-2"),
        DrainPod {
            controller: None,
            ..pod("default", "debug")
        },
        daemon_set_pod("agent-1"),
        daemon_set_pod("agent-2"),
        DrainPod {
            is_mirror: true,
            ..pod("kube-system", "etcd")
        },
    ];
    let budgets = [budget("api-pdb", 2, 2, 1)];
    let plan = node_plan("wk-04", &pods, &budgets, &DrainOptions::default());
    let lines = preview_lines(&[plan], |_| PodCheck::Waiting);
    let texts: Vec<String> = lines.iter().map(text_of).collect();
    assert_eq!(
        texts,
        [
            "default/debug: No controller, needs Force",
            "payments/api-2: Waits on PDB api-pdb (allows 1)",
            "payments/api-1: PDB api-pdb allows 1",
            "a/free: Will be rescheduled",
            "a/going: Already terminating",
            "2 DaemonSet pods · Skipped",
            "1 static pod · Skipped",
        ]
    );
}

#[test]
fn preview_order_is_refused_blocked_needs_waits_allows_evict_terminating() {
    let pods = [
        pod("z", "evict"),
        DrainPod {
            is_terminating: true,
            ..pod("z", "term")
        },
        pod("payments", "allowed"),
        pod("payments", "waits"),
        DrainPod {
            controller: None,
            ..pod("z", "needs")
        },
    ];
    let budgets = [budget("api-pdb", 2, 2, 1)];
    let plan = node_plan("wk-04", &pods, &budgets, &DrainOptions::default());
    let order: Vec<String> = preview_lines(&[plan], |_| PodCheck::Waiting)
        .iter()
        .filter_map(|line| match line {
            PreviewLine::Pod { name, .. } => Some(name.to_string()),
            _ => None,
        })
        .collect();
    assert_eq!(order, ["needs", "waits", "allowed", "evict", "term"]);
    // A blocked budget floats above everything but a refusal.
    let blocked = [budget("api-pdb", 2, 2, 0)];
    let plan = node_plan("wk-04", &pods, &blocked, &DrainOptions::default());
    let first = preview_lines(&[plan], |_| PodCheck::Waiting)
        .into_iter()
        .find_map(|line| match line {
            PreviewLine::Pod { name, .. } => Some(name.to_string()),
            _ => None,
        });
    assert_eq!(first.as_deref(), Some("allowed"));
}

#[test]
fn several_nodes_get_a_header_each() {
    let first = node_plan("wk-01", &[pod("a", "one")], &[], &DrainOptions::default());
    let second = node_plan("wk-02", &[pod("a", "two")], &[], &DrainOptions::default());
    let texts: Vec<String> = preview_lines(&[first, second], |_| PodCheck::Waiting)
        .iter()
        .map(text_of)
        .collect();
    assert_eq!(
        texts,
        [
            "[wk-01]",
            "a/one: Will be rescheduled",
            "[wk-02]",
            "a/two: Will be rescheduled"
        ]
    );
}

#[test]
fn passing_dry_run_downgrades_blocked_and_waits() {
    let pods = [pod("payments", "api-1"), pod("payments", "api-2")];
    let budgets = [budget("api-pdb", 2, 2, 1)];
    let plan = node_plan("wk-04", &pods, &budgets, &DrainOptions::default());
    let waits = &plan.pods[1];
    assert!(matches!(
        waits.verdict,
        PodVerdict::Evict(Budget::Waits { .. })
    ));
    assert_eq!(
        pod_result(waits, &PodCheck::Waiting, BudgetPolicy::Respect).0,
        "Waits on PDB api-pdb (allows 1)"
    );
    let (text, tone) = pod_result(waits, &PodCheck::Accepted, BudgetPolicy::Respect);
    assert_eq!((text.as_ref(), tone), ("Dry-run accepted", StatusTone::Ok));
    // A local pass keeps its text.
    assert_eq!(
        pod_result(&plan.pods[0], &PodCheck::Accepted, BudgetPolicy::Respect).0,
        "PDB api-pdb allows 1"
    );
    let blocked = [budget("api-pdb", 2, 2, 0)];
    let plan = node_plan("wk-04", &pods, &blocked, &DrainOptions::default());
    assert_eq!(
        pod_result(&plan.pods[0], &PodCheck::Accepted, BudgetPolicy::Respect).0,
        "Dry-run accepted"
    );
}

#[test]
fn a_budget_refusal_reads_in_a_short_form_with_the_full_words_as_detail() {
    let full = "The disruption budget api-pdb needs 2 healthy pods and has 2 currently";
    let pods = [pod("payments", "api-1")];
    let plan = node_plan("wk-04", &pods, &[], &DrainOptions::default());
    let refused = PodCheck::Refused(full.into());
    let (text, tone) = pod_result(&plan.pods[0], &refused, BudgetPolicy::Respect);
    assert_eq!(text, "PDB api-pdb: 0 allowed (2/2 healthy)");
    assert_eq!(tone, StatusTone::Bad);
    let lines = preview_lines(&[plan], |_| refused.clone());
    let PreviewLine::Pod { detail, .. } = &lines[0] else {
        panic!("a pod line");
    };
    assert_eq!(detail.as_deref(), Some(full));
}

#[test]
fn a_budget_refusal_without_numbers_says_only_that_nothing_is_allowed() {
    let refusal = BudgetRefusal::parse("The disruption budget api-pdb is exhausted");
    assert_eq!(
        refusal.map(|refusal| refusal.summary()).as_deref(),
        Some("PDB api-pdb: 0 allowed")
    );
    assert!(BudgetRefusal::parse("needs 2 healthy pods").is_none());
}

#[test]
fn a_refused_or_failed_dry_run_is_the_server_truth() {
    let pods = [pod("payments", "api-1")];
    let plan = node_plan("wk-04", &pods, &[], &DrainOptions::default());
    let planned = &plan.pods[0];
    let refused = PodCheck::Refused("needs 2 healthy pods".into());
    let (text, tone) = pod_result(planned, &refused, BudgetPolicy::Respect);
    assert_eq!(text, "Blocked by PDB: needs 2 healthy pods");
    assert_eq!(tone, StatusTone::Bad);
    let (text, tone) = pod_result(
        planned,
        &PodCheck::Failed("not permitted".into()),
        BudgetPolicy::Respect,
    );
    assert_eq!(text, "Failed: not permitted");
    assert_eq!(tone, StatusTone::Bad);
    // And the pod floats to the top.
    let all = [pod("a", "first"), pod("payments", "api-1")];
    let plan = node_plan("wk-04", &all, &[], &DrainOptions::default());
    let lines = preview_lines(&[plan], |uid| {
        if uid == "uid-api-1" {
            PodCheck::Refused("x".into())
        } else {
            PodCheck::Waiting
        }
    });
    assert!(text_of(&lines[0]).starts_with("payments/api-1"));
}

#[test]
fn evictions_and_steps_count_only_evict_verdicts() {
    let pods = [
        pod("a", "one"),
        pod("a", "two"),
        daemon_set_pod("agent"),
        DrainPod {
            controller: None,
            ..pod("a", "three")
        },
    ];
    let plan = node_plan("wk-04", &pods, &[], &DrainOptions::default());
    assert_eq!(eviction_count(&[plan]), 2);
}

#[test]
fn heads_up_names_the_budgets_that_make_a_pod_wait() {
    let pods = [pod("payments", "api-1"), pod("payments", "api-2")];
    let none = node_plan("wk-04", &pods, &[], &DrainOptions::default());
    assert_eq!(heads_up(&[none], DEFAULT_TIMEOUT), None);
    let waits = node_plan(
        "wk-04",
        &pods,
        &[budget("api-pdb", 2, 2, 1)],
        &DrainOptions::default(),
    );
    assert_eq!(
        heads_up(&[waits], DEFAULT_TIMEOUT).as_deref(),
        Some(
            "Drain will wait on api-pdb until a replacement pod is ready elsewhere, or stop at the 5m timeout."
        )
    );
    let mut other = budget("kafka-pdb", 1, 1, 0);
    other.namespace = "data".to_owned();
    let kafka = DrainPod {
        namespace: "data".to_owned(),
        ..pod("data", "kafka-0")
    };
    let several = node_plan(
        "wk-04",
        &[pods[0].clone(), pods[1].clone(), kafka],
        &[budget("api-pdb", 2, 2, 0), other],
        &DrainOptions::default(),
    );
    assert_eq!(
        heads_up(&[several], Duration::from_secs(600)).as_deref(),
        Some(
            "Drain will wait on api-pdb and 1 more until a replacement pod is ready elsewhere, or stop at the 10m timeout."
        )
    );
}

#[test]
fn choices_read_as_the_selects_show_them() {
    assert_eq!(grace_text(GracePeriod::PodDefault), "Pod default");
    assert_eq!(grace_text(GracePeriod::Seconds(30)), "30 s");
    let timeouts: Vec<String> = TIMEOUT_CHOICES.into_iter().map(timeout_text).collect();
    assert_eq!(timeouts, ["2m", "5m", "10m", "30m"]);
    assert!(TIMEOUT_CHOICES.contains(&DEFAULT_TIMEOUT));
}

fn secs(seconds: u64) -> Duration {
    Duration::from_secs(seconds)
}

#[test]
fn the_dry_run_is_running_until_every_check_answered() {
    let running = [CordonCheck::Passed, CordonCheck::Running];
    let pods = [PodCheck::Accepted, PodCheck::Waiting];
    assert_eq!(
        drain_dry_run(&running, &pods, secs(1)),
        DryRunState::Running
    );
    assert_eq!(
        drain_dry_run(&[CordonCheck::Passed], &[PodCheck::Running], secs(1)),
        DryRunState::Running
    );
    assert_eq!(
        drain_dry_run(&[CordonCheck::Waiting], &[], secs(1)),
        DryRunState::Running
    );
}

#[test]
fn a_429_is_not_a_failure_but_any_other_error_is() {
    let passed = [CordonCheck::Passed];
    let refused = [
        PodCheck::Accepted,
        PodCheck::Refused("needs 2 healthy".into()),
    ];
    assert_eq!(
        drain_dry_run(&passed, &refused, secs(2)),
        DryRunState::Passed { elapsed: secs(2) }
    );
    let failed = [PodCheck::Accepted, PodCheck::Failed("not permitted".into())];
    assert_eq!(
        drain_dry_run(&passed, &failed, secs(2)),
        DryRunState::Failed("Dry-run failed: not permitted".into())
    );
    let cordon = [CordonCheck::Failed("not permitted: patch nodes".into())];
    assert_eq!(
        drain_dry_run(&cordon, &[PodCheck::Accepted], secs(2)),
        DryRunState::Failed("Dry-run failed: cordon: not permitted: patch nodes".into())
    );
}

#[test]
fn a_drain_without_checks_passes() {
    assert_eq!(
        drain_dry_run(&[], &[], secs(0)),
        DryRunState::Passed { elapsed: secs(0) }
    );
    assert_eq!(
        dry_run_text(
            &DryRunState::Passed { elapsed: secs(0) },
            &[],
            &[],
            BudgetPolicy::Respect
        ),
        "Server dry-run: nothing to check"
    );
}

#[test]
fn the_dry_run_line_counts_accepted_and_refused_evictions() {
    let cordons = [CordonCheck::Passed];
    let mut pods = vec![PodCheck::Accepted; 21];
    pods.extend([PodCheck::Refused("a".into()), PodCheck::Refused("b".into())]);
    let state = drain_dry_run(&cordons, &pods, secs(1));
    assert_eq!(
        dry_run_text(&state, &cordons, &pods, BudgetPolicy::Respect),
        "Server dry-run: cordon passed · 21 of 23 evictions accepted, 2 refused by PDB"
    );
    // Nothing refused: the clause goes.
    let pods = vec![PodCheck::Accepted; 3];
    let state = drain_dry_run(&cordons, &pods, secs(1));
    assert_eq!(
        dry_run_text(&state, &cordons, &pods, BudgetPolicy::Respect),
        "Server dry-run: cordon passed · 3 of 3 evictions accepted"
    );
    // A node that was cordoned already has no cordon check.
    assert_eq!(
        dry_run_text(&state, &[], &pods, BudgetPolicy::Respect),
        "Server dry-run: 3 of 3 evictions accepted"
    );
}

#[test]
fn the_running_dry_run_line_shows_progress() {
    let cordons = [CordonCheck::Passed];
    let pods = [PodCheck::Accepted, PodCheck::Running, PodCheck::Waiting];
    assert_eq!(
        dry_run_text(
            &DryRunState::Running,
            &cordons,
            &pods,
            BudgetPolicy::Respect
        ),
        "Server dry-run… 2 of 4"
    );
}

// ---- Skip PodDisruptionBudgets (spec 0040) ----

fn skip() -> DrainOptions {
    DrainOptions {
        budgets: BudgetPolicy::Skip,
        ..DrainOptions::default()
    }
}

#[test]
fn the_budget_policy_defaults_to_respect() {
    assert_eq!(DrainOptions::default().budgets, BudgetPolicy::Respect);
}

#[test]
fn skip_pdbs_turns_blocked_and_refused_pods_into_deletes() {
    let api = pod("payments", "api-1");
    let blocked = [budget("api-pdb", 2, 2, 0)];
    let bypassed = |names: &[&str]| {
        PodVerdict::Evict(Budget::Bypassed {
            names: names.iter().map(|name| (*name).to_owned()).collect(),
        })
    };
    // Blocked, waiting, and allowed pods all read the same: the budget is not asked.
    assert_eq!(
        pod_verdict(&api, &blocked, &skip(), 1),
        bypassed(&["api-pdb"])
    );
    let waiting = [budget("api-pdb", 2, 2, 1)];
    assert_eq!(
        pod_verdict(&api, &waiting, &skip(), 2),
        bypassed(&["api-pdb"])
    );
    // Two budgets: the API refuses an eviction for good, a delete does not ask.
    let two = [budget("b-pdb", 2, 2, 1), budget("a-pdb", 2, 2, 1)];
    assert!(matches!(
        pod_verdict(&api, &two, &DrainOptions::default(), 1),
        PodVerdict::Refused(_)
    ));
    assert_eq!(
        pod_verdict(&api, &two, &skip(), 1),
        bypassed(&["a-pdb", "b-pdb"])
    );
    // No budget: nothing to name.
    assert_eq!(
        pod_verdict(&api, &[], &skip(), 0),
        PodVerdict::Evict(Budget::None)
    );
    // The other rows are unchanged: a DaemonSet pod still needs its option, a mirror is skipped.
    let strict = DrainOptions {
        ignore_daemon_sets: false,
        ..skip()
    };
    assert_eq!(
        pod_verdict(&daemon_set_pod("agent-1"), &[], &strict, 0),
        PodVerdict::Needs(DrainOption::IgnoreDaemonSets)
    );
    let mirror = DrainPod {
        is_mirror: true,
        ..pod("kube-system", "etcd")
    };
    assert_eq!(
        pod_verdict(&mirror, &blocked, &skip(), 0),
        PodVerdict::Skip(SkipReason::Mirror)
    );
    // The run reads no budgets at all.
    assert_eq!(run_verdict(&api, &skip()), PodVerdict::Evict(Budget::None));
}

#[test]
fn skip_pdbs_preview_names_bypassed_budgets() {
    let pods = [
        pod("payments", "api-1"),
        pod("payments", "api-2"),
        DrainPod {
            labels: vec!["app=web".to_owned()],
            ..pod("web", "web-1")
        },
    ];
    let budgets = [budget("api-pdb", 2, 2, 0)];
    let plan = node_plan("wk-04", &pods, &budgets, &skip());
    assert_eq!(plan.budgets, BudgetPolicy::Skip);
    let lines = preview_lines(std::slice::from_ref(&plan), |_| PodCheck::Accepted);
    let rows: Vec<(String, String, StatusTone)> = lines
        .iter()
        .filter_map(|line| match line {
            PreviewLine::Pod {
                name, result, tone, ..
            } => Some((name.to_string(), result.to_string(), *tone)),
            _ => None,
        })
        .collect();
    // The bypassed pods sort with the waiting ones, before the pod no budget covers.
    let bypassed = "Deleted directly; PDB api-pdb not checked".to_owned();
    assert_eq!(
        rows,
        [
            ("api-1".to_owned(), bypassed.clone(), StatusTone::Warn),
            ("api-2".to_owned(), bypassed, StatusTone::Warn),
            (
                "web-1".to_owned(),
                "Will be rescheduled".to_owned(),
                StatusTone::Ok
            ),
        ]
    );
    // Several budgets over one pod: the first by name and a count.
    let several = [budget("b-pdb", 2, 2, 1), budget("a-pdb", 2, 2, 1)];
    let plan = node_plan("wk-04", &pods[..1], &several, &skip());
    let line = &preview_lines(&[plan], |_| PodCheck::Waiting)[0];
    let PreviewLine::Pod { result, .. } = line else {
        panic!("a pod line");
    };
    assert_eq!(result, "Deleted directly; PDB a-pdb and 1 more not checked");
}

#[test]
fn skip_pdbs_words_a_refusal_as_rate_limiting_and_counts_deletes() {
    let pods = [pod("payments", "api-1")];
    let plan = node_plan("wk-04", &pods, &[], &skip());
    let refused = PodCheck::Refused("Too many requests".into());
    let (text, tone) = pod_result(&plan.pods[0], &refused, BudgetPolicy::Skip);
    assert_eq!(
        (text.as_ref(), tone),
        ("Refused: Too many requests", StatusTone::Bad)
    );
    let cordons = [CordonCheck::Passed];
    let mut checks = vec![PodCheck::Accepted; 23];
    checks.push(PodCheck::Refused("slow".into()));
    let state = drain_dry_run(&cordons, &checks, secs(1));
    assert_eq!(
        dry_run_text(&state, &cordons, &checks, BudgetPolicy::Skip),
        "Server dry-run: cordon passed · 23 of 24 deletes accepted, 1 refused"
    );
}

#[test]
fn the_bypass_note_names_the_budgets_and_counts_the_pods() {
    let pods = [
        pod("payments", "api-1"),
        pod("payments", "api-2"),
        pod("payments", "api-3"),
    ];
    let budgets = [budget("api-pdb", 3, 3, 1)];
    let plan = node_plan("wk-04", &pods, &budgets, &skip());
    assert_eq!(
        bypass_note(std::slice::from_ref(&plan)).as_deref(),
        Some(
            "PodDisruptionBudgets are not checked. 3 pods protected by api-pdb go down without \
             waiting for replacements."
        )
    );
    let one = node_plan("wk-04", &pods[..1], &budgets, &skip());
    assert_eq!(
        bypass_note(&[one]).as_deref(),
        Some(
            "PodDisruptionBudgets are not checked. 1 pod protected by api-pdb goes down without \
             waiting for replacements."
        )
    );
    // Only when a budget protects a pod.
    let unprotected = node_plan("wk-04", &pods, &[], &skip());
    assert_eq!(bypass_note(&[unprotected]), None);
    // The respecting plan has none, and its own waiting note is untouched.
    let respect = node_plan("wk-04", &pods, &budgets, &DrainOptions::default());
    assert_eq!(bypass_note(std::slice::from_ref(&respect)), None);
    assert!(heads_up(&[respect], DEFAULT_TIMEOUT).is_some());
}
