//! Pricing a Copilot CLI run from the AI credits it reports.
//!
//! Copilot CLI prints no dollars and no token totals. It prints `session.usage_checkpoint` events
//! with a running total of AI units, and the last one is the run's usage. The fixture is a real
//! run's event stream trimmed to one of each kind, with its content replaced and its figures kept:
//! a 47-minute Opus 5.5 run, 15 premium requests and 1,510.58 credits.

use layover_core::cost::{CostSource, USD_PER_COPILOT_CREDIT};
use layover_tower::from_transcript;

const RUN: &str = include_str!("fixtures/copilot-run.jsonl");

/// The fixture's checkpoint, which a test replaces to say something else.
const CHECKPOINT: &str = r#"{"type":"session.usage_checkpoint","id":"8f1c2e5a-0000-4000-8000-000000000012","parentId":"8f1c2e5a-0000-4000-8000-000000000011","timestamp":"2026-09-30T08:48:57.402Z","data":{"totalNanoAiu":1510581560000,"totalPremiumRequests":15,"modelCacheState":[],"promptCacheBreakState":[]}}"#;

fn checkpoint(nano: &str, premium: &str) -> String {
    format!(
        r#"{{"type":"session.usage_checkpoint","id":"x","parentId":null,"timestamp":"2026-09-30T08:10:00.000Z","data":{{"totalNanoAiu":{nano},"totalPremiumRequests":{premium},"modelCacheState":[],"promptCacheBreakState":[]}}}}"#
    )
}

/// The run with its checkpoint replaced by `lines`.
fn run_with(lines: &[String]) -> String {
    assert!(
        RUN.contains(CHECKPOINT),
        "the fixture carries its checkpoint"
    );
    RUN.replace(CHECKPOINT, &lines.join("\n"))
}

#[test]
fn a_captured_run_is_priced_from_its_credits() {
    let got = from_transcript(RUN, USD_PER_COPILOT_CREDIT);

    assert_eq!(got.source, CostSource::CopilotCredits);
    assert!((got.usd - 15.105_815_6).abs() < 1e-9, "{}", got.usd);
    assert!(got.source.is_measured());
    assert_eq!((got.input_tokens, got.output_tokens), (0, 0));
}

#[test]
fn a_run_killed_before_its_first_checkpoint_is_unreported_not_free() {
    // The result line's premiumRequests is still there, and must not stand in for a price: it is
    // a flat model multiplier that says 15 for a 47-minute run and for a 6-minute one alike.
    let got = from_transcript(&run_with(&[]), USD_PER_COPILOT_CREDIT);

    assert_eq!(got.source, CostSource::Unreported);
    assert!(got.usd.abs() < f64::EPSILON);
}

#[test]
fn the_last_of_several_checkpoints_is_the_run_total() {
    let text = run_with(&[
        checkpoint("159636740000", "15"),
        checkpoint("819772800000", "15"),
        checkpoint("1510581560000", "15"),
    ]);
    let got = from_transcript(&text, USD_PER_COPILOT_CREDIT);

    assert_eq!(got.source, CostSource::CopilotCredits);
    assert!((got.usd - 15.105_815_6).abs() < 1e-9, "{}", got.usd);
}

#[test]
fn a_checkpoint_that_cannot_be_believed_leaves_the_run_unreported() {
    // Negative, a string, a fraction, missing, a number too large to be one, and a line that is
    // not JSON at all. None is a measurement, and none may become a zero that looks like one.
    for garbage in [
        checkpoint("-1510581560000", "15"),
        checkpoint(r#""1510581560000""#, "15"),
        checkpoint("1.5e12", "15"),
        checkpoint("null", "15"),
        checkpoint("1e999", "15"),
        r#"{"type":"session.usage_checkpoint","data":{"totalPremiumRequests":15}}"#.to_owned(),
        r#"{"type":"session.usage_checkpoint","data":{"totalNanoAiu":151058"#.to_owned(),
    ] {
        let text = run_with(&[checkpoint("159636740000", "15"), garbage.clone()]);
        let got = from_transcript(&text, USD_PER_COPILOT_CREDIT);

        assert_eq!(got.source, CostSource::Unreported, "{garbage}");
        assert!(got.usd.abs() < f64::EPSILON, "{garbage}");
    }
}

#[test]
fn a_readable_checkpoint_after_a_broken_one_is_still_the_total() {
    let text = run_with(&[
        checkpoint(r#""broken""#, "15"),
        checkpoint("1510581560000", "15"),
    ]);

    assert_eq!(
        from_transcript(&text, USD_PER_COPILOT_CREDIT).source,
        CostSource::CopilotCredits
    );
}

#[test]
fn zero_credits_for_premium_requests_is_silence_and_zero_for_nothing_is_a_real_zero() {
    // The credit analogue of zero dollars beside real tokens: requests were made and not priced.
    let silent = from_transcript(&run_with(&[checkpoint("0", "15")]), USD_PER_COPILOT_CREDIT);
    assert_eq!(silent.source, CostSource::Unreported);

    let nothing = from_transcript(&run_with(&[checkpoint("0", "0")]), USD_PER_COPILOT_CREDIT);
    assert_eq!(nothing.source, CostSource::CopilotCredits);
    assert!(nothing.usd.abs() < f64::EPSILON);
}

#[test]
fn a_factory_that_pays_a_different_rate_is_priced_at_its_own() {
    let got = from_transcript(RUN, 0.02);

    assert!((got.usd - 30.211_631_2).abs() < 1e-9, "{}", got.usd);
}

#[test]
fn a_rate_that_is_not_a_price_prices_nothing() {
    // Validation refuses these before a factory starts; the parser does not trust them either.
    for rate in [0.0, -0.01, f64::NAN, f64::INFINITY] {
        let got = from_transcript(RUN, rate);

        assert_eq!(got.source, CostSource::Unreported, "{rate}");
        assert!(got.usd.abs() < f64::EPSILON, "{rate}");
    }
}

#[test]
fn a_runner_that_prints_dollars_is_still_read_as_dollars() {
    // Pricing credits must not change how the other CLIs are read.
    let claude = r#"{"type":"result","total_cost_usd":1.25,"usage":{"input_tokens":900000}}"#;
    let got = from_transcript(claude, USD_PER_COPILOT_CREDIT);

    assert_eq!(got.source, CostSource::Reported);
    assert!((got.usd - 1.25).abs() < f64::EPSILON);
}
