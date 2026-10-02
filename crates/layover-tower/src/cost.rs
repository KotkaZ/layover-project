//! Reading what a run cost out of what it printed.
//!
//! # Why this is allowed to fail, and what it does when it does
//!
//! Fuel and the Reserve are debited from a figure the child prints about itself. Layover does not
//! control any of those output formats: they belong to three CLIs that version independently and
//! may change shape without warning.
//!
//! So the design assumption is that parsing *will* break, and the question is what happens when it
//! does. A parser that returned zero on failure would be the worst possible answer — spending
//! would continue, the rails would never trip, and every total would look precise. Instead an
//! unreadable figure is [`CostSource::Unreported`], which already means "this number is a lower
//! bound" everywhere it is displayed, and the run cap — which needs no cooperation from the child —
//! becomes the rail that still holds.
//!
//! # Why a plausible-looking figure can still be rejected
//!
//! A runner reporting a cent for a four-dollar run is believed by arithmetic and wrong in fact,
//! and both money rails then under-count by the same factor. Where token counts are also reported,
//! they are a second opinion: a cost more than an order of magnitude below what those tokens imply
//! is treated as unreported rather than as a measurement.
//!
//! # Copilot CLI, which reports credits rather than dollars
//!
//! Copilot CLI prints no dollars and no token totals. With `--output-format json` it prints
//! `session.usage_checkpoint` events carrying a running total of AI units in billionths —
//! `data.totalNanoAiu` — and the last one seen is the run's usage. It is priced at the factory's
//! `[copilot] usd_per_credit` and recorded as [`CostSource::CopilotCredits`], which debits Fuel and
//! draws on the Reserve like dollars a runner printed itself.
//!
//! The same rules apply as to dollars. A last checkpoint that cannot be read, or reads as negative
//! or as not an integer, makes the run unreported rather than priced from an earlier, smaller
//! total. Zero credits beside premium requests is silence, as zero dollars beside tokens is. A run
//! killed before its first checkpoint has nothing to price and is unreported. The final `result`
//! event's `premiumRequests` is never priced: it is a flat multiplier per prompt that says the same
//! for a six-minute run as for a forty-seven-minute one.

use layover_core::agent::AgentName;
use layover_core::config::Config;
use layover_core::cost::{CostSource, RateCard, TokenUsage, credits_to_usd};
use layover_core::model::ModelChoice;
use serde::Deserialize;
use serde_json::Value;

/// What a run reported about its own cost.
#[derive(Debug, Clone, PartialEq)]
pub struct Reported {
    /// The figure, in dollars.
    pub usd: f64,
    /// Where it came from, and therefore how much it can be trusted.
    pub source: CostSource,
    /// Fresh tokens in, when the runner said.
    pub input_tokens: u64,
    /// Tokens out, when the runner said.
    pub output_tokens: u64,
    /// Input tokens served from the prompt cache, when the runner said.
    pub cache_read_tokens: u64,
}

impl Reported {
    /// A run whose cost could not be read.
    ///
    /// Zero dollars, because nothing may be debited — but explicitly unreported, so no total built
    /// from it ever claims to be measured.
    #[must_use]
    pub const fn unreported() -> Self {
        Self {
            usd: 0.0,
            source: CostSource::Unreported,
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
        }
    }

    /// Silence about dollars, beside the tokens a runner did report.
    const fn tokens_only(input: u64, output: u64, cache_read: u64) -> Self {
        Self {
            usd: 0.0,
            source: CostSource::Unreported,
            input_tokens: input,
            output_tokens: output,
            cache_read_tokens: cache_read,
        }
    }
}

/// What prices a run beside what it printed: the factory's price of a Copilot credit, and its rate
/// card for the model the run's command line selects.
#[derive(Debug, Clone, Copy)]
pub struct Prices<'a> {
    /// Dollars per Copilot AI credit, from `[copilot] usd_per_credit`.
    pub usd_per_credit: f64,
    /// The factory's `[rates]`.
    pub rates: &'a RateCard,
    /// The model the run ran on, as its command line says.
    pub model: Option<&'a str>,
}

/// The shapes the supported CLIs emit.
///
/// Deliberately permissive about field names and strict about types. Every one of these is a
/// different CLI's idea of the same fact, and none of them promised to keep it.
#[derive(Debug, Deserialize)]
struct Line {
    #[serde(alias = "total_cost_usd", alias = "cost_usd", alias = "costUSD")]
    cost: Option<f64>,
    usage: Option<Usage>,
}

