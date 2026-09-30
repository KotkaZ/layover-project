//! Runs in parallel, up to `max_concurrent_runs`, without loosening any rail.
//!
//! Measured from history: every run's recorded start and finish. The Tower stamps the start just
//! before spawning and the finish when it sees the process gone, so two runs in one slot never
//! overlap on paper, and an overlap in history is two processes alive at once.

use std::time::Instant;

use layover_core::agent::AgentName;
use layover_core::flight::{ItineraryId, Origin};
use layover_core::queue::Queued;
use layover_core::run::{Outcome, RunRecord};
use layover_tower::{Dispatched, Factory, Refusal};

mod runs;
use runs::{Temp, costs, factory, history, human, of, overlap, sleeps, to};

#[test]
fn k_runs_start_at_once_and_the_rest_start_as_slots_free() {
    // The Eagle Eye case: a sweep spawns a review per pull request, each its own chain.
    let temp = Temp::new("k-at-once");
    let factory = factory(
        &temp,
        &format!(
            r#"
[layover]
work_dir = "work"

[defaults]
runner = "slow"
max_concurrent_runs = 3
timeout_sec = 60

[runners.slow]
command = {}

[agents.eagle]
prompt = "review"
entry = true
"#,
            sleeps(2)
        ),
    );

    let began = Instant::now();
    let mut results = Vec::new();
    factory.drain(
        (0..7).map(|_| human("eagle")).collect(),
        |_| {},
        |_, result| results.push(matches!(result, Dispatched::Ran { .. })),
    );
    let took = began.elapsed();

    let runs = history(&temp.0);
    assert_eq!(runs.len(), 7, "every flight ran, none refused or dropped");
    assert!(results.iter().all(|ran| *ran), "{results:?}");
    assert_eq!(overlap(&runs), 3, "exactly the limit, never more");
    // Seven two-second runs one at a time is fourteen seconds; three at a time is three rounds.
    assert!(took.as_secs() < 11, "took {took:?}, as if one at a time");
}

#[test]
fn an_agent_with_its_own_cap_never_overlaps_itself_while_others_run_beside_it() {
    let temp = Temp::new("agent-cap");
    let factory = factory(
        &temp,
        &format!(
            r#"
[layover]
work_dir = "work"

[defaults]
runner = "slow"
max_concurrent_runs = 4
timeout_sec = 60

[runners.slow]
command = {}

[agents.mailman]
prompt = "post"
entry = true
max_concurrent = 1

[agents.eagle]
prompt = "review"
entry = true
"#,
            sleeps(1)
        ),
    );

    factory.drain(
        vec![
            human("mailman"),
            human("mailman"),
            human("mailman"),
            human("eagle"),
            human("eagle"),
            human("eagle"),
        ],
        |_| {},
        |_, _| {},
    );

    let runs = history(&temp.0);
    assert_eq!(runs.len(), 6);
    assert_eq!(
        overlap(of(&runs, "mailman")),
        1,
        "a single sender never overlaps"
    );
    assert!(
        overlap(of(&runs, "eagle")) >= 2,
        "the others still run in parallel"
    );
    assert!(overlap(&runs) <= 4);
}

/// A chain where `analyst` fans out to five `helper` runs at once.
fn fan_out(temp: &Temp, defaults: &str) -> Factory {
    factory(
        temp,
        &format!(
            r#"
[layover]
work_dir = "work"

[defaults]
runner = "priced"
max_concurrent_runs = 5
timeout_sec = 60
{defaults}

[runners.priced]
command = {}

[agents.analyst]
prompt = "analyse"
entry = true

[agents.helper]
prompt = "help"

[[routes]]
from = "analyst"
to = "helper"
"#,
            costs(temp)
        ),
    )
}

