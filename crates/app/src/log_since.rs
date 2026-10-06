//! How far back a log tab reads: the saved tail, or the last few minutes or hours.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum LogSince {
    /// The saved tail line count, then follow.
    #[default]
    Tail,
    Minutes5,
    Minutes15,
    Hour1,
    Hours6,
}

impl LogSince {
    pub(crate) const ALL: [Self; 5] = [
        Self::Tail,
        Self::Minutes5,
        Self::Minutes15,
        Self::Hour1,
        Self::Hours6,
    ];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Tail => "tail",
            Self::Minutes5 => "5m",
            Self::Minutes15 => "15m",
            Self::Hour1 => "1h",
            Self::Hours6 => "6h",
        }
    }

    /// The `sinceSeconds` of the log request; `None` keeps the tail.
    pub(crate) fn seconds(self) -> Option<u32> {
        match self {
            Self::Tail => None,
            Self::Minutes5 => Some(5 * 60),
            Self::Minutes15 => Some(15 * 60),
            Self::Hour1 => Some(60 * 60),
            Self::Hours6 => Some(6 * 60 * 60),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tail_is_the_default_and_has_no_window() {
        assert_eq!(LogSince::default(), LogSince::Tail);
        assert_eq!(LogSince::Tail.seconds(), None);
    }

    #[test]
    fn each_window_maps_to_its_seconds() {
        let seconds: Vec<_> = LogSince::ALL.map(LogSince::seconds).into();
        assert_eq!(
            seconds,
            [None, Some(300), Some(900), Some(3600), Some(21_600)]
        );
    }

    #[test]
    fn labels_are_distinct() {
        let labels: Vec<_> = LogSince::ALL.map(LogSince::label).into();
        assert_eq!(labels, ["tail", "5m", "15m", "1h", "6h"]);
    }
}
