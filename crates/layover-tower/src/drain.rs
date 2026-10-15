//! Draining: letting what is running finish while starting nothing new.
//!
//! Not [`Factory::drain`], which runs a queue to the end; this is the Tower winding down.
//!
//! Stopping a Tower used to mean ending its process, which cuts every live run off: the next Tower
//! settles each as `interrupted` and restarts it where its agent allows, repeating the work and the
//! spend, and any arrival parked at a rendezvous is lost with the old Tower's memory. A drain is the
//! other way to stop. Nothing new starts — queued work stays on disk for the next Tower — and the
//! factory is left to go quiet by itself.
//!
//! # The one thing a drain still starts
//!
//! A rendezvous holding an arrival only releases when its other upstreams deliver, and those may
//! still be queued. Holding them would leave the barrier parked for ever, and a parked barrier is
//! lost when the Tower stops. So work that completes a parked rendezvous — a flight to the joined
//! agent, or to an upstream it still waits for, in the same chain — is started during a drain, and
//! everything else waits.
//!
//! # Quiescent
//!
//! No run alive, no flight between leaving the queue and becoming a run, and no rendezvous holding
//! an arrival. Queued work, layovers, help requests and restarts backing off are all on disk, and
//! survive the Tower stopping, so none of them keeps a factory from being quiet.
//!
//! # Cutting off
//!
//! The other way to stop, when waiting is not wanted. Each live run is killed where it stands —
//! its whole process tree, so nothing it started outlives the Tower that could no longer hear from
//! it — and is not written down: its live record stays for the next Tower, which settles it as
//! `interrupted` and restarts it where its agent's `recovery` allows, exactly as if this Tower had
//! gone away. Nothing new starts, not even work for a parked rendezvous.

use std::sync::atomic::Ordering;

use layover_core::agent::AgentName;
use layover_core::queue::Queued;

use crate::factory::Factory;

impl Factory {
    /// Starts draining: from now on nothing new starts except work that completes a parked
    /// rendezvous. What is running carries on.
    pub fn start_draining(&self) {
        self.draining.store(true, Ordering::SeqCst);
    }

    /// Stops draining, and cutting off: queued work starts again as slots free.
    pub fn stop_draining(&self) {
        self.cutting_off.store(false, Ordering::SeqCst);
        self.draining.store(false, Ordering::SeqCst);
    }

    /// Settles the runs this Tower cut off, as the next Tower would have: each is recorded as
    /// `interrupted` and restarted through `queue` where its agent's `recovery` allows. For a Tower
    /// that cut runs off to install a release and then, the install having failed, carries on.
    pub fn settle_cut_off(
        &self,
        queue: &mut dyn FnMut(Queued) -> Result<(), String>,
    ) -> Vec<crate::recovery::Settled> {
        let Some(owner) = self.owner_id().map(str::to_owned) else {
            return Vec::new();
        };
        let alive: std::collections::BTreeSet<_> = self
            .inflight
            .lock()
            .map(|inflight| inflight.keys().cloned().collect())
            .unwrap_or_default();
        let Ok(recorded) = self.live.live() else {
            return Vec::new();
        };
        recorded
            .into_iter()
            .filter(|live| {
                live.owner.as_deref() == Some(owner.as_str()) && !alive.contains(&live.run)
            })
            .map(|live| self.settle_left_behind(&live, queue))
            .collect()
    }

    /// Whether the factory is draining.
    #[must_use]
    pub fn is_draining(&self) -> bool {
        self.draining.load(Ordering::SeqCst)
    }

    /// Cuts off what is running: each live run is killed and left for the next Tower to settle,
    /// and nothing new starts. There is no undoing it; the Tower is expected to exit once
    /// [`Factory::is_idle`].
    pub fn cut_off(&self) {
        self.draining.store(true, Ordering::SeqCst);
        self.cutting_off.store(true, Ordering::SeqCst);
    }

    /// Whether the factory is cutting off what is running.
    #[must_use]
    pub fn is_cutting_off(&self) -> bool {
        self.cutting_off.load(Ordering::SeqCst)
    }

    /// Whether a run killed just now is left for the next Tower rather than recorded: when it was
    /// cut off, and not halted by a Ground Stop, which records what it ends.
    pub(crate) fn leaves_it_to_the_next_tower(&self) -> bool {
        self.is_cutting_off() && !self.ground_stop_engaged()
    }

    /// Whether no run is alive and no flight is on its way to becoming one.
    #[must_use]
    pub fn is_idle(&self) -> bool {
        self.alive_runs() == 0 && self.launching.load(Ordering::SeqCst) == 0
    }

    /// Whether nothing is happening that stopping the Tower would cut off or lose: no run alive, no
    /// flight on its way to becoming one, and no rendezvous holding an arrival.
    #[must_use]
    pub fn is_quiescent(&self) -> bool {
        self.is_idle() && self.barriers.waiting().is_empty()
    }

    /// The agents with a run alive now, for saying what a drain waits for.
    #[must_use]
    pub fn alive_agent_names(&self) -> Vec<AgentName> {
        self.inflight.lock().map_or_else(
            |_| Vec::new(),
            |inflight| inflight.values().map(|run| run.agent.clone()).collect(),
        )
    }

    /// How many rendezvous are holding an arrival, for saying what a drain waits for.
    #[must_use]
    pub fn parked_rendezvous(&self) -> usize {
        self.barriers.waiting().len()
    }

    /// Whether `queued` may start while the factory drains: only work that completes a parked
    /// rendezvous of its own chain, and nothing once the factory is cutting off.
    pub(crate) fn may_start_while_draining(&self, queued: &Queued) -> bool {
        !self.is_cutting_off()
            && self.barriers.waiting().iter().any(|(key, missing)| {
                key.itinerary == queued.flight.itinerary
                    && (key.to == queued.flight.to || missing.contains(&queued.flight.to))
            })
    }
}
