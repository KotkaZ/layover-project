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

use layover_core::cost::CostSource;
use serde::Deserialize;

/// What a run reported about its own cost.
#[derive(Debug, Clone, PartialEq)]
pub struct Reported {
    /// The figure, in dollars.
    pub usd: f64,
    /// Where it came from, and therefore how much it can be trusted.
    pub source: CostSource,
    /// Tokens in, when the runner said.
    pub input_tokens: u64,
    /// Tokens out, when the runner said.
    pub output_tokens: u64,
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
        }
    }
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
#[must_use]
pub fn from_transcript(text: &str) -> Reported {
    let mut best: Option<Reported> = None;

    for line in text.lines() {
        let line = line.trim();
        if !line.starts_with('{') {
            continue;
        }

        let Ok(parsed) = serde_json::from_str::<Line>(line) else {
            continue;
        };

        let input = parsed.usage.as_ref().and_then(|u| u.input).unwrap_or(0);
        let output = parsed.usage.as_ref().and_then(|u| u.output).unwrap_or(0);

        let Some(usd) = parsed.cost else {
            // Tokens without a cost is still worth keeping: it is what makes a later cost
            // checkable, and on its own it says a run did work it could not price.
            if input + output > 0 {
                best = Some(Reported {
                    usd: 0.0,
                    source: CostSource::Unreported,
                    input_tokens: input,
                    output_tokens: output,
                });
            }
            continue;
        };

        best = Some(judge(usd, input, output));
    }

    best.unwrap_or_else(Reported::unreported)
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
            };
        }
    }

    Reported {
        usd,
        source: CostSource::Reported,
        input_tokens: input,
        output_tokens: output,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_total_is_read() {
        let got = from_transcript(r#"{"total_cost_usd": 1.25, "usage": {"input_tokens": 900000}}"#);

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

        assert!((from_transcript(text).usd - 0.90).abs() < f64::EPSILON);
    }

    #[test]
    fn prose_between_the_json_is_ignored() {
        let text = "Thinking about it.\nI will run the suite.\n{\"cost_usd\": 0.4, \"usage\": {\"input_tokens\": 350000}}\nDone.";
        assert!((from_transcript(text).usd - 0.4).abs() < f64::EPSILON);
    }

    #[test]
    fn a_transcript_with_no_numbers_is_unreported_not_free() {
        // The difference matters: zero-and-measured would let a chain run forever.
        let got = from_transcript("I looked at the file and it seemed fine.");

        assert_eq!(got.source, CostSource::Unreported);
        assert!(got.usd.abs() < f64::EPSILON);
    }

    #[test]
    fn zero_dollars_with_real_tokens_is_silence() {
        let got = from_transcript(r#"{"total_cost_usd": 0.0, "usage": {"input_tokens": 50000}}"#);

        assert_eq!(got.source, CostSource::Unreported);
        assert_eq!(
            got.input_tokens, 50_000,
            "the tokens are still worth keeping"
        );
    }

    #[test]
    fn a_negative_cost_cannot_credit_fuel_back() {
        let got = from_transcript(r#"{"total_cost_usd": -5.0, "usage": {"output_tokens": 1000}}"#);

        assert!(got.usd.abs() < f64::EPSILON);
        assert_eq!(got.source, CostSource::Unreported);
    }

    #[test]
    fn a_cost_wildly_below_what_the_tokens_imply_is_not_believed() {
        // Risk 18: a runner reporting a cent for a four-dollar run defeats Fuel and the Reserve
        // together, because both read the same figure.
        let got =
            from_transcript(r#"{"total_cost_usd": 0.01, "usage": {"input_tokens": 4000000}}"#);

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
        let got = from_transcript(r#"{"total_cost_usd": 0.5, "usage": {"input_tokens": 1000000}}"#);

        assert_eq!(got.source, CostSource::Reported);
        assert!((got.usd - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn tokens_without_a_cost_still_record_that_work_happened() {
        let got = from_transcript(r#"{"usage": {"input_tokens": 1200, "output_tokens": 300}}"#);

        assert_eq!(got.source, CostSource::Unreported);
        assert_eq!(got.input_tokens, 1200);
        assert_eq!(got.output_tokens, 300);
    }

    #[test]
    fn malformed_json_does_not_stop_the_readable_lines_being_read() {
        // The assumption is that these formats will break. What must not happen is that one bad
        // line loses a cost the runner did report.
        let text = "{\"total_cost_usd\": oops}\n{\"total_cost_usd\": 2.0, \"usage\": {\"input_tokens\": 1900000}}";
        assert!((from_transcript(text).usd - 2.0).abs() < f64::EPSILON);
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
                (from_transcript(text).usd - 3.0).abs() < f64::EPSILON,
                "{text}"
            );
        }
    }
}
