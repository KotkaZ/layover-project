//! Every run records who sent it, so a chain can be drawn as the path its work actually took.
//!
//! A released join is the case worth testing: its agent runs once on a flight that carries every
//! arrival, and naming only the first sender would draw one of the two routes that led there.

use std::time::{Duration, Instant};

use layover_core::agent::AgentName;
use layover_core::flight::{ItineraryId, Origin};
use layover_core::queue::Queued;
use layover_tower::Dispatched;

mod runs;
use runs::{Temp, factory, history, live_records, of, sleeps, to};

fn names(sent_by: Option<&Vec<AgentName>>) -> Option<Vec<&str>> {
    sent_by.map(|senders| senders.iter().map(AgentName::as_str).collect())
}

#[test]
fn every_run_records_who_sent_it_and_a_released_join_names_every_arrival() {
    let temp = Temp::new("senders");
    let factory = factory(
        &temp,
        &format!(
            r#"
[layover]
work_dir = "work"

[defaults]
runner = "quick"
max_concurrent_runs = 4
timeout_sec = 60

[runners.quick]
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
            sleeps(0)
        ),
    );
    let chain = ItineraryId::generate();
    let dev = Origin::Agent(AgentName::new("dev"));

    let verdicts: std::cell::RefCell<Vec<Queued>> = std::cell::RefCell::default();
    let mut fanned = false;
    factory.drain_with(
        vec![to(&chain, Origin::Human, "dev")],
        &mut |_| {},
        &mut |flight, result| {
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
    let devs: Vec<_> = of(&runs, "dev").collect();
    assert_eq!(devs.len(), 2, "{runs:#?}");
    assert_eq!(
        names(devs[0].sent_by.as_ref()),
        Some(vec![]),
        "a person started it: recorded, and nobody in the mesh"
    );
    for upstream in ["tester", "reviewer"] {
        let run = of(&runs, upstream).next().expect("ran");
        assert_eq!(names(run.sent_by.as_ref()), Some(vec!["dev"]), "{upstream}");
    }
    assert_eq!(
        names(devs[1].sent_by.as_ref()),
        Some(vec!["reviewer", "tester"]),
        "the released join ran on both verdicts, not only the first to arrive"
    );
}

#[test]
fn a_run_still_going_says_who_sent_it() {
    // The dashboard draws a chain while it is working, and a run that has not ended is only in its
    // live record.
    let temp = Temp::new("senders-live");
    let factory = factory(
        &temp,
        &format!(
            r#"
[layover]
work_dir = "work"

[defaults]
runner = "slow"
timeout_sec = 60

[runners.slow]
command = {}

[agents.analyst]
prompt = "analyse"
entry = true

[agents.coder]
prompt = "code"

[[routes]]
from = "analyst"
to = "coder"
"#,
            sleeps(3)
        ),
    );
    let chain = ItineraryId::generate();
    let root = temp.0.clone();

    let watcher = std::thread::spawn(move || {
        let until = Instant::now() + Duration::from_secs(20);
        while Instant::now() < until {
            for path in live_records(&root) {
                let text = std::fs::read_to_string(&path).unwrap_or_default();
                let Ok(live) = serde_json::from_str::<layover_store::live::Live>(&text) else {
                    continue;
                };
                if live.agent.as_str() == "coder" {
                    return live.sent_by;
                }
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        None
    });

    factory.drain(
        vec![to(
            &chain,
            Origin::Agent(AgentName::new("analyst")),
            "coder",
        )],
        |_| {},
        |_, _| {},
    );

    let seen = watcher.join().expect("the watcher ends");
    assert_eq!(names(seen.as_ref()), Some(vec!["analyst"]));
}
