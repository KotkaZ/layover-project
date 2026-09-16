//! Slots: how many runs may be in the air at once.
//!
//! The existing rails each bound a different thing, and none of them bounds this one.
//!
//! | Rail | Bounds |
//! |---|---|
//! | Hops | How **deep** one chain runs |
//! | Fuel | What one chain **spends** |
//! | Run cap | How many runs one chain starts **in total** |
//! | Reserve | What the **factory** spends over a window |
//! | **Slots** | How many runs are live **at this instant** |
//!
//! The gap was found by writing a real pipeline: a scanner that dispatches one reviewer per pull
//! request assigned to you. The configuration is byte-identical whether it finds one PR or fifty,
//! and fifty means fifty headless agent CLIs starting simultaneously on the machine you are also
//! using. Every existing rail permits it. Hops counts depth and this is width; the run cap is
//! cumulative, so fifty at once and fifty over an hour are the same number to it; Fuel eventually
//! halts the chain, but only after the damage, and it halts *arbitrarily* — whichever runs
//! happened to finish first are the ones that got done.
//!
//! # Why queue rather than refuse
//!
//! Every other rail refuses, because every other rail is protecting a budget: once the money is
//! gone, doing the work later does not make it affordable. This one protects a machine, and a
//! machine that is busy now will not be busy in a minute. Refusing would turn "review twelve pull
//! requests" into "review four and silently drop eight", which is the worst possible reading of a
//! concurrency limit.

use std::collections::VecDeque;

use crate::flight::RunId;

/// What happened to a request for a slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Admission {
    /// Start now.
    Cleared,
    /// Wait. The run is queued, and the number is how many are ahead of it.
    Queued {
        /// How many runs are waiting in front of this one.
        ahead: usize,
    },
}

impl Admission {
    /// Returns `true` when the run may start immediately.
    #[must_use]
    pub fn is_cleared(self) -> bool {
        matches!(self, Self::Cleared)
    }
}

/// A bound on how many runs may be live at once, with a queue for the rest.
#[derive(Debug, Clone)]
pub struct Slots {
    capacity: usize,
    live: Vec<RunId>,
    waiting: VecDeque<RunId>,
}

