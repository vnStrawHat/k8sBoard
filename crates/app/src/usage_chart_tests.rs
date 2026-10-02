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
    assert_eq!(cpu(0.021), 0.05);
    assert_eq!(cpu(0.31), 0.5);
    assert_eq!(cpu(1.0), 1.0);
    assert_eq!(cpu(1.3), 2.0);
    assert_eq!(cpu(2.6), 5.0);
    assert_eq!(cpu(7.0), 10.0);
    let bytes = |value| nice_max(value, Measure::Bytes);
    assert_eq!(bytes(0.0), MI);
    // Powers of two in the value's own binary unit: the midline is a whole number too.
    assert_eq!(bytes(477. * MI), 512. * MI);
    assert_eq!(bytes(300. * MI), 512. * MI);
    assert_eq!(bytes(100. * MI), 128. * MI);
    assert_eq!(bytes(900. * MI), 1024. * MI);
    assert_eq!(bytes(513. * MI), 1024. * MI);
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
    assert_eq!(y_max(&without), 0.5);
    let with_limit = model(&[Some(0.1), Some(0.2)], &[1.0], Measure::Cpu);
    assert_eq!(y_max(&with_limit), 2.0);
    let empty = model(&[], &[], Measure::Bytes);
    assert_eq!(y_max(&empty), MI);
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
    // Far from the top: just above the line.
    assert_eq!(reference_label_y(60.), 60. - LABEL_SIZE - 3.);
    // A limit at the very top (y = the top gutter) would put the label above the chart.
    assert_eq!(reference_label_y(GUTTER_TOP), GUTTER_TOP + 3.);
    for line in [0., 3., 6., 12., 13.] {
        assert!(reference_label_y(line) >= 0., "{line}");
    }
}