/// Drains `chain`: the analyst first, then five helpers it sends at once, then `then` if given.
fn drain_fan_out(factory: &Factory, chain: &ItineraryId, then: Option<&Queued>) -> Vec<String> {
    let analyst = Origin::Agent(AgentName::new("analyst"));
    let mut outcomes = Vec::new();
    let mut sent = false;
    let mut followed = false;
    let mut helped = 0;

    factory.drain_with(
        vec![to(chain, Origin::Human, "analyst")],
        &mut |_| {},
        &mut |flight, result| {
            outcomes.push(format!(
                "{} {}",
                flight.to,
                match result {
                    Dispatched::Ran { .. } => "ran".to_owned(),
                    Dispatched::Refused(Refusal::Rail(denial)) => format!("{denial:?}"),
                    other => other.to_string(),
                }
            ));
        },
        |done| {
            if !sent {
                sent = true;
                return (0..5)
                    .map(|_| to(chain, analyst.clone(), "helper"))
                    .collect();
            }
            // The follow-up is sent once every helper has been dealt with, so it is admitted
            // against everything the fan-out spent.
            helped += done
                .iter()
                .filter(|flight| flight.to.as_str() == "helper")
                .count();
            if helped >= 5 && !followed {
                followed = true;
                return then.cloned().into_iter().collect();
            }
            Vec::new()
        },
    );
    outcomes
}

#[test]
fn a_fan_out_is_cut_by_the_run_cap_exactly_however_many_start_at_once() {
    // Five helpers admitted in the same instant against a cap of four runs, one of them already
    // the analyst's. Admission charges the cap atomically, so exactly three start.
    let temp = Temp::new("cap");
    let factory = fan_out(&temp, "max_runs = 4\nfuel_usd = 1000.0");
    let outcomes = drain_fan_out(&factory, &ItineraryId::generate(), None);

    let ran = outcomes.iter().filter(|o| o.ends_with(" ran")).count();
    let capped = outcomes
        .iter()
        .filter(|o| o.ends_with("RunCapReached"))
        .count();
    assert_eq!((ran, capped), (4, 2), "{outcomes:?}");
}

#[test]
fn every_parallel_run_in_a_chain_is_debited_from_its_fuel() {
    // The analyst spends $15.11, then five helpers are admitted together against the $54.89 left
    // and all run — Fuel is metered after the fact, which is a known overshoot. What must hold is
    // that every one of the six debits lands: if one were lost to another, the chain would still
    // have Fuel and the next flight would run.
    let temp = Temp::new("fuel");
    let factory = fan_out(&temp, "fuel_usd = 70.0\nmax_runs = 20");
    let chain = ItineraryId::generate();
    let next = to(&chain, Origin::Agent(AgentName::new("analyst")), "helper");
    let outcomes = drain_fan_out(&factory, &chain, Some(&next));

    assert_eq!(
        outcomes.last().map(String::as_str),
        Some("helper FuelExhausted"),
        "six runs cost $90.66 of $70: {outcomes:?}"
    );
    let spent: f64 = history(&temp.0).iter().map(|run| run.usd).sum();
    assert!((spent - 6.0 * 15.105_815_6).abs() < 1e-6, "{spent}");
}

#[test]
fn a_barrier_waits_for_upstreams_that_ran_in_parallel_and_releases_once() {
    let temp = Temp::new("barrier");
    let factory = factory(
        &temp,
        &format!(
            r#"
[layover]
work_dir = "work"

[defaults]
runner = "slow"
max_concurrent_runs = 4
timeout_sec = 60

[runners.slow]
command = {}

[agents.dev]
prompt = "develop"
entry = true

[agents.tester]
prompt = "test"

[agents.reviewer]
prompt = "review"

[[routes]]
from = "dev"
to = ["tester", "reviewer"]

[[routes]]
from = ["tester", "reviewer"]
to = "dev"
join = "all"
"#,
            sleeps(1)
        ),
    );
    let chain = ItineraryId::generate();
    let dev = Origin::Agent(AgentName::new("dev"));

    let mut parked = 0;
    let verdicts: std::cell::RefCell<Vec<Queued>> = std::cell::RefCell::default();
    let mut fanned = false;
    factory.drain_with(
        vec![to(&chain, Origin::Human, "dev")],
        &mut |_| {},
        &mut |flight, result| {
            if matches!(result, Dispatched::Parked { .. }) {
                parked += 1;
            }
            // Each upstream that finishes sends its verdict back, as `layover_send` would.
            if matches!(result, Dispatched::Ran { .. })
                && matches!(flight.to.as_str(), "tester" | "reviewer")
            {
                verdicts
                    .borrow_mut()
                    .push(to(&chain, Origin::Agent(flight.to.clone()), "dev"));
            }
        },
        |_| {
            if !fanned {
                fanned = true;
                return vec![
                    to(&chain, dev.clone(), "tester"),
                    to(&chain, dev.clone(), "reviewer"),
                ];
            }
            verdicts.take()
        },
    );

    let runs = history(&temp.0);
    let upstreams: Vec<&RunRecord> = of(&runs, "tester").chain(of(&runs, "reviewer")).collect();
    assert_eq!(
        overlap(upstreams.iter().copied()),
        2,
        "the upstreams ran side by side"
    );
    assert_eq!(parked, 1, "the first verdict waited for the second");
    let devs: Vec<&RunRecord> = of(&runs, "dev").collect();
    assert_eq!(
        devs.len(),
        2,
        "the dev that fanned out, and one released join"
    );
    let released = devs[1];
    assert!(
        upstreams.iter().all(|run| run
            .finished_at
            .is_some_and(|end| end <= released.started_at)),
        "the join released only after both upstreams finished"
    );
}

