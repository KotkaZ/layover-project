//! Turning token counts into dollars, when the runner will not.
//!
//! This is a fallback, never a substitute. Everything it produces is labelled
//! [`CostSource::RateCard`] so it can never be mistaken for a measured figure — see the module
//! documentation on [`crate::cost`] for why that distinction is load-bearing.

use std::collections::BTreeMap;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use super::{CostSource, RunCost, TokenUsage};
use crate::agent::AgentName;
use crate::flight::{ItineraryId, RunId};

/// What one model costs, in US dollars per million tokens.
///
/// Four separate rates rather than one, because providers price cached tokens far below fresh
/// input — often ten to one — and a single blended rate is wrong by whatever the cache hit rate
/// happens to be that day.
#[derive(Debug, Clone, Copy, Default, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelRates {
    /// Dollars per million prompt tokens.
    #[serde(default)]
    pub input_usd: f64,
    /// Dollars per million generated tokens.
    #[serde(default)]
    pub output_usd: f64,
    /// Dollars per million tokens served from the prompt cache.
    #[serde(default)]
    pub cache_read_usd: f64,
    /// Dollars per million tokens written to the prompt cache.
    #[serde(default)]
    pub cache_write_usd: f64,
}

impl ModelRates {
    /// Returns the cost of `usage`, or `None` if any rate is not a usable number.
    ///
    /// A negative or non-finite rate is a configuration mistake, and guessing past it would
    /// produce a total that looks authoritative and is not.
    #[must_use]
    pub fn cost_of(&self, usage: TokenUsage) -> Option<f64> {
        let rates = [
            (self.input_usd, usage.input),
            (self.output_usd, usage.output),
            (self.cache_read_usd, usage.cache_read),
            (self.cache_write_usd, usage.cache_write),
        ];

        let mut total = 0.0;
        for (rate, tokens) in rates {
            if !rate.is_finite() || rate < 0.0 {
                return None;
            }
            #[allow(clippy::cast_precision_loss)]
            let tokens = tokens as f64;
            total += rate * tokens / 1_000_000.0;
        }

        total.is_finite().then_some(total)
    }
}

/// Published prices, keyed by model identifier.
///
/// Deliberately not bundled with Layover. Prices change, they differ per provider and per context
/// tier, and a stale table baked into a release is exactly how a cost estimate drifts by a factor
/// of two without anyone noticing. Whoever runs the factory owns this.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(transparent)]
pub struct RateCard {
    models: BTreeMap<String, ModelRates>,
}

impl RateCard {
    /// Creates an empty rate card.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds rates for a model.
    #[must_use]
    pub fn with(mut self, model: impl Into<String>, rates: ModelRates) -> Self {
        self.models.insert(model.into(), rates);
        self
    }

    /// Returns the rates for `model`, if any are published.
    #[must_use]
    pub fn rates_for(&self, model: &str) -> Option<&ModelRates> {
        self.models.get(model)
    }

