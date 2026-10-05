use super::*;

fn totals(received: u64, sent: u64) -> TrafficTotals {
    TrafficTotals { received, sent }
}

fn process() -> ProcessReading {
    ProcessReading {
        cpu_percent: 1.25,
        cores: 16,
        private_working_set: Some(49 << 20),
        resident: 94 << 20,
        commit_or_virtual: Some(182 << 20),
        peak_working_set: Some(101 << 20),
        threads: Some(31),
        uptime: Duration::from_secs(245),
    }
}

#[test]
fn rate_is_the_counter_delta_over_the_elapsed_time() {
    let rate = network_rate(totals(1000, 100), totals(3000, 150), Duration::from_secs(2));
    assert_eq!(
        rate,
        NetworkRate {
            received: 1000.0,
            sent: 25.0
        }
    );
}

#[test]
fn rate_of_a_counter_that_went_down_is_zero() {
    // The cluster was switched: the new connection counts from 0.
    let rate = network_rate(totals(5000, 100), totals(40, 160), Duration::from_secs(1));
    assert_eq!(
        rate,
        NetworkRate {
            received: 0.0,
            sent: 60.0
        }
    );
}

#[test]
fn rate_over_no_time_is_zero() {
    assert_eq!(
        network_rate(totals(0, 0), totals(10, 10), Duration::ZERO),
        NetworkRate::default()
    );
}

#[test]
fn cpu_is_the_share_of_the_whole_machine() {
    assert_eq!(machine_cpu_percent(200.0, 8), 25.0);
    assert_eq!(machine_cpu_percent(5.0, 0), 5.0);
    assert_eq!(machine_cpu_percent(900.0, 4), 100.0);
}

#[test]
fn the_first_reading_has_no_rate_and_the_second_has_one() {
    let start = Instant::now();
    let mut state = UsageState::default();
    assert!(state.latest().is_none());
    state.record(None, Some(totals(100, 10)), start);
    assert_eq!(
        state.latest().map(|usage| usage.rate),
        Some(NetworkRate::default())
    );
    state.record(
        None,
        Some(totals(600, 30)),
        start + Duration::from_millis(500),
    );
    let usage = state.latest().expect("a reading");
    assert_eq!(usage.rate.received, 1000.0);
    assert_eq!(usage.rate.sent, 40.0);
    assert_eq!(usage.traffic, Some(totals(600, 30)));
}

#[test]
fn closing_the_cluster_forgets_the_baseline() {
    let start = Instant::now();
    let mut state = UsageState::default();
    state.record(None, Some(totals(100, 10)), start);
    state.record(None, None, start + Duration::from_secs(1));
    assert_eq!(state.latest().and_then(|usage| usage.traffic), None);
    // The next cluster starts a fresh baseline instead of diffing against the old one.
    state.record(
        None,
        Some(totals(9000, 900)),
        start + Duration::from_secs(2),
    );
    assert_eq!(
        state.latest().map(|usage| usage.rate),
        Some(NetworkRate::default())
    );
}

#[test]
fn memory_scales_at_binary_thresholds() {
    assert_eq!(format_memory(0), "0 B");
    assert_eq!(format_memory(1023), "1023 B");
    assert_eq!(format_memory(1024), "1.0 KB");
    assert_eq!(format_memory(1536), "1.5 KB");
    assert_eq!(format_memory(1024 * 1024), "1.0 MB");
    assert_eq!(format_memory(142 << 20), "142.0 MB");
    assert_eq!(format_memory(3 << 29), "1.50 GB");
}

#[test]
fn uptime_scales_to_its_size() {
    assert_eq!(format_uptime(Duration::ZERO), "0.0 s");
    assert_eq!(format_uptime(Duration::from_millis(59_940)), "59.9 s");
    assert_eq!(format_uptime(Duration::from_secs(60)), "1m 00s");
    assert_eq!(format_uptime(Duration::from_secs(3599)), "59m 59s");
    assert_eq!(
        format_uptime(Duration::from_secs(3 * 3600 + 7 * 60 + 9)),
        "3h 07m"
    );
}

