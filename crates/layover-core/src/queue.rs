//! Work that has been asked for and not yet dispatched.
//!
//! # Why a queued flight is not just a flight
//!
//! A flight in motion needs nothing but itself. By the time it moves, the run it triggers has
//! already had its prompt composed, and the pipeline's flags are baked into that text — so the
//! flags have done their job and the flight can forget them.
//!
//! A *queued* flight has not reached that moment. Nothing has composed a prompt, because nothing
//! has spawned a process. The pipeline it was triggered through and the flags the operator chose
//! therefore have to survive in storage until something dispatches it, which may be after a
//! restart and will certainly be after the request that set them has returned.
//!
//! Storing only the [`Flight`] loses both, silently: the operator sets `run_e2e`, the dialog
//! reports success, and the run — whenever it happens — is composed as though they had not.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::flight::Flight;
use crate::pipeline::PipelineName;

/// A flight waiting to be dispatched, with the instructions needed to dispatch it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct Queued {
    /// The flight itself.
    pub flight: Flight,
    /// The pipeline it was triggered through, when one was named rather than a bare agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pipeline: Option<PipelineName>,
    /// Every flag the pipeline declares, resolved to the value this run will see.
    ///
    /// Resolved, not merely the ones the operator changed: a default that shifts between queueing
    /// and dispatch would otherwise change the meaning of work already booked.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub flags: BTreeMap<String, bool>,
}

impl Queued {
    /// Books `flight`, recording how it was asked for.
    #[must_use]
    pub fn new(
        flight: Flight,
        pipeline: Option<PipelineName>,
        flags: BTreeMap<String, bool>,
    ) -> Self {
        Self {
            flight,
            pipeline,
            flags,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::AgentName;
    use crate::flight::{ItineraryId, Origin};

    fn flight() -> Flight {
        Flight::new(
            ItineraryId::generate(),
            Origin::Human,
            AgentName::new("analyst"),
            "look at 4821",
            8,
        )
    }

    #[test]
    fn flags_survive_a_round_trip_through_storage() {
        let mut flags = BTreeMap::new();
        flags.insert("run_e2e".to_owned(), true);
        flags.insert("draft_pr".to_owned(), false);

        let queued = Queued::new(flight(), Some(PipelineName::new("development")), flags);

        let json = serde_json::to_string(&queued).expect("serialises");
        let read: Queued = serde_json::from_str(&json).expect("deserialises");

        assert_eq!(read, queued);
        assert_eq!(
            read.flags.get("run_e2e"),
            Some(&true),
            "the operator set this and the run must see it"
        );
        assert_eq!(
            read.flags.get("draft_pr"),
            Some(&false),
            "a flag left at its default is still a decision, and is recorded as one"
        );
    }

    #[test]
    fn a_bare_agent_trigger_carries_no_pipeline_and_no_flags() {
        let queued = Queued::new(flight(), None, BTreeMap::new());
        let json = serde_json::to_string(&queued).expect("serialises");

        assert!(!json.contains("pipeline"), "{json}");
        assert!(!json.contains("flags"), "{json}");
    }
}
