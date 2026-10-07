use super::*;
use jiff::{Timestamp, ToSpan as _};
use layover_core::agent::AgentName;
use layover_core::cost::CostSource;
use layover_core::cost::TokenUsage;
use layover_core::flight::{Flight, ItineraryId, Origin, RunId};

fn chain() -> ItineraryId {
    ItineraryId::generate()
}

fn run(
    itinerary: &ItineraryId,
    agent: &str,
    outcome: Outcome,
    usd: f64,
    finished: bool,
) -> RunRecord {
    let started_at = Timestamp::now();

    RunRecord {
        run: RunId::generate(),
        itinerary: itinerary.clone(),
        agent: AgentName::new(agent),
        pipeline: None,
        model: None,
        effort: None,
        context: None,
        outcome,
        started_at,
        finished_at: finished.then(|| started_at.checked_add(1_i32.minute()).expect("in range")),
        usd,
        source: CostSource::Reported,
        usage: TokenUsage::default(),
        exit_code: None,
        detail: None,
        blocked_on: None,
        pid: None,
        queued_at: None,
        flags: std::collections::BTreeMap::new(),
        continues: None,
        sent_by: None,
    }
}

fn queued_for(itinerary: &ItineraryId) -> Queued {
    Queued::new(
        Flight::new(
            itinerary.clone(),
            Origin::Human,
            AgentName::new("worker"),
            "go",
            3,
        ),
        None,
        BTreeMap::new(),
    )
}

#[test]
fn a_chain_whose_runs_all_succeeded_and_stopped_is_finished() {
    let id = chain();
    let records = vec![
        run(&id, "analyst", Outcome::Succeeded, 0.10, true),
        run(&id, "developer", Outcome::Succeeded, 0.20, true),
    ];

    let built = itineraries(&records, Context::default());

    assert_eq!(built.len(), 1);
    assert_eq!(built[0].state, ItineraryState::Finished);
    assert_eq!(built[0].runs, 2);
    assert!((built[0].usd - 0.30).abs() < f64::EPSILON);
}

#[test]
fn a_stalled_chain_is_not_reported_as_finished_even_though_every_run_succeeded() {
    // The whole reason this view exists. Without the stall record these runs are
    // indistinguishable from a chain that completed.
    let id = chain();
    let records = vec![
        run(&id, "tester", Outcome::Succeeded, 0.10, true),
        run(&id, "reviewer", Outcome::Succeeded, 0.10, true),
    ];
    let stalls = vec![Stall::new(
        id.clone(),
        AgentName::new("publisher"),
        vec![AgentName::new("reviewer")],
        1,
        Timestamp::now(),
    )];

    let built = itineraries(
        &records,
        Context {
            stalls: &stalls,
            ..Context::default()
        },
    );

    assert_eq!(built[0].state, ItineraryState::Stalled);
    assert!(
        built[0]
            .detail
            .as_deref()
            .is_some_and(|d| d.contains("publisher")),
        "{:?}",
        built[0].detail
    );
}

#[test]
fn a_stall_belonging_to_another_chain_does_not_taint_this_one() {
    let id = chain();
    let records = vec![run(&id, "analyst", Outcome::Succeeded, 0.10, true)];
    let stalls = vec![Stall::new(
        chain(),
        AgentName::new("publisher"),
        vec![AgentName::new("reviewer")],
        1,
        Timestamp::now(),
    )];

    let built = itineraries(
        &records,
        Context {
            stalls: &stalls,
            ..Context::default()
        },
    );

    assert_eq!(built[0].state, ItineraryState::Finished);
}

#[test]
fn a_chain_with_a_run_that_has_not_ended_is_working() {
    let id = chain();
    let records = vec![run(&id, "analyst", Outcome::Succeeded, 0.0, false)];

    let built = itineraries(&records, Context::default());

    assert_eq!(built[0].state, ItineraryState::Working);
    assert_eq!(built[0].finished_at, None, "it has not finished");
}