#[derive(Debug, Deserialize)]
struct Usage {
    #[serde(alias = "input_tokens", alias = "prompt_tokens")]
    input: Option<u64>,
    #[serde(alias = "output_tokens", alias = "completion_tokens")]
    output: Option<u64>,
    /// Codex's count of input tokens served from the cache, which are *part of* its input count —
    /// unlike Claude's `cache_read_input_tokens`, which are not and are deliberately not read.
    cached_input_tokens: Option<u64>,
}

/// Roughly what a dollar buys, used only to sanity-check a reported figure.
///
/// Not a rate card and not used to price anything — a single order-of-magnitude yardstick for
/// deciding whether a claimed cost is in the right universe. Deliberately generous: the job is to
/// catch a runner claiming a cent for a four-dollar run, not to second-guess pricing.
const TOKENS_PER_DOLLAR: f64 = 1_000_000.0;

/// Reads the last cost a transcript reports.
///
/// The last, not the first: agent CLIs emit running totals, and the final one is the total. Lines
/// that are not JSON are skipped, because a transcript is mostly the agent talking.
///
/// Dollars a runner printed win over anything derived. Failing those, the last Copilot usage
/// checkpoint is priced at `usd_per_credit` dollars an AI credit. No rate card is consulted; see
/// [`priced`] for that.
#[must_use]
pub fn from_transcript(text: &str, usd_per_credit: f64) -> Reported {
    priced(
        text,
        &Prices {
            usd_per_credit,
            rates: &RateCard::new(),
            model: None,
        },
    )
}

/// Reads what a run cost, as [`from_transcript`] does, and estimates from the factory's rate card
/// what a run cost that printed token counts and no dollars at all.
///
/// Only silence is estimated. A figure the runner printed and that was not believed stays
/// unreported: replacing it with an estimate would invent a different number with no better claim
/// to the truth. And an estimate is [`CostSource::RateCard`], which is not a measurement — it is
/// shown, but it never debits Fuel and never draws on the Reserve.
#[must_use]
pub fn priced(text: &str, prices: &Prices<'_>) -> Reported {
    let mut best: Option<Reported> = None;
    let mut checkpoint: Option<Checkpoint> = None;
    // Whether the runner printed a dollar figure at all, believed or not.
    let mut printed = false;

    for line in text.lines() {
        let line = line.trim();
        if !line.starts_with('{') {
            continue;
        }

        if let Some(seen) = Checkpoint::of(line) {
            checkpoint = Some(seen);
            continue;
        }

        let Ok(parsed) = serde_json::from_str::<Line>(line) else {
            continue;
        };

        let usage = parsed.usage.as_ref();
        let cached = usage.and_then(|u| u.cached_input_tokens).unwrap_or(0);
        let input = usage
            .and_then(|u| u.input)
            .unwrap_or(0)
            .saturating_sub(cached);
        let output = usage.and_then(|u| u.output).unwrap_or(0);

        let Some(usd) = parsed.cost else {
            // Tokens without a cost is still worth keeping: it is what makes a later cost
            // checkable, what a rate card can price, and on its own it says a run did work.
            if input + output + cached > 0 {
                best = Some(Reported::tokens_only(input, output, cached));
            }
            continue;
        };

        printed = true;
        best = Some(Reported {
            cache_read_tokens: cached,
            ..judge(usd, input, output)
        });
    }

    match (best, checkpoint) {
        (Some(dollars), _) if dollars.source == CostSource::Reported => dollars,
        (_, Some(checkpoint)) => checkpoint.price(prices.usd_per_credit),
        (Some(tokens), None) if !printed => estimate(tokens, prices),
        (best, None) => best.unwrap_or_else(Reported::unreported),
    }
}

/// What a run of `agent` cost, read from its transcript and priced as `config` says, and the model
/// to record it under.
///
/// The rate card is consulted for the model the agent's command line selects — `--model`, or the
/// declared model its runner's `{model}` carries — because that is what the run ran on. A model the
/// agent declares and its runner never passes is not priced; it is still what the record names,
/// as it always was, so history reads the same as before.
#[must_use]
pub fn of_run(config: &Config, agent: &AgentName, transcript: &str) -> (Reported, Option<String>) {
    let ran_on = ModelChoice::of(config, agent).model;
    let reported = priced(
        transcript,
        &Prices {
            usd_per_credit: config.copilot.usd_per_credit,
            rates: &config.rates,
            model: ran_on.as_deref(),
        },
    );
    let named = ran_on.or_else(|| {
        config
            .agents
            .get(agent)
            .and_then(|definition| definition.model.clone())
    });
    (reported, named)
}

