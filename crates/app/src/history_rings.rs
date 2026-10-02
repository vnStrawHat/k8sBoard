//! Bounded per-series history shared by the usage histories: one timeline per feed and one
//! ring of points per series. A ring never outgrows `FINE_TICKS`, and a series that stops
//! reporting is freed and later dropped.

use std::collections::VecDeque;

/// One hour at the 15 s metrics interval.
pub(crate) const FINE_TICKS: usize = 240;
/// A series unseen for a day is dropped altogether.
const DROP_AFTER_TICKS: u64 = FINE_TICKS as u64 * 24;

/// The arrival time of each recent tick, oldest first. A ring that is shorter than the timeline
/// is aligned to its end: ring index `i` is timeline index `len - ring.len() + i`.
#[derive(Default)]
pub(crate) struct Timeline {
    ticks: VecDeque<jiff::Timestamp>,
}

impl Timeline {
    pub(crate) fn push(&mut self, at: jiff::Timestamp) {
        if self.ticks.len() == FINE_TICKS {
            self.ticks.pop_front();
        }
        self.ticks.push_back(at);
    }

    pub(crate) fn len(&self) -> usize {
        self.ticks.len()
    }
}

/// What `Rings::age` decided for a series.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Retention {
    Keep,
    Drop,
}

/// The points of one series. `None` marks a tick where the series reported nothing.
pub(crate) struct Rings<P> {
    /// `None` once freed: the series was not seen for a whole hour.
    fine: Option<VecDeque<Option<P>>>,
    /// The tick (1-based, counted by the owner) that last carried a value.
    last_seen: u64,
}

impl<P: Copy> Rings<P> {
    /// A series with no points yet; `record` adds the first one.
    pub(crate) fn new() -> Self {
        Self {
            fine: None,
            last_seen: 0,
        }
    }

    /// Opens a new tick: every live ring gets an empty entry that `record` may fill.
    pub(crate) fn begin_tick(&mut self) {
        let Some(fine) = &mut self.fine else {
            return;
        };
        if fine.len() == FINE_TICKS {
            fine.pop_front();
        }
        fine.push_back(None);
    }

    /// Sets the entry of the open tick `tick`. A freed or new ring starts again with this tick.
    pub(crate) fn record(&mut self, point: P, tick: u64) {
        let fine = self.fine.get_or_insert_with(|| VecDeque::from([None]));
        if let Some(newest) = fine.back_mut() {
            *newest = Some(point);
        }
        self.last_seen = tick;
    }

    /// The value of the newest tick; `None` when the series did not report it.
    pub(crate) fn newest(&self) -> Option<P> {
        self.fine.as_ref()?.back().copied().flatten()
    }

    /// Frees the ring after an hour without a value and drops the series after a day.
    pub(crate) fn age(&mut self, tick: u64) -> Retention {
        let unseen = tick.saturating_sub(self.last_seen);
        if unseen >= DROP_AFTER_TICKS {
            return Retention::Drop;
        }
        if unseen >= FINE_TICKS as u64 {
            self.fine = None;
        }
        Retention::Keep
    }

    /// The number of kept ticks; 0 once freed.
    pub(crate) fn len(&self) -> usize {
        self.fine.as_ref().map_or(0, VecDeque::len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(seconds: i64) -> jiff::Timestamp {
        jiff::Timestamp::from_second(seconds).expect("valid timestamp")
    }

    fn tick(rings: &mut Rings<u32>, tick: u64, point: Option<u32>) {
        rings.begin_tick();
        if let Some(point) = point {
            rings.record(point, tick);
        }
    }

    #[test]
    fn timeline_keeps_the_newest_ticks() {
        let mut timeline = Timeline::default();
        for second in 0..(FINE_TICKS as i64 + 10) {
            timeline.push(at(second));
        }
        assert_eq!(timeline.len(), FINE_TICKS);
    }

    #[test]
    fn ring_is_capped_at_the_fine_ticks() {
        let mut rings = Rings::new();
        for step in 1..=(FINE_TICKS as u64 + 60) {
            tick(&mut rings, step, Some(step as u32));
        }
        assert_eq!(rings.len(), FINE_TICKS);
        assert_eq!(rings.newest(), Some(FINE_TICKS as u32 + 60));
    }

    #[test]
    fn ring_reads_none_when_the_newest_tick_has_no_value() {
        let mut rings = Rings::new();
        tick(&mut rings, 1, Some(7));
        tick(&mut rings, 2, None);
        assert_eq!(rings.newest(), None);
        tick(&mut rings, 3, Some(9));
        assert_eq!(rings.newest(), Some(9));
    }

    #[test]
    fn ring_is_freed_after_an_hour_and_dropped_after_a_day() {
        let mut rings = Rings::new();
        tick(&mut rings, 1, Some(1));
        let freed_at = 1 + FINE_TICKS as u64;
        assert_eq!(rings.age(freed_at - 1), Retention::Keep);
        assert_eq!(rings.len(), 1);
        assert_eq!(rings.age(freed_at), Retention::Keep);
        assert_eq!(rings.len(), 0);
        assert_eq!(rings.age(1 + DROP_AFTER_TICKS - 1), Retention::Keep);
        assert_eq!(rings.age(1 + DROP_AFTER_TICKS), Retention::Drop);
    }

    #[test]
    fn freed_ring_starts_again_when_the_series_returns() {
        let mut rings = Rings::new();
        tick(&mut rings, 1, Some(1));
        rings.age(1 + FINE_TICKS as u64);
        tick(&mut rings, 2 + FINE_TICKS as u64, Some(5));
        assert_eq!(rings.len(), 1);
        assert_eq!(rings.newest(), Some(5));
    }
}
