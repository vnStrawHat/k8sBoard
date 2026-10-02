//! Bounded per-series history shared by the usage histories: fine and coarse timelines per feed
//! and a pair of rings per series. A fine ring never outgrows `FINE_TICKS` (1 h at 15 s), a
//! coarse ring never outgrows `COARSE_POINTS` (24 h at 5 min), and a series that stops reporting
//! is freed and later dropped.

use std::collections::VecDeque;
use std::time::Duration;

use cluster::METRICS_INTERVAL;

/// One hour at the 15 s metrics interval.
pub(crate) const FINE_TICKS: usize = 240;
/// Fine ticks folded into one coarse point: 5 minutes.
pub(crate) const TICKS_PER_COARSE: usize = 20;
/// One day of coarse points.
pub(crate) const COARSE_POINTS: usize = 288;
/// A series unseen for a day is dropped altogether.
pub(crate) const DROP_AFTER_TICKS: u64 = FINE_TICKS as u64 * 24;

/// How finely a series is read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Resolution {
    Fine,
    /// The coarse points, then the fine ticks newer than the last one.
    Coarse,
}

/// How a coarse point is made from the fine points it covers.
pub(crate) trait RingPoint: Copy {
    /// The field-wise mean of `points`, which is never empty.
    fn mean(points: &[Self]) -> Self;
}

/// The arrival time of each recent tick, oldest first.
struct Timeline {
    cap: usize,
    ticks: VecDeque<jiff::Timestamp>,
}

impl Timeline {
    fn new(cap: usize) -> Self {
        Self {
            cap,
            ticks: VecDeque::new(),
        }
    }

    fn push(&mut self, at: jiff::Timestamp) {
        if self.ticks.len() == self.cap {
            self.ticks.pop_front();
        }
        self.ticks.push_back(at);
    }
}

/// The fine and coarse timelines of one feed. A ring that is shorter than its timeline is aligned
/// to the timeline's end: ring index `i` is timeline index `len - ring.len() + i`.
pub(crate) struct Timelines {
    fine: Timeline,
    coarse: Timeline,
}

impl Default for Timelines {
    fn default() -> Self {
        Self {
            fine: Timeline::new(FINE_TICKS),
            coarse: Timeline::new(COARSE_POINTS),
        }
    }
}

impl Timelines {
    /// Adds the tick the feed has just counted as number `tick` (1-based). Returns whether it
    /// also closes a coarse point, so the rings must fold.
    pub(crate) fn push(&mut self, at: jiff::Timestamp, tick: u64) -> bool {
        self.fine.push(at);
        let closes_coarse = tick.is_multiple_of(TICKS_PER_COARSE as u64);
        if closes_coarse {
            self.coarse.push(at);
        }
        closes_coarse
    }

    pub(crate) fn fine_len(&self) -> usize {
        self.fine.ticks.len()
    }

    #[cfg(test)]
    pub(crate) fn coarse_len(&self) -> usize {
        self.coarse.ticks.len()
    }

    /// The arrival time of the newest tick.
    pub(crate) fn newest(&self) -> Option<jiff::Timestamp> {
        self.fine.ticks.back().copied()
    }

    /// From the oldest kept point to the newest tick.
    pub(crate) fn span(&self) -> Option<Duration> {
        // A coarse point is a mean of older ticks, but the fine ring may reach further back.
        let oldest = self
            .coarse
            .ticks
            .front()
            .into_iter()
            .chain(self.fine.ticks.front())
            .min()?;
        Some(self.newest()?.duration_since(*oldest).unsigned_abs())
    }

    /// The spacing of the points of `resolution`, for splitting a line at a gap.
    pub(crate) fn step(resolution: Resolution) -> Duration {
        match resolution {
            Resolution::Fine => METRICS_INTERVAL,
            Resolution::Coarse => METRICS_INTERVAL * TICKS_PER_COARSE as u32,
        }
    }

