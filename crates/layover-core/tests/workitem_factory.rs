//! The executable specification of `examples/workitem-factory/` — the mesh itself.
//!
//! That factory is the reference scenario: a request is investigated, built, tested and reviewed
//! until two independent agents agree, and then published. This file covers the shape of the
//! mesh — validation, the route map, workspace access and the two rendezvous barriers. Triggers,
//! flags, prompt composition and the hop arithmetic are in `workitem_factory_runtime.rs`.

mod common;

use std::collections::BTreeSet;

use common::{factory, prompts};
use layover_core::{
    Access, AgentName, Barrier, Delivery, Flight, ItineraryId, Origin, RouteGraph, validate,
    validate_prompts,
};

fn from_agent(sender: &str, to: &str, itinerary: &ItineraryId) -> Flight {
    Flight::new(
        itinerary.clone(),
        Origin::Agent(sender.into()),
        to.into(),
        format!("result from {sender}"),
        9,
    )
}

fn from_human(to: &str, itinerary: &ItineraryId) -> Flight {
    Flight::new(
        itinerary.clone(),
        Origin::Human,
        to.into(),
        "a new request",
        22,
    )
}

fn barrier_on(graph: &RouteGraph, agent: &str) -> Barrier {
    let Some(spec) = graph.join_for(&AgentName::from(agent)) else {
        panic!("`{agent}` must be guarded by a rendezvous barrier");
    };
    Barrier::from_spec(spec)
}

#[test]
fn the_example_factory_validates_cleanly() {
    let config = factory();

    assert_eq!(
        validate(&config),
        Vec::new(),
        "the reference factory must load with no findings at all"
    );
    assert_eq!(config.agents.len(), 8);
}

#[test]
fn every_prompt_in_the_example_composes() {
    let config = factory();

    assert_eq!(
        validate_prompts(&config, &prompts()),
        Vec::new(),
        "every prompt file must exist and name only declared flags"
    );
}

#[test]
fn the_route_map_permits_the_documented_edges_and_no_short_cuts() {
    let graph = RouteGraph::from_config(&factory());

    for (from, to) in [
        ("pr_scanner", "analyst"),
        ("analyst", "investigator"),
        ("analyst", "kusto"),
        ("investigator", "analyst"),
        ("kusto", "analyst"),
        ("analyst", "developer"),
        ("developer", "tester"),
        ("developer", "reviewer"),
        ("tester", "developer"),
        ("reviewer", "developer"),
        ("developer", "publisher"),
    ] {
        assert!(
            graph.permits(&from.into(), &to.into()),
            "`{from}` must be able to reach `{to}`"
        );
    }

    for (from, to) in [
        ("analyst", "publisher"),
        ("tester", "publisher"),
        ("reviewer", "publisher"),
        ("developer", "analyst"),
        ("investigator", "developer"),
        ("kusto", "developer"),
        ("analyst", "pr_scanner"),
    ] {
        assert!(
            !graph.permits(&from.into(), &to.into()),
            "`{from}` must not be able to reach `{to}`; only the developer decides to publish"
        );
    }
}

#[test]
fn exactly_one_agent_writes_during_development() {
    let config = factory();

    for inspector in [
        "analyst",
        "investigator",
        "kusto",
        "tester",
        "reviewer",
        "pr_scanner",
    ] {
        assert_eq!(
            config.agents[&AgentName::from(inspector)].access,
            Access::ReadOnly,
            "`{inspector}` inspects, so it must get a worktree snapshot rather than the live tree"
        );
    }

    assert_eq!(
        config.agents[&AgentName::from("developer")].access,
        Access::ReadWrite
    );
    assert_eq!(
        config.agents[&AgentName::from("publisher")].access,
        Access::ReadWrite
    );
}

// ── Intake: the analyst is behind a barrier and is also an entry point ────────────────────

#[test]
fn a_human_trigger_wakes_the_analyst_instead_of_parking() {
    // Open question 11, settled: a barrier speaks only for the upstreams it names. If a human
    // flight were parked here, the analyst could never be triggered at all, because the agents the
    // barrier waits for are the ones the analyst itself has to dispatch.
    let graph = RouteGraph::from_config(&factory());
    let itinerary = ItineraryId::generate();
    let mut barrier = barrier_on(&graph, "analyst");

    let outcome = barrier.deliver(from_human("analyst", &itinerary));

    assert!(
        matches!(outcome, Delivery::Direct(_)),
        "the human entry point must bypass the rendezvous"
    );
    assert_eq!(barrier.parked_count(), 0);
}

#[test]
fn the_scheduled_scanner_also_bypasses_the_analyst_barrier() {
    let graph = RouteGraph::from_config(&factory());
    let itinerary = ItineraryId::generate();
    let mut barrier = barrier_on(&graph, "analyst");

    let outcome = barrier.deliver(from_agent("pr_scanner", "analyst", &itinerary));

    assert!(
        matches!(outcome, Delivery::Direct(_)),
        "a scheduled trigger reaches the analyst the same way a human does"
    );
}

