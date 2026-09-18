//! A chain that stopped and will not start again on its own.
//!
//! # Why this is written down rather than inferred
//!
//! Every other failure in the system leaves a mark somebody can point at: a failed run, a timeout,
//! a help request. A stall leaves none. Every run in the chain *succeeded* — the tester reported,
//! the reviewer reported — and then the publisher never woke, because the barrier it was waiting
//! behind could no longer be completed.
//!
//! Read back from history, that chain is indistinguishable from one that finished. The runs all
//! say `succeeded` and nothing says the work never happened. So the moment the Tower gives up on
//! a rendezvous, it says so in a place that outlives the process, and the dashboard reads it.
//!
//! That is the whole reason this type exists: silent permanent stalling is the worst outcome in
//! this system, and the only thing worse than a factory that stops is one that stops quietly.

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use crate::agent::AgentName;
use crate::flight::ItineraryId;

/// A chain the Tower has given up on, and why.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct Stall {
    /// The chain that will not continue.
    pub itinerary: ItineraryId,
    /// The agent that was waiting.
    pub agent: AgentName,
    /// Upstreams that never arrived and now never can.
    pub missing: Vec<AgentName>,
    /// How many flights were being held.
    ///
    /// Work somebody asked for that will not happen. A count rather than the flights themselves,
    /// because the point is to prompt a person to look, not to replay it.
    pub stranded: usize,
    /// When the Tower gave up.
    pub at: Timestamp,
}

impl Stall {
    /// Records a chain that can no longer continue.
    #[must_use]
    pub fn new(
        itinerary: ItineraryId,
        agent: AgentName,
        missing: Vec<AgentName>,
        stranded: usize,
        at: Timestamp,
    ) -> Self {
        Self {
            itinerary,
            agent,
            missing,
            stranded,
            at,
        }
    }

    /// One line saying what went wrong, written for a person reading a dashboard.
    #[must_use]
    pub fn summary(&self) -> String {
        let missing = self
            .missing
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ");

        format!(
            "`{}` never woke: nothing live could still deliver {missing}",
            self.agent
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stall(missing: &[&str]) -> Stall {
        Stall::new(
            ItineraryId::generate(),
            AgentName::new("publisher"),
            missing.iter().map(|n| AgentName::new(*n)).collect(),
            1,
            Timestamp::now(),
        )
    }

    #[test]
    fn a_stall_names_the_agent_that_never_woke() {
        // The dashboard shows this next to a chain whose runs all say `succeeded`, so it has to
        // say what did *not* happen rather than what did.
        let summary = stall(&["reviewer"]).summary();

        assert!(summary.contains("publisher"), "{summary}");
        assert!(summary.contains("never woke"), "{summary}");
        assert!(summary.contains("reviewer"), "{summary}");
    }

    #[test]
    fn every_missing_upstream_is_named() {
        let summary = stall(&["reviewer", "tester"]).summary();

        assert!(summary.contains("reviewer"), "{summary}");
        assert!(summary.contains("tester"), "{summary}");
    }

    #[test]
    fn a_stall_survives_a_round_trip_through_json() {
        // It is written to disk precisely because it has to outlive the process that noticed it.
        let original = stall(&["reviewer"]);
        let text = serde_json::to_string(&original).expect("serialises");
        let read: Stall = serde_json::from_str(&text).expect("deserialises");

        assert_eq!(read, original);
    }
}
