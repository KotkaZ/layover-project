//! Hops, Fuel and the deterministic run cap.
//!
//! These are the rails that keep an unattended factory from running away. The distinction that
//! matters: **Hops bounds depth, not breadth.** A hop is spent per flight and branches inherit
//! the remaining count rather than splitting it, so a branching factor of `b` at `max_hops = h`
//! permits up to `b^h` runs. Fuel is what bounds total work, and the run cap is the deterministic
//! backstop for when a runner does not report its cost.

use crate::flight::ItineraryId;

/// Why the Tower refused to start more work on an itinerary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Denial {
    /// The chain has reached its maximum depth.
    #[error("hops exhausted: the chain reached its maximum depth")]
    HopsExhausted,
    /// The itinerary has spent its shared budget.
    #[error("fuel exhausted: the itinerary spent its budget")]
    FuelExhausted,
    /// The itinerary has started as many runs as it is allowed.
    #[error("run cap reached: the itinerary started its maximum number of runs")]
    RunCapReached,
    /// The whole factory has spent its rolling-window budget.
    ///
    /// Distinct from [`Denial::FuelExhausted`]: this itinerary may have plenty of Fuel left, and
    /// still be refused because everything else running has drained the shared Reserve.
    #[error("reserve exhausted: the factory spent its budget for the current window")]
    ReserveExhausted,
    /// Spawning would open an itinerary too many generations from the trigger that began it.
    #[error("spawn depth reached: already {max_generations} generation(s) from the trigger")]
    SpawnDepthReached {
        /// How many generations of spawning are permitted.
        max_generations: u32,
    },
}

/// Accounting for one causal chain of flights.
#[derive(Debug, Clone)]
pub struct Itinerary {
    id: ItineraryId,
    max_hops: u32,
    fuel_budget_usd: f64,
    fuel_spent_usd: f64,
    max_runs: u32,
    runs_started: u32,
    unreported_runs: u32,
    generation: u32,
}

impl Itinerary {
    /// Opens an itinerary with the given bounds.
    #[must_use]
    pub fn new(id: ItineraryId, max_hops: u32, fuel_budget_usd: f64, max_runs: u32) -> Self {
        Self {
            id,
            max_hops,
            fuel_budget_usd,
            fuel_spent_usd: 0.0,
            max_runs,
            runs_started: 0,
            unreported_runs: 0,
            generation: 0,
        }
    }

    /// How many spawns separate this itinerary from one a human or a schedule started.
    ///
    /// Zero for a chain a trigger opened. One for a chain an agent spawned, and so on.
    #[must_use]
    pub fn generation(&self) -> u32 {
        self.generation
    }

    /// Opens a sibling itinerary spawned from this one.
    ///
    /// The generation is the only thing that carries across, and it is the whole point. A spawned
    /// itinerary gets *fresh* Hops, fresh Fuel and a fresh run cap — that is what makes per-item
    /// work affordable, and it is also exactly what makes spawning unbounded: Hops counts depth
    /// within a chain and cannot see across chains, so an agent that spawns an agent that spawns
    /// an agent recurses forever while every individual chain stays perfectly inside its rails.
    ///
    /// Generation is the rail that closes that. It is Hops, one level up.
    ///
    /// # Errors
    ///
    /// Returns [`Denial::SpawnDepthReached`] once `max_generations` spawns separate this chain
    /// from the trigger that began it.
    pub fn spawn(
        &self,
        id: ItineraryId,
        fuel_budget_usd: f64,
        max_generations: u32,
    ) -> Result<Self, Denial> {
        let generation = self.generation.saturating_add(1);
        if generation > max_generations {
            return Err(Denial::SpawnDepthReached { max_generations });
        }

        Ok(Self {
            id,
            max_hops: self.max_hops,
            fuel_budget_usd,
            fuel_spent_usd: 0.0,
            max_runs: self.max_runs,
            runs_started: 0,
            unreported_runs: 0,
            generation,
        })
    }

    /// Returns the itinerary identifier.
    #[must_use]
    pub fn id(&self) -> &ItineraryId {
        &self.id
    }

    /// Hops carried by the first flight of this itinerary.
    #[must_use]
    pub fn initial_hops(&self) -> u32 {
        self.max_hops
    }

