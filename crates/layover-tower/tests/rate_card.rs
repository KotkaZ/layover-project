//! Pricing a run that reports tokens but no dollars from the factory's rate card.
//!
//! `[rates.<model>]` was documented as the fallback for exactly such a runner — Codex prints token
//! counts and never dollars — and was parsed, validated and then never applied: every such run was
//! `unreported`, and a factory that had written a rate card still showed no spend at all.
//!
//! An estimate is labelled `rate_card` and is not a measurement. It is shown, and it is counted in
//! what the dashboard calls an estimate, but it never debits Fuel and never draws on the Reserve.

use std::collections::BTreeMap;

use layover_core::agent::AgentName;
use layover_core::config::Config;
use layover_core::cost::{CostSource, ModelRates, RateCard, USD_PER_COPILOT_CREDIT};
use layover_core::flight::{Flight, ItineraryId, Origin};
use layover_core::queue::Queued;
use layover_tower::cost::{Prices, priced};
use layover_tower::{Dispatched, Factory};

mod runs;
use runs::{Temp, history};

/// What `codex exec --json` prints when a turn ends: cached tokens are part of the input.
const CODEX: &str = r#"{"type":"thread.started","thread_id":"t1"}
{"type":"item.completed","item":{"id":"item_0","item_type":"assistant_message","text":"Reviewed."}}
{"type":"turn.completed","usage":{"input_tokens":1200000,"cached_input_tokens":1000000,"output_tokens":100000}}"#;

/// $1.25 a million fresh input tokens, $0.125 cached, $10 out: $0.25 + $0.125 + $1.00.
const ESTIMATE: f64 = 1.375;

fn card() -> RateCard {
    RateCard::new().with(
        "gpt-x",
        ModelRates {
            input_usd: 1.25,
            output_usd: 10.0,
            cache_read_usd: 0.125,
            cache_write_usd: 0.0,
        },
    )
}

fn price(text: &str, rates: &RateCard, model: Option<&str>) -> layover_tower::Reported {
    priced(
        text,
        &Prices {
            usd_per_credit: USD_PER_COPILOT_CREDIT,
            rates,
            model,
        },
    )
}

#[test]
fn tokens_and_no_dollars_are_priced_from_the_card_with_cached_input_at_the_cache_rate() {
    let got = price(CODEX, &card(), Some("gpt-x"));

    assert_eq!(got.source, CostSource::RateCard);
    assert!((got.usd - ESTIMATE).abs() < 1e-9, "{}", got.usd);
    assert!(
        !got.source.is_measured(),
        "an estimate is not a measurement"
    );
    assert_eq!(
        (got.input_tokens, got.output_tokens, got.cache_read_tokens),
        (200_000, 100_000, 1_000_000),
        "cached input is recorded as cache reads, not counted twice"
    );
}

#[test]
fn without_a_card_for_the_model_it_runs_on_the_run_stays_unreported() {
    for (rates, model) in [
        (RateCard::new(), Some("gpt-x")),
        (card(), Some("another-model")),
        (card(), None),
    ] {
        let got = price(CODEX, &rates, model);
        assert_eq!(got.source, CostSource::Unreported, "{model:?}");
        assert!(got.usd.abs() < f64::EPSILON);
    }
}

#[test]
fn a_figure_the_runner_printed_and_was_disbelieved_is_not_replaced_by_an_estimate() {
    // A cent for over a million tokens is not believed. Substituting the card would invent a
    // different number with no better claim to the truth, so it stays unreported.
    let text = r#"{"type":"result","total_cost_usd":0.01,"usage":{"input_tokens":1200000,"output_tokens":100000}}"#;

    let got = price(text, &card(), Some("gpt-x"));

    assert_eq!(got.source, CostSource::Unreported);
}

#[test]
fn printed_dollars_and_copilot_credits_win_over_the_card() {
    let dollars = r#"{"type":"result","total_cost_usd":4.2,"usage":{"input_tokens":1200000,"output_tokens":100000}}"#;
    let got = price(dollars, &card(), Some("gpt-x"));
    assert_eq!(got.source, CostSource::Reported);
    assert!((got.usd - 4.2).abs() < f64::EPSILON);

    let credits = r#"{"type":"session.usage_checkpoint","data":{"totalNanoAiu":150000000000,"totalPremiumRequests":1}}"#;
    let got = price(credits, &card(), Some("gpt-x"));
    assert_eq!(got.source, CostSource::CopilotCredits);
}

#[test]
fn a_run_is_recorded_at_its_estimate_on_the_model_it_ran_and_its_chain_is_not_debited() {
    // $0.50 of Fuel and two runs estimated at $1.375 each. Both run: an estimate is shown, never
    // charged, so Fuel stays the rail that only measured figures move.
    let temp = Temp::new("rate-card");
    std::fs::write(temp.0.join("gpt-x.jsonl"), format!("{CODEX}\n")).expect("writes");
    let stream = temp.0.join("{model}.jsonl");
    let stream = stream.display();
    let runner = if cfg!(windows) {
        format!(r#"["cmd", "/c", "type", '{stream}']"#)
    } else {
        format!(r#"["cat", '{stream}']"#)
    };
    let config: Config = toml::from_str(&format!(
        r#"
[layover]
work_dir = "work"

[defaults]
runner = "codex"
timeout_sec = 30
max_concurrent_runs = 1
fuel_usd = 0.5

[runners.codex]
command = {runner}

[agents.reviewer]
prompt = "review"
model = "gpt-x"
entry = true

[rates.gpt-x]
input_usd = 1.25
output_usd = 10.0
cache_read_usd = 0.125
"#
    ))
    .expect("the fixture parses");
    let factory = Factory::new(config, &temp.0).expect("opens");

    let chain = ItineraryId::generate();
    let flight = || {
        Queued::new(
            Flight::new(
                chain.clone(),
                Origin::Human,
                AgentName::new("reviewer"),
                "go",
                4,
            ),
            None,
            BTreeMap::new(),
        )
    };
    let mut ran = 0;
    factory.drain(
        vec![flight(), flight()],
        |_| {},
        |_, result| {
            if matches!(result, Dispatched::Ran { .. }) {
                ran += 1;
            }
        },
    );

    assert_eq!(
        ran, 2,
        "an estimate debits no Fuel, so the second run is not refused"
    );
    let runs = history(&temp.0);
    assert_eq!(runs.len(), 2);
    for run in &runs {
        assert_eq!(run.source, CostSource::RateCard);
        assert!((run.usd - ESTIMATE).abs() < 1e-9, "{}", run.usd);
        assert_eq!(run.model.as_deref(), Some("gpt-x"));
        assert_eq!(run.usage.cache_read, 1_000_000);
    }
}
