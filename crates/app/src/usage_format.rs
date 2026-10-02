//! Human-readable CPU and memory usage: `310m`, `2.5 cores`, `498Mi`, `15.6Gi`, and the pair
//! forms `9.8 / 15.8 cores` and `498 of 512Mi`.

use crate::status_tone::StatusTone;

const BYTE_UNITS: [&str; 7] = ["B", "Ki", "Mi", "Gi", "Ti", "Pi", "Ei"];
/// Units below this index are shown whole; larger ones keep one decimal below 100, unless it is
/// zero.
const MILLICORES: &str = "m";
const FIRST_DECIMAL_UNIT: usize = 3;
const UNIT_STEP: f64 = 1024.;

/// A usage ratio at which a bar turns yellow.
const WARN_RATIO: f64 = 0.8;
/// A usage ratio at which a bar turns red.
const BAD_RATIO: f64 = 0.9;

/// What a value measures: CPU in cores or memory in bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Measure {
    Cpu,
    Bytes,
}

/// A number and its unit, kept apart so a pair can show a shared unit once.
struct Parts {
    number: String,
    unit: &'static str,
}

impl Measure {
    /// `value` is in cores or bytes.
    pub(crate) fn format(self, value: f64) -> String {
        let parts = self.parts(value);
        format!("{}{}", parts.number, parts.unit)
    }

    /// The unit once when both values share it (`9.8 / 15.8 cores`, `498 of 512Mi`); else both
    /// (`310m of 1 core`).
    pub(crate) fn format_pair(self, used: f64, total: f64, separator: &str) -> String {
        let (used, total) = (self.parts(used), self.parts(total));
        if same_unit(used.unit, total.unit) {
            return format!("{}{separator}{}{}", used.number, total.number, total.unit);
        }
        format!(
            "{}{}{separator}{}{}",
            used.number, used.unit, total.number, total.unit
        )
    }

    fn parts(self, value: f64) -> Parts {
        match self {
            Self::Cpu => cpu_parts(value),
            Self::Bytes => byte_parts(value),
        }
    }
}

/// Whether a pair prints its unit once. `core` and `cores` are one unit (`1 / 15.8 cores`).
/// Millicores never share: a bare `44 of 300m` reads like cores, so both keep the `m`.
fn same_unit(left: &str, right: &str) -> bool {
    left != MILLICORES && left.trim_end_matches('s') == right.trim_end_matches('s')
}

fn cpu_parts(cores: f64) -> Parts {
    let part = |number: String, unit| Parts { number, unit };
    if cores <= 0. {
        return part("0".to_owned(), "m");
    }
    let millicores = (cores * 1000.).round();
    if millicores < 1. {
        return part("<1".to_owned(), "m");
    }
    if millicores < 1000. {
        return part(format!("{millicores:.0}"), "m");
    }
    let tenths = (cores * 10.).round() / 10.;
    let unit = if tenths == 1. { " core" } else { " cores" };
    part(trim_decimal(tenths), unit)
}

/// One decimal, dropped when it is zero: `2.5`, `12`.
fn trim_decimal(value: f64) -> String {
    if value.fract() == 0. {
        format!("{value:.0}")
    } else {
        format!("{value:.1}")
    }
}

fn byte_parts(bytes: f64) -> Parts {
    let mut scaled = bytes.max(0.);
    let mut unit = 0;
    loop {
        let (number, rounded) = round_for_unit(unit, scaled);
        // A value that rounds up to 1024 reads better in the next unit: `1.0Gi`, not `1024Mi`.
        if rounded < UNIT_STEP || unit == BYTE_UNITS.len() - 1 {
            return Parts {
                number,
                unit: BYTE_UNITS[unit],
            };
        }
        scaled /= UNIT_STEP;
        unit += 1;
    }
}

/// The text of `scaled` in unit `unit`, and the value that text shows.
fn round_for_unit(unit: usize, scaled: f64) -> (String, f64) {
    if unit >= FIRST_DECIMAL_UNIT {
        let tenths = (scaled * 10.).round() / 10.;
        if tenths < 100. {
            return (trim_decimal(tenths), tenths);
        }
    }
    let whole = scaled.round();
    (format!("{whole:.0}"), whole)
}

