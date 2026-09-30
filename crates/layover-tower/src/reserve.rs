//! The Reserve, checked before a run starts.
//!
//! Fuel bounds one chain; the Reserve bounds the factory, over a rolling window, because a
//! scheduled pipeline mints a fresh chain with fresh Fuel on every tick. It is measured from
//! history rather than held in memory: history is where every finished run's cost already lives,
//! it survives a restart, and it is what the dashboard's Reserve card reads — so the rail and the
//! figure an operator sees cannot disagree.
//!
//! Only measured spend counts: dollars a runner printed and Copilot credits priced at the published
//! rate. A run that reported nothing adds nothing, which is why a Reserve over an unreporting
//! runner never binds, and why the run cap exists.
//!
//! What it does not do: account for runs still in flight. Four runs starting at once against the
//! last few dollars all pass, and the factory overshoots by what they cost — a known limit, in
//! `docs/risks.md`.

use std::path::Path;

use jiff::{SignedDuration, Timestamp};
use layover_core::config::Config;
use layover_core::cost::{Span, Window};
use layover_store::History;

/// How the detail of a run the Reserve refused begins, so `layover doctor` can find them.
pub const REFUSED: &str = "refused: the Reserve is exhausted";

/// Why the Reserve refuses to start a run now, or `None` when it does not.
///
/// Fails open when history cannot be read. History is best-effort everywhere else, and a factory
/// that stopped because a log file was momentarily unreadable would have turned an observability
/// problem into an outage.
#[must_use]
pub fn refusal(config: &Config, history: &Path, now: Timestamp) -> Option<String> {
    if config.reserve.is_unlimited() {
        return None;
    }

    let hours = i64::try_from(config.reserve.window_hours).unwrap_or(i64::MAX);
    let window = SignedDuration::from_hours(hours.min(1_000_000));
    let start = now.checked_sub(window).unwrap_or(Timestamp::MIN);
    let span = Span {
        window: Window::AllTime,
        start: Some(start),
        end: now
            .checked_add(SignedDuration::from_secs(1))
            .unwrap_or(Timestamp::MAX),
        reckoned_in: None,
        horizon: start,
    };
    let ledger = History::open(history).ok()?.ledger(&span).ok()?;

    let mut spent: Vec<(Timestamp, f64)> = ledger
        .entries()
        .filter(|cost| cost.source.is_measured() && cost.usd.is_finite() && cost.usd > 0.0)
        .map(|cost| (cost.at, cost.usd))
        .collect();
    spent.sort_by_key(|(at, _)| *at);

    let mut reserve = config.reserve.to_reserve();
    for (at, usd) in &spent {
        reserve.record(*at, *usd);
    }
    reserve.authorize(now).err()?;

    let cap = config.reserve.fuel_usd;
    let total: f64 = spent.iter().map(|(_, usd)| usd).sum();
    Some(format!(
        "{REFUSED} — ${total:.2} of ${cap:.2} spent in the last {}h. {}",
        config.reserve.window_hours,
        match frees_at(&spent, total, cap, window) {
            Some(at) => format!(
                "New work can start again at {}, when enough of it has rolled out of the \
                 window, or sooner if `[reserve] fuel_usd` is raised and the Tower restarted.",
                at.strftime("%Y-%m-%d %H:%M:%S UTC")
            ),
            None => "Raise `[reserve] fuel_usd` and restart the Tower to start new work sooner."
                .to_owned(),
        }
    ))
}

/// When enough spend has left the window for the total to fall below the cap again.
///
/// The oldest charges leave first, so this is the moment the charge that tips the total under the
/// cap turns a window old.
fn frees_at(
    spent: &[(Timestamp, f64)],
    total: f64,
    cap: f64,
    window: SignedDuration,
) -> Option<Timestamp> {
    let mut left = total;
    for (at, usd) in spent {
        left -= usd;
        if left < cap {
            return at.checked_add(window).ok();
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_window_frees_room_when_the_charge_that_tips_it_under_the_cap_ages_out() {
        let now = Timestamp::now();
        let hour = SignedDuration::from_hours(1);
        let spent = [
            (now - hour * 10, 15.0),
            (now - hour * 5, 15.0),
            (now - hour, 15.0),
        ];

        // $45 against a $20 cap: two of the three have to go, the second leaves at +24h from 5h ago.
        let at = frees_at(&spent, 45.0, 20.0, hour * 24).expect("it frees up");
        assert_eq!(at, now - hour * 5 + hour * 24);
    }
}
