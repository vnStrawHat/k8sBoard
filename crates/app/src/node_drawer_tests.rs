use cluster::ConditionStatus;

use super::*;

fn at(seconds: i64) -> jiff::Timestamp {
    jiff::Timestamp::from_second(seconds).expect("valid timestamp")
}

fn system(os: &str, arch: &str, image: &str) -> NodeSystemInfo {
    NodeSystemInfo {
        operating_system: os.to_owned(),
        architecture: arch.to_owned(),
        os_image: image.to_owned(),
        kernel_version: String::new(),
        container_runtime: String::new(),
    }
}

#[test]
fn os_text_leaves_out_empty_parts_and_their_separators() {
    assert_eq!(
        os_text(&system("linux", "amd64", "Ubuntu 22.04.4 LTS")).as_deref(),
        Some("linux/amd64 · Ubuntu 22.04.4 LTS")
    );
    assert_eq!(os_text(&system("linux", "", "")).as_deref(), Some("linux"));
    assert_eq!(
        os_text(&system("", "", "Flatcar")).as_deref(),
        Some("Flatcar")
    );
    assert_eq!(os_text(&system("", "", "")), None);
}

#[test]
fn condition_detail_joins_reason_and_age() {
    let condition = |reason: Option<&str>, changed: Option<i64>| NodeCondition {
        name: "Ready".to_owned(),
        status: ConditionStatus::True,
        reason: reason.map(str::to_owned),
        message: None,
        changed_at: changed.map(at),
    };
    let now = at(7_200);
    assert_eq!(
        condition_detail(&condition(Some("KubeletReady"), Some(0)), now),
        "KubeletReady · since 2h"
    );
    assert_eq!(
        condition_detail(&condition(Some("KubeletReady"), None), now),
        "KubeletReady"
    );
    assert_eq!(condition_detail(&condition(None, Some(0)), now), "since 2h");
    assert_eq!(condition_detail(&condition(None, None), now), "");
}
