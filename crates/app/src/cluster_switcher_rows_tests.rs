use std::path::PathBuf;
use std::time::{Duration, Instant};

use super::*;
use crate::cluster_form::{ClusterRow, RowOrigin};
use crate::cluster_health::ProbeResult;
use crate::cluster_registry::ClusterProfile;
use crate::write_guard::ConfirmMode;

fn cluster(context: &str) -> ClusterRef {
    ClusterRef {
        kubeconfig: PathBuf::from("/home/me/.kube/config.yaml"),
        context: context.to_owned(),
    }
}

fn row(context: &str, label: &str, environment: Environment) -> ClusterRow {
    ClusterRow {
        cluster: cluster(context),
        profile: ClusterProfile {
            display_name: label.to_owned(),
            environment,
            default_namespace: None,
            read_only: false,
            confirm: ConfirmMode::Click,
            allow_node_shell: false,
            debug_image: cluster::DEFAULT_DEBUG_IMAGE.to_owned(),
            node_shell_namespace: "kube-system".to_owned(),
        },
        label: label.to_owned(),
        meta: String::new(),
        guessed: environment,
        origin: RowOrigin::Chain,
    }
}

fn group(title: &'static str, rows: Vec<ClusterRow>) -> ClusterGroup {
    ClusterGroup { title, rows }
}

fn three_groups() -> Vec<ClusterGroup> {
    vec![
        group(
            "Production",
            vec![row("eu-ctx", "eu-prod", Environment::Production)],
        ),
        group(
            "Staging",
            vec![
                row("uat-ctx", "uat", Environment::Staging),
                row("stg-ctx", "stg", Environment::Staging),
            ],
        ),
        group(
            "Development · Local",
            vec![row("kind-ctx", "kind-dev", Environment::Local)],
        ),
    ]
}

fn sections(
    health: &HealthBoard,
    active: Option<(&ClusterRef, RowHealth)>,
) -> Vec<SwitcherSection> {
    let viewed: Vec<ViewedCluster> = active
        .into_iter()
        .map(|(cluster, health)| ViewedCluster {
            cluster: cluster.clone(),
            health,
            is_primary: true,
        })
        .collect();
    switcher_sections(&three_groups(), health, &viewed, &[])
}

fn labels(sections: &[SwitcherSection]) -> Vec<&str> {
    sections
        .iter()
        .flat_map(|section| section.rows.iter().map(|row| row.label.as_str()))
        .collect()
}

#[test]
fn sections_follow_env_groups() {
    let sections = sections(&HealthBoard::default(), None);
    let titles: Vec<_> = sections.iter().map(|section| section.title).collect();
    assert_eq!(titles, ["Production", "Staging", "Development · Local"]);
    let counts: Vec<_> = sections.iter().map(|section| section.rows.len()).collect();
    assert_eq!(counts, [1, 2, 1]);
    assert_eq!(row_count(&sections), 4);
}

#[test]
fn shortcuts_number_the_first_nine_rows() {
    let rows: Vec<_> = (0..10)
        .map(|index| {
            row(
                &format!("c{index}"),
                &format!("c{index}"),
                Environment::Staging,
            )
        })
        .collect();
    let sections = switcher_sections(&[group("Staging", rows)], &HealthBoard::default(), &[], &[]);
    let shortcuts: Vec<_> = sections[0].rows.iter().map(|row| row.shortcut).collect();
    let expected: Vec<_> = (1..=9).map(Some).chain([None]).collect();
    assert_eq!(shortcuts, expected);
}

#[test]
fn shortcuts_run_on_across_sections() {
    let sections = sections(&HealthBoard::default(), None);
    let shortcuts: Vec<_> = sections
        .iter()
        .flat_map(|section| section.rows.iter().map(|row| row.shortcut))
        .collect();
    assert_eq!(shortcuts, [Some(1), Some(2), Some(3), Some(4)]);
}

#[test]
fn active_row_is_marked() {
    let active = cluster("uat-ctx");
    let sections = sections(
        &HealthBoard::default(),
        Some((&active, RowHealth::Live(Duration::from_millis(38)))),
    );
    let marked: Vec<_> = sections
        .iter()
        .flat_map(|section| &section.rows)
        .filter(|row| row.is_active)
        .collect();
    assert_eq!(marked.len(), 1);
    assert_eq!(marked[0].cluster, active);
    assert_eq!(marked[0].health, RowHealth::Live(Duration::from_millis(38)));
}