    /// Returns `true` when no rates are published at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.models.is_empty()
    }

    /// Estimates what `usage` cost on `model`.
    ///
    /// Returns `None` when the model is unknown or its rates are unusable, which the caller must
    /// treat as [`CostSource::Unreported`] rather than as zero.
    #[must_use]
    pub fn estimate(&self, model: &str, usage: TokenUsage) -> Option<f64> {
        self.rates_for(model)?.cost_of(usage)
    }

    /// Builds a [`RunCost`] for a run that reported tokens but no dollars.
    ///
    /// Falls back to [`CostSource::Unreported`] — not to a zero that would flatter the totals —
    /// when there is no model, no published rate, or no usage to price.
    #[must_use]
    pub fn price(
        &self,
        run: RunId,
        itinerary: ItineraryId,
        agent: AgentName,
        model: Option<String>,
        usage: TokenUsage,
    ) -> RunCost {
        let estimate = model
            .as_deref()
            .filter(|_| !usage.is_empty())
            .and_then(|model| self.estimate(model, usage));

        let (usd, source) = match estimate {
            Some(usd) => (usd, CostSource::RateCard),
            None => (0.0, CostSource::Unreported),
        };

        RunCost {
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
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Anthropic's published Claude Opus pricing, used here as a realistic shape.
    fn opus() -> ModelRates {
        ModelRates {
            input_usd: 5.0,
            output_usd: 25.0,
            cache_read_usd: 0.5,
            cache_write_usd: 6.25,
        }
    }

    fn card() -> RateCard {
        RateCard::new().with("claude-opus-5", opus())
    }

    fn usage() -> TokenUsage {
        TokenUsage {
            input: 1_000_000,
            output: 1_000_000,
            cache_read: 1_000_000,
            cache_write: 1_000_000,
        }
    }

    fn priced(card: &RateCard, model: Option<&str>, usage: TokenUsage) -> RunCost {
        card.price(
            RunId::generate(),
            ItineraryId::generate(),
            "analyst".into(),
            model.map(ToOwned::to_owned),
            usage,
        )
    }

    #[test]
    fn a_million_of_each_costs_the_sum_of_the_rates() {
        let cost = card().estimate("claude-opus-5", usage()).expect("priced");

        assert!((cost - 36.75).abs() < 1e-9, "got {cost}");
    }

    #[test]
    fn cached_tokens_are_priced_apart_from_fresh_input() {
        // Blending them would be wrong by whatever the cache hit rate happens to be.
        let fresh = card()
            .estimate(
                "claude-opus-5",
                TokenUsage {
                    input: 1_000_000,
                    ..TokenUsage::default()
                },
            )
            .expect("priced");
        let cached = card()
            .estimate(
                "claude-opus-5",
                TokenUsage {
                    cache_read: 1_000_000,
                    ..TokenUsage::default()
                },
            )
            .expect("priced");

        assert!((fresh - 5.0).abs() < 1e-9);
        assert!((cached - 0.5).abs() < 1e-9);
        assert!(cached < fresh);
    }

    #[test]
    fn an_estimate_is_labelled_as_an_estimate() {
        let cost = priced(&card(), Some("claude-opus-5"), usage());

        assert_eq!(cost.source, CostSource::RateCard);
        assert!(
            !cost.source.is_measured(),
            "an estimate must never count as measured"
        );
    }

    #[test]
    fn an_unknown_model_is_unreported_rather_than_free() {
        // Returning zero would quietly shrink the bill and make the rail look healthy.
        let cost = priced(&card(), Some("some-new-model"), usage());

        assert_eq!(cost.source, CostSource::Unreported);
        assert!((cost.usd - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn a_run_with_no_model_cannot_be_priced() {
        assert_eq!(
            priced(&card(), None, usage()).source,
            CostSource::Unreported
        );
    }

    #[test]
    fn a_run_with_no_tokens_cannot_be_priced() {
        assert_eq!(
            priced(&card(), Some("claude-opus-5"), TokenUsage::default()).source,
            CostSource::Unreported
        );
    }

    #[test]
    fn a_nonsense_rate_is_refused_rather_than_propagated() {
        for bad in [f64::NAN, f64::INFINITY, -1.0] {
            let card = RateCard::new().with(
                "broken",
                ModelRates {
                    output_usd: bad,
                    ..ModelRates::default()
                },
            );

            assert_eq!(
                card.estimate("broken", usage()),
                None,
                "{bad} must not produce a price"
            );
        }
    }

    #[test]
    fn rates_parse_from_configuration() {
        let card: RateCard = toml::from_str(
            r"
            [claude-opus-5]
            input_usd = 5.0
            output_usd = 25.0
            cache_read_usd = 0.5
            cache_write_usd = 6.25
            ",
        )
        .expect("a rate card parses");

        assert!(!card.is_empty());
        assert_eq!(card.rates_for("claude-opus-5"), Some(&opus()));
    }

    #[test]
    fn an_unknown_rate_field_is_rejected() {
        // A typo such as `output_used` would otherwise silently price output at zero.
        let error = toml::from_str::<RateCard>(
            r"
            [claude-opus-5]
            output_used = 25.0
            ",
        );

        assert!(error.is_err());
    }
}