    /// How many fine ticks are newer than the newest coarse point (all of them without one).
    fn fine_tail_len(&self) -> usize {
        let Some(last_coarse) = self.coarse.ticks.back() else {
            return self.fine.ticks.len();
        };
        self.fine
            .ticks
            .iter()
            .rev()
            .take_while(|tick| *tick > last_coarse)
            .count()
    }

    /// The arrival time of each point of a series at `resolution`, oldest first.
    pub(crate) fn times(&self, resolution: Resolution) -> Vec<jiff::Timestamp> {
        match resolution {
            Resolution::Fine => self.fine.ticks.iter().copied().collect(),
            Resolution::Coarse => {
                let tail = self.fine_tail_len();
                let skip = self.fine.ticks.len() - tail;
                self.coarse
                    .ticks
                    .iter()
                    .chain(self.fine.ticks.iter().skip(skip))
                    .copied()
                    .collect()
            }
        }
    }

    /// One value per entry of `times(resolution)` for the series `rings`; a series that started
    /// later, or was freed, reads `None` before and where it has no point.
    pub(crate) fn values<P: Copy>(
        &self,
        rings: &Rings<P>,
        resolution: Resolution,
    ) -> Vec<Option<P>> {
        let fine = aligned(rings.fine.as_ref(), self.fine.ticks.len());
        match resolution {
            Resolution::Fine => fine,
            Resolution::Coarse => {
                let mut values = aligned(Some(&rings.coarse), self.coarse.ticks.len());
                let tail = self.fine_tail_len();
                values.extend_from_slice(&fine[fine.len() - tail..]);
                values
            }
        }
    }

    /// `values` for a series with no rings: as many `None`s as there are points.
    pub(crate) fn blank<P: Copy>(&self, resolution: Resolution) -> Vec<Option<P>> {
        vec![None; self.times(resolution).len()]
    }
}

/// `ring` padded with leading `None`s to `len` entries.
fn aligned<P: Copy>(ring: Option<&VecDeque<Option<P>>>, len: usize) -> Vec<Option<P>> {
    let ring_len = ring.map_or(0, VecDeque::len);
    let mut values = vec![None; len.saturating_sub(ring_len)];
    values.extend(ring.into_iter().flatten().copied());
    values
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
    coarse: VecDeque<Option<P>>,
    /// The tick (1-based, counted by the owner) that last carried a value.
    last_seen: u64,
}

