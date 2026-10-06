use super::*;

fn at(seconds: i64) -> jiff::Timestamp {
    jiff::Timestamp::from_second(seconds).expect("valid timestamp")
}

fn model(values: &[Option<f64>], references: &[f64], unit: Measure) -> UsageChartModel {
    UsageChartModel {
        id: "test".into(),
        title: "Test".into(),
        unit,
        step: Duration::from_secs(15),
        start: at(0),
        end: at(900),
        series: vec![ChartSeries {
            name: "used".into(),
            points: values
                .iter()
                .enumerate()
                .map(|(index, value)| (at(index as i64 * 15), *value))
                .collect(),
        }],
        references: references
            .iter()
            .map(|value| ReferenceLine {
                kind: ReferenceKind::Limit,
                label: "limit".into(),
                value: *value,
            })
            .collect(),
        markers: Vec::new(),
        notice: None,
    }
}

const MI: f64 = 1_048_576.;

#[test]
fn nice_max_steps_and_floors() {
    let cpu = |value| nice_max(value, Measure::Cpu);
    assert_eq!(cpu(0.0), 0.01);
    assert_eq!(cpu(0.003), 0.01);
    assert_eq!(cpu(0.011), 0.02);
    // No 2.5 step: its midline would read 13m for 12.5m.
    assert_eq!(cpu(0.021), 0.03);
    assert_eq!(cpu(0.31), 0.4);
    assert_eq!(cpu(1.0), 1.0);
    assert_eq!(cpu(1.3), 2.0);
    assert_eq!(cpu(2.6), 3.0);
    assert_eq!(cpu(7.0), 8.0);
    assert_eq!(cpu(9.0), 10.0);
    // A 26-core node: not 50 cores, and the midline stays a whole 15.
    assert_eq!(cpu(26.0), 30.0);
    let bytes = |value| nice_max(value, Measure::Bytes);
    assert_eq!(bytes(0.0), MI);
    // Powers of two (and three quarters of them from 6 up) in the value's own binary unit: the
    // midline is a whole number too.
    assert_eq!(bytes(477. * MI), 512. * MI);
    assert_eq!(bytes(300. * MI), 384. * MI);
    assert_eq!(bytes(100. * MI), 128. * MI);
    assert_eq!(bytes(900. * MI), 1024. * MI);
    assert_eq!(bytes(513. * MI), 768. * MI);
    assert_eq!(bytes(770. * MI), 1024. * MI);
    // A pod near 4.8Gi gets a 6Gi chart, not an 8Gi one; below 6 the three-quarter step would
    // halve into a fraction (3Mi to 1.5Mi, which the axis text would round).
    assert_eq!(bytes(4.8 * 1024. * MI), 6. * 1024. * MI);
    assert_eq!(bytes(2.6 * MI), 4. * MI);
    assert_eq!(bytes(5.1 * 1024. * MI), 6. * 1024. * MI);
    assert_eq!(bytes(6.1 * 1024. * MI), 8. * 1024. * MI);
    assert_eq!(bytes(1.3 * 1024. * MI), 2. * 1024. * MI);
    assert_eq!(bytes(1024. * MI), 1024. * MI);
    for value in [3., 90., 477., 900., 1500.] {
        let top = bytes(value * MI);
        assert_eq!((top / 2.).fract(), 0., "{value}");
    }
}

#[test]
fn y_max_includes_reference_lines() {
    let without = model(&[Some(0.1), Some(0.2)], &[], Measure::Cpu);
    assert_eq!(y_max(&without), 0.3);
    let with_limit = model(&[Some(0.1), Some(0.2)], &[1.0], Measure::Cpu);
    // A reference line takes no headroom: a 1-core limit puts the top at 1 core.
    assert_eq!(y_max(&with_limit), 1.0);
    let empty = model(&[], &[], Measure::Bytes);
    assert_eq!(y_max(&empty), MI);
}

#[test]
fn y_max_follows_the_node_capacity_not_a_power_step() {
    // 26 cores allocatable, 5 requested, 3 used: the axis is allocatable rounded up.
    let node = model(&[Some(3.)], &[26., 5.], Measure::Cpu);
    assert_eq!(y_max(&node), 30.);
    // 7.6Gi allocatable: 8Gi, not the 12Gi the 6% headroom used to push it to.
    let memory = model(
        &[Some(4. * 1024. * MI)],
        &[7.6 * 1024. * MI],
        Measure::Bytes,
    );
    assert_eq!(y_max(&memory), 8. * 1024. * MI);
    // Usage above the capacity still gets its headroom.
    let busy = model(&[Some(4.)], &[4.], Measure::Cpu);
    assert_eq!(y_max(&busy), 5.);
}

