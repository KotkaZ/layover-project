//! What the route map shows of what is happening: across a workflow, or in one chain.
//!
//! # Why a workflow's map is not enough
//!
//! Trigger one workflow three times and its map is still one drawing. It colours an agent while
//! *any* of the three runs it, so "the coder is running" is true and says nothing about which
//! chain is where. A workflow's map now counts — `×2` on an agent alive in two chains — and a chain
//! is drawn on its own from what happened in it alone.
//!
//! # What a chain's map cannot show
//!
//! A flight parked at a barrier lives only in the Tower's memory, so a chain's map shows an agent
//! as `queued` (work waiting for a slot) and never as waiting at its barrier. The upstreams that
//! have reported are `done`, which is what is known.

use std::collections::BTreeMap;

use layover_core::agent::AgentName;
use layover_core::cost::Window;
use layover_core::diagram::{Activity, Leg, Live, Sender, Tally};
use layover_core::flight::Origin;
use layover_core::pipeline::PipelineName;
use layover_core::queue::Queued;
use layover_core::run::{Outcome, RunRecord};
use layover_store::RunFilter;

use crate::api::Dashboard;

/// What a chain's map counts.
const IN_THIS_CHAIN: &str = "in this chain";
/// What a workflow's map counts.
const ALIVE_NOW: &str = "alive now";

impl Dashboard {
    /// What a workflow's map — or, with no scope, the whole factory's — shows of the factory now.
    ///
    /// An agent is running while a live record names it, with a count of how many, and failed when
    /// its latest run in the last day failed. A run of another workflow's chain colours nothing
    /// here: the agent may be shared, but the work is not this workflow's. A run whose chain no
    /// workflow opened — a spawned review, say — could be anybody's, so it counts wherever its agent
    /// is drawn.
    pub(crate) fn live(&self, scope: Option<&PipelineName>) -> Live {
        let ours = |pipeline: Option<&PipelineName>| match (scope, pipeline) {
            (Some(wanted), Some(pipeline)) => pipeline == wanted,
            _ => true,
        };
        let history = &self.state().history;
        let span = history.resolve(Window::Last24Hours);
        let mut live = Live::default();

        // Oldest first so that a later run of the same agent overwrites an earlier one: the most
        // recent state is the one worth showing.
        let records = history
            .runs(&span, &RunFilter::default())
            .unwrap_or_default();
        for record in records.into_iter().rev() {
            if !ours(record.pipeline.as_ref()) {
                continue;
            }
            match record.outcome {
                Outcome::Running => {
                    live.activity.insert(record.agent, Activity::Running);
                }
                outcome if outcome.is_failure() => {
                    live.activity.insert(record.agent, Activity::Failed);
                }
                _ => {
                    live.activity.remove(&record.agent);
                }
            }
        }

        // Last, because what is running now outranks how it last ended.
        for record in self.live_records() {
            let pipeline = record
                .queued
                .as_ref()
                .and_then(|queued| queued.pipeline.as_ref());
            if !ours(pipeline) {
                continue;
            }
            live.activity
                .insert(record.agent.clone(), Activity::Running);
            live.tally
                .entry(record.agent)
                .or_insert(Tally {
                    runs: 0,
                    of: ALIVE_NOW,
                })
                .runs += 1;
        }

        live
    }
}

/// One chain drawn on its own: how each agent fared in it, how often it ran, and the routes its
/// work took.
///
/// `records` are every run of the chain, alive or over; `pending` its queued flights; `pipeline`
/// the workflow that opened it, which is where work from outside the mesh came in.
pub(crate) fn chain(
    records: &[RunRecord],
    pending: &[Queued],
    pipeline: Option<&PipelineName>,
) -> Live {
    let mut live = Live::default();
    // Work from outside the mesh came in by the workflow's way in, when the chain has one.
    let way_in = pipeline.map(|pipeline| Sender::Pipeline(pipeline.clone()));
    let mut legs: Vec<Leg> = Vec::new();

    let mut latest: BTreeMap<&AgentName, &RunRecord> = BTreeMap::new();
    for record in records {
        live.tally
            .entry(record.agent.clone())
            .or_insert(Tally {
                runs: 0,
                of: IN_THIS_CHAIN,
            })
            .runs += 1;
        let seen = latest.entry(&record.agent).or_insert(record);
        if record.started_at >= seen.started_at {
            *seen = record;
        }

        // Not recorded lights nothing: a route guessed from who happened to run before would be
        // drawn exactly like one that was taken.
        let senders: Vec<Sender> = match record.sent_by.as_deref() {
            None => Vec::new(),
            Some([]) => way_in.iter().cloned().collect(),
            Some(agents) => agents.iter().cloned().map(Sender::Agent).collect(),
        };
        legs.extend(senders.into_iter().map(|from| Leg {
            from,
            to: record.agent.clone(),
        }));
    }

    for (agent, record) in latest {
        let state = match record.outcome {
            Outcome::Running => Activity::Running,
            Outcome::Succeeded => Activity::Done,
            // Halted is not the agent's failure, but it is the chain's: the step did not happen.
            Outcome::Failed | Outcome::TimedOut | Outcome::Interrupted | Outcome::Halted => {
                Activity::Failed
            }
        };
        live.activity.insert(agent.clone(), state);
    }
    // A run alive anywhere in the chain is where the chain is, whatever ran after it began.
    for record in records.iter().filter(|r| r.outcome == Outcome::Running) {
        live.activity
            .insert(record.agent.clone(), Activity::Running);
    }

    // What is queued is what happens next, so it outranks how an agent's last run here went.
    for queued in pending {
        let to = &queued.flight.to;
        if live.activity.get(to) != Some(&Activity::Running) {
            live.activity.insert(to.clone(), Activity::Queued);
        }
        let from = match &queued.flight.from {
            Origin::Agent(sender) => Some(Sender::Agent(sender.clone())),
            Origin::Human | Origin::Schedule(_) | Origin::Resumed(_) => way_in.clone(),
        };
        legs.extend(from.map(|from| Leg {
            from,
            to: to.clone(),
        }));
    }

    live.travelled.extend(legs);
    live
}