    /// Decides whether a run holding `parent_hops` may send a further flight.
    ///
    /// Returns the hop count the new flight should carry.
    ///
    /// # Errors
    ///
    /// Returns [`Denial::HopsExhausted`] when the chain has reached its depth limit,
    /// [`Denial::FuelExhausted`] when the shared budget is spent, or [`Denial::RunCapReached`]
    /// when the itinerary has already started its maximum number of runs.
    pub fn authorize_send(&self, parent_hops: u32) -> Result<u32, Denial> {
        let next = parent_hops.checked_sub(1).ok_or(Denial::HopsExhausted)?;
        if next == 0 {
            return Err(Denial::HopsExhausted);
        }
        if self.fuel_exhausted() {
            return Err(Denial::FuelExhausted);
        }
        if self.runs_started >= self.max_runs {
            return Err(Denial::RunCapReached);
        }
        Ok(next)
    }

    /// Records that a run has been started against this itinerary.
    ///
    /// # Errors
    ///
    /// Returns [`Denial::RunCapReached`] if the cap has already been reached, or
    /// [`Denial::FuelExhausted`] if the budget is spent.
    pub fn record_run_started(&mut self) -> Result<(), Denial> {
        if self.fuel_exhausted() {
            return Err(Denial::FuelExhausted);
        }
        if self.runs_started >= self.max_runs {
            return Err(Denial::RunCapReached);
        }
        self.runs_started += 1;
        Ok(())
    }

    /// Debits reported cost against the shared budget.
    ///
    /// Negative and non-finite values are ignored rather than trusted, because the figure comes
    /// from parsing a child process's output.
    pub fn debit_fuel(&mut self, usd: f64) {
        if usd.is_finite() && usd > 0.0 {
            self.fuel_spent_usd += usd;
        }
    }

    /// Records that a completed run reported no cost.
    ///
    /// Fuel is the only bound on breadth, so a runner that reports nothing would otherwise let
    /// the rail disappear silently while still appearing to be enforced. The Tower is expected to
    /// surface this, and the run cap is what actually holds in its absence.
    ///
    /// A count rather than a flag: "three of forty runs went unmetered" and "every run went
    /// unmetered" are the difference between a gap and a broken rail, and a boolean cannot tell
    /// them apart.
    pub fn note_unreported_cost(&mut self) {
        self.unreported_runs = self.unreported_runs.saturating_add(1);
    }

    /// Returns `true` if any run has completed without reporting its cost.
    #[must_use]
    pub fn has_cost_reporting_gap(&self) -> bool {
        self.unreported_runs > 0
    }

    /// How many completed runs reported no cost.
    #[must_use]
    pub fn unreported_runs(&self) -> u32 {
        self.unreported_runs
    }

    /// Fraction of started runs whose cost was actually reported, from 0.0 to 1.0.
    ///
    /// This is what says whether Fuel is metering the itinerary or merely appearing to. A value
    /// below 1.0 means the remaining budget is an upper bound, not a measurement.
    #[must_use]
    pub fn metered_share(&self) -> f64 {
        if self.runs_started == 0 {
            return 1.0;
        }
        f64::from(self.runs_started.saturating_sub(self.unreported_runs))
            / f64::from(self.runs_started)
    }

    /// Returns `true` once the shared budget is spent.
    #[must_use]
    pub fn fuel_exhausted(&self) -> bool {
        self.fuel_spent_usd >= self.fuel_budget_usd
    }

    /// Budget left, never negative.
    #[must_use]
    pub fn fuel_remaining_usd(&self) -> f64 {
        (self.fuel_budget_usd - self.fuel_spent_usd).max(0.0)
    }

    /// Number of runs started so far.
    #[must_use]
    pub fn runs_started(&self) -> u32 {
        self.runs_started
    }