impl Slots {
    /// Creates a flight line with `capacity` slots.
    ///
    /// A capacity of zero is treated as one. A factory that cannot start anything is not a safer
    /// factory, it is a broken one, and the configuration is checked separately.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            live: Vec::new(),
            waiting: VecDeque::new(),
        }
    }

    /// How many runs may be live at once.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// How many are live now.
    #[must_use]
    pub fn live(&self) -> usize {
        self.live.len()
    }

    /// How many are waiting.
    #[must_use]
    pub fn waiting(&self) -> usize {
        self.waiting.len()
    }

    /// Returns `true` when every slot is taken.
    #[must_use]
    pub fn is_full(&self) -> bool {
        self.live.len() >= self.capacity
    }

    /// Returns `true` when this run is currently occupying a slot.
    #[must_use]
    pub fn is_live(&self, run: &RunId) -> bool {
        self.live.contains(run)
    }

    /// Asks for a slot.
    ///
    /// Asking twice for the same run is not an error and does not take a second slot: the Tower
    /// may retry, and a rail that leaked a slot per retry would throttle itself to a standstill
    /// without anything looking wrong.
    pub fn request(&mut self, run: RunId) -> Admission {
        if self.live.contains(&run) {
            return Admission::Cleared;
        }
        if let Some(position) = self.waiting.iter().position(|queued| *queued == run) {
            return Admission::Queued { ahead: position };
        }

        if self.is_full() {
            self.waiting.push_back(run);
            return Admission::Queued {
                ahead: self.waiting.len() - 1,
            };
        }

        self.live.push(run);
        Admission::Cleared
    }

    /// Gives a slot back, returning whichever queued run may now start.
    ///
    /// Releasing a run that was only queued withdraws it instead — a run cancelled while waiting
    /// must not take a slot it no longer needs.
    pub fn release(&mut self, run: &RunId) -> Option<RunId> {
        if let Some(index) = self.waiting.iter().position(|queued| queued == run) {
            self.waiting.remove(index);
            return None;
        }

        let index = self.live.iter().position(|held| held == run)?;
        self.live.remove(index);

        let next = self.waiting.pop_front()?;
        self.live.push(next.clone());
        Some(next)
    }

    /// Everything waiting, in the order it will start.
    pub fn queue(&self) -> impl Iterator<Item = &RunId> {
        self.waiting.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run() -> RunId {
        RunId::generate()
    }

    #[test]
    fn runs_start_immediately_while_there_is_room() {
        let mut slots = Slots::new(3);

        for _ in 0..3 {
            assert_eq!(slots.request(run()), Admission::Cleared);
        }
        assert_eq!(slots.live(), 3);
        assert!(slots.is_full());
    }

    #[test]
    fn the_rest_wait_rather_than_being_turned_away() {
        // The distinction that separates this rail from every other one. Fuel refuses because
        // money spent is gone; a machine that is busy now will not be busy in a minute, so
        // refusing would turn "review twelve pull requests" into "review two and drop ten".
        let mut slots = Slots::new(2);
        slots.request(run());
        slots.request(run());

        assert_eq!(slots.request(run()), Admission::Queued { ahead: 0 });
        assert_eq!(slots.request(run()), Admission::Queued { ahead: 1 });
        assert_eq!(slots.waiting(), 2);
    }

    #[test]
    fn finishing_a_run_lets_the_next_one_in() {
        let mut slots = Slots::new(1);
        let first = run();
        let second = run();
        slots.request(first.clone());
        slots.request(second.clone());

        let started = slots.release(&first);

        assert_eq!(started, Some(second.clone()));
        assert!(slots.is_live(&second));
        assert_eq!(slots.waiting(), 0);
    }

    #[test]
    fn the_queue_is_served_in_order() {
        // Arbitrary order is what makes Fuel exhaustion unsatisfying: which work got done is
        // whatever happened to finish first. A queue should not repeat that.
        let mut slots = Slots::new(1);
        let held = run();
        slots.request(held.clone());

        let queued: Vec<RunId> = (0..3).map(|_| run()).collect();
        for run in &queued {
            slots.request(run.clone());
        }

        assert_eq!(slots.queue().cloned().collect::<Vec<_>>(), queued);
        assert_eq!(slots.release(&held), Some(queued[0].clone()));
    }

    #[test]
    fn asking_twice_does_not_take_two_slots() {
        // The Tower may retry. A rail that leaked a slot per retry would throttle itself to a
        // standstill with nothing looking wrong.
        let mut slots = Slots::new(2);
        let run = run();

        assert_eq!(slots.request(run.clone()), Admission::Cleared);
        assert_eq!(slots.request(run.clone()), Admission::Cleared);
        assert_eq!(slots.live(), 1);
    }

    #[test]
    fn asking_twice_while_queued_reports_the_same_place_in_line() {
        let mut slots = Slots::new(1);
        slots.request(run());
        let waiting = run();

        assert_eq!(
            slots.request(waiting.clone()),
            Admission::Queued { ahead: 0 }
        );
        assert_eq!(slots.request(waiting), Admission::Queued { ahead: 0 });
        assert_eq!(slots.waiting(), 1);
    }

    #[test]
    fn a_run_cancelled_while_waiting_leaves_the_queue_without_taking_a_slot() {
        let mut slots = Slots::new(1);
        let held = run();
        let abandoned = run();
        let next = run();
        slots.request(held.clone());
        slots.request(abandoned.clone());
        slots.request(next.clone());

        assert_eq!(slots.release(&abandoned), None, "it never held a slot");
        assert_eq!(slots.waiting(), 1);
        assert_eq!(slots.release(&held), Some(next));
    }

    #[test]
    fn releasing_something_unknown_changes_nothing() {
        let mut slots = Slots::new(2);
        slots.request(run());

        assert_eq!(slots.release(&run()), None);
        assert_eq!(slots.live(), 1);
    }

    #[test]
    fn a_capacity_of_zero_still_runs_one_at_a_time() {
        // A factory that cannot start anything is not a safer factory, it is a broken one.
        let mut slots = Slots::new(0);

        assert_eq!(slots.capacity(), 1);
        assert!(slots.request(run()).is_cleared());
    }

    #[test]
    fn fifty_arrivals_at_a_capacity_of_four_leave_four_running_and_none_lost() {
        // The scenario that exposed the gap: a scanner dispatching one reviewer per assigned
        // pull request, where the count is not known until it looks.
        let mut slots = Slots::new(4);
        let arrivals: Vec<RunId> = (0..50).map(|_| run()).collect();

        for run in &arrivals {
            slots.request(run.clone());
        }

        assert_eq!(slots.live(), 4);
        assert_eq!(slots.waiting(), 46, "the rest are delayed, not dropped");

        // And the whole queue drains rather than stalling: releasing one live run admits exactly
        // one waiting run, all the way down.
        let mut admitted = 4;
        while slots.waiting() > 0 {
            let holder = arrivals
                .iter()
                .find(|run| slots.is_live(run))
                .cloned()
                .expect("something is live while anything waits");

            assert!(slots.release(&holder).is_some(), "a release must admit one");
            admitted += 1;
            assert!(admitted <= arrivals.len(), "the queue stopped draining");
        }

        assert_eq!(admitted, arrivals.len(), "every arrival eventually started");
    }
}
