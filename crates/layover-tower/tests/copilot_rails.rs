//! Copilot credits bind the money rails.
//!
//! A runner that prints a Copilot usage checkpoint debits its chain's Fuel and draws on the
//! factory's Reserve, like one that prints dollars. Before credits were priced, every Copilot run
//! was unreported: Fuel debited nothing, the Reserve counted nothing, and neither could ever stop a
//! Copilot factory.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use jiff::{SignedDuration, Timestamp};
use layover_core::agent::AgentName;
use layover_core::config::Config;
use layover_core::cost::{CostSource, TokenUsage};
use layover_core::flight::{Flight, ItineraryId, Origin, RunId};
use layover_core::itinerary::Denial;
use layover_core::queue::Queued;
use layover_core::run::{Outcome, RunRecord};
use layover_store::History;
use layover_tower::{Dispatched, Factory, Refusal};

/// A 47-minute Opus 5.5 run: 1,510.58 credits, $15.11 at a cent a credit.
const CHECKPOINT: &str = r#"{"type":"session.usage_checkpoint","id":"c1","parentId":null,"timestamp":"2026-09-30T08:48:57.402Z","data":{"totalNanoAiu":1510581560000,"totalPremiumRequests":15,"modelCacheState":[],"promptCacheBreakState":[]}}"#;

struct Temp(PathBuf);