/// `0.314` is `31%`; may pass 100 %.
pub(crate) fn format_percent(ratio: f64) -> String {
    format!("{:.0}%", (ratio * 100.).max(0.).round())
}

/// Yellow from 80 %, red from 90 %.
pub(crate) fn usage_tone(ratio: f64) -> Option<StatusTone> {
    if ratio >= BAD_RATIO {
        Some(StatusTone::Bad)
    } else if ratio >= WARN_RATIO {
        Some(StatusTone::Warn)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KI: f64 = 1024.;
    const MI: f64 = KI * 1024.;
    const GI: f64 = MI * 1024.;

    #[test]
    fn cpu_format_uses_millicores_below_one_core() {
        let text = |cores| Measure::Cpu.format(cores);
        assert_eq!(text(0.), "0m");
        assert_eq!(text(0.0004), "<1m");
        assert_eq!(text(0.31), "310m");
        assert_eq!(text(0.9996), "1 core");
        assert_eq!(text(1.), "1 core");
        assert_eq!(text(2.5), "2.5 cores");
        assert_eq!(text(12.04), "12 cores");
    }

    #[test]
    fn bytes_format_uses_binary_units() {
        let text = |bytes| Measure::Bytes.format(bytes);
        assert_eq!(text(0.), "0B");
        assert_eq!(text(512.), "512B");
        assert_eq!(text(2. * KI), "2Ki");
        assert_eq!(text(498. * MI), "498Mi");
        assert_eq!(text(1.1 * GI), "1.1Gi");
        assert_eq!(text(15.6 * GI), "15.6Gi");
        assert_eq!(text(120. * GI), "120Gi");
        assert_eq!(text(1.5 * GI * 1024.), "1.5Ti");
        // A value that rounds to 1024 moves up a unit.
        assert_eq!(text(1023.7 * MI), "1Gi");
        assert_eq!(text(64. * GI), "64Gi");
        assert_eq!(text(1023.9 * KI), "1Mi");
        assert_eq!(text(1023.7), "1Ki");
    }

    #[test]
    fn format_pair_shows_a_shared_unit_once() {
        assert_eq!(
            Measure::Cpu.format_pair(9.8, 15.8, " / "),
            "9.8 / 15.8 cores"
        );
        assert_eq!(
            Measure::Bytes.format_pair(498. * MI, 512. * MI, " of "),
            "498 of 512Mi"
        );
        assert_eq!(Measure::Cpu.format_pair(0.31, 1., " of "), "310m of 1 core");
        assert_eq!(Measure::Cpu.format_pair(1., 15.8, " / "), "1 / 15.8 cores");
        assert_eq!(
            Measure::Bytes.format_pair(900. * MI, 2. * GI, " of "),
            "900Mi of 2Gi"
        );
    }

    #[test]
    fn format_pair_keeps_the_unit_on_millicores() {
        assert_eq!(Measure::Cpu.format_pair(0.044, 0.3, " of "), "44m of 300m");
        assert_eq!(Measure::Cpu.format_pair(0.0004, 0.3, " of "), "<1m of 300m");
        assert_eq!(Measure::Cpu.format_pair(0.3, 0.5, " / "), "300m / 500m");
        // Bytes and cores still share their unit.
        assert_eq!(
            Measure::Bytes.format_pair(498. * MI, 512. * MI, " of "),
            "498 of 512Mi"
        );
    }

    #[test]
    fn format_percent_rounds() {
        assert_eq!(format_percent(0.314), "31%");
        assert_eq!(format_percent(1.04), "104%");
        assert_eq!(format_percent(0.), "0%");
    }

    #[test]
    fn usage_tone_thresholds() {
        assert_eq!(usage_tone(0.79), None);
        assert_eq!(usage_tone(0.8), Some(StatusTone::Warn));
        assert_eq!(usage_tone(0.89), Some(StatusTone::Warn));
        assert_eq!(usage_tone(0.9), Some(StatusTone::Bad));
        assert_eq!(usage_tone(1.3), Some(StatusTone::Bad));
    }
}