/// What the rate card says tokens a run reported cost, or the run unchanged — unreported — when
/// there is no model to price or no rates published for it.
fn estimate(tokens: Reported, prices: &Prices<'_>) -> Reported {
    let usage = TokenUsage {
        input: tokens.input_tokens,
        output: tokens.output_tokens,
        cache_read: tokens.cache_read_tokens,
        cache_write: 0,
    };
    match prices
        .model
        .and_then(|model| prices.rates.estimate(model, usage))
    {
        Some(usd) => Reported {
            usd,
            source: CostSource::RateCard,
            ..tokens
        },
        None => tokens,
    }
}

/// The last thing a Copilot usage checkpoint said.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Checkpoint {
    /// A total that reads as one.
    Read {
        /// AI units used so far, in billionths of a credit.
        nano_aiu: u64,
        /// Premium requests made so far. Never priced; only a witness that work was done.
        premium_requests: u64,
    },
    /// A checkpoint whose total cannot be believed.
    Unreadable,
}

impl Checkpoint {
    /// The checkpoint `line` is, or `None` when it is some other event.
    ///
    /// A line that names the event but is not JSON is unreadable rather than skipped: skipping it
    /// would price the run from an earlier, smaller total and call that measured.
    fn of(line: &str) -> Option<Self> {
        if !line.contains("session.usage_checkpoint") {
            return None;
        }
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            return Some(Self::Unreadable);
        };
        if value.get("type").and_then(Value::as_str) != Some("session.usage_checkpoint") {
            // An agent talking about the event, not the event.
            return None;
        }

        let data = value.get("data");
        let count = |key: &str| data.and_then(|data| data.get(key)).and_then(Value::as_u64);
        Some(match count("totalNanoAiu") {
            Some(nano_aiu) => Self::Read {
                nano_aiu,
                premium_requests: count("totalPremiumRequests").unwrap_or(0),
            },
            None => Self::Unreadable,
        })
    }

    /// What the run cost, at `usd_per_credit` dollars an AI credit.
    fn price(self, usd_per_credit: f64) -> Reported {
        let Self::Read {
            nano_aiu,
            premium_requests,
        } = self
        else {
            return Reported::unreported();
        };

        // Requests made and nothing counted is silence, not a free run.
        if nano_aiu == 0 && premium_requests > 0 {
            return Reported::unreported();
        }
        if !(usd_per_credit.is_finite() && usd_per_credit > 0.0) {
            return Reported::unreported();
        }

        let usd = credits_to_usd(nano_aiu, usd_per_credit);
        if !usd.is_finite() || usd < 0.0 {
            return Reported::unreported();
        }
        Reported {
            usd,
            source: CostSource::CopilotCredits,
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
        }
    }
}

