use super::*;

const READ: &str = "container_fs_reads_bytes_total";
const WRITE: &str = "container_fs_writes_bytes_total";

fn series(family: &str, labels: &str, value: &str) -> String {
    format!("{family}{{{labels}}} {value}")
}

fn container_labels(device: &str, container: &str) -> String {
    format!(
        "container=\"{container}\",device=\"{device}\",id=\"/kubepods/x\",image=\"img\",name=\"abc\",namespace=\"shop\",pod=\"web-1\""
    )
}

fn sample(lines: &[String]) -> DiskIoSample {
    let mut builder = DiskIoBuilder::default();
    for line in lines {
        builder.push_line(line);
    }
    builder.finish()
}

#[test]
fn reads_container_read_and_write_lines() {
    let labels = container_labels("/dev/sda", "app");
    let sample = sample(&[
        series(READ, &labels, "1000 1700000000000"),
        series(WRITE, &labels, "2000 1700000015000"),
    ]);

    assert_eq!(sample.node, None);
    let [container] = sample.containers.as_slice() else {
        panic!("one container expected: {:?}", sample.containers);
    };
    assert_eq!(container.namespace, "shop");
    assert_eq!(container.pod, "web-1");
    assert_eq!(container.container, "app");
    assert_eq!(container.counters.read_bytes, 1000);
    assert_eq!(container.counters.write_bytes, 2000);
    assert_eq!(
        container.counters.sampled_at,
        Some(jiff::Timestamp::from_millisecond(1_700_000_015_000).expect("timestamp"))
    );
}

#[test]
fn skips_other_families_and_comments() {
    let labels = container_labels("/dev/sda", "app");
    let sample = sample(&[
        "# HELP container_fs_reads_bytes_total Cumulative count of bytes read".to_owned(),
        "# TYPE container_fs_reads_bytes_total counter".to_owned(),
        series("container_cpu_usage_seconds_total", &labels, "12.5"),
        series("container_fs_reads_total", &labels, "7"),
        String::new(),
    ]);
    assert_eq!(sample, DiskIoSample::default());
}

#[test]
fn unescapes_label_values() {
    let labels =
        "container=\"app\",id=\"/x\",image=\"a\\\"b\\\\c\\nd\",namespace=\"shop\",pod=\"web-1\"";
    let text = series(READ, labels, "5");
    let line = disk_io_line(&text).expect("line");
    assert_eq!(line.pod.as_deref(), Some("web-1"));
    assert_eq!(line.container.as_deref(), Some("app"));
    assert_eq!(unescape("a\\\"b\\\\c\\nd"), "a\"b\\c\nd");
    assert_eq!(unescape("a\\qb"), "a\\qb");
    let escaped_pod = "container=\"app\",namespace=\"shop\",pod=\"we\\\"b\"";
    let text = series(READ, escaped_pod, "5");
    let line = disk_io_line(&text).expect("line");
    assert_eq!(line.pod.as_deref(), Some("we\"b"));
}

#[test]
fn allows_a_trailing_comma() {
    let text = series(
        WRITE,
        "container=\"app\",namespace=\"shop\",pod=\"web-1\",",
        "9",
    );
    let line = disk_io_line(&text).expect("line");
    assert_eq!(line.value, 9);
    assert_eq!(line.direction, Direction::Write);
}

#[test]
fn sums_devices_per_container() {
    let sample = sample(&[
        series(READ, &container_labels("/dev/sda", "app"), "100"),
        series(READ, &container_labels("/dev/sdb", "app"), "50"),
        series(WRITE, &container_labels("/dev/sda", "app"), "7"),
    ]);
    let [container] = sample.containers.as_slice() else {
        panic!("one container expected: {:?}", sample.containers);
    };
    assert_eq!(container.counters.read_bytes, 150);
    assert_eq!(container.counters.write_bytes, 7);
}

#[test]
fn a_missing_direction_stays_zero_and_containers_sort_by_key() {
    let sample = sample(&[
        series(READ, &container_labels("d", "zeta"), "1"),
        series(WRITE, &container_labels("d", "alpha"), "2"),
    ]);
    let names: Vec<_> = sample
        .containers
        .iter()
        .map(|c| c.container.as_str())
        .collect();
    assert_eq!(names, ["alpha", "zeta"]);
    assert_eq!(sample.containers[0].counters.read_bytes, 0);
    assert_eq!(sample.containers[1].counters.write_bytes, 0);
}

#[test]
fn root_cgroup_is_the_node_series() {
    let sample = sample(&[
        series(READ, "device=\"/dev/sda\",id=\"/\"", "400"),
        series(READ, "device=\"/dev/sdb\",id=\"/\"", "100"),
        series(WRITE, "device=\"/dev/sda\",id=\"/\"", "9"),
    ]);
    let node = sample.node.expect("node series");
    assert_eq!(node.read_bytes, 500);
    assert_eq!(node.write_bytes, 9);
    assert!(sample.containers.is_empty());
}

#[test]
fn ignores_pause_pod_and_system_cgroups() {
    let sample = sample(&[
        series(READ, &container_labels("d", "POD"), "1"),
        series(READ, &container_labels("d", ""), "1"),
        series(
            READ,
            "device=\"d\",id=\"/system.slice/kubelet.service\"",
            "1",
        ),
        series(
            READ,
            "container=\"app\",id=\"/x\",namespace=\"\",pod=\"web-1\"",
            "1",
        ),
        series(READ, "container=\"app\",id=\"/x\",namespace=\"shop\"", "1"),
    ]);
    assert_eq!(sample, DiskIoSample::default());
}

#[test]
fn rejects_malformed_and_non_finite_values() {
    let labels = "container=\"app\",namespace=\"shop\",pod=\"web-1\"";
    for value in ["NaN", "-1", "+Inf", "Inf", "abc", ""] {
        assert!(
            disk_io_line(&series(READ, labels, value)).is_none(),
            "{value}"
        );
    }
    let no_brace = format!("{READ}{{{labels} 5");
    assert!(disk_io_line(&no_brace).is_none());
    let no_quote = format!("{READ}{{container=\"app}} 5");
    assert!(disk_io_line(&no_quote).is_none());
    let no_comma = format!("{READ}{{container=\"app\"pod=\"a\"}} 5");
    assert!(disk_io_line(&no_comma).is_none());
    let extra = series(READ, labels, "5 1700000000000 9");
    assert!(disk_io_line(&extra).is_none());
}

#[test]
fn a_bad_timestamp_only_drops_the_time() {
    let labels = "container=\"app\",namespace=\"shop\",pod=\"web-1\"";
    let text = series(READ, labels, "5 soon");
    let line = disk_io_line(&text).expect("line");
    assert_eq!(line.value, 5);
    assert_eq!(line.at, None);
}

#[test]
fn scientific_values_parse() {
    let labels = "container=\"app\",namespace=\"shop\",pod=\"web-1\"";
    let text = series(READ, labels, "1.2345e+06");
    let line = disk_io_line(&text).expect("line");
    assert_eq!(line.value, 1_234_500);
}