    /// Runs still permitted before the deterministic cap bites.
    #[must_use]
    pub fn runs_remaining(&self) -> u32 {
        self.max_runs.saturating_sub(self.runs_started)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_trigger_opens_generation_zero() {
        let itinerary = Itinerary::new(ItineraryId::generate(), 8, 5.0, 64);

        assert_eq!(itinerary.generation(), 0);
    }

    #[test]
    fn a_spawned_itinerary_gets_its_own_budget_and_the_next_generation() {
        // Fresh Fuel is the point: per-item work wants per-item budget, so that reviewing twelve
        // pull requests does not become reviewing four and halting.
        let parent = Itinerary::new(ItineraryId::generate(), 8, 5.0, 64);

        let child = parent
            .spawn(ItineraryId::generate(), 3.0, 2)
            .expect("one generation is within the limit");

        assert_eq!(child.generation(), 1);
        assert_eq!(
            child.initial_hops(),
            8,
            "fresh Hops, not the parent's remainder"
        );
        assert!((child.fuel_remaining_usd() - 3.0).abs() < 1e-9);
        assert_eq!(child.runs_remaining(), 64);
    }

    #[test]
    fn spawning_is_bounded_even_though_every_chain_stays_inside_its_own_rails() {
        // The hazard that generation exists for. A spawned itinerary gets fresh Hops, so Hops
        // cannot see across chains: an agent that spawns an agent that spawns an agent recurses
        // forever while each individual chain looks perfectly well behaved.
        let mut current = Itinerary::new(ItineraryId::generate(), 8, 5.0, 64);

        for expected in 1..=2 {
            current = current
                .spawn(ItineraryId::generate(), 5.0, 2)
                .expect("within the limit");
            assert_eq!(current.generation(), expected);
        }

        assert_eq!(
            current.spawn(ItineraryId::generate(), 5.0, 2).unwrap_err(),
            Denial::SpawnDepthReached { max_generations: 2 }
        );
    }

    #[test]
    fn a_factory_that_forbids_spawning_says_so_on_the_first_attempt() {
        let itinerary = Itinerary::new(ItineraryId::generate(), 8, 5.0, 64);

        assert_eq!(
            itinerary
                .spawn(ItineraryId::generate(), 5.0, 0)
                .unwrap_err(),
            Denial::SpawnDepthReached { max_generations: 0 }
        );
    }

    fn itinerary() -> Itinerary {
        Itinerary::new(ItineraryId::generate(), 8, 5.0, 64)
    }

    /// Fuel is money, so compare it with a tolerance rather than bit-for-bit.
    fn approx(actual: f64, expected: f64) -> bool {
        (actual - expected).abs() < 1e-9
    }

    #[test]
    fn a_flight_costs_one_hop() {
        let it = itinerary();
        assert_eq!(it.authorize_send(8), Ok(7));
        assert_eq!(it.authorize_send(7), Ok(6));
    }

    #[test]
    fn the_last_hop_cannot_be_spent() {
        let it = itinerary();
        assert_eq!(it.authorize_send(1), Err(Denial::HopsExhausted));
        assert_eq!(it.authorize_send(0), Err(Denial::HopsExhausted));
    }

    #[test]
    fn spent_fuel_stops_the_chain() {
        let mut it = itinerary();
        it.debit_fuel(5.0);

        assert!(it.fuel_exhausted());
        assert_eq!(it.authorize_send(8), Err(Denial::FuelExhausted));
        assert!(approx(it.fuel_remaining_usd(), 0.0));
    }

    #[test]
    fn implausible_cost_reports_are_ignored() {
        let mut it = itinerary();
        it.debit_fuel(-100.0);
        it.debit_fuel(f64::NAN);
        it.debit_fuel(f64::INFINITY);

        assert!(approx(it.fuel_remaining_usd(), 5.0));
    }

    #[test]
    fn hops_alone_do_not_bound_a_fan_out() {
        // The finding that moved Fuel into v0.1: hops bound depth, not breadth. Simulate a
        // branching factor of three and count how many runs eight hops would permit.
        let branching = 3_u32;
        let mut runs_at_depth = 1_u32;
        let mut total = 0_u32;
        let mut hops = 8_u32;

        while hops > 1 {
            runs_at_depth *= branching;
            total += runs_at_depth;
            hops -= 1;
        }

        assert_eq!(total, 3_279);
        assert!(
            total > 64,
            "hops alone permit far more runs than the deterministic cap allows"
        );
    }

    #[test]
    fn the_run_cap_bounds_breadth_without_any_cost_reporting() {
        let mut it = Itinerary::new(ItineraryId::generate(), 8, 5.0, 3);

        for _ in 0..3 {
            it.record_run_started().expect("within cap");
        }

        assert_eq!(it.record_run_started(), Err(Denial::RunCapReached));
        assert_eq!(it.authorize_send(8), Err(Denial::RunCapReached));
        assert_eq!(it.runs_remaining(), 0);
        assert!(!it.fuel_exhausted(), "the cap held with fuel untouched");
    }

    #[test]
    fn a_missing_cost_report_is_remembered() {
        let mut it = itinerary();
        assert!(!it.has_cost_reporting_gap());

        it.note_unreported_cost();

        assert!(it.has_cost_reporting_gap());
    }
}
