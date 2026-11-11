//! What the sampler has read: the last hour as sampled, and the last day as minute averages.

use std::collections::VecDeque;

use jiff::{SignedDuration, Timestamp};

/// How far back each history reaches.
const HOUR: SignedDuration = SignedDuration::from_hours(1);
const DAY: SignedDuration = SignedDuration::from_hours(24);

/// One reading of the machine, or the average of a minute of them.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Point {
    /// When it was read; for an average, the start of its minute.
    pub at: Timestamp,
    /// How busy every processor was, 0 to 100.
    pub cpu: f64,
    /// Layover's share of every processor, 0 to 100.
    pub layover_cpu: f64,
    /// Physical memory in use, in bytes.
    pub memory: u64,
    /// Physical memory Layover's processes hold, in bytes.
    pub layover_memory: u64,
    /// How many processes Layover's share counted.
    pub layover_processes: u32,
    /// Bytes used on each drive the monitor watches, in its order.
    pub disks: Vec<u64>,
}

/// Every reading of the last hour, and the last day's minute averages.
#[derive(Debug, Default)]
pub(crate) struct History {
    hour: VecDeque<Point>,
    minutes: VecDeque<Point>,
    /// The readings of the minute under way, averaged when the next minute begins.
    minute: Vec<Point>,
}

impl History {
    /// Adds a reading, and lets go of what has fallen out of both windows.
    pub(crate) fn push(&mut self, point: Point) {
        if self
            .minute
            .first()
            .is_some_and(|first| minute_of(first.at) != minute_of(point.at))
        {
            self.minutes.extend(average(&self.minute));
            self.minute.clear();
        }
        self.minute.push(point.clone());
        let newest = point.at;
        self.hour.push_back(point);
        trim(&mut self.hour, newest, HOUR);
        trim(&mut self.minutes, newest, DAY);
    }

    /// Every reading of the last hour, oldest first.
    pub(crate) fn last_hour(&self) -> Vec<Point> {
        self.hour.iter().cloned().collect()
    }

    /// The last day as minute averages, oldest first, ending with the minute under way so far.
    pub(crate) fn last_day(&self) -> Vec<Point> {
        let mut points: Vec<Point> = self.minutes.iter().cloned().collect();
        points.extend(average(&self.minute));
        points
    }
}

fn minute_of(at: Timestamp) -> i64 {
    at.as_second().div_euclid(60)
}

/// Drops the points read more than `span` before `newest`.
fn trim(points: &mut VecDeque<Point>, newest: Timestamp, span: SignedDuration) {
    let Ok(oldest) = newest.checked_sub(span) else {
        return;
    };
    while points.front().is_some_and(|point| point.at < oldest) {
        points.pop_front();
    }
}

/// The average of `points`, dated at the start of the first one's minute.
fn average(points: &[Point]) -> Option<Point> {
    let first = points.first()?;
    let count = u32::try_from(points.len()).unwrap_or(u32::MAX);
    let mean = |value: &dyn Fn(&Point) -> u64| -> u64 {
        let total: u128 = points.iter().map(|point| u128::from(value(point))).sum();
        u64::try_from(total / u128::from(count)).unwrap_or(u64::MAX)
    };
    let mean_f = |value: &dyn Fn(&Point) -> f64| -> f64 {
        points.iter().map(value).sum::<f64>() / f64::from(count)
    };
    Some(Point {
        at: Timestamp::from_second(minute_of(first.at) * 60).unwrap_or(first.at),
        cpu: mean_f(&|point| point.cpu),
        layover_cpu: mean_f(&|point| point.layover_cpu),
        memory: mean(&|point| point.memory),
        layover_memory: mean(&|point| point.layover_memory),
        layover_processes: u32::try_from(mean(&|point| u64::from(point.layover_processes)))
            .unwrap_or(u32::MAX),
        disks: (0..first.disks.len())
            .map(|drive| mean(&|point| point.disks.get(drive).copied().unwrap_or_default()))
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `second` seconds after the start of a minute.
    fn at(second: i64) -> Timestamp {
        Timestamp::from_second(1_800_000_000 + second).expect("valid")
    }

    fn point(second: i64, cpu: f64) -> Point {
        Point {
            at: at(second),
            cpu,
            layover_cpu: cpu / 10.0,
            memory: 1_000,
            layover_memory: 100,
            layover_processes: 3,
            disks: vec![5_000],
        }
    }

    #[test]
    fn the_hour_keeps_every_reading_and_lets_older_ones_go() {
        let mut history = History::default();
        for second in (0..=70 * 60).step_by(5) {
            history.push(point(second, 1.0));
        }
        let hour = history.last_hour();
        assert_eq!(
            hour.len(),
            721,
            "an hour of five-second readings, both ends included"
        );
        assert_eq!(hour.first().map(|point| point.at), Some(at(10 * 60)));
        assert_eq!(hour.last().map(|point| point.at), Some(at(70 * 60)));
    }

    #[test]
    fn a_minute_is_averaged_and_dated_at_its_start() {
        let mut history = History::default();
        history.push(point(10, 10.0));
        history.push(point(40, 30.0));
        history.push(point(70, 50.0));

        let day = history.last_day();
        assert_eq!(day.len(), 2, "{day:?}");
        assert_eq!(day[0].at, at(0));
        assert!((day[0].cpu - 20.0).abs() < 1e-9);
        assert!((day[0].layover_cpu - 2.0).abs() < 1e-9);
        assert_eq!(day[0].disks, [5_000]);
        assert_eq!(day[1].at, at(60), "the minute under way, so far");
        assert!((day[1].cpu - 50.0).abs() < 1e-9);
    }

    #[test]
    fn the_day_lets_minutes_older_than_a_day_go() {
        let mut history = History::default();
        for minute in 0..=25 * 60 {
            history.push(point(minute * 60, 1.0));
        }
        let day = history.last_day();
        assert_eq!(
            day.len(),
            24 * 60 + 1,
            "a day of minutes and the one under way"
        );
        assert_eq!(day[0].at, at(60 * 60));
    }
}
