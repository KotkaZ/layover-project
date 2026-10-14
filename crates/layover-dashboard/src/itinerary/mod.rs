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
//!
//! # The chain that died
//!
//! `failed` is the plainest of the three, and was the easiest to miss. A run fails — its tests, the
//! model service, its timeout — and so never sends the flight that would have carried the work on.
//! Nothing is queued, nothing is stalled at a barrier, nothing was refused: the chain simply stops,
//! and without a state of its own it read as `finished`. So a chain whose last run failed, timed out
//! or was interrupted, and that nobody has continued, says so.

use std::collections::BTreeMap;

use layover_core::agent::AgentName;
use layover_core::help::HelpRequest;
use layover_core::pipeline::PipelineName;
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
    /// Workflows a person has switched off: a chain of one that has not started is held.
    pub switched_off: &'a [PipelineName],
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
            .map(|(id, queued)| not_started(id, &queued, context)),
    );

    // Most recently started first: a dashboard is read from the top, and the thing somebody wants
    // is almost always the thing that just happened.
    built.sort_by(|a, b| b.started_at.cmp(&a.started_at));
    built
}

/// A chain with flights queued and nothing run yet: `working`, unless none of it is about to
/// start — a Ground Stop is engaged, or the workflow it opens is switched off.
fn not_started(id: &str, queued: &[&Queued], context: Context<'_>) -> Itinerary {
    let held = queued.iter().find_map(|queued| {
        queued
            .way_in()
            .filter(|pipeline| context.switched_off.contains(pipeline))
    });
    let state = state_of(
        false,
        Activity::Waiting,
        None,
        Ending::Clean,
        context.ground_stop || held.is_some(),
    );
    let detail = match held {
        Some(pipeline) if !context.ground_stop => Some(format!(
            "`{pipeline}` is switched off; this starts once it is switched back on"
        )),
        _ => detail_for(state, None, None, &[], None),
    };
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
        detail,
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
        switched_off: _,
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

    // Both directions of a reply: the chain this one continues, and every chain that continues
    // it, whether its runs have started yet or it is only the answer on a request.
    let continued_by = continued_by(id, help, everything);

    // A chain somebody has continued has been dealt with, however it ended.
    let died = ended_on_failure(runs).filter(|_| continued_by.is_empty());

    let ending = if runs.iter().any(|record| record.outcome == Outcome::Halted) {
        Ending::Halted
    } else if died.is_some() {
        Ending::Failed
    } else {
        Ending::Clean
    };
    let state = state_of(stall.is_some(), activity, asked, ending, ground_stop);

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
        detail: detail_for(state, stall, asked, runs, died)
            .or_else(|| backing_off(id, pending))
            .or_else(|| {
                (!continued_by.is_empty())
                    .then(|| format!("continued as {}", continued_by.join(", ")))
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

/// What a chain waiting only for a restart's back-off is waiting for, so `working` with nothing
/// running does not read as stuck.
fn backing_off(id: &str, pending: &[Queued]) -> Option<String> {
    let now = jiff::Timestamp::now();
    pending
        .iter()
        .filter(|queued| queued.flight.itinerary.as_str() == id && !queued.is_due(now))
        .map(|queued| {
            format!(
                "`{}` restarts after the model service failed — attempt {}, once its wait is over",
                queued.flight.to,
                queued.attempt()
            )
        })
        .next()
}

/// The run a chain ended on, when it ended on one that failed, timed out or was interrupted: the
/// last to finish, so in a fan-out a branch that died while another went on does not count.
fn ended_on_failure<'a>(runs: &[&'a RunRecord]) -> Option<&'a RunRecord> {
    runs.iter()
        .copied()
        .filter(|record| record.finished_at.is_some())
        .max_by_key(|record| (record.finished_at, record.started_at))
        .filter(|record| record.outcome.is_failure())
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

/// How a chain's runs ended, for a chain where nothing is happening.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ending {
    /// Nothing went wrong that the runs can show.
    Clean,
    /// A rail refused one of its runs, or a Ground Stop ended one.
    Halted,
    /// The run it ended on failed, timed out or was interrupted, and nobody has continued it.
    Failed,
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
    ending: Ending,
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
        Activity::Quiet => match ending {
            Ending::Halted => ItineraryState::Halted,
            // Nothing will carry the work on: the run that would have sent it did not finish.
            Ending::Failed => ItineraryState::Failed,
            Ending::Clean => ItineraryState::Finished,
        },
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
    died: Option<&RunRecord>,
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
        ItineraryState::Failed => died.map(|record| {
            let how = record.outcome.slug().replace('_', " ");
            match &record.detail {
                Some(why) => format!("`{}` {how}, and nothing came after it: {why}", record.agent),
                None => format!("`{}` {how}, and nothing came after it", record.agent),
            }
        }),
        ItineraryState::Working | ItineraryState::Finished => None,
    }
}

#[cfg(test)]
mod tests;
