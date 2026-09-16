//! The cost ledger: an append-only record of what every run spent.
//!
//! Fuel answers "may this itinerary continue?" and forgets everything else. The ledger is what
//! makes the answer explicable afterwards — which agent, which model, which chain, and how much
//! of the total is actually measured rather than guessed.
//!
//! Every total carries its own [`CostSource`], taken as the *weakest* source that contributed to
//! it. A figure that is 90% measured and 10% estimated reports as an estimate, because that is
//! what it is.

use jiff::Timestamp;
use std::collections::BTreeMap;

use super::{CostSource, RunCost, TokenUsage};
use crate::agent::AgentName;
use crate::flight::ItineraryId;

/// Totals over some set of runs.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Summary {
    /// How many runs contributed.
    pub runs: u32,
    /// Total cost in US dollars.
    pub usd: f64,
    /// Total tokens.
    pub usage: TokenUsage,
    /// How many runs reported nothing at all.
    pub unreported_runs: u32,
    /// How many runs were priced from a rate card rather than measured.
    pub estimated_runs: u32,
}

impl Summary {
    /// Folds one run into the totals.
    fn add(&mut self, cost: &RunCost) {
        self.runs = self.runs.saturating_add(1);
        self.usd += cost.usd;
        self.usage = self.usage.saturating_add(cost.usage);

        match cost.source {
            CostSource::Reported => {}
            CostSource::RateCard => self.estimated_runs = self.estimated_runs.saturating_add(1),
            CostSource::Unreported => {
                self.unreported_runs = self.unreported_runs.saturating_add(1);
            }
        }
    }

    /// How much of this total is measured rather than inferred.
    ///
    /// The weakest source wins: one unreported run in a hundred makes the whole figure a lower
    /// bound, and saying so is the point.
    #[must_use]
    pub fn confidence(&self) -> CostSource {
        if self.unreported_runs > 0 {
            CostSource::Unreported
        } else if self.estimated_runs > 0 {
            CostSource::RateCard
        } else {
            CostSource::Reported
        }
    }

    /// Fraction of runs whose cost the runner actually reported, from 0.0 to 1.0.
    ///
    /// Returns 1.0 for an empty summary: nothing is unaccounted for when nothing has run.
    #[must_use]
    pub fn measured_share(&self) -> f64 {
        if self.runs == 0 {
            return 1.0;
        }
        let inferred = self.unreported_runs.saturating_add(self.estimated_runs);
        f64::from(self.runs.saturating_sub(inferred)) / f64::from(self.runs)
    }

    /// Returns `true` when every figure in this total came from a runner.
    #[must_use]
    pub fn is_fully_measured(&self) -> bool {
        self.confidence().is_measured()
    }
}

/// An append-only record of run costs.
///
/// Held in memory here; persisting it is the store crate's job. Ordering is insertion order, which
/// is also chronological as long as the Tower records a run when it finishes.
#[derive(Debug, Clone, Default)]
pub struct Ledger {
    entries: Vec<RunCost>,
}

impl Ledger {
    /// Creates an empty ledger.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Records what a run cost.
    pub fn record(&mut self, cost: RunCost) {
        self.entries.push(cost);
    }

    /// Every recorded run, oldest first.
    pub fn entries(&self) -> impl Iterator<Item = &RunCost> {
        self.entries.iter()
    }

    /// Number of recorded runs.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns `true` when nothing has been recorded.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Totals over everything recorded.
    #[must_use]
    pub fn total(&self) -> Summary {
        self.summarise(|_| true)
    }

    /// Totals over the runs matching `keep`.
    #[must_use]
    pub fn summarise(&self, keep: impl Fn(&RunCost) -> bool) -> Summary {
        let mut summary = Summary::default();
        for cost in self.entries.iter().filter(|cost| keep(cost)) {
            summary.add(cost);
        }
        summary
    }

    /// Totals for one chain.
    #[must_use]
    pub fn for_itinerary(&self, itinerary: &ItineraryId) -> Summary {
        self.summarise(|cost| &cost.itinerary == itinerary)
    }

    /// Totals for runs that finished at or after `since`.
    #[must_use]
    pub fn since(&self, since: Timestamp) -> Summary {
        self.summarise(|cost| cost.at >= since)
    }

    /// Totals per agent.
    ///
    /// This is the breakdown that answers "what is expensive here", which a single Fuel figure
    /// never could.
    #[must_use]
    pub fn by_agent(&self) -> BTreeMap<AgentName, Summary> {
        let mut grouped: BTreeMap<AgentName, Summary> = BTreeMap::new();
        for cost in &self.entries {
            grouped.entry(cost.agent.clone()).or_default().add(cost);
        }
        grouped
    }

    /// Totals per model, skipping runs whose model was never reported.
    #[must_use]
    pub fn by_model(&self) -> BTreeMap<String, Summary> {
        let mut grouped: BTreeMap<String, Summary> = BTreeMap::new();
        for cost in &self.entries {
            if let Some(model) = &cost.model {
                grouped.entry(model.clone()).or_default().add(cost);
            }
        }
        grouped
    }

    /// The agents that cost the most, most expensive first.
    #[must_use]
    pub fn top_agents(&self, limit: usize) -> Vec<(AgentName, Summary)> {
        let mut ranked: Vec<(AgentName, Summary)> = self.by_agent().into_iter().collect();
        // Totals are money, so compare with `total_cmp` rather than risking a partial order.
        ranked.sort_by(|(left_name, left), (right_name, right)| {
            right
                .usd
                .total_cmp(&left.usd)
                .then_with(|| left_name.cmp(right_name))
        });
        ranked.truncate(limit);
        ranked
    }

