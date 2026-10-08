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
//!
//! # The chain that is waiting for you
//!
//! `awaiting_human` is the other state that looks finished and is not. An agent that cannot go on
//! without a person files a fatal help request and ends its run, so every run in the chain ended
//! cleanly and nothing is queued — and nothing will run until somebody answers. The open request is
//! the record that says so.

use std::collections::BTreeMap;

use layover_core::agent::AgentName;
use layover_core::help::HelpRequest;
use layover_core::queue::Queued;
use layover_core::run::{Outcome, RunRecord};
use layover_core::stall::Stall;
use layover_http::{HelpAsk, Itinerary, ItineraryState};

/// What else is known about chains beside their runs: what the Tower gave up on, what is queued,
/// and what agents asked people.
#[derive(Debug, Default, Clone, Copy)]
pub struct Context<'a> {
    /// Chains the Tower gave up on.
    pub stalls: &'a [Stall],
    /// Work waiting to start.
    pub pending: &'a [Queued],
    /// Help requests raised in the window, answered or not.
    pub help: &'a [HelpRequest],
    /// Whether a Ground Stop is engaged.
    pub ground_stop: bool,
}

/// Groups runs into chains and works out what became of each.
///
/// `ground_stop` changes what waiting means: work queued while everything is halted is not about
/// to run, and calling it `working` would make a stopped factory look busy.
#[must_use]
pub fn itineraries(records: &[RunRecord], context: Context<'_>) -> Vec<Itinerary> {
    let mut chains: BTreeMap<String, Vec<&RunRecord>> = BTreeMap::new();
    for record in records {
        chains
            .entry(record.itinerary.as_str().to_owned())
            .or_default()
            .push(record);
    }

    // A chain whose first flight is still queued has no run to be grouped from, and it is exactly
    // the chain somebody looks for after triggering a workflow while every slot is taken.
    let mut unstarted: BTreeMap<&str, Vec<&Queued>> = BTreeMap::new();
    for queued in context.pending {
        let id = queued.flight.itinerary.as_str();
        if !chains.contains_key(id) {
            unstarted.entry(id).or_default().push(queued);
        }
    }

    let mut built: Vec<Itinerary> = chains
        .into_iter()
        .map(|(id, mut runs)| {
            runs.sort_by_key(|record| record.started_at);
            build(&id, &runs, records, context)
        })
        .collect();
    built.extend(
        unstarted
            .into_iter()
            .map(|(id, queued)| not_started(id, &queued, context.ground_stop)),
    );

    // Most recently started first: a dashboard is read from the top, and the thing somebody wants
    // is almost always the thing that just happened.
    built.sort_by(|a, b| b.started_at.cmp(&a.started_at));
    built
}

/// A chain with flights queued and nothing run yet: `working`, unless a Ground Stop means none of
/// it is about to start.
fn not_started(id: &str, queued: &[&Queued], ground_stop: bool) -> Itinerary {
    let state = state_of(false, Activity::Waiting, None, false, ground_stop);
    let opened = queued
        .iter()
        .map(|queued| queued.flight.sent_at)
        .min()
        .unwrap_or_else(jiff::Timestamp::now);

    Itinerary {
        itinerary_id: id.to_owned(),
        pipeline: queued
            .iter()
            .find_map(|queued| queued.pipeline.as_ref().map(ToString::to_string)),
        name: queued.iter().find_map(|queued| queued.chosen.name.clone()),
        state,
        agents: Some(Vec::new()),
        runs: 0,
        usd: 0.0,
        measured: Some(true),
        started_at: opened.to_string(),
        finished_at: None,
        detail: detail_for(state, None, None, &[]),
        waiting_for: None,
        flags: queued
            .iter()
            .find(|queued| queued.pipeline.is_some())
            .map(|queued| queued.flags.clone()),
        continues: queued
            .iter()
            .find_map(|queued| queued.continues.as_ref())
            .map(|earlier| earlier.as_str().to_owned()),
        continued_by: None,
        running: None,
        queued: names(queued.iter().map(|queued| &queued.flight.to)),
    }
}

/// Agent names in the order first met, once each, or `None` when there are none.
fn names<'a>(agents: impl Iterator<Item = &'a AgentName>) -> Option<Vec<String>> {
    let mut found: Vec<String> = Vec::new();
    for agent in agents {
        let name = agent.to_string();
        if !found.contains(&name) {
            found.push(name);
        }
    }
    (!found.is_empty()).then_some(found)
}

