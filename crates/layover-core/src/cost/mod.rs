//! What a run actually cost, and where that figure came from.
//!
//! Fuel answers "may this itinerary keep going?". This module answers the questions an operator
//! asks afterwards: *where did the money go, and do I believe the number?*
//!
//! # Why provenance is a first-class field
//!
//! A cost figure can come from three places, and they are not interchangeable:
//!
//! - the runner reported it, which is the only figure worth trusting;
//! - Layover derived it from token counts and a rate card, which is an estimate;
//! - nothing was reported at all, which is a hole.
//!
//! Collapsing those into one number is how budgets quietly become fiction. A sibling project that
//! priced runs from a hand-maintained rate card ran **2.7× over actual** — it billed one model at
//! `$75` per million output tokens where the provider charged `$25` — and nothing in the totals
//! said "this is a guess". [`CostSource`] exists so that can never be invisible here: totals are
//! reported separately by source, and [`ledger::Summary`] will tell you what share of a bill is
//! actually measured.

pub mod ledger;
pub mod rates;
pub mod reserve;

use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::agent::AgentName;
use crate::flight::{ItineraryId, RunId};

pub use ledger::{Ledger, Summary};
pub use rates::{ModelRates, RateCard};
pub use reserve::{Reserve, ReserveState};

/// Tokens consumed by one run.
///
/// Cache reads and writes are tracked apart from ordinary input because providers price them
/// differently — often by an order of magnitude — so folding them together would make any derived
/// cost wrong in a way that looks plausible.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct TokenUsage {
    /// Prompt tokens billed at the input rate.
    pub input: u64,
    /// Generated tokens.
    pub output: u64,
    /// Tokens served from the provider's prompt cache.
    pub cache_read: u64,
    /// Tokens written into the provider's prompt cache.
    pub cache_write: u64,
}

impl TokenUsage {
    /// Total tokens, however they were billed.
    #[must_use]
    pub fn total(&self) -> u64 {
        self.input
            .saturating_add(self.output)
            .saturating_add(self.cache_read)
            .saturating_add(self.cache_write)
    }

    /// Returns `true` when nothing was recorded.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.total() == 0
    }

    /// Adds another run's usage to this one, saturating rather than wrapping.
    #[must_use]
    pub fn saturating_add(self, other: Self) -> Self {
        Self {
            input: self.input.saturating_add(other.input),
            output: self.output.saturating_add(other.output),
            cache_read: self.cache_read.saturating_add(other.cache_read),
            cache_write: self.cache_write.saturating_add(other.cache_write),
        }
    }
}

/// Where a cost figure came from.
///
/// Ordered by how much it can be trusted, so `max` over a set of sources gives the weakest link.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CostSource {
    /// The runner reported the figure. The only kind worth billing against.
    Reported,
    /// Derived from token counts and a [`RateCard`]. An estimate, and labelled as one.
    RateCard,
    /// The runner reported neither cost nor tokens. The figure is zero and means nothing.
    Unreported,
}

impl CostSource {
    /// Returns `true` when the figure came from the runner itself.
    #[must_use]
    pub fn is_measured(&self) -> bool {
        matches!(self, Self::Reported)
    }
}

/// What one run cost, and how well that is known.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct RunCost {
    /// The run this describes.
    pub run: RunId,
    /// The chain it belonged to.
    pub itinerary: ItineraryId,
    /// Which agent was run.
    pub agent: AgentName,
    /// Which model, when the runner said.
    pub model: Option<String>,
    /// Tokens consumed, as far as they are known.
    pub usage: TokenUsage,
    /// Cost in US dollars. Always zero when `source` is [`CostSource::Unreported`].
    pub usd: f64,
    /// Where `usd` came from.
    pub source: CostSource,
    /// When the run finished.
    pub at: SystemTime,
}

impl RunCost {
    /// Records a cost the runner reported directly.
    ///
    /// A non-finite or negative figure is treated as no report at all rather than trusted: it
    /// comes from parsing a child process's output, and a `NaN` in a budget poisons every total
    /// downstream of it.
    #[must_use]
    pub fn reported(
        run: RunId,
        itinerary: ItineraryId,
        agent: AgentName,
        model: Option<String>,
        usage: TokenUsage,
        usd: f64,
    ) -> Self {
        let (usd, source) = if usd.is_finite() && usd >= 0.0 {
            (usd, CostSource::Reported)
        } else {
            (0.0, CostSource::Unreported)
        };

        Self {
            run,
            itinerary,
            agent,
            model,
            usage,
            usd,
            source,
            at: SystemTime::now(),
        }
    }

    /// Records a run whose runner said nothing about cost.
    #[must_use]
    pub fn unreported(
        run: RunId,
        itinerary: ItineraryId,
        agent: AgentName,
        model: Option<String>,
    ) -> Self {
        Self {
            run,
            itinerary,
            agent,
            model,
            usage: TokenUsage::default(),
            usd: 0.0,
            source: CostSource::Unreported,
            at: SystemTime::now(),
        }
    }

    /// Overrides when the run finished, for tests and for replaying a persisted ledger.
    #[must_use]
    pub fn at(mut self, at: SystemTime) -> Self {
        self.at = at;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usage() -> TokenUsage {
        TokenUsage {
            input: 1_000,
            output: 500,
            cache_read: 250,
            cache_write: 100,
        }
    }

    fn run_cost(usd: f64) -> RunCost {
        RunCost::reported(
            RunId::generate(),
            ItineraryId::generate(),
            "analyst".into(),
            Some("claude-opus-5".to_owned()),
            usage(),
            usd,
        )
    }

    #[test]
    fn token_totals_cover_every_billed_kind() {
        assert_eq!(usage().total(), 1_850);
        assert!(!usage().is_empty());
        assert!(TokenUsage::default().is_empty());
    }

    #[test]
    fn token_usage_adds_without_wrapping() {
        let huge = TokenUsage {
            input: u64::MAX,
            ..TokenUsage::default()
        };

        assert_eq!(huge.saturating_add(usage()).input, u64::MAX);
    }

    #[test]
    fn a_reported_cost_is_marked_as_measured() {
        let cost = run_cost(1.25);

        assert_eq!(cost.source, CostSource::Reported);
        assert!(cost.source.is_measured());
        assert!((cost.usd - 1.25).abs() < 1e-9);
    }

    #[test]
    fn a_zero_report_is_still_a_report() {
        // A run that genuinely cost nothing is different from a runner that said nothing, and the
        // difference decides whether the budget rail is working.
        let cost = run_cost(0.0);

        assert_eq!(cost.source, CostSource::Reported);
    }

    #[test]
    fn an_implausible_report_is_downgraded_rather_than_trusted() {
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1.0] {
            let cost = run_cost(bad);

            assert_eq!(
                cost.source,
                CostSource::Unreported,
                "{bad} must not be treated as a measured cost"
            );
            assert!((cost.usd - 0.0).abs() < f64::EPSILON);
        }
    }

    #[test]
    fn an_unreported_run_carries_no_figures_at_all() {
        let cost = RunCost::unreported(
            RunId::generate(),
            ItineraryId::generate(),
            "analyst".into(),
            None,
        );

        assert_eq!(cost.source, CostSource::Unreported);
        assert!(!cost.source.is_measured());
        assert!(cost.usage.is_empty());
    }

    #[test]
    fn sources_order_from_most_to_least_trustworthy() {
        // `max` over a set of sources therefore yields the weakest link, which is what a summary
        // should report rather than the most flattering one.
        assert!(CostSource::Reported < CostSource::RateCard);
        assert!(CostSource::RateCard < CostSource::Unreported);
    }
}
