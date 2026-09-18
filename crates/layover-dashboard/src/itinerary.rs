//! Turning a list of runs into the chains they belonged to.
//!
//! # Why a chain is not just a group of runs
//!
//! The dashboard already lists runs, and a run is a real thing: one agent, one process, one
//! outcome. But every safety rail in Layover is per *chain* — Hops, Fuel, the run cap are shared
//! by everything one trigger caused — so "what did this cost" and "did this finish" are questions
//! about a chain, not a run.
//!
//! # The state that cannot be inferred
//!
//! Three of the four states here are read off the runs. `stalled` is not, and that is the point.
//!
//! A stalled chain is one where a joined agent never woke because the barrier it was waiting
//! behind could no longer be completed. Every run in it *succeeded*: the tester reported, the
//! reviewer reported, and then nothing. Read back from history it is indistinguishable from a
//! chain that finished, because there is no failed run to point at and no record that the last
//! step never happened.
//!
//! So the Tower writes a [`Stall`] when it gives up, and that record is what makes the difference
//! visible. Without it this whole view would quietly report broken work as complete.

use std::collections::BTreeMap;

use layover_core::cost::CostSource;
use layover_core::queue::Queued;
use layover_core::run::{Outcome, RunRecord};
use layover_core::stall::Stall;
use layover_http::{Itinerary, ItineraryState};

/// Groups runs into chains and works out what became of each.
///
/// `ground_stop` changes what waiting means: work queued while everything is halted is not about
/// to run, and calling it `working` would make a stopped factory look busy.
#[must_use]
pub fn itineraries(
    records: &[RunRecord],
    stalls: &[Stall],
    pending: &[Queued],
    ground_stop: bool,
) -> Vec<Itinerary> {
    let mut chains: BTreeMap<String, Vec<&RunRecord>> = BTreeMap::new();
    for record in records {
        chains
            .entry(record.itinerary.as_str().to_owned())
            .or_default()
            .push(record);
    }

    let mut built: Vec<Itinerary> = chains
        .into_iter()
        .map(|(id, mut runs)| {
            runs.sort_by_key(|record| record.started_at);
            build(&id, &runs, stalls, pending, ground_stop)
        })
        .collect();

    // Most recently started first: a dashboard is read from the top, and the thing somebody wants
    // is almost always the thing that just happened.
    built.sort_by(|a, b| b.started_at.cmp(&a.started_at));
    built
}

fn build(
    id: &str,
    runs: &[&RunRecord],
    stalls: &[Stall],
    pending: &[Queued],
    ground_stop: bool,
) -> Itinerary {
    let stall = stalls.iter().find(|stall| stall.itinerary.as_str() == id);

    let activity = if runs.iter().any(|record| record.finished_at.is_none()) {
        Activity::Running
    } else if pending
        .iter()
        .any(|queued| queued.flight.itinerary.as_str() == id)
    {
        Activity::Waiting
    } else {
        Activity::Quiet
    };

    let state = state_of(
        stall.is_some(),
        activity,
        runs.iter().any(|record| record.outcome == Outcome::Halted),
        ground_stop,
    );

    // Named in the order they first ran, which is the order somebody reading the chain thinks
    // about it. Alphabetical would put the publisher before the analyst.
    let mut agents: Vec<String> = Vec::new();
    for record in runs {
        let name = record.agent.to_string();
        if !agents.contains(&name) {
            agents.push(name);
        }
    }

    // A total built partly from silence is a floor, not a figure. Saying so is the difference
    // between a number somebody can plan against and one they should not.
    let measured = runs
        .iter()
        .all(|record| record.source == CostSource::Reported);

    let started_at = runs.first().map_or_else(
        || jiff::Timestamp::now().to_string(),
        |r| r.started_at.to_string(),
    );

    let finished_at = if activity == Activity::Quiet {
        runs.iter()
            .filter_map(|record| record.finished_at)
            .max()
            .map(|at| at.to_string())
    } else {
        None
    };

    Itinerary {
        itinerary_id: id.to_owned(),
        pipeline: runs
            .iter()
            .find_map(|r| r.pipeline.as_ref().map(ToString::to_string)),
        state,
        agents: Some(agents),
        runs: i32::try_from(runs.len()).unwrap_or(i32::MAX),
        usd: runs.iter().map(|record| record.usd).sum(),
        measured: Some(measured),
        started_at,
        finished_at,
        detail: detail_for(state, stall, runs),
    }
}