/// What a chain is called: from its runs, or — for a chain whose runs were recorded before names
/// were kept, or have not been recorded yet — from work of it still queued.
fn name_of(id: &str, runs: &[&RunRecord], pending: &[Queued]) -> Option<String> {
    runs.iter()
        .find_map(|record| record.chain_name.clone())
        .or_else(|| {
            pending
                .iter()
                .filter(|queued| queued.flight.itinerary.as_str() == id)
                .find_map(|queued| queued.chosen.name.clone())
        })
}

fn build(
    id: &str,
    runs: &[&RunRecord],
    everything: &[RunRecord],
    context: Context<'_>,
) -> Itinerary {
    let Context {
        stalls,
        pending,
        help,
        ground_stop,
    } = context;
    let stall = stalls.iter().find(|stall| stall.itinerary.as_str() == id);

    // Asked by the run that ended the chain, fatally, and nobody has answered: the chain stopped
    // to wait for a person. A request from an earlier run that the chain went on past is not this.
    let last = runs.last().map(|record| &record.run);
    let asked = help.iter().find(|request| {
        request.itinerary.as_str() == id
            && Some(&request.run) == last
            && request.fatal
            && request.is_open()
    });

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
        asked,
        runs.iter().any(|record| record.outcome == Outcome::Halted),
        ground_stop,
    );

    // Both directions of a reply: the chain this one continues, and every chain that continues
    // it, whether its runs have started yet or it is only the answer on a request.
    let continued_by = continued_by(id, help, everything);

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
    let measured = runs.iter().all(|record| record.source.is_measured());

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
        name: name_of(id, runs, pending),
        state,
        agents: Some(agents),
        runs: i32::try_from(runs.len()).unwrap_or(i32::MAX),
        usd: runs.iter().map(|record| record.usd).sum(),
        measured: Some(measured),
        started_at,
        finished_at,
        detail: detail_for(state, stall, asked, runs).or_else(|| {
            (!continued_by.is_empty()).then(|| format!("continued as {}", continued_by.join(", ")))
        }),
        waiting_for: asked.map(|request| HelpAsk {
            run_id: request.run.to_string(),
            agent: request.agent.to_string(),
            summary: request.summary.clone(),
        }),
        flags: runs
            .iter()
            .find(|record| !record.flags.is_empty())
            .map(|record| record.flags.clone()),
        continues: runs
            .iter()
            .find_map(|record| record.continues.as_ref())
            .map(|earlier| earlier.as_str().to_owned()),
        continued_by: (!continued_by.is_empty()).then_some(continued_by),
        running: names(
            runs.iter()
                .filter(|record| record.finished_at.is_none())
                .map(|record| &record.agent),
        ),
        queued: names(
            pending
                .iter()
                .filter(|queued| queued.flight.itinerary.as_str() == id)
                .map(|queued| &queued.flight.to),
        ),
    }
}

/// Every chain that continues `id`: the chains replies to its help requests started, and any run
/// that says its chain continues this one.
fn continued_by(id: &str, help: &[HelpRequest], everything: &[RunRecord]) -> Vec<String> {
    let mut found: Vec<String> = help
        .iter()
        .filter(|request| request.itinerary.as_str() == id)
        .filter_map(|request| request.reply.as_ref())
        .map(|reply| reply.itinerary.as_str().to_owned())
        .chain(
            everything
                .iter()
                .filter(|record| {
                    record
                        .continues
                        .as_ref()
                        .is_some_and(|earlier| earlier.as_str() == id)
                })
                .map(|record| record.itinerary.as_str().to_owned()),
        )
        .collect();
    found.sort();
    found.dedup();
    found
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
    asked: Option<&HelpRequest>,
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
        // Before a halt: whatever stopped the factory, this chain needs a person either way.
        Activity::Quiet if asked.is_some() => ItineraryState::AwaitingHuman,
        Activity::Quiet if halted_run => ItineraryState::Halted,
        Activity::Quiet => ItineraryState::Finished,
    }
}

/// A line explaining a state that is not self-evident.
///
/// `working` and `finished` speak for themselves. The other two happened *to* the chain rather
/// than being something it did, and a list that does not say so reads as though it chose to stop.
fn detail_for(
    state: ItineraryState,
    stall: Option<&Stall>,
    asked: Option<&HelpRequest>,
    runs: &[&RunRecord],
) -> Option<String> {
    match state {
        ItineraryState::Stalled => stall.map(Stall::summary),
        ItineraryState::AwaitingHuman => {
            asked.map(|request| format!("waiting for you: {}", request.summary))
        }
        ItineraryState::Halted => runs
            .iter()
            .find(|record| record.outcome == Outcome::Halted)
            .and_then(|record| record.detail.clone())
            .or_else(|| Some("a Ground Stop is engaged".to_owned())),
        ItineraryState::Working | ItineraryState::Finished => None,
    }
}

#[cfg(test)]
mod tests;