    /// Drops entries older than `before`, returning how many were removed.
    ///
    /// The ledger grows without bound otherwise, and an unattended factory runs for weeks.
    pub fn prune(&mut self, before: Timestamp) -> usize {
        let was = self.entries.len();
        self.entries.retain(|cost| cost.at >= before);
        was - self.entries.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cost::TokenUsage;
    use crate::flight::RunId;
    use std::time::Duration;

    fn usage(output: u64) -> TokenUsage {
        TokenUsage {
            input: 100,
            output,
            ..TokenUsage::default()
        }
    }

    fn reported(agent: &str, model: &str, usd: f64) -> RunCost {
        RunCost::reported(
            RunId::generate(),
            ItineraryId::generate(),
            agent.into(),
            Some(model.to_owned()),
            usage(10),
            usd,
        )
    }

    fn ledger() -> Ledger {
        let mut ledger = Ledger::new();
        ledger.record(reported("developer", "claude-opus-5", 4.00));
        ledger.record(reported("tester", "codex-mini", 0.50));
        ledger.record(reported("developer", "claude-opus-5", 2.00));
        ledger.record(reported("reviewer", "codex-mini", 1.00));
        ledger
    }

    #[test]
    fn totals_add_up_across_runs() {
        let total = ledger().total();

        assert_eq!(total.runs, 4);
        assert!((total.usd - 7.50).abs() < 1e-9);
        assert_eq!(total.usage.output, 40);
    }

    #[test]
    fn spend_breaks_down_by_agent() {
        let by_agent = ledger().by_agent();

        assert!((by_agent[&AgentName::from("developer")].usd - 6.00).abs() < 1e-9);
        assert_eq!(by_agent[&AgentName::from("developer")].runs, 2);
        assert!((by_agent[&AgentName::from("tester")].usd - 0.50).abs() < 1e-9);
    }

    #[test]
    fn spend_breaks_down_by_model() {
        let by_model = ledger().by_model();

        assert!((by_model["claude-opus-5"].usd - 6.00).abs() < 1e-9);
        assert!((by_model["codex-mini"].usd - 1.50).abs() < 1e-9);
    }

    #[test]
    fn the_biggest_spender_is_identifiable() {
        let top = ledger().top_agents(2);

        assert_eq!(top[0].0, AgentName::from("developer"));
        assert_eq!(top[1].0, AgentName::from("reviewer"));
        assert_eq!(top.len(), 2);
    }

    #[test]
    fn a_run_with_no_model_is_left_out_of_the_model_breakdown() {
        // Bucketing it under "unknown" would invent a model that does not exist; the run still
        // counts in the agent breakdown and the overall total.
        let mut ledger = Ledger::new();
        ledger.record(RunCost::reported(
            RunId::generate(),
            ItineraryId::generate(),
            "analyst".into(),
            None,
            usage(10),
            1.00,
        ));

        assert!(ledger.by_model().is_empty());
        assert_eq!(ledger.total().runs, 1);
    }

    #[test]
    fn one_unreported_run_makes_the_whole_total_a_lower_bound() {
        // The weakest source wins. A total that is mostly measured is still not measured.
        let mut ledger = ledger();
        ledger.record(RunCost::unreported(
            RunId::generate(),
            ItineraryId::generate(),
            "kusto".into(),
            None,
        ));

        let total = ledger.total();
        assert_eq!(total.confidence(), CostSource::Unreported);
        assert!(!total.is_fully_measured());
        assert_eq!(total.unreported_runs, 1);
        assert!((total.measured_share() - 0.8).abs() < 1e-9);
    }

    #[test]
    fn an_estimate_downgrades_confidence_without_hiding_the_money() {
        let mut ledger = Ledger::new();
        ledger.record(reported("developer", "claude-opus-5", 4.00));
        ledger.record(RunCost {
            source: CostSource::RateCard,
            ..reported("tester", "codex-mini", 1.00)
        });

        let total = ledger.total();
        assert_eq!(total.confidence(), CostSource::RateCard);
        assert_eq!(total.estimated_runs, 1);
        assert!(
            (total.usd - 5.00).abs() < 1e-9,
            "an estimate still counts towards the bill"
        );
        assert!((total.measured_share() - 0.5).abs() < 1e-9);
    }

    #[test]
    fn an_empty_ledger_is_fully_measured() {
        let total = Ledger::new().total();

        assert!(total.is_fully_measured());
        assert!((total.measured_share() - 1.0).abs() < f64::EPSILON);
        assert!(Ledger::new().is_empty());
    }

    #[test]
    fn spend_is_attributable_to_one_chain() {
        let itinerary = ItineraryId::generate();
        let mut ledger = ledger();
        ledger.record(RunCost {
            itinerary: itinerary.clone(),
            ..reported("publisher", "codex-mini", 3.00)
        });

        let summary = ledger.for_itinerary(&itinerary);
        assert_eq!(summary.runs, 1);
        assert!((summary.usd - 3.00).abs() < 1e-9);
    }

    #[test]
    fn old_entries_can_be_pruned() {
        let now = Timestamp::now();
        let mut ledger = Ledger::new();
        ledger.record(reported("old", "codex-mini", 1.00).at(now - Duration::from_secs(7_200)));
        ledger.record(reported("new", "codex-mini", 2.00).at(now));

        let cutoff = now - Duration::from_secs(3_600);
        assert_eq!(ledger.since(cutoff).runs, 1);
        assert_eq!(ledger.prune(cutoff), 1);
        assert_eq!(ledger.len(), 1);
    }
}
