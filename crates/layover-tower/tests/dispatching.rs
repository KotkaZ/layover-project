//! The queue as a Tower runs it: work arriving while runs are alive, cancellations, and stopping.

use std::cell::Cell;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use layover_core::agent::AgentName;
use layover_core::flight::{Flight, ItineraryId, Origin};
use layover_core::queue::Queued;
use layover_tower::Dispatched;

mod runs;
use runs::{Temp, factory, history, human, sleeps, to};

/// A queue shared with the test, as the journal is shared with the dashboard.
#[derive(Clone, Default)]
struct Journal(Arc<Mutex<Vec<Queued>>>);

impl Journal {
    fn queue(&self, queued: Queued) {
        self.0.lock().expect("lock").push(queued);
    }

    fn pending(&self) -> Vec<Queued> {
        self.0.lock().expect("lock").clone()
    }

    fn unqueue(&self, flight: &Flight) -> bool {
        let mut pending = self.0.lock().expect("lock");
        let before = pending.len();
        pending.retain(|queued| queued.flight.id != flight.id);
        pending.len() != before
    }
}

#[test]
fn work_queued_while_runs_are_alive_starts_without_waiting_for_them() {
    // A person triggers something while a sweep's hour-long run is going, and a slot is free.
    let temp = Temp::new("arrives");
    let factory = factory(
        &temp,
        &format!(
            r#"
[layover]
work_dir = "work"

[defaults]
max_concurrent_runs = 2
timeout_sec = 60

[runners.slow]
command = {}

[runners.quick]
command = {}

[agents.sweep]
prompt = "sweep"
runner = "slow"
entry = true

[agents.manual]
prompt = "do it now"
runner = "quick"
entry = true
"#,
            sleeps(6),
            sleeps(1)
        ),
    );

    let journal = Journal::default();
    journal.queue(human("sweep"));
    let later = journal.clone();
    let person = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(1));
        later.queue(human("manual"));
    });

    factory.dispatch(
        journal.pending(),
        &mut |flight| journal.unqueue(flight),
        &mut |_, _| {},
        |_| journal.pending(),
        &|| false,
    );
    person.join().expect("joins");

    let runs = history(&temp.0);
    let find = |agent: &str| {
        runs.iter()
            .find(|run| run.agent.as_str() == agent)
            .unwrap_or_else(|| panic!("{agent} ran: {runs:?}"))
    };
    let (sweep, manual) = (find("sweep"), find("manual"));
    assert!(
        manual
            .finished_at
            .zip(sweep.finished_at)
            .is_some_and(|(manual, sweep)| manual < sweep),
        "the manual run started while the sweep was alive: {sweep:?} {manual:?}"
    );
    assert!(journal.pending().is_empty());
}

#[test]
fn a_flight_cancelled_before_it_starts_is_not_run() {
    let temp = Temp::new("cancelled");
    let factory = factory(
        &temp,
        r#"
[layover]
work_dir = "work"

[defaults]
runner = "shell"

[runners.shell]
command = ["echo", "never started"]

[agents.eagle]
prompt = "review"
entry = true
"#,
    );

    let mut results = Vec::new();
    factory.dispatch(
        vec![human("eagle")],
        // Somebody else took it off the queue a moment ago.
        &mut |_| false,
        &mut |_, result| results.push(result.to_string()),
        |_| Vec::new(),
        &|| false,
    );

    assert!(results.is_empty(), "{results:?}");
    assert!(history(&temp.0).is_empty());
}

#[test]
fn a_barrier_is_not_given_up_while_its_upstream_is_still_queued() {
    // The Tower stops — or every slot is busy — with `b`'s work still waiting. The join `a` and
    // `b` feed is not unreachable: `b` is going to run.
    let temp = Temp::new("still-queued");
    let factory = factory(
        &temp,
        &format!(
            r#"
[layover]
work_dir = "work"

[defaults]
runner = "slow"
max_concurrent_runs = 1
timeout_sec = 60

[runners.slow]
command = {}

[agents.a]
prompt = "a"

[agents.b]
prompt = "b"
entry = true

[agents.c]
prompt = "c"

[agents.x]
prompt = "x"
entry = true

[[routes]]
from = ["a", "b"]
to = "c"
join = "all"
"#,
            sleeps(1)
        ),
    );

    let chain = ItineraryId::generate();
    let from_a = to(&chain, Origin::Agent(AgentName::new("a")), "c");
    let pending = vec![
        from_a,
        human("x"),
        Queued::new(
            Flight::new(chain, Origin::Human, AgentName::new("b"), "go", 6),
            None,
            BTreeMap::new(),
        ),
    ];

    let stopping = Cell::new(false);
    let mut parked = 0;
    let drained = factory.dispatch(
        pending,
        &mut |_| true,
        &mut |flight, result| {
            if matches!(result, Dispatched::Parked { .. }) {
                parked += 1;
            }
            if flight.to.as_str() == "x" {
                stopping.set(true);
            }
        },
        |_| Vec::new(),
        &|| stopping.get(),
    );

    assert_eq!(parked, 1, "a's verdict waits for b's");
    assert_eq!(history(&temp.0).len(), 1, "only x ran before the stop");
    assert!(
        drained.abandoned.is_empty(),
        "given up while b was still queued: {:?}",
        drained
            .abandoned
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
    );
}
