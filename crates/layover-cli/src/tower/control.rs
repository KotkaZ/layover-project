//! What `serve` needs of a running Tower to stop it well, or to hand it over to a new release: to
//! drain it, to know when it has gone quiet, to cut it off, and to leave the ticks it held in the
//! queue for whoever runs next. Cheap to clone and shareable across threads, unlike the Tower.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use jiff::Timestamp;
use layover_core::agent::AgentName;
use layover_core::handover::RecoveryPolicy;
use layover_core::pipeline::PipelineName;
use layover_store::Journal;
use layover_tower::{Clock, Factory};

/// A handle on a running Tower.
#[derive(Clone)]
pub struct TowerControl {
    factory: Arc<Factory>,
    clock: Arc<Clock>,
    journal: Arc<Journal>,
    passes: Arc<AtomicU64>,
}

impl TowerControl {
    pub(super) fn new(factory: Arc<Factory>, clock: Arc<Clock>, journal: Arc<Journal>) -> Self {
        Self {
            factory,
            clock,
            journal,
            passes: Arc::new(AtomicU64::new(0)),
        }
    }

    /// The counter the Tower's loop adds one to on every pass.
    pub(super) fn passes(&self) -> Arc<AtomicU64> {
        Arc::clone(&self.passes)
    }

    /// How many passes the Tower's loop has begun.
    pub fn passes_begun(&self) -> u64 {
        self.passes.load(Ordering::Relaxed)
    }

    /// Starts draining: nothing new starts except work that completes a parked rendezvous.
    pub fn start_draining(&self) {
        self.factory.start_draining();
    }

    /// Stops draining: queued work starts again, and ticks held while draining fire.
    pub fn stop_draining(&self) {
        self.factory.stop_draining();
    }

    /// Cuts off what is running, leaving it for the next Tower to settle.
    pub fn cut_off(&self) {
        self.factory.cut_off();
    }

    /// Whether nothing is running, starting, or parked at a rendezvous.
    pub fn is_quiescent(&self) -> bool {
        self.factory.is_quiescent()
    }

    /// Whether nothing is running or starting.
    pub fn is_idle(&self) -> bool {
        self.factory.is_idle()
    }

    /// The agents with a run alive.
    pub fn alive_agents(&self) -> Vec<AgentName> {
        self.factory.alive_agent_names()
    }

    /// The agents with a run alive whose `recovery` is `manual`: work that must not be cut off and
    /// done twice.
    pub fn alive_manual_agents(&self) -> Vec<AgentName> {
        let agents = &self.factory.config().agents;
        self.alive_agents()
            .into_iter()
            .filter(|name| {
                agents
                    .get(name)
                    .is_some_and(|agent| agent.recovery == RecoveryPolicy::Manual)
            })
            .collect()
    }

    /// How many rendezvous hold an arrival.
    pub fn parked_rendezvous(&self) -> usize {
        self.factory.parked_rendezvous()
    }

    /// Settles the runs this Tower cut off, restarting each where its agent's `recovery` allows,
    /// and says what became of each: for a Tower that carries on after an install that failed.
    pub fn settle_cut_off(&self) -> Vec<String> {
        self.factory
            .settle_cut_off(&mut |restart| {
                self.journal
                    .queue(restart)
                    .map_err(|error| error.to_string())
            })
            .iter()
            .map(ToString::to_string)
            .collect()
    }

    /// Queues every scheduled tick that came due while the Tower drained, for the Tower that runs
    /// next, and says which workflows they were.
    pub fn queue_held_ticks(&self, announce: &dyn Fn(String)) -> Vec<PipelineName> {
        super::fire_due_now(
            &self.factory,
            &self.journal,
            &self.clock,
            self.factory.config(),
            Timestamp::now(),
            announce,
        )
    }
}