#[test]
fn axis_ticks_share_one_unit() {
    let ticks =
        |unit, top: f64| [0., 0.5, 1.].map(|fraction| axis_label(unit, top * fraction, top));
    assert_eq!(
        ticks(Measure::Cpu, 30.),
        ["0 cores", "15 cores", "30 cores"]
    );
    assert_eq!(ticks(Measure::Cpu, 5.), ["0 cores", "2.5 cores", "5 cores"]);
    assert_eq!(ticks(Measure::Cpu, 1.), ["0 cores", "0.5 cores", "1 core"]);
    // Below a core every tick is millicores, 0 included.
    assert_eq!(ticks(Measure::Cpu, 0.4), ["0m", "200m", "400m"]);
    assert_eq!(ticks(Measure::Cpu, 0.01), ["0m", "5m", "10m"]);
    assert_eq!(
        ticks(Measure::Bytes, 12. * 1024. * MI),
        ["0", "6Gi", "12Gi"]
    );
}

#[test]
fn x_at_maps_time_linearly() {
    let (start, end) = (at(0), at(900));
    assert_eq!(x_at(at(0), start, end, 200.), 0.);
    assert_eq!(x_at(at(450), start, end, 200.), 100.);
    assert_eq!(x_at(at(900), start, end, 200.), 200.);
    // Outside the window it clamps; an empty window puts everything at the right edge.
    assert_eq!(x_at(at(-5), start, end, 200.), 0.);
    assert_eq!(x_at(at(1_000), start, end, 200.), 200.);
    assert_eq!(x_at(at(5), at(5), at(5), 200.), 200.);
    assert_eq!(time_at(100., start, end, 200.), at(450));
}

#[test]
fn nearest_tick_picks_the_closest_point() {
    let points: Vec<(jiff::Timestamp, Option<f64>)> = [0, 15, 30, 45]
        .into_iter()
        .map(|seconds| (at(seconds), Some(1.)))
        .collect();
    assert_eq!(nearest_tick(&points, at(-10)), Some(0));
    assert_eq!(nearest_tick(&points, at(6)), Some(0));
    assert_eq!(nearest_tick(&points, at(9)), Some(1));
    assert_eq!(nearest_tick(&points, at(40)), Some(3));
    assert_eq!(nearest_tick(&points, at(500)), Some(3));
    assert_eq!(nearest_tick(&[], at(0)), None);
    // A point without a value still counts: the tooltip says "not running" there.
    let gappy = [(at(0), Some(1.)), (at(15), None)];
    assert_eq!(nearest_tick(&gappy, at(14)), Some(1));
}

fn points(values: &[(i64, Option<f64>)]) -> Vec<(jiff::Timestamp, Option<f64>)> {
    values
        .iter()
        .map(|(seconds, value)| (at(*seconds), *value))
        .collect()
}

#[test]
fn segments_split_at_none_and_gaps() {
    let max_gap = Duration::from_secs(37);
    let runs = segments(
        &points(&[
            (0, Some(1.)),
            (15, Some(2.)),
            (30, None),
            (45, Some(3.)),
            (60, Some(4.)),
            // 3 steps later: a gap of more than 2.5 steps.
            (105, Some(5.)),
        ]),
        at(0),
        max_gap,
    );
    let shape: Vec<Vec<f64>> = runs
        .iter()
        .map(|run| run.iter().map(|(_, value)| *value).collect())
        .collect();
    assert_eq!(shape, [vec![1., 2.], vec![3., 4.], vec![5.]]);
    // Exactly 2.5 steps apart still joins.
    let joined = segments(&points(&[(0, Some(1.)), (37, Some(2.))]), at(0), max_gap);
    assert_eq!(joined.len(), 1);
    assert!(segments(&points(&[(0, None)]), at(0), max_gap).is_empty());
}

#[test]
fn segments_drop_points_before_start() {
    let runs = segments(
        &points(&[(0, Some(1.)), (15, Some(2.)), (30, Some(3.))]),
        at(15),
        Duration::from_secs(37),
    );
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].len(), 2);
    assert_eq!(runs[0][0].0, at(15));
}

#[test]
fn reference_labels_stay_inside_the_chart() {
    const FLOOR: f32 = 100.;
    // Far from the top: just above the line.
    assert_eq!(reference_label_y(60., &[], FLOOR), 60. - LABEL_SIZE - 3.);
    // A limit at the very top (y = the top gutter) would put the label above the chart.
    assert_eq!(reference_label_y(GUTTER_TOP, &[], FLOOR), GUTTER_TOP + 3.);
    for line in [0., 3., 6., 12., 13.] {
        assert!(reference_label_y(line, &[], FLOOR) >= 0., "{line}");
    }
}

#[test]
fn a_label_does_not_sit_on_another_reference_line() {
    const FLOOR: f32 = 100.;
    let above = 60. - LABEL_SIZE - 3.;
    // Another line 8 px above the line would cross the text above it, so the label goes below.
    assert_eq!(reference_label_y(60., &[52.], FLOOR), 63.);
    // A line well clear of the text changes nothing.
    assert_eq!(reference_label_y(60., &[20., 90.], FLOOR), above);
    // Both sides crossed, or no room below: the label stays above.
    assert_eq!(reference_label_y(60., &[52., 66.], FLOOR), above);
    assert_eq!(reference_label_y(95., &[88.], FLOOR), 95. - LABEL_SIZE - 3.);
}

