//! The Fuel Reserve: a bound on what the whole factory may spend, not just one chain.
//!
//! Fuel is per-itinerary, and that is not enough on its own. A scheduled pipeline mints a *fresh*
//! itinerary on every tick, each with a full Fuel budget, so an hourly pipeline at `fuel_usd = 20`
//! permits `24 × 20 = $480` a day while every individual itinerary stays perfectly within its
//! rail. Fuel bounds a chain; the Reserve bounds the factory.
//!
//! # Why a rolling window and not a calendar day
//!
//! A daily cap sounds simpler and is worse, for two reasons:
//!
//! - **Midnight doubles it.** Spend the cap at 23:59 and the bucket resets a minute later, so a
//!   "$50 a day" limit permits $100 in two minutes.
//! - **Days need a timezone.** A sibling project's daily gate bucketed by UTC while its ledger
//!   bucketed by local time, so between local midnight and the UTC offset the gate read the wrong
//!   day's total and let spending through.
//!
//! A rolling window has no midnight, no timezone and no daylight-saving edge. "At most $50 in any
//! 24 hours" is both stricter and easier to reason about than "at most $50 per calendar day".

use std::collections::VecDeque;
use std::time::Duration;

use jiff::{SignedDuration, Timestamp};

use crate::itinerary::Denial;

/// A snapshot of the Reserve, for display.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReserveState {
    /// Ceiling for the window, in US dollars. `None` when the Reserve is unlimited.
    pub cap_usd: Option<f64>,
    /// Spent within the window.
    pub spent_usd: f64,
    /// Left before work stops, or `None` when unlimited.
    pub remaining_usd: Option<f64>,
    /// How far back the window reaches.
    pub window: Duration,
    /// Whether new work is currently refused.
    pub exhausted: bool,
}

/// A rolling-window spend cap across every itinerary in the factory.
#[derive(Debug, Clone)]
pub struct Reserve {
    cap_usd: Option<f64>,
    window: Duration,
    entries: VecDeque<(Timestamp, f64)>,
}

impl Reserve {
    /// Creates a Reserve permitting at most `cap_usd` within any `window`.
    ///
    /// A cap that is not a usable positive number is treated as unlimited rather than as zero: a
    /// misconfigured rail that halts the whole factory is its own kind of outage, and the
    /// configuration is validated separately.
    #[must_use]
    pub fn new(cap_usd: f64, window: Duration) -> Self {
        let cap_usd = (cap_usd.is_finite() && cap_usd > 0.0).then_some(cap_usd);
        Self {
            cap_usd,
            window,
            entries: VecDeque::new(),
        }
    }

    /// Creates a Reserve that never refuses anything.
    #[must_use]
    pub fn unlimited() -> Self {
        Self {
            cap_usd: None,
            window: Duration::from_secs(24 * 60 * 60),
            entries: VecDeque::new(),
        }
    }

    /// Returns `true` when no ceiling is enforced.
    #[must_use]
    pub fn is_unlimited(&self) -> bool {
        self.cap_usd.is_none()
    }

    /// Records spend against the Reserve.
    ///
    /// Non-finite and negative figures are ignored, as they are for Fuel: the number comes from
    /// parsing a child process's output, and one `NaN` would make the rail unenforceable forever.
    pub fn record(&mut self, at: Timestamp, usd: f64) {
        if usd.is_finite() && usd > 0.0 {
            self.entries.push_back((at, usd));
        }
    }

    /// Spend within the window ending at `now`, discarding anything older.
    pub fn spent(&mut self, now: Timestamp) -> f64 {
        self.expire(now);
        self.entries.iter().map(|(_, usd)| usd).sum()
    }

    /// What is left before the Reserve refuses new work.
    pub fn remaining(&mut self, now: Timestamp) -> Option<f64> {
        let cap = self.cap_usd?;
        Some((cap - self.spent(now)).max(0.0))
    }

    /// Returns `true` once the window's spend has reached the cap.
    pub fn is_exhausted(&mut self, now: Timestamp) -> bool {
        match self.cap_usd {
            None => false,
            Some(cap) => self.spent(now) >= cap,
        }
    }

    /// Decides whether the factory may start more work.
    ///
    /// # Errors
    ///
    /// Returns [`Denial::ReserveExhausted`] once the window's spend has reached the cap. This is
    /// checked *before* an itinerary is minted, because refusing to start is cheap and stopping
    /// half way through leaves work stranded.
    pub fn authorize(&mut self, now: Timestamp) -> Result<(), Denial> {
        if self.is_exhausted(now) {
            return Err(Denial::ReserveExhausted);
        }
        Ok(())
    }

    /// Returns a snapshot for display.
    pub fn state(&mut self, now: Timestamp) -> ReserveState {
        ReserveState {
            cap_usd: self.cap_usd,
            spent_usd: self.spent(now),
            remaining_usd: self.remaining(now),
            window: self.window,
            exhausted: self.is_exhausted(now),
        }
    }