#[test]
fn a_run_that_times_out_is_killed_without_touching_the_run_beside_it() {
    // `slow` outlives the timeout and is killed at about four seconds. `steady` starts after
    // `quick` finishes, is alive when `slow` is killed, and finishes inside its own four seconds.
    let temp = Temp::new("timeout");
    let factory = factory(
        &temp,
        &format!(
            r#"
[layover]
work_dir = "work"

[defaults]
max_concurrent_runs = 2
timeout_sec = 4

[runners.slow]
command = {}

[runners.quick]
command = {}

[runners.steady]
command = {}

[agents.slow]
prompt = "hang"
runner = "slow"
entry = true

[agents.quick]
prompt = "finish"
runner = "quick"
entry = true

[agents.steady]
prompt = "work"
runner = "steady"
entry = true
"#,
            sleeps(60),
            sleeps(2),
            sleeps(3)
        ),
    );

    let mut sent = false;
    factory.drain_with(
        vec![human("slow"), human("quick")],
        &mut |_| {},
        &mut |_, _| {},
        |done| {
            if !sent && done.iter().any(|flight| flight.to.as_str() == "quick") {
                sent = true;
                return vec![human("steady")];
            }
            Vec::new()
        },
    );

    let runs = history(&temp.0);
    let find = |agent: &str| {
        runs.iter()
            .find(|run| run.agent.as_str() == agent)
            .unwrap_or_else(|| panic!("{agent} ran: {runs:?}"))
    };
    let (slow, steady) = (find("slow"), find("steady"));
    assert_eq!(slow.outcome, Outcome::TimedOut);
    assert_eq!(steady.outcome, Outcome::Succeeded, "{steady:?}");
    let killed = slow.finished_at.expect("finished");
    assert!(
        steady.started_at < killed && steady.finished_at.is_some_and(|end| end > killed),
        "steady was alive when slow was killed: {slow:?} {steady:?}"
    );
}

#[test]
fn a_ground_stop_lets_nothing_new_start_and_ends_what_is_running() {
    let temp = Temp::new("ground-stop");
    let factory = factory(
        &temp,
        &format!(
            r#"
[layover]
work_dir = "work"

[defaults]
runner = "slow"
max_concurrent_runs = 2
timeout_sec = 60

[runners.slow]
command = {}

[agents.eagle]
prompt = "review"
entry = true
"#,
            sleeps(30)
        ),
    );

    let stop = temp.0.join(".layover").join("ground-stop");
    let halter = stop.clone();
    let pull = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(2));
        std::fs::write(halter, "").expect("engages");
    });

    let began = Instant::now();
    factory.drain((0..4).map(|_| human("eagle")).collect(), |_| {}, |_, _| {});
    pull.join().expect("joins");

    let runs = history(&temp.0);
    assert_eq!(
        runs.len(),
        2,
        "two were running; the other two never started"
    );
    assert!(
        runs.iter().all(|run| run.outcome == Outcome::Halted),
        "{runs:?}"
    );
    assert!(
        began.elapsed().as_secs() < 20,
        "the running pair was ended, not waited out"
    );
    let _ = std::fs::remove_file(stop);
}
