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

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use crate::chosen::Chosen;
use crate::flight::{Flight, ItineraryId};
use crate::handover::Recovery;
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
    /// The interrupted run this flight restarts, when it restarts one.
    ///
    /// Told to the run above its work, so that it checks before repeating a side effect. Carried on
    /// the queued flight so a crash loop is bounded by `max_recovery_attempts` across restarts,
    /// which are exactly when a count held in memory would be lost.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovering: Option<Recovery>,
    /// When it may start, for a restart backing off after the model service failed — see
    /// [`crate::handover::provider_backoff`]. `None` for everything else, which may start at once.
    ///
    /// Kept on the queued flight rather than in a timer, so a Tower restarted while the restart
    /// waits still waits, and one that comes back after it was due starts it at once.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub not_before: Option<Timestamp>,
    /// Whether the flight is a rendezvous that already released, carrying every arrival.
    ///
    /// Only a restarted run of a released join is queued like this. The barrier it passed was
    /// discarded with the Tower that held it, and delivered to a fresh one it would wait for
    /// upstreams whose runs finished long ago.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub released: bool,
    /// The chain this one continues, when a person answered a help request it raised.
    ///
    /// The new chain has its own budget, so it is a different itinerary; this is what links the
    /// two, so each can say where the other went.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub continues: Option<ItineraryId>,
    /// What the person who triggered the chain chose for it: its name, and agents to run
    /// differently. Carried like the flags, so a restart, a hand-off, a spawned chain and a
    /// follow-up all keep it. See [`crate::chosen`].
    #[serde(default, skip_serializing_if = "Chosen::is_empty")]
    pub chosen: Chosen,
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
            recovering: None,
            not_before: None,
            released: false,
            continues: None,
            chosen: Chosen::default(),
        }
    }

    /// Records what the person who triggered the chain chose for it.
    #[must_use]
    pub fn choosing(mut self, chosen: Chosen) -> Self {
        self.chosen = chosen;
        self
    }

    /// Marks the flight as continuing the chain `earlier`.
    #[must_use]
    pub fn continuing(mut self, earlier: ItineraryId) -> Self {
        self.continues = Some(earlier);
        self
    }

    /// Which attempt at its work this is: 1 as it was asked for, 2 for the first restart after a
    /// run of it was interrupted, and so on.
    #[must_use]
    pub fn attempt(&self) -> u32 {
        self.recovering
            .as_ref()
            .map_or(1, |recovery| recovery.attempt)
    }

    /// Marks the flight as restarting an interrupted run.
    #[must_use]
    pub fn restarting(mut self, recovery: Recovery) -> Self {
        self.recovering = Some(recovery);
        self
    }

    /// Holds the flight until `at`.
    #[must_use]
    pub fn waiting_until(mut self, at: Timestamp) -> Self {
        self.not_before = Some(at);
        self
    }

    /// Whether the flight may start at `now`.
    #[must_use]
    pub fn is_due(&self, now: Timestamp) -> bool {
        self.not_before.is_none_or(|at| at <= now)
    }

    /// Marks the flight as a join that already released, to be delivered straight to its agent.
    #[must_use]
    pub fn already_released(mut self) -> Self {
        self.released = true;
        self
    }

    /// Records the pipelines the chain must also stay within.
    #[must_use]
    pub fn narrowed_by(mut self, within: BTreeSet<Option<PipelineName>>) -> Self {
        self.within = within;
        self
    }

    /// The workflow this flight is a way into, when it is one: the first flight of a chain that a
    /// person, a schedule or a resuming pipeline asked a workflow for.
    ///
    /// What a switched-off workflow holds back. Every other flight belongs to a chain already
    /// under way — a hand-off or a spawn an agent sent, a restart of an interrupted run, a join
    /// that released, the continuation a reply to a help request starts — and a chain under way
    /// finishes whatever its workflow's switch says. `pipeline` alone cannot tell them apart:
    /// every flight of a chain carries it.
    #[must_use]
    pub fn way_in(&self) -> Option<&PipelineName> {
        let opens = self.flight.from.agent().is_none()
            && self.recovering.is_none()
            && self.continues.is_none()
            && !self.released;
        if opens { self.pipeline.as_ref() } else { None }
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

    #[test]
    fn a_restart_survives_storage_and_counts_its_attempt() {
        // The attempt count bounds a crash loop, and a crash loop is a sequence of restarts — the
        // moment anything held only in memory is gone.
        let queued = Queued::new(flight(), None, BTreeMap::new())
            .restarting(crate::handover::Recovery {
                previous: crate::flight::RunId::generate(),
                interruption: crate::handover::Interruption::TowerRestart,
                attempt: 2,
            })
            .already_released();

        let json = serde_json::to_string(&queued).expect("serialises");
        let read: Queued = serde_json::from_str(&json).expect("deserialises");

        assert_eq!(read, queued);
        assert_eq!(read.attempt(), 2);
        assert!(read.released);
    }

    #[test]
    fn only_the_first_flight_a_workflow_was_asked_for_is_a_way_into_it() {
        let development = || Some(PipelineName::new("development"));
        let opening = |from: Origin| {
            let mut flight = flight();
            flight.from = from;
            Queued::new(flight, development(), BTreeMap::new())
        };

        for from in [
            Origin::Human,
            Origin::Schedule(PipelineName::new("development")),
            Origin::Resumed(crate::layover::LayoverId::generate()),
        ] {
            assert_eq!(
                opening(from.clone()).way_in(),
                development().as_ref(),
                "{from:?}"
            );
        }

        let under_way = [
            opening(Origin::Agent(AgentName::new("analyst"))),
            opening(Origin::Human).restarting(crate::handover::Recovery {
                previous: crate::flight::RunId::generate(),
                interruption: crate::handover::Interruption::TowerRestart,
                attempt: 2,
            }),
            opening(Origin::Human).already_released(),
            opening(Origin::Human).continuing(ItineraryId::generate()),
        ];
        for queued in under_way {
            assert_eq!(queued.way_in(), None, "{queued:?}");
        }

        assert_eq!(Queued::new(flight(), None, BTreeMap::new()).way_in(), None);
    }

    #[test]
    fn a_restart_backing_off_waits_across_storage_and_then_is_due() {
        let at: Timestamp = "2026-10-14T11:00:00Z".parse().expect("an instant");
        let queued = Queued::new(flight(), None, BTreeMap::new()).waiting_until(at);

        let json = serde_json::to_string(&queued).expect("serialises");
        let read: Queued = serde_json::from_str(&json).expect("deserialises");

        assert_eq!(read.not_before, Some(at));
        assert!(!read.is_due(at - jiff::SignedDuration::from_secs(1)));
        assert!(read.is_due(at));
        assert!(Queued::new(flight(), None, BTreeMap::new()).is_due(Timestamp::MIN));
    }

    #[test]
    fn work_as_asked_for_is_the_first_attempt_and_says_nothing_about_restarts() {
        let queued = Queued::new(flight(), None, BTreeMap::new());
        let json = serde_json::to_string(&queued).expect("serialises");

        assert_eq!(queued.attempt(), 1);
        assert!(!json.contains("recovering"), "{json}");
        assert!(!json.contains("not_before"), "{json}");
        assert!(!json.contains("released"), "{json}");
    }
}