/// Whether anything is still happening in a chain.
///
/// An enum rather than two booleans because "running" and "waiting" are mutually exclusive
/// answers to one question, and a pair of flags admits a fourth state that means nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Activity {
    /// A run in it has not ended.
    Running,
    /// Nothing is running, but a flight for it is queued.
    Waiting,
    /// Nothing is running and nothing is waiting.
    Quiet,
}

/// What became of a chain, decided in the order the answers matter.
///
/// A stall is definitive and comes first: the Tower said this chain is over, and nothing observed
/// about the runs can override that. A running chain beats everything else because it is
/// demonstrably still going. Only then does waiting work get interpreted, and what it means
/// depends on whether anything is allowed to start.
fn state_of(
    stalled: bool,
    activity: Activity,
    halted_run: bool,
    ground_stop: bool,
) -> ItineraryState {
    if stalled {
        return ItineraryState::Stalled;
    }

    match activity {
        // Waiting under a Ground Stop is not working. Nothing is going to run, and a stopped
        // factory that looks busy is a stopped factory somebody walks away from.
        Activity::Waiting if ground_stop => ItineraryState::Halted,
        Activity::Running | Activity::Waiting => ItineraryState::Working,
        Activity::Quiet if halted_run => ItineraryState::Halted,
        Activity::Quiet => ItineraryState::Finished,
    }
}

/// A line explaining a state that is not self-evident.
///
/// `working` and `finished` speak for themselves. The other two happened *to* the chain rather
/// than being something it did, and a list that does not say so reads as though it chose to stop.
fn detail_for(state: ItineraryState, stall: Option<&Stall>, runs: &[&RunRecord]) -> Option<String> {
    match state {
        ItineraryState::Stalled => stall.map(Stall::summary),
        ItineraryState::Halted => runs
            .iter()
            .find(|record| record.outcome == Outcome::Halted)
            .and_then(|record| record.detail.clone())
            .or_else(|| Some("a Ground Stop is engaged".to_owned())),
        ItineraryState::Working | ItineraryState::Finished => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::{Timestamp, ToSpan as _};
    use layover_core::agent::AgentName;
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
            outcome,
            started_at,
            finished_at: finished
                .then(|| started_at.checked_add(1_i32.minute()).expect("in range")),
            usd,
            source: CostSource::Reported,
            usage: TokenUsage::default(),
            detail: None,
            blocked_on: None,
            pid: None,
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

        let built = itineraries(&records, &[], &[], false);

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

        let built = itineraries(&records, &stalls, &[], false);

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

        let built = itineraries(&records, &stalls, &[], false);

        assert_eq!(built[0].state, ItineraryState::Finished);
    }

    #[test]
    fn a_chain_with_a_run_that_has_not_ended_is_working() {
        let id = chain();
        let records = vec![run(&id, "analyst", Outcome::Succeeded, 0.0, false)];

        let built = itineraries(&records, &[], &[], false);

        assert_eq!(built[0].state, ItineraryState::Working);
        assert_eq!(built[0].finished_at, None, "it has not finished");
    }

    #[test]
    fn a_chain_with_work_still_queued_is_working() {
        let id = chain();
        let records = vec![run(&id, "analyst", Outcome::Succeeded, 0.10, true)];

        let built = itineraries(&records, &[], &[queued_for(&id)], false);

        assert_eq!(built[0].state, ItineraryState::Working);
    }

    #[test]
    fn queued_work_under_a_ground_stop_is_halted_rather_than_working() {
        // A stopped factory should not look busy. Nothing is going to run.
        let id = chain();
        let records = vec![run(&id, "analyst", Outcome::Succeeded, 0.10, true)];

        let built = itineraries(&records, &[], &[queued_for(&id)], true);

        assert_eq!(built[0].state, ItineraryState::Halted);
    }

    #[test]
    fn a_chain_whose_run_was_halted_says_so_rather_than_looking_finished() {
        let id = chain();
        let records = vec![run(&id, "analyst", Outcome::Halted, 0.10, true)];

        let built = itineraries(&records, &[], &[], false);

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
        let built = itineraries(&records, &[], &[], false);

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

        let built = itineraries(&[second, first], &[], &[], false);

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

        let built = itineraries(&records, &[], &[], false);

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

        let built = itineraries(&records, &[], &[], false);

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
        let built = itineraries(&records, &[], &[], false);

        assert_eq!(built[0].itinerary_id, newer.as_str());
    }
}