#[test]
fn a_chain_with_work_still_queued_is_working() {
    let id = chain();
    let records = vec![run(&id, "analyst", Outcome::Succeeded, 0.10, true)];

    let built = itineraries(
        &records,
        Context {
            pending: &[queued_for(&id)],
            ..Context::default()
        },
    );

    assert_eq!(built[0].state, ItineraryState::Working);
}

#[test]
fn queued_work_under_a_ground_stop_is_halted_rather_than_working() {
    // A stopped factory should not look busy. Nothing is going to run.
    let id = chain();
    let records = vec![run(&id, "analyst", Outcome::Succeeded, 0.10, true)];

    let built = itineraries(
        &records,
        Context {
            pending: &[queued_for(&id)],
            ground_stop: true,
            ..Context::default()
        },
    );

    assert_eq!(built[0].state, ItineraryState::Halted);
}

#[test]
fn a_chain_whose_run_was_halted_says_so_rather_than_looking_finished() {
    let id = chain();
    let records = vec![run(&id, "analyst", Outcome::Halted, 0.10, true)];

    let built = itineraries(&records, Context::default());

    assert_eq!(built[0].state, ItineraryState::Halted);
    assert!(built[0].detail.is_some(), "a halt is not self-evident");
}

#[test]
fn a_total_built_partly_from_silence_is_marked_unmeasured() {
    // "$0.30" from one reported run and one silent one is a floor, not a figure.
    let id = chain();
    let mut silent = run(&id, "developer", Outcome::Succeeded, 0.0, true);
    silent.source = CostSource::Unreported;

    let records = vec![run(&id, "analyst", Outcome::Succeeded, 0.30, true), silent];
    let built = itineraries(&records, Context::default());

    assert_eq!(
        built[0].measured,
        Some(false),
        "one silent run makes the total a floor"
    );
}

#[test]
fn agents_are_listed_in_the_order_they_first_ran() {
    // Alphabetical would put the publisher before the analyst, which is not how anybody
    // reading a chain thinks about it.
    let id = chain();
    let mut first = run(&id, "analyst", Outcome::Succeeded, 0.0, true);
    first.started_at = Timestamp::now()
        .checked_sub(5_i32.minutes())
        .expect("in range");
    let second = run(&id, "developer", Outcome::Succeeded, 0.0, true);

    let built = itineraries(&[second, first], Context::default());

    assert_eq!(
        built[0].agents.as_deref(),
        Some(["analyst".to_owned(), "developer".to_owned()].as_slice())
    );
}

#[test]
fn an_agent_that_ran_twice_is_named_once() {
    // A develop/test loop runs the same agent repeatedly; listing it four times says nothing.
    let id = chain();
    let records = vec![
        run(&id, "developer", Outcome::Succeeded, 0.0, true),
        run(&id, "developer", Outcome::Succeeded, 0.0, true),
    ];

    let built = itineraries(&records, Context::default());

    assert_eq!(
        built[0].agents.as_deref(),
        Some(["developer".to_owned()].as_slice())
    );
    assert_eq!(built[0].runs, 2, "but both runs are counted");
}

#[test]
fn runs_from_different_chains_are_not_mixed() {
    let first = chain();
    let second = chain();
    let records = vec![
        run(&first, "analyst", Outcome::Succeeded, 0.10, true),
        run(&second, "analyst", Outcome::Succeeded, 0.20, true),
    ];

    let built = itineraries(&records, Context::default());

    assert_eq!(built.len(), 2);
    assert!(built.iter().all(|chain| chain.runs == 1));
}

#[test]
fn the_most_recently_started_chain_comes_first() {
    let older = chain();
    let newer = chain();
    let mut old_run = run(&older, "analyst", Outcome::Succeeded, 0.0, true);
    old_run.started_at = Timestamp::now()
        .checked_sub(1_i32.hour())
        .expect("in range");

    let records = vec![
        old_run,
        run(&newer, "analyst", Outcome::Succeeded, 0.0, true),
    ];
    let built = itineraries(&records, Context::default());

    assert_eq!(built[0].itinerary_id, newer.as_str());
}
