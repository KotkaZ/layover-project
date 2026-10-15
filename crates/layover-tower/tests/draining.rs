//! A draining factory lets what is running finish and starts nothing new — except the work that
//! lets a parked rendezvous release, which would otherwise be lost when the Tower stops. One that
//! cuts off kills what is running and leaves it for the next Tower.

use std::time::{Duration, Instant};

use layover_core::agent::AgentName;
use layover_core::flight::{ItineraryId, Origin};
use layover_core::queue::Queued;
use layover_tower::{Dispatched, Factory};

mod runs;
use runs::{Temp, factory, history, human, sleeps, to};

fn text(slow: &str) -> String {
    let quick = if cfg!(windows) {
        r#"["cmd", "/c", "echo done"]"#
    } else {
        r#"["sh", "-c", "echo done"]"#
    };
    format!(
        r#"
[layover]
work_dir = "work"

[defaults]
runner = "quick"
timeout_sec = 60

[runners.quick]
command = {quick}

[runners.slow]
command = {slow}

[agents.slow]
prompt = "take a while"
entry = true
runner = "slow"

[agents.eagle]
prompt = "review"
entry = true

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
"#
    )
}

fn results(tower: &Factory, work: Vec<Queued>) -> Vec<Dispatched> {
    let mut seen = Vec::new();
    tower.drain(work, |_| {}, |_, result| seen.push(result.clone()));
    seen
}

#[test]
fn a_draining_factory_starts_nothing_new_and_is_quiet() {
    let temp = Temp::new("drain-nothing");
    let tower = factory(&temp, &text(&sleeps(1)));

    tower.start_draining();
    let seen = results(&tower, vec![human("eagle")]);

    assert!(seen.is_empty(), "{seen:?}");
    assert!(history(&temp.0).is_empty());
    assert!(tower.is_draining());
    assert!(tower.is_quiescent());

    tower.stop_draining();
    let seen = results(&tower, vec![human("eagle")]);
    assert!(
        matches!(&seen[..], [Dispatched::Ran { .. }]),
        "work starts again once the drain ends: {seen:?}"
    );
}

#[test]
fn a_run_alive_when_draining_begins_finishes_and_then_the_factory_is_quiet() {
    let temp = Temp::new("drain-finish");
    let tower = factory(&temp, &text(&sleeps(2)));
    let mut handed_more = false;

    std::thread::scope(|scope| {
        let runner = scope.spawn(|| {
            let mut seen = Vec::new();
            tower.drain_with(
                vec![human("slow")],
                &mut |_| {},
                &mut |_, result| seen.push(result.clone()),
                // Work queued while the run is going: a drain must leave it queued.
                |_| {
                    if handed_more {
                        Vec::new()
                    } else {
                        handed_more = true;
                        vec![human("eagle")]
                    }
                },
            );
            seen
        });

        let started = Instant::now();
        while tower.alive_runs() == 0 && started.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(20));
        }
        tower.start_draining();
        assert!(!tower.is_quiescent(), "a run is alive");
        assert_eq!(tower.alive_agent_names(), vec![AgentName::new("slow")]);

        let seen = runner.join().expect("the drain ends");
        assert!(
            matches!(&seen[..], [Dispatched::Ran { .. }]),
            "the live run finished, and nothing else started: {seen:?}"
        );
    });

    assert!(tower.is_quiescent());
    let runs = history(&temp.0);
    assert_eq!(runs.len(), 1, "{runs:?}");
    assert_eq!(runs[0].agent, AgentName::new("slow"));
}

#[test]
fn work_that_completes_a_parked_rendezvous_still_starts_while_draining() {
    let temp = Temp::new("drain-join");
    let tower = factory(&temp, &text(&sleeps(1)));
    let chain = ItineraryId::generate();
    let mut seen = Vec::new();
    let mut while_parked = None;

    // One drain, as a Tower runs it: the tester's verdict parks, then — draining — the reviewer's
    // arrives beside unrelated work.
    tower.drain_with(
        vec![to(&chain, Origin::Agent(AgentName::new("tester")), "dev")],
        &mut |_| {},
        &mut |_, result| seen.push(result.clone()),
        |_| {
            if while_parked.is_some() {
                return Vec::new();
            }
            while_parked = Some((tower.parked_rendezvous(), tower.is_quiescent()));
            tower.start_draining();
            vec![
                human("eagle"),
                to(&chain, Origin::Agent(AgentName::new("reviewer")), "dev"),
            ]
        },
    );

    assert_eq!(
        while_parked,
        Some((1, false)),
        "an arrival parked at a rendezvous is lost if the Tower stops, so it is not quiet"
    );
    assert!(
        matches!(
            &seen[..],
            [Dispatched::Parked { .. }, Dispatched::Ran { .. }]
        ),
        "the join released and its agent ran; the unrelated work waited: {seen:?}"
    );
    let ran: Vec<_> = history(&temp.0)
        .into_iter()
        .map(|record| record.agent)
        .collect();
    assert_eq!(ran, vec![AgentName::new("dev")]);
    assert!(tower.is_quiescent());
}

#[test]
fn a_run_cut_off_is_killed_and_left_for_the_next_tower_to_settle() {
    let temp = Temp::new("drain-cut");
    let tower = factory(&temp, &text(&sleeps(30)));

    let began = Instant::now();
    let seen = std::thread::scope(|scope| {
        let runner = scope.spawn(|| results(&tower, vec![human("slow")]));
        while tower.alive_runs() == 0 && began.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(20));
        }
        tower.cut_off();
        runner.join().expect("the drain ends")
    });

    assert!(
        began.elapsed() < Duration::from_secs(20),
        "the run was killed, not waited for"
    );
    assert!(
        matches!(&seen[..], [Dispatched::Failed(why)] if why.contains("cut off")),
        "{seen:?}"
    );
    assert!(tower.is_idle());
    assert!(
        history(&temp.0).is_empty(),
        "not recorded: the next Tower records it as interrupted"
    );
    let live = std::fs::read_dir(temp.0.join(".layover").join("state").join("runs"))
        .expect("live records")
        .flatten()
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
        .count();
    assert_eq!(live, 1, "its live record is left for the next Tower");

    // Nothing starts once cut off, not even work a rendezvous waits for.
    let seen = results(&tower, vec![human("eagle")]);
    assert!(seen.is_empty(), "{seen:?}");
}