#[test]
fn filter_matches_label_context_badge_and_file() {
    let sections = sections(&HealthBoard::default(), None);
    let find = |filter: &str| {
        let visible = visible_sections(&sections, filter, SwitcherSegment::All);
        labels(&visible)
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };
    assert_eq!(find("prod"), ["eu-prod"]);
    // The badge, in either case.
    assert_eq!(find("PROD"), ["eu-prod"]);
    assert_eq!(find("stg"), ["uat", "stg"]);
    // The file name, which every fixture row shares.
    assert_eq!(find("config").len(), 4);
    // A match on the context name only.
    assert_eq!(find("kind-ctx"), ["kind-dev"]);
    assert_eq!(find("  UAT  "), ["uat"]);
    assert!(find("nothing").is_empty());
}

#[test]
fn filter_drops_empty_sections() {
    let sections = sections(&HealthBoard::default(), None);
    let visible = visible_sections(&sections, "uat", SwitcherSegment::All);
    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0].title, "Staging");
}

#[test]
fn connected_segment_keeps_live_and_reachable() {
    let now = Instant::now();
    let mut board = HealthBoard::default();
    board.record(
        cluster("eu-ctx"),
        ProbeResult::Reachable {
            latency: Duration::from_millis(9),
        },
        now,
    );
    board.record(
        cluster("stg-ctx"),
        ProbeResult::Unreachable {
            reason: "refused".to_owned(),
        },
        now,
    );
    let active = cluster("uat-ctx");
    let sections = sections(
        &board,
        Some((&active, RowHealth::Live(Duration::from_millis(3)))),
    );
    assert_eq!(connected_count(&sections), 2);
    let visible = visible_sections(&sections, "", SwitcherSegment::Connected);
    assert_eq!(labels(&visible), ["eu-prod", "uat"]);
    // Unknown and unreachable rows are not counted as connected, but stay in All.
    assert_eq!(row_count(&sections), 4);
}

#[test]
fn interrupted_active_row_is_not_connected() {
    let active = cluster("uat-ctx");
    let sections = sections(
        &HealthBoard::default(),
        Some((&active, RowHealth::Interrupted)),
    );
    assert_eq!(connected_count(&sections), 0);
}

#[test]
fn unreachable_row_carries_the_probe_reason() {
    let mut board = HealthBoard::default();
    board.record(
        cluster("stg-ctx"),
        ProbeResult::Unreachable {
            reason: "connection refused".to_owned(),
        },
        Instant::now(),
    );
    let sections = sections(&board, None);
    let stg = &sections[1].rows[1];
    assert_eq!(stg.health, RowHealth::Unreachable);
    assert_eq!(stg.failure.as_deref(), Some("connection refused"));
}

#[test]
fn nth_cluster_ignores_the_filter() {
    let all = sections(&HealthBoard::default(), None);
    // The filter shows one row, but the numbers are those of the unfiltered list.
    let visible = visible_sections(&all, "kind", SwitcherSegment::All);
    assert_eq!(labels(&visible), ["kind-dev"]);
    assert_eq!(nth_cluster(&all, 4), Some(&cluster("kind-ctx")));
    assert_eq!(nth_cluster(&all, 2), Some(&cluster("uat-ctx")));
    assert_eq!(nth_cluster(&all, 5), None);
}

#[test]
fn highlight_moves_and_wraps() {
    let all = sections(&HealthBoard::default(), None);
    let next = |current: &ClusterRef| move_highlight(&all, Some(current), HighlightStep::Next);
    let previous =
        |current: &ClusterRef| move_highlight(&all, Some(current), HighlightStep::Previous);
    assert_eq!(next(&cluster("eu-ctx")), Some(cluster("uat-ctx")));
    assert_eq!(next(&cluster("kind-ctx")), Some(cluster("eu-ctx")));
    assert_eq!(previous(&cluster("uat-ctx")), Some(cluster("eu-ctx")));
    assert_eq!(previous(&cluster("eu-ctx")), Some(cluster("kind-ctx")));
}

#[test]
fn highlight_without_a_current_row_starts_at_the_ends() {
    let all = sections(&HealthBoard::default(), None);
    assert_eq!(
        move_highlight(&all, None, HighlightStep::Next),
        Some(cluster("eu-ctx"))
    );
    assert_eq!(
        move_highlight(&all, None, HighlightStep::Previous),
        Some(cluster("kind-ctx"))
    );
    // A highlight the filter hid counts as none.
    let visible = visible_sections(&all, "kind", SwitcherSegment::All);
    assert_eq!(
        move_highlight(&visible, Some(&cluster("eu-ctx")), HighlightStep::Next),
        Some(cluster("kind-ctx"))
    );
    assert_eq!(move_highlight(&[], None, HighlightStep::Next), None);
}