    /// Drops entries that have fallen out of the window.
    ///
    /// A window that cannot be represented as an instant offset leaves the queue alone: expiring
    /// nothing overstates spend, which refuses work that might have been affordable. Expiring
    /// everything would understate it, and a spend rail must fail towards not spending.
    fn expire(&mut self, now: Timestamp) {
        let Ok(window) = SignedDuration::try_from(self.window) else {
            return;
        };
        let Ok(cutoff) = now.checked_sub(window) else {
            return;
        };
        while let Some((at, _)) = self.entries.front() {
            if *at < cutoff {
                self.entries.pop_front();
            } else {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUR: Duration = Duration::from_secs(3_600);
    const DAY: Duration = Duration::from_secs(24 * 60 * 60);

    fn reserve() -> Reserve {
        Reserve::new(50.0, DAY)
    }

    #[test]
    fn spend_accumulates_across_itineraries() {
        // The whole point: each of these is a separate chain, each within its own Fuel budget.
        let now = Timestamp::now();
        let mut reserve = reserve();

        for _ in 0..3 {
            reserve.record(now, 10.0);
        }

        assert!((reserve.spent(now) - 30.0).abs() < 1e-9);
        assert!((reserve.remaining(now).expect("capped") - 20.0).abs() < 1e-9);
        assert!(!reserve.is_exhausted(now));
    }

    #[test]
    fn work_is_refused_once_the_cap_is_reached() {
        let now = Timestamp::now();
        let mut reserve = reserve();
        reserve.record(now, 50.0);

        assert!(reserve.is_exhausted(now));
        assert_eq!(reserve.authorize(now), Err(Denial::ReserveExhausted));
        assert!((reserve.remaining(now).expect("capped") - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn spend_falls_out_of_the_window_as_it_ages() {
        let now = Timestamp::now();
        let mut reserve = reserve();
        reserve.record(now - DAY - HOUR, 40.0);
        reserve.record(now, 10.0);

        assert!(
            (reserve.spent(now) - 10.0).abs() < 1e-9,
            "yesterday's spend must not count against today"
        );
    }

    #[test]
    fn a_rolling_window_cannot_be_doubled_across_a_boundary() {
        // The failure a calendar-day cap has: spend it all just before midnight, then again just
        // after, and a "$50 a day" limit has permitted $100 in two minutes.
        let now = Timestamp::now();
        let mut reserve = reserve();

        reserve.record(now - Duration::from_secs(60), 50.0);

        assert!(
            reserve.is_exhausted(now),
            "a rolling window has no boundary to reset across"
        );
        assert!(
            reserve.is_exhausted(now + Duration::from_secs(60)),
            "and one minute later it is still exhausted"
        );
        assert!(
            !reserve.is_exhausted(now + DAY + HOUR),
            "only the passage of the whole window releases it"
        );
    }

    #[test]
    fn an_unlimited_reserve_never_refuses() {
        let now = Timestamp::now();
        let mut reserve = Reserve::unlimited();
        reserve.record(now, 1_000_000.0);

        assert!(reserve.is_unlimited());
        assert!(!reserve.is_exhausted(now));
        assert_eq!(reserve.remaining(now), None);
        assert_eq!(reserve.authorize(now), Ok(()));
    }

    #[test]
    fn an_unusable_cap_is_unlimited_rather_than_zero() {
        // A cap of zero or NaN would halt the entire factory on its first run. Configuration is
        // validated separately; the runtime refuses to turn a typo into an outage.
        for bad in [0.0, -5.0, f64::NAN, f64::INFINITY] {
            assert!(
                Reserve::new(bad, DAY).is_unlimited(),
                "{bad} must not become a hard stop"
            );
        }
    }

    #[test]
    fn implausible_spend_reports_are_ignored() {
        let now = Timestamp::now();
        let mut reserve = reserve();

        reserve.record(now, f64::NAN);
        reserve.record(now, f64::INFINITY);
        reserve.record(now, -100.0);

        assert!((reserve.spent(now) - 0.0).abs() < f64::EPSILON);
        assert!(
            !reserve.is_exhausted(now),
            "a NaN must not disable the rail"
        );
    }

    #[test]
    fn the_state_snapshot_reports_the_whole_picture() {
        let now = Timestamp::now();
        let mut reserve = reserve();
        reserve.record(now, 20.0);

        let state = reserve.state(now);
        assert_eq!(state.cap_usd, Some(50.0));
        assert!((state.spent_usd - 20.0).abs() < 1e-9);
        assert!((state.remaining_usd.expect("capped") - 30.0).abs() < 1e-9);
        assert_eq!(state.window, DAY);
        assert!(!state.exhausted);
    }
}
