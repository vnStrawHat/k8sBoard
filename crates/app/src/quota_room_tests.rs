use cluster::{ContainerResource, QuotaItem};

use super::*;

const MESSAGE: &str = "pods \"filler-6445cd8899-dm5gk\" is forbidden: exceeded quota: team-quota, requested: limits.memory=150Mi, used: limits.memory=600Mi, limited: limits.memory=640Mi";

fn item(resource: &str, hard: &str, used: &str) -> QuotaItem {
    QuotaItem {
        resource: resource.to_owned(),
        hard: hard.to_owned(),
        used: Some(used.to_owned()),
    }
}

fn quota(items: Vec<QuotaItem>) -> ResourceQuotaSummary {
    ResourceQuotaSummary {
        namespace: "lab-quota".to_owned(),
        name: "team-quota".to_owned(),
        created_at: None,
        labels: Vec::new(),
        items,
        scopes: Vec::new(),
    }
}

fn template(limit_memory: &str, request_cpu: &str) -> Vec<TemplateContainer> {
    let resource = |name: &str, request: Option<&str>, limit: Option<&str>| ContainerResource {
        name: name.to_owned(),
        request: request.map(str::to_owned),
        limit: limit.map(str::to_owned),
    };
    vec![TemplateContainer {
        name: "app".to_owned(),
        image: "app:1".to_owned(),
        ports: Vec::new(),
        resources: vec![
            resource("cpu", Some(request_cpu), None),
            resource("memory", None, Some(limit_memory)),
        ],
    }]
}

#[test]
fn an_exceeded_quota_message_names_the_quota_and_the_numbers() {
    assert_eq!(
        quota_exceeded(MESSAGE),
        Some(QuotaExceeded {
            quota: "team-quota".to_owned(),
            lines: vec!["limits.memory 600Mi of 640Mi used, needs 150Mi".to_owned()],
        })
    );
}

#[test]
fn several_resources_get_a_line_each() {
    let message = "exceeded quota: q, requested: requests.cpu=500m,pods=1, used: requests.cpu=1800m,pods=10, limited: requests.cpu=2,pods=10";
    let found = quota_exceeded(message).expect("a quota failure");
    assert_eq!(
        found.lines,
        [
            "requests.cpu 1800m of 2 used, needs 500m",
            "pods 10 of 10 used, needs 1"
        ]
    );
}

#[test]
fn other_messages_are_not_a_quota_failure() {
    assert_eq!(quota_exceeded("Back-off pulling image"), None);
    assert_eq!(quota_exceeded("exceeded quota: q, requested: a=1"), None);
}

#[test]
fn a_scale_that_overruns_the_quota_says_how_many_pods_will_not_start() {
    let quotas = [quota(vec![item("limits.memory", "640Mi", "600Mi")])];
    let containers = template("150Mi", "10m");
    assert_eq!(
        quota_scale_warnings(&quotas, &containers, 4, 5),
        ["needs 150Mi limits.memory per pod, team-quota has 40Mi left: the new pod will not start"]
    );
    assert_eq!(
        quota_scale_warnings(&quotas, &containers, 4, 7),
        [
            "needs 150Mi limits.memory per pod, team-quota has 40Mi left: none of the 3 new pods will start"
        ]
    );
}

#[test]
fn some_new_pods_may_fit() {
    let quotas = [quota(vec![item("limits.memory", "1Gi", "600Mi")])];
    // 424Mi left fits two 150Mi pods.
    assert_eq!(
        quota_scale_warnings(&quotas, &template("150Mi", "10m"), 1, 4),
        [
            "needs 150Mi limits.memory per pod, team-quota has 424Mi left: 1 of the 3 new pods will not start"
        ]
    );
    assert!(quota_scale_warnings(&quotas, &template("150Mi", "10m"), 1, 3).is_empty());
}

#[test]
fn the_pod_count_and_cpu_requests_are_checked_too() {
    let quotas = [quota(vec![
        item("pods", "10", "9"),
        item("requests.cpu", "2", "1900m"),
    ])];
    assert_eq!(
        quota_scale_warnings(&quotas, &template("1Mi", "500m"), 1, 3),
        [
            "team-quota allows 1 more pods: 1 of the 2 new pods will not start",
            "needs 500m requests.cpu per pod, team-quota has 100m left: none of the 2 new pods will start"
        ]
    );
}

#[test]
fn a_resource_the_template_does_not_set_or_a_quota_not_synced_is_skipped() {
    let mut unsynced = item("limits.memory", "640Mi", "0");
    unsynced.used = None;
    let quotas = [quota(vec![
        unsynced,
        item("limits.cpu", "1", "1"),
        item("services", "3", "3"),
    ])];
    assert!(quota_scale_warnings(&quotas, &template("150Mi", "10m"), 1, 2).is_empty());
}

#[test]
fn scaling_down_or_to_the_same_count_checks_nothing() {
    let quotas = [quota(vec![item("limits.memory", "640Mi", "640Mi")])];
    assert!(quota_scale_warnings(&quotas, &template("150Mi", "10m"), 3, 3).is_empty());
}
