//! A chain runs on its workflow's rails, and a chain that starts at an agent with its own Fuel runs
//! on that.

use layover_core::agent::AgentName;
use layover_core::flight::{ItineraryId, Origin};
use layover_core::pipeline::PipelineName;
use layover_core::queue::Queued;
use layover_core::run::Outcome;
use layover_tower::{Dispatched, Factory, Refusal};

mod runs;
use runs::{Temp, costs, factory, sleeps, to};

fn in_workflow(mut queued: Queued, pipeline: &str) -> Queued {
    queued.pipeline = Some(PipelineName::new(pipeline));
    queued
}

/// A trigger and two hand-offs of one chain, from `first` to `worker` and back.
fn one_chain(pipeline: &str) -> Vec<Queued> {
    let chain = ItineraryId::generate();
    vec![
        in_workflow(to(&chain, Origin::Human, "worker"), pipeline),
        in_workflow(
            to(&chain, Origin::Agent(AgentName::new("worker")), "worker"),
            pipeline,
        ),
        in_workflow(
            to(&chain, Origin::Agent(AgentName::new("worker")), "worker"),
            pipeline,
        ),
    ]
}

/// What became of each flight drained, in order.
fn outcomes(factory: &Factory, work: Vec<Queued>) -> Vec<String> {
    let mut seen = Vec::new();
    factory.drain(
        work,
        |_| {},
        |_, result| {
            seen.push(match result {
                Dispatched::Ran { outcome, .. } => format!("{outcome:?}"),
                Dispatched::Refused(Refusal::Rail(denial)) => format!("refused: {denial}"),
                other => other.to_string(),
            });
        },
    );
    seen
}

fn text(runner: &str, rails: &str) -> String {
    format!(
        r#"
[layover]
work_dir = "work"

[defaults]
runner = "quick"
max_runs = 1
timeout_sec = 60
# One at a time, so each flight of a chain is admitted against what the one before it spent.
max_concurrent_runs = 1

[runners.quick]
command = {runner}

[agents.worker]
prompt = "work"
entry = true

[pipelines.small]
entry = "worker"

[pipelines.large]
entry = "worker"
{rails}

[[routes]]
from = "worker"
to = "worker"
"#
    )
}

#[test]
fn a_chain_of_a_workflow_with_its_own_run_cap_runs_to_that_cap() {
    let temp = Temp::new("rails-runs");
    let factory = factory(&temp, &text(&sleeps(0), "max_runs = 3"));

    let small = outcomes(&factory, one_chain("small"));
    let succeeded = format!("{:?}", Outcome::Succeeded);
    assert_eq!(
        small.iter().filter(|seen| **seen == succeeded).count(),
        1,
        "the factory's run cap of 1: {small:?}"
    );
    assert_eq!(
        small.iter().filter(|seen| seen.contains("run cap")).count(),
        2,
        "{small:?}"
    );

    let large = outcomes(&factory, one_chain("large"));
    assert!(
        large
            .iter()
            .all(|seen| *seen == format!("{:?}", Outcome::Succeeded)),
        "the workflow's own cap of 3: {large:?}"
    );
}

#[test]
fn a_run_of_a_workflow_with_its_own_timeout_is_stopped_at_it() {
    let temp = Temp::new("rails-timeout");
    let factory = factory(&temp, &text(&sleeps(20), "timeout = \"1s\""));

    let began = std::time::Instant::now();
    let seen = outcomes(&factory, vec![one_chain("large").remove(0)]);

    assert_eq!(seen, [format!("{:?}", Outcome::TimedOut)]);
    assert!(began.elapsed().as_secs() < 15, "{:?}", began.elapsed());
}

#[test]
fn a_chain_that_starts_at_an_agent_with_its_own_fuel_runs_on_it() {
    // Each run costs $15.11. The factory's $100 would fund all three; the worker's own $1 funds
    // the first only, as `fuel_usd` on an agent has always been documented to.
    let temp = Temp::new("rails-agent-fuel");
    let text = text(&costs(&temp), "")
        .replace("max_runs = 1", "max_runs = 10\nfuel_usd = 100.0")
        .replace(
            "prompt = \"work\"\nentry = true",
            "prompt = \"work\"\nentry = true\nfuel_usd = 1.0",
        );
    let factory = factory(&temp, &text);

    let seen = outcomes(&factory, one_chain("small"));

    let succeeded = format!("{:?}", Outcome::Succeeded);
    assert_eq!(
        seen.iter().filter(|seen| **seen == succeeded).count(),
        1,
        "{seen:?}"
    );
    assert_eq!(
        seen.iter()
            .filter(|seen| seen.contains("fuel exhausted"))
            .count(),
        2,
        "{seen:?}"
    );
}
