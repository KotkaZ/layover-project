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

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::flight::Flight;
use crate::pipeline::PipelineName;

/// A flight waiting to be dispatched, with the instructions needed to dispatch it.
///
/// `pipeline` and `flags` belong to the *chain*, not to this one flight. The first flight of a
/// chain carries what the trigger chose; every flight the chain sends afterwards — a hand-off, a
/// spawned chain, a resumed layover — carries the same, so a queue read back after a restart still
/// knows how the work it holds was asked for.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct Queued {
    /// The flight itself.
    pub flight: Flight,
    /// The pipeline that opened this flight's chain, when one did rather than a bare agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pipeline: Option<PipelineName>,
    /// Every flag the chain's pipeline declares, resolved to the value its runs see.
    ///
    /// Resolved, not merely the ones the operator changed: a default that shifts between queueing
    /// and dispatch would otherwise change the meaning of work already booked.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub flags: BTreeMap<String, bool>,
    /// Pipelines whose routes the chain must also stay within, when it resumes work another
    /// chain set down — see [`crate::scope::ChainScope`]. Carried here, like the pipeline, so a
    /// Tower restarted with this work still queued cannot widen what the chain may reach.
    ///
    /// A `null` entry is an ancestor chain no pipeline opened. Empty for every other chain.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub within: BTreeSet<Option<PipelineName>>,
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
            within: BTreeSet::new(),
        }
    }

    /// Records the pipelines the chain must also stay within.
    #[must_use]
    pub fn narrowed_by(mut self, within: BTreeSet<Option<PipelineName>>) -> Self {
        self.within = within;
        self
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
