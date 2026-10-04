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

fn cpu_growth(nanocores: u64) -> DemandChange {
    change(
        WorkloadDemand::default(),
        WorkloadDemand {
            requests_cpu: nanocores,
            ..WorkloadDemand::default()
        },
    )
}

fn cpu_quota(used: &str) -> QuotaInput {
    QuotaInput::Quotas(vec![quota("compute", &[("requests.cpu", "4", used)])])
}

#[test]
fn quota_headroom_is_rounded_down_and_need_up() {
    // 2.75 used of 4, 1.5 added: exactly 1.25 left after the change, shown as 1.2 and never 1.3.
    assert_eq!(
        quota_line(Some(&cpu_growth(1_500_000_000)), &cpu_quota("1250m")),
        QuotaLine::Fits("Namespace quota OK (1.2 requests.cpu left)".into())
    );
    // 1.0 left, 1.04 needed: the need reads 1.1 (never 1), the left reads 1.
    assert_eq!(
        quota_line(Some(&cpu_growth(1_040_000_000)), &cpu_quota("3")),
        QuotaLine::Exceeds(vec![
            "Quota compute: requests.cpu needs 1.1 more, 1 left".into()
        ])
    );
    // A need just under a core stays in millicores and rounds up.
    assert_eq!(
        quota_line(Some(&cpu_growth(999_400_000)), &cpu_quota("3500m")),
        QuotaLine::Exceeds(vec![
            "Quota compute: requests.cpu needs 1 more, 500m left".into()
        ])
    );
}

#[test]
fn quota_memory_is_rounded_the_same_way() {
    let quotas =
        |used: &str| QuotaInput::Quotas(vec![quota("q", &[("requests.memory", "64Gi", used)])]);
    // 1.25Gi left after the change reads 1.2Gi, not 1.3Gi.
    let line = quota_line(Some(&memory_growth(GI)), &quotas("61.75Gi"));
    assert_eq!(
        line,
        QuotaLine::Fits("Namespace quota OK (1.2Gi requests.memory left)".into())
    );
    // Needing 1.01Gi with 1Gi left: 1.1Gi needed, 1Gi left.
    let needed = GI + GI / 100;
    assert_eq!(
        quota_line(Some(&memory_growth(needed)), &quotas("63Gi")),
        QuotaLine::Exceeds(vec![
            "Quota q: requests.memory needs 1.1Gi more, 1Gi left".into()
        ])
    );
    // Below a gibibyte the numbers are whole mebibytes, rounded the same way.
    assert_eq!(memory_text(1536 * 1024, Round::Down), "1Mi");
    assert_eq!(memory_text(1536 * 1024, Round::Up), "2Mi");
    assert_eq!(memory_text(1024 * 1024 * 1024 - 1, Round::Up), "1Gi");
    assert_eq!(memory_text(1024 * 1024 * 1024 - 1, Round::Down), "1023Mi");
}