#[test]
fn nice_max_floors_rates_at_one_kb() {
    let rate = |value| nice_max(value, Measure::Rate);
    assert_eq!(rate(0.), 1_000.);
    assert_eq!(rate(120.), 1_000.);
    assert_eq!(rate(1_000.), 1_000.);
    assert_eq!(rate(1_001.), 2_000.);
    assert_eq!(rate(3_000.), 4_000.);
    assert_eq!(rate(420_000.), 1_000_000.);
    assert_eq!(rate(1_300_000.), 2_000_000.);
}

#[test]
fn rate_midlines_are_whole_in_their_unit() {
    let mid = |value| Measure::Rate.format(nice_max(value, Measure::Rate) / 2.);
    assert_eq!(mid(500.), "500 B/s");
    assert_eq!(mid(3_000.), "2 KB/s");
    // A 5 KB/s top would have put `3 KB/s` on the midline.
    assert_eq!(mid(4_500.), "5 KB/s");
    assert_eq!(mid(420_000.), "500 KB/s");
    assert_eq!(mid(2_600_000.), "2.0 MB/s");
    assert_eq!(mid(7_000_000.), "5.0 MB/s");
    for exponent in 3..10 {
        for mantissa in [1., 1.5, 2., 3., 4.5, 6., 9.] {
            let top = nice_max(mantissa * 10f64.powi(exponent), Measure::Rate);
            let half = top / 2.;
            // The half of a top is 1, 2, 4, 5 or 0.5 times a power of ten: no rounding in its text.
            let digits = format!("{half:e}");
            let significand: f64 = digits
                .split('e')
                .next()
                .and_then(|text| text.parse().ok())
                .unwrap_or(0.);
            assert!([1., 2., 4., 5.].contains(&significand), "{top} -> {half}");
        }
    }
}

#[test]
fn points_in_range_ignore_gaps_and_old_points() {
    let mut chart = model(&[None, None], &[], Measure::Rate);
    assert!(!has_points_in_range(&chart));
    chart.series[0].points = vec![(at(10), Some(5.))];
    chart.start = at(20);
    assert!(!has_points_in_range(&chart));
    chart.start = at(0);
    assert!(has_points_in_range(&chart));
}

#[test]
fn range_label_reads_whole_days_for_source_ranges() {
    let mut chart = model(&[], &[], Measure::Cpu);
    for (seconds, label) in [
        (900, "-15m"),
        (86_400, "-24h"),
        (7 * 86_400, "-7d"),
        (30 * 86_400, "-30d"),
        (7 * 86_400 + 3_600, "-169h"),
    ] {
        chart.end = at(seconds);
        assert_eq!(range_label(&chart), label);
    }
}

fn model_over(seconds: &[i64], step: u64, unit: Measure) -> UsageChartModel {
    UsageChartModel {
        step: Duration::from_secs(step),
        series: vec![ChartSeries {
            name: "used".into(),
            points: seconds
                .iter()
                .map(|second| (at(*second), Some(1.)))
                .collect(),
        }],
        ..model(&[], &[], unit)
    }
}

#[test]
fn a_short_history_says_how_much_data_there_is() {
    // Eight samples over the last 105 s of a 15 min range.
    let seconds: Vec<i64> = (0..8).map(|index| 795 + index * 15).collect();
    assert_eq!(
        collecting_text(&model_over(&seconds, 15, Measure::Cpu)).as_deref(),
        Some("Collecting · 1 min of data (kept 24 h)")
    );
    assert_eq!(
        collecting_text(&model_over(&[900], 15, Measure::Cpu)).as_deref(),
        Some("Collecting · under 1 min of data (kept 24 h)")
    );
}

#[test]
fn a_filled_chart_or_a_source_chart_does_not_collect() {
    let seconds: Vec<i64> = (0..60).map(|index| index * 15).collect();
    assert_eq!(
        collecting_text(&model_over(&seconds, 15, Measure::Cpu)),
        None
    );
    // A quarter of 900 s is 225 s: 240 s of data is enough.
    assert_eq!(
        collecting_text(&model_over(&[600, 840], 15, Measure::Cpu)),
        None
    );
    // A coarse step means a metrics source, which has its own retention.
    assert_eq!(
        collecting_text(&model_over(&[900], 300, Measure::Cpu)),
        None
    );
    assert_eq!(collecting_text(&model_over(&[], 15, Measure::Cpu)), None);
}

#[test]
fn span_text_reads_minutes_then_hours() {
    assert_eq!(span_text(20), "under 1 min");
    assert_eq!(span_text(120), "2 min");
    assert_eq!(span_text(3600), "1 h");
    assert_eq!(span_text(4800), "1 h 20 min");
}