impl Temp {
    fn new(tag: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("layover-credits-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("a temporary directory");
        std::fs::write(path.join("stream.jsonl"), format!("{CHECKPOINT}\n")).expect("writes");
        Self(path)
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A factory whose one agent prints a Copilot checkpoint and exits, as a Copilot run would.
fn factory(temp: &Temp, extra: &str) -> Factory {
    let stream = temp.0.join("stream.jsonl");
    let stream = stream.display();
    let runner = if cfg!(windows) {
        format!(r#"["cmd", "/c", "type", '{stream}']"#)
    } else {
        format!(r#"["cat", '{stream}']"#)
    };
    let text = format!(
        r#"
[layover]
work_dir = "work"

[defaults]
runner = "copilot"
timeout_sec = 30
{extra}

[runners.copilot]
command = {runner}

[agents.reviewer]
prompt = "review"
entry = true
"#
    );
    let config: Config = toml::from_str(&text).expect("the fixture parses");
    Factory::new(config, &temp.0).expect("opens")
}

fn flight(chain: &ItineraryId) -> Queued {
    Queued::new(
        Flight::new(
            chain.clone(),
            Origin::Human,
            AgentName::new("reviewer"),
            "review it",
            4,
        ),
        None,
        BTreeMap::new(),
    )
}

/// What became of a flight, as far as these tests look.
#[derive(Debug)]
enum Seen {
    Ran {
        usd: f64,
        source: CostSource,
    },
    Refused(Refusal),
    #[allow(dead_code, reason = "read by the Debug output of a failing assertion")]
    Other(String),
}

fn drain(factory: &Factory, work: Vec<Queued>) -> Vec<Seen> {
    let mut results = Vec::new();
    factory.drain(
        work,
        |_| {},
        |_, result| {
            results.push(match result {
                Dispatched::Ran { usd, source, .. } => Seen::Ran {
                    usd: *usd,
                    source: *source,
                },
                Dispatched::Refused(refusal) => Seen::Refused(refusal.clone()),
                other => Seen::Other(other.to_string()),
            });
        },
    );
    results
}

#[test]
fn a_copilot_run_is_recorded_at_the_price_of_its_credits() {
    let temp = Temp::new("recorded");
    let results = drain(&factory(&temp, ""), vec![flight(&ItineraryId::generate())]);

    let [Seen::Ran { usd, source }] = results.as_slice() else {
        panic!("{results:?}");
    };
    assert_eq!(*source, CostSource::CopilotCredits);
    assert!((usd - 15.105_815_6).abs() < 1e-9, "{usd}");
}

#[test]
fn credits_debit_the_chains_fuel_until_it_is_refused() {
    // $10 of Fuel, and the first run costs $15.11. The second run in the same chain is refused;
    // before credits were priced it ran, because the first had debited nothing.
    let temp = Temp::new("fuel");
    let chain = ItineraryId::generate();
    let results = drain(
        &factory(&temp, "fuel_usd = 10.0"),
        vec![flight(&chain), flight(&chain)],
    );

    assert!(matches!(results[0], Seen::Ran { .. }), "{results:?}");
    assert!(
        matches!(
            results[1],
            Seen::Refused(Refusal::Rail(Denial::FuelExhausted))
        ),
        "{results:?}"
    );
}

#[test]
fn the_reserve_refuses_new_work_once_credits_have_spent_it() {
    // A $20 Reserve: the first chain spends $15.11 and leaves room, the second takes the factory
    // past the cap, and the third is refused however much Fuel of its own it has.
    let temp = Temp::new("reserve");
    let factory = factory(
        &temp,
        "fuel_usd = 100.0\n\n[reserve]\nfuel_usd = 20.0\nwindow_hours = 24",
    );
    let results = drain(
        &factory,
        vec![
            flight(&ItineraryId::generate()),
            flight(&ItineraryId::generate()),
            flight(&ItineraryId::generate()),
        ],
    );

    assert!(matches!(results[0], Seen::Ran { .. }), "{results:?}");
    assert!(matches!(results[1], Seen::Ran { .. }), "{results:?}");
    assert!(
        matches!(
            results[2],
            Seen::Refused(Refusal::Rail(Denial::ReserveExhausted))
        ),
        "{results:?}"
    );

    // Refused work is not silent: the chain is recorded as halted, saying why and until when.
    let halted = history(&temp.0)
        .into_iter()
        .find(|record| record.outcome == Outcome::Halted)
        .expect("the refusal is in history");
    let detail = halted.detail.expect("with a reason");
    assert!(detail.contains("Reserve"), "{detail}");
    assert!(detail.contains("$30.21 of $20.00"), "{detail}");
    assert!(
        halted.usd.abs() < f64::EPSILON && halted.source == CostSource::Reported,
        "a refused run cost a measured nothing, and must not turn totals into a lower bound"
    );
}

#[test]
fn spend_that_has_rolled_out_of_the_window_no_longer_counts() {
    let temp = Temp::new("window");
    let reserve = "fuel_usd = 100.0\n\n[reserve]\nfuel_usd = 20.0\nwindow_hours = 24";

    // $50 a day and an hour ago: outside the window, so it holds nothing back.
    spent(&temp.0, 50.0, SignedDuration::from_hours(25));
    let results = drain(
        &factory(&temp, reserve),
        vec![flight(&ItineraryId::generate())],
    );
    assert!(matches!(results[0], Seen::Ran { .. }), "{results:?}");

    // $50 an hour ago, read by a Tower that has just started: inside the window, so it binds.
    spent(&temp.0, 50.0, SignedDuration::from_hours(1));
    let results = drain(
        &factory(&temp, reserve),
        vec![flight(&ItineraryId::generate())],
    );
    assert!(
        matches!(
            results[0],
            Seen::Refused(Refusal::Rail(Denial::ReserveExhausted))
        ),
        "{results:?}"
    );
}

#[test]
fn a_reserve_of_zero_is_unlimited_and_never_refuses() {
    let temp = Temp::new("unlimited");
    spent(&temp.0, 10_000.0, SignedDuration::from_hours(1));

    let results = drain(
        &factory(&temp, "fuel_usd = 100.0\n\n[reserve]\nfuel_usd = 0"),
        vec![flight(&ItineraryId::generate())],
    );
    assert!(matches!(results[0], Seen::Ran { .. }), "{results:?}");
}

#[test]
fn a_factory_that_writes_no_reserve_gets_the_documented_hundred_dollars_a_day() {
    // `[reserve]` defaults to $100 in any rolling 24 hours, and that default now binds. A factory
    // that wants no ceiling says `fuel_usd = 0`.
    let temp = Temp::new("default");
    spent(&temp.0, 99.0, SignedDuration::from_hours(1));
    let results = drain(
        &factory(&temp, "fuel_usd = 100.0"),
        vec![flight(&ItineraryId::generate())],
    );
    assert!(matches!(results[0], Seen::Ran { .. }), "{results:?}");

    let results = drain(
        &factory(&temp, "fuel_usd = 100.0"),
        vec![flight(&ItineraryId::generate())],
    );
    assert!(
        matches!(
            results[0],
            Seen::Refused(Refusal::Rail(Denial::ReserveExhausted))
        ),
        "$99 + $15.11 is past the default: {results:?}"
    );
}

/// Writes a finished, credit-priced run into history `ago` before now.
fn spent(root: &Path, usd: f64, ago: SignedDuration) {
    let at = Timestamp::now().checked_sub(ago).expect("in range");
    let record = RunRecord::started(
        RunId::generate(),
        ItineraryId::generate(),
        AgentName::new("reviewer"),
        at,
    )
    .finished(Outcome::Succeeded, at)
    .costing(usd, CostSource::CopilotCredits, TokenUsage::default());
    History::open(root.join(".layover").join("history"))
        .expect("opens")
        .append(&record)
        .expect("appends");
}

/// Every run in history, however old.
fn history(root: &Path) -> Vec<RunRecord> {
    let mut found = Vec::new();
    for entry in std::fs::read_dir(root.join(".layover").join("history")).expect("history") {
        let text = std::fs::read_to_string(entry.expect("entry").path()).expect("reads");
        found.extend(
            text.lines()
                .filter_map(|line| serde_json::from_str::<RunRecord>(line).ok()),
        );
    }
    found
}