impl<P: Copy> Rings<P> {
    /// A series with no points yet; `record` adds the first one.
    pub(crate) fn new() -> Self {
        Self {
            fine: None,
            coarse: VecDeque::new(),
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

    /// Closes a coarse point: the mean of the values among the last `TICKS_PER_COARSE` fine
    /// entries, or `None` when there are none or the fine ring is freed.
    pub(crate) fn fold(&mut self)
    where
        P: RingPoint,
    {
        let recent: Vec<P> = self
            .fine
            .iter()
            .flatten()
            .rev()
            .take(TICKS_PER_COARSE)
            .filter_map(|point| *point)
            .collect();
        if self.coarse.len() == COARSE_POINTS {
            self.coarse.pop_front();
        }
        self.coarse
            .push_back((!recent.is_empty()).then(|| P::mean(&recent)));
    }

    /// The value of the newest tick; `None` when the series did not report it.
    pub(crate) fn newest(&self) -> Option<P> {
        self.fine.as_ref()?.back().copied().flatten()
    }

    /// The values of the last two ticks, oldest first; `None` unless the series reported both.
    pub(crate) fn newest_pair(&self) -> Option<[P; 2]> {
        let fine = self.fine.as_ref()?;
        let previous = fine.get(fine.len().checked_sub(2)?).copied().flatten()?;
        let newest = fine.back().copied().flatten()?;
        Some([previous, newest])
    }

    /// Frees the fine ring after an hour without a value and drops the series after a day.
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

    /// The number of kept fine ticks; 0 once freed.
    pub(crate) fn len(&self) -> usize {
        self.fine.as_ref().map_or(0, VecDeque::len)
    }

    #[cfg(test)]
    /// The number of kept coarse points.
    pub(crate) fn coarse_len(&self) -> usize {
        self.coarse.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    impl RingPoint for u32 {
        fn mean(points: &[Self]) -> Self {
            points.iter().sum::<u32>() / points.len() as u32
        }
    }

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
    fn timelines_keep_the_newest_ticks_and_close_coarse_points() {
        let mut timelines = Timelines::default();
        let mut closed = 0;
        for tick in 1..=(FINE_TICKS as u64 + 10) {
            if timelines.push(at(tick as i64 * 15), tick) {
                closed += 1;
            }
        }
        assert_eq!(timelines.fine_len(), FINE_TICKS);
        assert_eq!(closed, (FINE_TICKS + 10) / TICKS_PER_COARSE);
        assert_eq!(timelines.coarse_len(), closed);
        assert_eq!(timelines.newest(), Some(at((FINE_TICKS as i64 + 10) * 15)));
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

    #[test]
    fn fold_averages_the_last_twenty_fine_values() {
        let mut rings = Rings::new();
        for step in 1..=TICKS_PER_COARSE as u64 {
            tick(&mut rings, step, Some(step as u32));
        }
        rings.fold();
        // The mean of 1..=20, truncated.
        assert_eq!(rings.coarse.back().copied().flatten(), Some(10));
        // A window with no value, or a freed ring, folds to `None`.
        for step in 21..=40_u64 {
            tick(&mut rings, step, None);
        }
        rings.fold();
        assert_eq!(rings.coarse.back().copied().flatten(), None);
        rings.age(41 + FINE_TICKS as u64);
        rings.fold();
        assert_eq!(rings.coarse.back().copied().flatten(), None);
        assert_eq!(rings.coarse_len(), 3);
    }

    #[test]
    fn coarse_ring_is_capped() {
        let mut rings = Rings::<u32>::new();
        for _ in 0..COARSE_POINTS + 5 {
            rings.fold();
        }
        assert_eq!(rings.coarse_len(), COARSE_POINTS);
    }

    #[test]
    fn values_align_to_the_end_of_the_timeline() {
        let mut timelines = Timelines::default();
        let mut rings = Rings::new();
        for step in 1..=5_u64 {
            timelines.push(at(step as i64), step);
            rings.begin_tick();
            // The series first appears at tick 4.
            if step >= 4 {
                rings.record(step as u32, step);
            }
        }
        let values = timelines.values(&rings, Resolution::Fine);
        assert_eq!(values, [None, None, None, Some(4), Some(5)]);
        assert_eq!(timelines.times(Resolution::Fine).len(), values.len());
        let blank: Vec<Option<u32>> = timelines.blank(Resolution::Fine);
        assert_eq!(blank, [None; 5]);
    }

    #[test]
    fn coarse_times_append_the_fine_ticks_after_the_last_coarse_point() {
        let mut timelines = Timelines::default();
        let mut rings = Rings::new();
        for step in 1..=45_u64 {
            let is_fold = timelines.push(at(step as i64 * 15), step);
            rings.begin_tick();
            rings.record(step as u32, step);
            if is_fold {
                rings.fold();
            }
        }
        // Coarse points at ticks 20 and 40, then fine ticks 41..=45.
        let times = timelines.times(Resolution::Coarse);
        let seconds: Vec<i64> = times.iter().map(|time| time.as_second()).collect();
        assert_eq!(seconds, [300, 600, 615, 630, 645, 660, 675]);
        let values = timelines.values(&rings, Resolution::Coarse);
        assert_eq!(
            values,
            [
                Some(10),
                Some(30),
                Some(41),
                Some(42),
                Some(43),
                Some(44),
                Some(45)
            ]
        );
        assert_eq!(
            Timelines::step(Resolution::Coarse),
            Duration::from_secs(300)
        );
        assert_eq!(timelines.span(), Some(Duration::from_secs(675 - 15)));
    }
}