/// Decides whether a reported figure can be believed.
fn judge(usd: f64, input: u64, output: u64) -> Reported {
    let tokens = input + output;

    // Negative, NaN and infinite are not measurements. Nothing may be debited from them, and
    // pretending otherwise would let a runner credit Fuel back to itself.
    if !usd.is_finite() || usd < 0.0 {
        return Reported {
            usd: 0.0,
            source: CostSource::Unreported,
            input_tokens: input,
            output_tokens: output,
            cache_read_tokens: 0,
        };
    }

    // Zero dollars alongside real tokens is silence, not a measurement: work was done and the
    // runner did not price it.
    if usd == 0.0 && tokens > 0 {
        return Reported {
            usd: 0.0,
            source: CostSource::Unreported,
            input_tokens: input,
            output_tokens: output,
            cache_read_tokens: 0,
        };
    }

    // A figure an order of magnitude below what the tokens imply is not believable. Treated as
    // unreported rather than corrected: a better guess is still a guess, and saying "this is a
    // lower bound" is the honest answer.
    if tokens > 0 {
        // Token counts are millions at most, far below the 2^53 an f64 represents exactly.
        #[expect(
            clippy::cast_precision_loss,
            reason = "token counts never approach 2^53"
        )]
        let implied = tokens as f64 / TOKENS_PER_DOLLAR;
        if usd > 0.0 && usd * 10.0 < implied {
            return Reported {
                usd: 0.0,
                source: CostSource::Unreported,
                input_tokens: input,
                output_tokens: output,
                cache_read_tokens: 0,
            };
        }
    }

    Reported {
        usd,
        source: CostSource::Reported,
        input_tokens: input,
        output_tokens: output,
        cache_read_tokens: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use layover_core::cost::USD_PER_COPILOT_CREDIT;

    #[test]
    fn a_plain_total_is_read() {
        let got = from_transcript(
            r#"{"total_cost_usd": 1.25, "usage": {"input_tokens": 900000}}"#,
            USD_PER_COPILOT_CREDIT,
        );

        assert!((got.usd - 1.25).abs() < f64::EPSILON);
        assert_eq!(got.source, CostSource::Reported);
        assert_eq!(got.input_tokens, 900_000);
    }

    #[test]
    fn the_last_total_wins_because_runners_emit_running_ones() {
        let text = "\
{\"total_cost_usd\": 0.10, \"usage\": {\"input_tokens\": 100000}}
the agent says something
{\"total_cost_usd\": 0.90, \"usage\": {\"input_tokens\": 800000}}";

        assert!((from_transcript(text, USD_PER_COPILOT_CREDIT).usd - 0.90).abs() < f64::EPSILON);
    }

    #[test]
    fn prose_between_the_json_is_ignored() {
        let text = "Thinking about it.\nI will run the suite.\n{\"cost_usd\": 0.4, \"usage\": {\"input_tokens\": 350000}}\nDone.";
        assert!((from_transcript(text, USD_PER_COPILOT_CREDIT).usd - 0.4).abs() < f64::EPSILON);
    }

    #[test]
    fn a_transcript_with_no_numbers_is_unreported_not_free() {
        // The difference matters: zero-and-measured would let a chain run forever.
        let got = from_transcript(
            "I looked at the file and it seemed fine.",
            USD_PER_COPILOT_CREDIT,
        );

        assert_eq!(got.source, CostSource::Unreported);
        assert!(got.usd.abs() < f64::EPSILON);
    }

    #[test]
    fn zero_dollars_with_real_tokens_is_silence() {
        let got = from_transcript(
            r#"{"total_cost_usd": 0.0, "usage": {"input_tokens": 50000}}"#,
            USD_PER_COPILOT_CREDIT,
        );

        assert_eq!(got.source, CostSource::Unreported);
        assert_eq!(
            got.input_tokens, 50_000,
            "the tokens are still worth keeping"
        );
    }

    #[test]
    fn a_negative_cost_cannot_credit_fuel_back() {
        let got = from_transcript(
            r#"{"total_cost_usd": -5.0, "usage": {"output_tokens": 1000}}"#,
            USD_PER_COPILOT_CREDIT,
        );

        assert!(got.usd.abs() < f64::EPSILON);
        assert_eq!(got.source, CostSource::Unreported);
    }

    #[test]
    fn a_cost_wildly_below_what_the_tokens_imply_is_not_believed() {
        // Risk 18: a runner reporting a cent for a four-dollar run defeats Fuel and the Reserve
        // together, because both read the same figure.
        let got = from_transcript(
            r#"{"total_cost_usd": 0.01, "usage": {"input_tokens": 4000000}}"#,
            USD_PER_COPILOT_CREDIT,
        );

        assert_eq!(
            got.source,
            CostSource::Unreported,
            "four million tokens does not cost a cent"
        );
    }

    #[test]
    fn a_cost_merely_cheaper_than_the_yardstick_is_still_believed() {
        // The check is an order of magnitude, not a price list. A cheap model must not be
        // constantly accused of lying.
        let got = from_transcript(
            r#"{"total_cost_usd": 0.5, "usage": {"input_tokens": 1000000}}"#,
            USD_PER_COPILOT_CREDIT,
        );

        assert_eq!(got.source, CostSource::Reported);
        assert!((got.usd - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn tokens_without_a_cost_still_record_that_work_happened() {
        let got = from_transcript(
            r#"{"usage": {"input_tokens": 1200, "output_tokens": 300}}"#,
            USD_PER_COPILOT_CREDIT,
        );

        assert_eq!(got.source, CostSource::Unreported);
        assert_eq!(got.input_tokens, 1200);
        assert_eq!(got.output_tokens, 300);
    }

    #[test]
    fn malformed_json_does_not_stop_the_readable_lines_being_read() {
        // The assumption is that these formats will break. What must not happen is that one bad
        // line loses a cost the runner did report.
        let text = "{\"total_cost_usd\": oops}\n{\"total_cost_usd\": 2.0, \"usage\": {\"input_tokens\": 1900000}}";
        assert!((from_transcript(text, USD_PER_COPILOT_CREDIT).usd - 2.0).abs() < f64::EPSILON);
    }

    #[test]
    fn an_alternative_field_name_is_accepted() {
        // Three CLIs, three spellings of the same fact.
        for text in [
            r#"{"total_cost_usd": 3.0, "usage": {"input_tokens": 2900000}}"#,
            r#"{"cost_usd": 3.0, "usage": {"prompt_tokens": 2900000}}"#,
            r#"{"costUSD": 3.0, "usage": {"input": 2900000}}"#,
        ] {
            assert!(
                (from_transcript(text, USD_PER_COPILOT_CREDIT).usd - 3.0).abs() < f64::EPSILON,
                "{text}"
            );
        }
    }
}
