use std::path::PathBuf;

use super::*;

fn cluster(context: &str) -> ClusterRef {
    ClusterRef {
        kubeconfig: PathBuf::from("kube.yaml"),
        context: context.to_owned(),
    }
}

fn clusters(contexts: &[&str]) -> Vec<ClusterRef> {
    contexts.iter().map(|context| cluster(context)).collect()
}

/// Display order A, B, C, D, E, F.
fn order() -> Vec<ClusterRef> {
    clusters(&["a", "b", "c", "d", "e", "f"])
}

fn plan(current: &[&str], primary: Option<&str>, wanted: &[&str]) -> ViewPlan {
    let primary = primary.map(cluster);
    plan_view(
        &clusters(current),
        primary.as_ref(),
        &clusters(wanted),
        &order(),
    )
    .expect("at most five")
    .expect("something to view")
}

#[test]
fn plan_keeps_releases_and_connects() {
    let plan = plan(&["a", "b"], Some("a"), &["b", "c"]);
    assert_eq!(plan.keep, clusters(&["b"]));
    assert_eq!(plan.release, clusters(&["a"]));
    assert_eq!(plan.connect, clusters(&["c"]));
}

#[test]
fn plan_is_empty_for_the_same_set() {
    let plan = plan(&["a", "b"], Some("a"), &["b", "a"]);
    assert!(plan.is_empty());
    assert_eq!(plan.keep, clusters(&["a", "b"]));
}

#[test]
fn plan_orders_by_display_order() {
    let plan = plan(&[], None, &["c", "a"]);
    assert_eq!(plan.connect, clusters(&["a", "c"]));
}

#[test]
fn plan_refuses_more_than_five() {
    let six = clusters(&["a", "b", "c", "d", "e", "f"]);
    assert_eq!(plan_view(&[], None, &six, &order()), Err(TooManyClusters));
    let five = &six[..5];
    assert!(plan_view(&[], None, five, &order()).is_ok());
    assert_eq!(
        TooManyClusters.to_string(),
        "View at most 5 clusters at once."
    );
}

#[test]
fn plan_drops_clusters_outside_the_display_order() {
    let wanted = clusters(&["a", "ghost"]);
    let plan = plan_view(&[], None, &wanted, &order())
        .expect("within the limit")
        .expect("one viewable cluster");
    assert_eq!(plan.connect, clusters(&["a"]));
    let none = plan_view(&[], None, &clusters(&["ghost"]), &order()).expect("within the limit");
    assert_eq!(none, None);
}

#[test]
fn primary_stays_when_in_the_set() {
    // A comes first in display order, but B is where the user stands.
    let plan = plan(&["b"], Some("b"), &["a", "b"]);
    assert_eq!(plan.primary, cluster("b"));
}

#[test]
fn primary_is_first_when_current_leaves() {
    let plan = plan(&["c"], Some("c"), &["b", "a"]);
    assert_eq!(plan.primary, cluster("a"));
    // No current cluster at launch: the first in display order.
    let launch = self::plan(&[], None, &["b", "a"]);
    assert_eq!(launch.primary, cluster("a"));
}

#[test]
fn border_uses_riskiest() {
    // The rule the title-bar border follows for several clusters: the highest environment.
    let riskiest = riskiest([Environment::Development, Environment::Production].into_iter());
    assert_eq!(riskiest, Some(Environment::Production));
}

#[test]
fn riskiest_is_the_max_environment() {
    assert_eq!(
        riskiest([Environment::Development, Environment::Production].into_iter()),
        Some(Environment::Production)
    );
    assert_eq!(
        riskiest([Environment::Local, Environment::Staging].into_iter()),
        Some(Environment::Staging)
    );
    assert_eq!(riskiest(std::iter::empty()), None);
}
