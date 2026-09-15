//! End-to-end behaviour of the v0.1 rendezvous design, exercised through the public API.
//!
//! These tests are the executable form of `docs/routing.md`. Each one corresponds to a v0.1
//! done-criterion in `docs/roadmap.md`.

use std::collections::BTreeSet;

use layover_core::{
    AgentName, Barrier, Config, Delivery, Denial, Flight, Itinerary, ItineraryId, Origin,
    RouteGraph, validate,
};

const FACTORY: &str = r#"
[defaults]
runner = "claude"
max_hops = 4
fuel_usd = 1.00
max_runs = 16

[runners.claude]
command = ["claude", "-p", "{prompt}"]

[agents.planner]
description = "Breaks the goal down and dispatches it"
prompt = "Break the goal down and dispatch it."
entry = true

[agents.probe_a]
description = "Does the work"
prompt = "Do the work."

[agents.probe_b]
description = "Inspects the work"
prompt = "Inspect the work."
access = "read-only"

[agents.collector]
description = "Combines both inputs"
prompt = "Combine both inputs."

[[routes]]
from = "planner"
to = ["probe_a", "probe_b"]

[[routes]]
from = ["probe_a", "probe_b"]
to = "collector"
join = "all"
timeout_sec = 1800

[[routes]]
from = "probe_a"
to = "planner"
"#;

fn factory() -> Config {
    Config::from_toml(FACTORY, "layover.toml").expect("the documented factory shape parses")
}

fn flight_to_collector(sender: &str, itinerary: &ItineraryId, hops: u32) -> Flight {
    Flight::new(
        itinerary.clone(),
        Origin::Agent(sender.into()),
        "collector".into(),
        format!("result from {sender}"),
        hops,
    )
}

#[test]
fn the_documented_factory_validates_cleanly() {
    let config = factory();

    assert_eq!(
        validate(&config),
        Vec::new(),
        "a read-only sibling makes the fan-out safe, so nothing should be reported"
    );
}

#[test]
fn the_route_map_permits_only_declared_edges() {
    let graph = RouteGraph::from_config(&factory());

    assert!(graph.permits(&"planner".into(), &"probe_a".into()));
    assert!(graph.permits(&"probe_a".into(), &"planner".into()));
    assert!(
        !graph.permits(&"planner".into(), &"collector".into()),
        "the planner must not be able to skip the probes"
    );
    assert!(!graph.permits(&"probe_b".into(), &"planner".into()));
}

#[test]
fn a_fan_out_then_rendezvous_wakes_the_collector_exactly_once() {
    let config = factory();
    let graph = RouteGraph::from_config(&config);
    let mut itinerary = Itinerary::new(
        ItineraryId::generate(),
        config.defaults.max_hops,
        config.defaults.fuel_usd,
        config.defaults.max_runs,
    );

    // A human triggers the planner.
    let entry_hops = itinerary.initial_hops();
    itinerary.record_run_started().expect("first run allowed");

    // The planner fans out; both branches inherit the same remaining count.
    let branch_hops = itinerary
        .authorize_send(entry_hops)
        .expect("the planner may dispatch");
    assert_eq!(branch_hops, entry_hops - 1);

    let spec = graph
        .join_for(&AgentName::from("collector"))
        .expect("the collector is guarded by a barrier");
    let mut barrier = Barrier::from_spec(spec);

    let parked = barrier.deliver(flight_to_collector(
        "probe_b",
        itinerary.id(),
        branch_hops - 1,
    ));
    assert_eq!(
        parked,
        Delivery::Parked {
            waiting_for: vec!["probe_a".into()]
        },
        "one input must not wake the collector"
    );

    let Delivery::Ready(inputs) = barrier.deliver(flight_to_collector(
        "probe_a",
        itinerary.id(),
        branch_hops - 1,
    )) else {
        panic!("the barrier should release once both branches report");
    };

    assert_eq!(
        inputs.len(),
        2,
        "the collector receives both inputs at once"
    );
    let senders: BTreeSet<Option<&AgentName>> = inputs.iter().map(|f| f.from.agent()).collect();
    assert_eq!(
        senders.len(),
        2,
        "sender identity must distinguish the inputs"
    );
}

#[test]
fn a_loop_back_leaves_a_barrier_that_is_abandoned_rather_than_parked_forever() {
    let config = factory();
    let graph = RouteGraph::from_config(&config);
    let itinerary_id = ItineraryId::generate();

    let spec = graph.join_for(&AgentName::from("collector")).unwrap();
    let mut barrier = Barrier::from_spec(spec);

    // probe_b reported, then probe_a chose the loop-back edge back to the planner instead.
    let _ = barrier.deliver(flight_to_collector("probe_b", &itinerary_id, 2));

    // While the planner is still running, probe_a remains reachable.
    let planner_live: BTreeSet<AgentName> = ["planner".into()].into_iter().collect();
    assert!(barrier.is_reachable(&graph, &planner_live));

    // Once nothing is live that can reach probe_a, the barrier is dead.
    assert!(
        !barrier.is_reachable(&graph, &BTreeSet::new()),
        "a barrier nothing can satisfy must be abandoned so the itinerary can be marked stalled"
    );
}

#[test]
fn a_re_dispatch_discards_the_stale_sibling_result() {
    let config = factory();
    let graph = RouteGraph::from_config(&config);
    let itinerary_id = ItineraryId::generate();

    let spec = graph.join_for(&AgentName::from("collector")).unwrap();
    let mut barrier = Barrier::from_spec(spec);

    // First attempt: probe_b reports, probe_a loops back instead of reporting.
    let _ = barrier.deliver(flight_to_collector("probe_b", &itinerary_id, 2));

    // After the fix the planner re-dispatches BOTH branches. probe_b reports again first.
    let outcome = barrier.deliver(flight_to_collector("probe_b", &itinerary_id, 2));

    assert_eq!(
        outcome,
        Delivery::Parked {
            waiting_for: vec!["probe_a".into()]
        },
        "a repeat delivery resets the barrier instead of completing it with stale state"
    );
}

#[test]
fn a_chain_terminates_when_hops_run_out() {
    let config = factory();
    let itinerary = Itinerary::new(
        ItineraryId::generate(),
        config.defaults.max_hops,
        config.defaults.fuel_usd,
        config.defaults.max_runs,
    );

    let mut hops = itinerary.initial_hops();
    let mut legs = 0;

    while let Ok(next) = itinerary.authorize_send(hops) {
        hops = next;
        legs += 1;
        assert!(legs < 100, "the chain must terminate");
    }

    assert_eq!(legs, config.defaults.max_hops - 1);
    assert_eq!(
        itinerary.authorize_send(hops),
        Err(Denial::HopsExhausted),
        "the chain is cut rather than running forever"
    );
}

#[test]
fn the_run_cap_holds_when_a_runner_reports_no_cost() {
    let mut itinerary = Itinerary::new(ItineraryId::generate(), 8, 1.00, 4);

    for _ in 0..4 {
        itinerary.record_run_started().expect("within the cap");
        itinerary.note_unreported_cost();
    }

    assert!(
        itinerary.has_cost_reporting_gap(),
        "the Tower must be able to surface silent metering"
    );
    assert!(
        !itinerary.fuel_exhausted(),
        "fuel metered nothing, so it cannot be what stops this"
    );
    assert_eq!(
        itinerary.record_run_started(),
        Err(Denial::RunCapReached),
        "the deterministic cap is what actually bounds breadth here"
    );
}