#[test]
fn the_analyst_wakes_once_holding_both_replies() {
    let graph = RouteGraph::from_config(&factory());
    let itinerary = ItineraryId::generate();
    let mut barrier = barrier_on(&graph, "analyst");

    assert_eq!(
        barrier.deliver(from_agent("investigator", "analyst", &itinerary)),
        Delivery::Parked {
            waiting_for: vec!["kusto".into()]
        },
        "one reply must not wake the analyst"
    );

    let Delivery::Ready(replies) = barrier.deliver(from_agent("kusto", "analyst", &itinerary))
    else {
        panic!("the barrier releases once both helpers report");
    };

    assert_eq!(replies.len(), 2, "the analyst sees both replies at once");

    let mut senders: Vec<String> = replies
        .iter()
        .filter_map(|f| f.from.agent())
        .map(ToString::to_string)
        .collect();
    senders.sort();
    assert_eq!(
        senders,
        ["investigator", "kusto"],
        "sender identity is what lets the analyst tell the replies apart"
    );
}

#[test]
fn a_helper_that_never_replies_strands_the_rendezvous() {
    // This is why the example always dispatches both helpers and has them answer "nothing to
    // add" rather than staying silent. A conditional fan-out turns the happy path into a stall.
    let graph = RouteGraph::from_config(&factory());
    let itinerary = ItineraryId::generate();
    let mut barrier = barrier_on(&graph, "analyst");

    let _ = barrier.deliver(from_agent("investigator", "analyst", &itinerary));

    assert_eq!(barrier.waiting_for(), vec![AgentName::from("kusto")]);
    assert!(
        !barrier.is_reachable(&graph, &BTreeSet::new()),
        "with nothing live to deliver `kusto`, the barrier is dead and the itinerary stalls"
    );
}

// ── Development: the review loop ──────────────────────────────────────────────────────────

#[test]
fn the_work_item_reaches_the_developer_without_disturbing_the_review_barrier() {
    let graph = RouteGraph::from_config(&factory());
    let itinerary = ItineraryId::generate();
    let mut barrier = barrier_on(&graph, "developer");

    let _ = barrier.deliver(from_agent("tester", "developer", &itinerary));

    let outcome = barrier.deliver(from_agent("analyst", "developer", &itinerary));

    assert!(
        matches!(outcome, Delivery::Direct(_)),
        "the analyst is not an upstream of the review barrier, so its work item is delivered"
    );
    assert_eq!(
        barrier.parked_count(),
        1,
        "a direct delivery must not discard a half-collected rendezvous"
    );
}

#[test]
fn the_developer_wakes_only_when_both_verdicts_arrive() {
    let graph = RouteGraph::from_config(&factory());
    let itinerary = ItineraryId::generate();
    let mut barrier = barrier_on(&graph, "developer");

    assert_eq!(
        barrier.deliver(from_agent("tester", "developer", &itinerary)),
        Delivery::Parked {
            waiting_for: vec!["reviewer".into()]
        },
        "a passing test suite alone must not be mistaken for approval"
    );

    let Delivery::Ready(verdicts) =
        barrier.deliver(from_agent("reviewer", "developer", &itinerary))
    else {
        panic!("both verdicts together release the developer");
    };

    assert_eq!(verdicts.len(), 2);
}

#[test]
fn a_rework_round_that_re_dispatches_only_one_branch_waits_forever() {
    // The constraint that today lives only in the developer's prompt: a fix must go to BOTH the
    // tester and the reviewer. Re-sending to one alone leaves the barrier holding a single fresh
    // verdict and waiting for a sibling that was never asked.
    let graph = RouteGraph::from_config(&factory());
    let itinerary = ItineraryId::generate();
    let mut barrier = barrier_on(&graph, "developer");

    // Round one: both report, the reviewer rejects, so the developer is woken and fixes the code.
    let _ = barrier.deliver(from_agent("tester", "developer", &itinerary));
    let _ = barrier.deliver(from_agent("reviewer", "developer", &itinerary));
    assert_eq!(barrier.parked_count(), 0, "the barrier drained on release");

    // Round two, done wrong: only the reviewer is asked again.
    let outcome = barrier.deliver(from_agent("reviewer", "developer", &itinerary));

    assert_eq!(
        outcome,
        Delivery::Parked {
            waiting_for: vec!["tester".into()]
        },
        "a stale pass from the previous version of the code must not clear the fix"
    );

    // Reachability is conservative, so the barrier survives while the developer is still running
    // and could yet dispatch the tester...
    let developer_live: BTreeSet<AgentName> = ["developer".into()].into_iter().collect();
    assert!(barrier.is_reachable(&graph, &developer_live));

    // ...and dies the moment that run exits without having done so. The itinerary is marked
    // stalled rather than hanging, but the work is abandoned either way.
    assert!(
        !barrier.is_reachable(&graph, &BTreeSet::new()),
        "a half re-dispatched rework round strands the fix"
    );
}

#[test]
fn a_full_re_dispatch_clears_the_stale_verdict_and_releases() {
    let graph = RouteGraph::from_config(&factory());
    let itinerary = ItineraryId::generate();
    let mut barrier = barrier_on(&graph, "developer");

    // Round one: the tester passed, the reviewer rejected.
    let _ = barrier.deliver(from_agent("tester", "developer", &itinerary));
    let _ = barrier.deliver(from_agent("reviewer", "developer", &itinerary));

    // Round two, done right: both are asked again about the fixed code.
    assert_eq!(
        barrier.deliver(from_agent("reviewer", "developer", &itinerary)),
        Delivery::Parked {
            waiting_for: vec!["tester".into()]
        }
    );

    let Delivery::Ready(verdicts) = barrier.deliver(from_agent("tester", "developer", &itinerary))
    else {
        panic!("re-dispatching both branches releases the barrier");
    };

    assert_eq!(verdicts.len(), 2, "both verdicts describe the same code");
}
