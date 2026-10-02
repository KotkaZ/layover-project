//! What a run actually cost, and where that figure came from.
//!
//! Fuel answers "may this itinerary keep going?". This module answers the questions an operator
//! asks afterwards: *where did the money go, and do I believe the number?*
//!
//! # Why provenance is a first-class field
//!
//! A cost figure can come from four places, and they are not interchangeable:
//!
//! - the runner reported it in dollars, which is the figure most worth trusting;
//! - the runner reported how many Copilot AI credits it used, and Layover priced them at the
//!   published rate of a credit — a measurement of use and one known price;
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
pub mod reporting;
pub mod reserve;
pub mod window;

use jiff::Timestamp;

use serde::{Deserialize, Serialize};

use crate::agent::AgentName;
use crate::flight::{ItineraryId, RunId};

pub use ledger::{Ledger, Summary};
pub use rates::{ModelRates, RateCard};
pub use reserve::{Reserve, ReserveState};
pub use window::{RETENTION_DAYS, Span, Window};

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
    /// The runner reported the figure in dollars.
    Reported,
    /// The runner reported the Copilot AI credits it used, priced at the rate of one credit.
    ///
    /// Measured: the count is Copilot's own, and the price of a credit is published. Named apart
    /// from [`Self::Reported`] because the dollars are Layover's arithmetic, and a rate that turned
    /// out wrong has to be findable.
    CopilotCredits,
    /// Derived from token counts and a [`RateCard`]. An estimate, and labelled as one.
    RateCard,
    /// The runner reported neither cost nor tokens. The figure is zero and means nothing.
    Unreported,
}

impl CostSource {
    /// Returns `true` when the figure is a measurement rather than a guess or a hole — the kinds
    /// that debit Fuel and draw on the Reserve.
    #[must_use]
    pub fn is_measured(&self) -> bool {
        matches!(self, Self::Reported | Self::CopilotCredits)
    }
}

/// What one Copilot AI credit costs, in US dollars, when a factory does not say otherwise.
///
/// GitHub's "Models and pricing for GitHub Copilot": "1 AI credit = $0.01 USD". Copilot CLI counts
/// usage in AI units, which its text output labels "AI Credits", and this takes one to be the
/// other. A factory overrides it with `[copilot] usd_per_credit`.
pub const USD_PER_COPILOT_CREDIT: f64 = 0.01;

/// Copilot CLI counts AI units in billionths.
const NANO_PER_CREDIT: f64 = 1_000_000_000.0;

/// Prices a count of Copilot nano-AI-units at `usd_per_credit` dollars a credit.
#[must_use]
pub fn credits_to_usd(nano_aiu: u64, usd_per_credit: f64) -> f64 {
    // A run uses at most thousands of credits, trillions of nano-units: far below the 2^53 an f64
    // represents exactly.
    #[expect(
        clippy::cast_precision_loss,
        reason = "nano-AIU counts never approach 2^53"
    )]
    let nano = nano_aiu as f64;
    nano / NANO_PER_CREDIT * usd_per_credit
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
    /// The pipeline whose trigger began this chain, when one did.
    ///
    /// Carried here rather than only on the run record so that spend can be attributed to the
    /// *workflow* that caused it. "What does the nightly sweep cost me" is the first question a
    /// factory with several pipelines raises, and per-agent totals cannot answer it when an agent
    /// belongs to more than one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pipeline: Option<crate::pipeline::PipelineName>,
    /// Which model, when the runner said.
    pub model: Option<String>,
    /// Tokens consumed, as far as they are known.
    pub usage: TokenUsage,
    /// Cost in US dollars. Always zero when `source` is [`CostSource::Unreported`].
    pub usd: f64,
    /// Where `usd` came from.
    pub source: CostSource,
    /// When the run finished.
    pub at: Timestamp,
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
        // A zero alongside real token counts is not a measurement, it is a runner that did not
        // fill the field in. Believing it is how every economic rail is defeated at once: Fuel
        // debits nothing, the Reserve records nothing, and the run cap — the deterministic
        // fallback for exactly this case — never engages, because the cost *was* reported.
        //
        // Tokens are kept so a rate card can price it later. Zero cost with zero tokens is left
        // alone: a run that genuinely did nothing is a real measurement, and the distinction
        // between that and silence is the whole reason this field exists.
        let implausible = usd == 0.0 && !usage.is_empty();

        let (usd, source) = if usd.is_finite() && usd >= 0.0 && !implausible {
            (usd, CostSource::Reported)
        } else {
            (0.0, CostSource::Unreported)
        };

        Self {
            run,
            itinerary,
            agent,
            pipeline: None,
            model,
            usage,
            usd,
            source,
            at: Timestamp::now(),
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
            pipeline: None,
            model,
            usage: TokenUsage::default(),
            usd: 0.0,
            source: CostSource::Unreported,
            at: Timestamp::now(),
        }
    }

    /// Attributes the cost to the workflow that caused it.
    #[must_use]
    pub fn from_pipeline(mut self, pipeline: crate::pipeline::PipelineName) -> Self {
        self.pipeline = Some(pipeline);
        self
    }

    /// Overrides when the run finished, for tests and for replaying a persisted ledger.
    #[must_use]
    pub fn at(mut self, at: Timestamp) -> Self {
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
    fn a_genuine_zero_is_still_a_report() {
        // A run that truly cost nothing is different from a runner that said nothing, and the
        // difference decides whether the budget rail is working. With no tokens either, zero is
        // a real measurement.
        let cost = RunCost::reported(
            RunId::generate(),
            ItineraryId::generate(),
            "analyst".into(),
            None,
            TokenUsage::default(),
            0.0,
        );

        assert_eq!(cost.source, CostSource::Reported);
    }

    #[test]
    fn zero_dollars_alongside_real_tokens_is_silence_rather_than_a_measurement() {
        // The cheapest way to defeat every economic rail at once. Believing a zero report means
        // Fuel debits nothing, the Reserve records nothing, and the run cap — the deterministic
        // fallback for exactly this case — never engages, because as far as the accounting is
        // concerned the cost *was* reported.
        let cost = run_cost(0.0);

        assert_eq!(cost.source, CostSource::Unreported);
        assert!(
            !cost.usage.is_empty(),
            "the tokens are kept so a rate card can price it"
        );
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
        assert!(CostSource::Reported < CostSource::CopilotCredits);
        assert!(CostSource::CopilotCredits < CostSource::RateCard);
        assert!(CostSource::RateCard < CostSource::Unreported);
    }

    #[test]
    fn copilot_credits_are_measured_and_named_apart() {
        // The runner measured the credits; Layover supplied only the published price of one. That
        // is a measurement, and it is also worth being able to tell apart from dollars a runner
        // printed itself.
        assert!(CostSource::CopilotCredits.is_measured());
        assert!(!CostSource::RateCard.is_measured());
        assert_eq!(
            serde_json::to_string(&CostSource::CopilotCredits).expect("serialises"),
            r#""copilot_credits""#
        );
    }

    #[test]
    fn nano_ai_units_become_dollars_at_the_credit_rate() {
        // 1,510.58 credits at a cent each: a real 47-minute Opus 5.5 run.
        let usd = credits_to_usd(1_510_581_560_000, USD_PER_COPILOT_CREDIT);
        assert!((usd - 15.105_815_6).abs() < 1e-9, "{usd}");

        let doubled = credits_to_usd(1_510_581_560_000, 0.02);
        assert!((doubled - 30.211_631_2).abs() < 1e-9, "{doubled}");
    }
}