#[test]
fn the_items_read_as_the_wireframe_and_oneterm_do() {
    assert_eq!(resource_text(&process()), "CPU 1.2%  MEM 49.0 MB");
    let rate = NetworkRate {
        received: 1_234_000.0,
        sent: 300.0,
    };
    assert_eq!(network_text(rate), "↓ 1.2 MB/s  ↑ 300 B/s");
    assert_eq!(network_text(NetworkRate::default()), "↓ 0 B/s  ↑ 0 B/s");
}

#[test]
fn the_resource_table_has_a_fixed_order() {
    let sections = resource_sections(&process());
    let rows: Vec<_> = sections
        .iter()
        .flat_map(|section| {
            section
                .rows
                .iter()
                .map(move |(name, value)| (section.title, *name, value.as_str()))
        })
        .collect();
    assert_eq!(
        rows,
        vec![
            ("Memory", "Private working set", "49.0 MB"),
            ("Memory", RESIDENT_NAME, "94.0 MB"),
            ("Memory", VIRTUAL_NAME, "182.0 MB"),
            ("Memory", "Peak working set", "101.0 MB"),
            ("CPU", "Usage", "1.2% of 16 logical cores"),
            ("CPU", "Threads", "31"),
            ("CPU", "Uptime", "4m 05s"),
        ]
    );
}

#[test]
fn figures_the_os_does_not_give_are_left_out() {
    // macOS: no private or peak working set and no thread count.
    let reading = ProcessReading {
        private_working_set: None,
        peak_working_set: None,
        commit_or_virtual: None,
        threads: None,
        ..process()
    };
    let names: Vec<_> = resource_sections(&reading)
        .iter()
        .flat_map(|section| section.rows.iter().map(|(name, _)| *name))
        .collect();
    assert_eq!(names, vec![RESIDENT_NAME, "Usage", "Uptime"]);
}

#[test]
fn the_memory_item_prefers_the_private_working_set() {
    assert_eq!(displayed_memory(Some(49 << 20), 94 << 20), 49 << 20);
}

#[test]
fn the_memory_item_falls_back_to_resident_when_unavailable() {
    // Not Windows, or the call failed.
    assert_eq!(displayed_memory(None, 94 << 20), 94 << 20);
    // An older Windows may leave the field at 0.
    assert_eq!(displayed_memory(Some(0), 94 << 20), 94 << 20);
}

#[test]
fn zero_is_not_a_given_figure() {
    assert_eq!(given(0), None);
    assert_eq!(given(5), Some(5));
}

#[test]
fn the_network_table_shows_rates_and_the_totals_of_the_connection() {
    let usage = Usage {
        process: None,
        rate: NetworkRate {
            received: 2_000.0,
            sent: 10.0,
        },
        traffic: Some(totals(3 << 20, 2048)),
    };
    let sections = network_sections(&usage);
    assert_eq!(
        sections[0].rows,
        vec![
            ("Received", "2 KB/s".to_owned()),
            ("Sent", "10 B/s".to_owned())
        ]
    );
    assert_eq!(
        sections[1].rows,
        vec![
            ("Received", "3.0 MB".to_owned()),
            ("Sent", "2.0 KB".to_owned())
        ]
    );
}

#[test]
fn without_a_cluster_the_totals_are_dashes() {
    let usage = Usage {
        process: None,
        rate: NetworkRate::default(),
        traffic: None,
    };
    let sections = network_sections(&usage);
    assert_eq!(
        sections[1].rows,
        vec![("Received", "—".to_owned()), ("Sent", "—".to_owned())]
    );
}

#[test]
fn the_sampler_reads_this_process() {
    let sampler = ProcessSampler::new().expect("the current process has an id");
    let reading = sampler.sample().expect("this process is listed");
    assert!(reading.resident > 0);
    assert!(displayed_memory(reading.private_working_set, reading.resident) > 0);
    assert!(reading.cores >= 1);
    assert!((0.0..=100.0).contains(&reading.cpu_percent));
}