#[test]
fn highlight_resets_to_first_visible_on_edit() {
    let all = sections(&HealthBoard::default(), None);
    // What the shell does after every edit of the filter text.
    let visible = visible_sections(&all, "st", SwitcherSegment::All);
    assert_eq!(
        move_highlight(&visible, None, HighlightStep::Next),
        Some(cluster("uat-ctx"))
    );
}

// ---- Ticks (0027) ----

#[test]
fn toggle_tick_adds_and_removes() {
    let mut ticked = Vec::new();
    assert_eq!(toggle_tick(&mut ticked, &cluster("uat-ctx")), Ok(()));
    assert_eq!(toggle_tick(&mut ticked, &cluster("stg-ctx")), Ok(()));
    assert_eq!(ticked, [cluster("uat-ctx"), cluster("stg-ctx")]);
    assert_eq!(toggle_tick(&mut ticked, &cluster("uat-ctx")), Ok(()));
    assert_eq!(ticked, [cluster("stg-ctx")]);
}

#[test]
fn toggle_tick_refuses_a_sixth() {
    let mut ticked: Vec<ClusterRef> = (0..5).map(|index| cluster(&format!("c{index}"))).collect();
    let result = toggle_tick(&mut ticked, &cluster("c5"));
    assert_eq!(result, Err(TooManyClusters));
    assert_eq!(ticked.len(), 5);
    assert_eq!(
        TooManyClusters.to_string(),
        "View at most 5 clusters at once."
    );
    // Unticking one of the five always works, and frees a place.
    assert_eq!(toggle_tick(&mut ticked, &cluster("c0")), Ok(()));
    assert_eq!(toggle_tick(&mut ticked, &cluster("c5")), Ok(()));
}

#[test]
fn ticks_differ_as_sets() {
    let (a, b, c) = (cluster("a"), cluster("b"), cluster("c"));
    assert!(!ticks_differ(
        &[a.clone(), b.clone()],
        &[b.clone(), a.clone()]
    ));
    assert!(ticks_differ(
        std::slice::from_ref(&a),
        &[a.clone(), b.clone()]
    ));
    assert!(ticks_differ(&[a.clone(), c], &[a.clone(), b]));
    assert!(!ticks_differ(&[], &[]));
}

#[test]
fn query_ignores_whitespace() {
    assert_eq!(normalize_query(" Prod  EU\t1 "), "prodeu1");
    let rows = vec![row("eu-ctx", "prod eu 1", Environment::Production)];
    let sections = switcher_sections(
        &[group("Production", rows)],
        &HealthBoard::default(),
        &[],
        &[],
    );
    for query in ["prod eu", "prodeu", "PROD  EU 1", "eu1"] {
        let visible = visible_sections(&sections, query, SwitcherSegment::All);
        assert_eq!(row_count(&visible), 1, "{query}");
    }
    // A query never matches across two parts of the row.
    let visible = visible_sections(&sections, "1eu-ctx", SwitcherSegment::All);
    assert_eq!(row_count(&visible), 0);
}

#[test]
fn ticked_and_primary_rows_are_marked() {
    let (eu, uat) = (cluster("eu-ctx"), cluster("uat-ctx"));
    let viewed = [
        ViewedCluster {
            cluster: eu.clone(),
            health: RowHealth::Live(Duration::from_millis(5)),
            is_primary: true,
        },
        ViewedCluster {
            cluster: uat.clone(),
            health: RowHealth::Connecting,
            is_primary: false,
        },
    ];
    let sections = switcher_sections(
        &three_groups(),
        &HealthBoard::default(),
        &viewed,
        std::slice::from_ref(&uat),
    );
    let rows: Vec<_> = sections.iter().flat_map(|section| &section.rows).collect();
    let flags = |context: &str| {
        let row = rows.iter().find(|row| row.cluster.context == context);
        row.map(|row| (row.is_active, row.is_primary, row.is_ticked))
    };
    assert_eq!(flags("eu-ctx"), Some((true, true, false)));
    assert_eq!(flags("uat-ctx"), Some((true, false, true)));
    assert_eq!(flags("stg-ctx"), Some((false, false, false)));
}
