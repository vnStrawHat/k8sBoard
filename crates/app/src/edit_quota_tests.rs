use cluster::{QuotaItem, WorkloadDemand};

use super::*;

const GI: u64 = 1 << 30;

fn change(before: WorkloadDemand, after: WorkloadDemand) -> DemandChange {
    DemandChange { before, after }
}

fn memory_growth(bytes: u64) -> DemandChange {
    change(
        WorkloadDemand::default(),
        WorkloadDemand {
            requests_memory: bytes,
            ..WorkloadDemand::default()
        },
    )
}

fn quota(name: &str, items: &[(&str, &str, &str)]) -> ResourceQuotaSummary {
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
                used: Some((*used).to_owned()),
            })
            .collect(),
        scopes: Vec::new(),
    }
}

#[test]
fn quota_line_fits_text() {
    let quotas = QuotaInput::Quotas(vec![quota(
        "compute-quota",
        &[("requests.memory", "64Gi", "40Gi")],
    )]);
    let line = quota_line(Some(&memory_growth(2 * GI)), &quotas);
    assert_eq!(
        line,
        QuotaLine::Fits("Namespace quota OK (22Gi requests.memory left)".into())
    );
    assert!(line.warnings().is_empty());
}

#[test]
fn quota_line_exceeds_adds_dialog_warnings() {
    let quotas = QuotaInput::Quotas(vec![quota(
        "compute",
        &[("requests.memory", "8Gi", "7Gi"), ("pods", "10", "9")],
    )]);
    let both = change(
        WorkloadDemand::default(),
        WorkloadDemand {
            pods: 3,
            requests_memory: 2 * GI,
            ..WorkloadDemand::default()
        },
    );
    let line = quota_line(Some(&both), &quotas);
    let expected = [
        "Quota compute: pods needs 3 more, 1 left",
        "Quota compute: requests.memory needs 2Gi more, 1Gi left",
    ];
    let QuotaLine::Exceeds(lines) = &line else {
        panic!("expected Exceeds, got {line:?}");
    };
    assert_eq!(
        lines.iter().map(AsRef::as_ref).collect::<Vec<&str>>(),
        expected
    );
    // The dialog repeats exactly these lines.
    assert_eq!(line.warnings(), lines.as_slice());
}

#[test]
fn quota_line_formats_cpu_as_cores_or_millicores() {
    let cpu = |requests: u64| {
        change(
            WorkloadDemand::default(),
            WorkloadDemand {
                requests_cpu: requests,
                ..WorkloadDemand::default()
            },
        )
    };
    let with_used =
        |used: &str| QuotaInput::Quotas(vec![quota("compute", &[("requests.cpu", "4", used)])]);
    // 4 cores, 2.25 used, 0.25 added: 1.5 cores left.
    assert_eq!(
        quota_line(Some(&cpu(250_000_000)), &with_used("2250m")),
        QuotaLine::Fits("Namespace quota OK (1.5 requests.cpu left)".into())
    );
    // 3.5 used, 0.25 added: 250m left.
    assert_eq!(
        quota_line(Some(&cpu(250_000_000)), &with_used("3500m")),
        QuotaLine::Fits("Namespace quota OK (250m requests.cpu left)".into())
    );
    // 2.5 used, 2 more needed than the 1.5 left.
    assert_eq!(
        quota_line(Some(&cpu(2_000_000_000)), &with_used("2500m")),
        QuotaLine::Exceeds(vec![
            "Quota compute: requests.cpu needs 2 more, 1.5 left".into()
        ])
    );
}

#[test]
fn quota_line_off_or_loading_is_muted() {
    let growth = memory_growth(GI);
    assert_eq!(
        quota_line(
            Some(&growth),
            &QuotaInput::Off("not permitted: list resourcequotas".into())
        ),
        QuotaLine::NotChecked("Quota not checked: not permitted: list resourcequotas".into())
    );
    assert_eq!(
        quota_line(Some(&growth), &QuotaInput::Loading),
        QuotaLine::NotChecked("Quota not checked: quotas are still loading".into())
    );
    assert!(
        quota_line(Some(&growth), &QuotaInput::Loading)
            .warnings()
            .is_empty()
    );
}

#[test]
fn quota_line_absent_when_not_affected() {
    let quotas = QuotaInput::Quotas(vec![quota("compute", &[("requests.memory", "8Gi", "7Gi")])]);
    // No demand (a ConfigMap), nothing that grows, and a growth no quota limits: no line, even when
    // the feed is off.
    assert_eq!(quota_line(None, &quotas), QuotaLine::None);
    let smaller = change(memory_growth(GI).after, WorkloadDemand::default());
    assert_eq!(
        quota_line(Some(&smaller), &QuotaInput::Off("off".into())),
        QuotaLine::None
    );
    let pods_only = change(
        WorkloadDemand::default(),
        WorkloadDemand {
            pods: 1,
            ..WorkloadDemand::default()
        },
    );
    assert_eq!(quota_line(Some(&pods_only), &quotas), QuotaLine::None);
    assert_eq!(
        quota_line(Some(&pods_only), &QuotaInput::Quotas(Vec::new())),
        QuotaLine::None
    );
}
