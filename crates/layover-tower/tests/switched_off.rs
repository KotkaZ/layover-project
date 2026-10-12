//! A switched-off workflow holds its ways in, and every chain of it already under way finishes.

use std::collections::BTreeSet;

use jiff::Timestamp;
use layover_core::flight::{FlightId, ItineraryId, Origin};
use layover_core::pipeline::PipelineName;
use layover_core::queue::Queued;
use layover_store::Journal;
use layover_tower::{Dispatched, Factory};

mod runs;
use runs::{Temp, factory, history, of, sleeps, to};

fn sweep_factory(temp: &Temp) -> Factory {
    factory(
        temp,
        &format!(
            r#"
[layover]
work_dir = "work"

[defaults]
runner = "quick"
timeout_sec = 60

[runners.quick]
command = {}

[agents.planner]
prompt = "plan"

[agents.eagle]
prompt = "review"

[pipelines.sweep]
entry = "planner"

[pipelines.build]
entry = "planner"

[[routes]]
from = "planner"
to = "eagle"
"#,
            sleeps(0)
        ),
    )
}

fn in_workflow(mut queued: Queued, pipeline: &str) -> Queued {
    queued.pipeline = Some(PipelineName::new(pipeline));
    queued
}

/// Drains `work`, and says which flights ran and which were taken off the queue.
fn drain(factory: &Factory, work: Vec<Queued>) -> (BTreeSet<FlightId>, BTreeSet<FlightId>) {
    let (mut ran, mut taken) = (BTreeSet::new(), BTreeSet::new());
    factory.drain(
        work,
        |flight| {
            taken.insert(flight.id.clone());
        },
        |flight, result| {
            if matches!(result, Dispatched::Ran { .. }) {
                ran.insert(flight.id.clone());
            }
        },
    );
    (ran, taken)
}

#[test]
fn a_trigger_waits_while_its_workflow_is_off_and_a_chain_under_way_carries_on() {
    let temp = Temp::new("switched-off");
    let factory = sweep_factory(&temp);
    Journal::open(temp.0.join(".layover").join("journal"))
        .expect("opens")
        .switch_off(&PipelineName::new("sweep"), Timestamp::now())
        .expect("switches off");

    let trigger = in_workflow(
        to(&ItineraryId::generate(), Origin::Human, "planner"),
        "sweep",
    );
    let hand_off = in_workflow(
        to(
            &ItineraryId::generate(),
            Origin::Agent("planner".into()),
            "eagle",
        ),
        "sweep",
    );
    let elsewhere = in_workflow(
        to(&ItineraryId::generate(), Origin::Human, "planner"),
        "build",
    );

    let (ran, taken) = drain(
        &factory,
        vec![trigger.clone(), hand_off.clone(), elsewhere.clone()],
    );

    assert!(
        !taken.contains(&trigger.flight.id),
        "the trigger stays queued, to start when switched on"
    );
    assert!(!ran.contains(&trigger.flight.id));
    assert!(
        ran.contains(&hand_off.flight.id),
        "a chain under way finishes"
    );
    assert!(
        ran.contains(&elsewhere.flight.id),
        "other workflows are untouched"
    );
    assert_eq!(of(&history(&temp.0), "planner").count(), 1);

    Journal::open(temp.0.join(".layover").join("journal"))
        .expect("opens")
        .switch_on(&PipelineName::new("sweep"))
        .expect("switches on");

    let (ran, _) = drain(&factory, vec![trigger.clone()]);
    assert!(ran.contains(&trigger.flight.id), "switched on, it starts");
}
