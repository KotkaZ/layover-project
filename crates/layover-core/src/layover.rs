//! A Layover: work set down now and picked up later.
//!
//! The project is named after this and, until now, did not have one.
//!
//! # The gap
//!
//! An itinerary is a burst. A trigger opens it, flights move through it, it ends. That is the
//! right shape for most work and the wrong shape for the most valuable kind: a chain that
//! publishes a pull request and then wants to react to the comments that arrive on it over the
//! following days.
//!
//! Neither option available before this worked. Keeping the chain alive and polling spends a hop
//! and real money on every tick, so Hops kills it long before a human replies — and the whole
//! point of Hops is that it should. Re-triggering on a schedule works mechanically but arrives
//! knowing nothing: a fresh itinerary has no idea which work item it is following up, what was
//! already tried, or what the earlier chain concluded.
//!
//! # The shape
//!
//! A run *books a layover* instead of waiting. The work is set down with everything needed to
//! resume it, and a scheduled pipeline picks up whatever is due and opens a **new** itinerary
//! seeded with that context.
//!
//! Nothing stays alive in between. No process, no parked chain, no held budget — which is the
//! same answer recovery and steering arrived at, and for the same reason: what the later run
//! needs is not the earlier one's *process* but its *context*.
//!
//! It also reuses [`crate::handover::Handover`] rather than inventing a second way to say "here
//! is what an earlier run knew". That type already carries the flights and the progress; a
//! layover adds only *when* to come back and *what to look for*.

use std::collections::BTreeMap;
use std::fmt;

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use ulid::Ulid;

use crate::agent::AgentName;
use crate::flight::{ItineraryId, RunId};
use crate::handover::Handover;
use crate::scope::ChainScope;

/// Identifier of a booked layover.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize, Serialize)]
#[serde(transparent)]
pub struct LayoverId(String);

impl LayoverId {
    /// Mints a new identifier.
    #[must_use]
    pub fn generate() -> Self {
        Self(format!("lay_{}", Ulid::generate()))
    }

    /// Returns the identifier as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for LayoverId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Reads an identifier that came from outside — storage, a URL path.
///
/// Not validated, for the same reason as [`crate::flight::FlightId`]: this names a layover, it
/// does not certify that one exists.
impl From<&str> for LayoverId {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

/// Where a booked layover has got to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Standing {
    /// Waiting for its time to come round.
    Booked,
    /// A new itinerary has been opened for it.
    Resumed,
}

impl Standing {
    /// Returns `true` when this layover may still be picked up.
    #[must_use]
    pub fn is_pending(self) -> bool {
        matches!(self, Self::Booked)
    }

    /// The identifier used in JSON.
    #[must_use]
    pub fn slug(self) -> &'static str {
        match self {
            Self::Booked => "booked",
            Self::Resumed => "resumed",
        }
    }

    /// Parses a slug.
    #[must_use]
    pub fn from_slug(slug: &str) -> Option<Self> {
        [Self::Booked, Self::Resumed]
            .into_iter()
            .find(|standing| standing.slug() == slug)
    }
}

impl fmt::Display for Standing {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.slug())
    }
}

/// Work set down now to be picked up later.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct Layover {
    /// Identifier.
    pub id: LayoverId,
    /// Which agent the resumed work should go to.
    pub agent: AgentName,
    /// The itinerary that booked it.
    pub booked_by: ItineraryId,
    /// One line saying what this is waiting for, for a human reading a list.
    pub waiting_for: String,
    /// What the resumed run needs to know.
    pub handover: Handover,
    /// When it was booked.
    pub booked_at: Timestamp,
    /// The soonest it should be picked up.
    pub due_at: Timestamp,
    /// Always 0. A layover is picked up once: a run that finds nothing yet books a new one.
    ///
    /// Written because releases up to 1.7.0 require it when they read a layover back, so a
    /// downgrade does not lose the work set down.
    #[serde(default)]
    pub checks: u32,
    /// Always 0, and written for the same reason as `checks`.
    #[serde(default)]
    pub max_checks: u32,
    /// Where it has got to.
    pub standing: Standing,
    /// The flags the booking chain's prompts were composed with.
    ///
    /// A resumed layover is new work about an old subject, and the operator's choices about that
    /// subject — run the end-to-end suite, open the pull request as a draft — were made when the
    /// work was triggered. Resuming with the resuming pipeline's defaults instead would quietly
    /// undo them on the follow-up. Absent in layovers booked before this was recorded.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub flags: BTreeMap<String, bool>,
    /// The run that set this down, so its report can be handed to the run that picks it up.
    ///
    /// Looked up when the layover is resumed rather than copied when it is booked, because a run
    /// usually reports *after* it books: the conclusion does not exist yet at that moment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run: Option<RunId>,
    /// Which routes the chain that set this down could use.
    ///
    /// The resumed chain belongs to the resuming pipeline but may use only what this scope also
    /// permits, so setting work down and waiting cannot carry a chain into another workflow's
    /// routes. Taken from the Tower's record of the booking run, never from the agent. `None` for
    /// a layover booked before this was recorded, which resumes with the resuming pipeline's
    /// routes as it always did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<ChainScope>,
}

impl Layover {
    /// Books a layover.
    #[must_use]
    pub fn book(
        agent: AgentName,
        booked_by: ItineraryId,
        waiting_for: impl Into<String>,
        handover: Handover,
        booked_at: Timestamp,
        due_at: Timestamp,
    ) -> Self {
        Self {
            id: LayoverId::generate(),
            agent,
            booked_by,
            waiting_for: waiting_for.into(),
            handover,
            booked_at,
            // A layover due before it was booked would be picked up instantly and spin, so the
            // booking time is the floor.
            due_at: due_at.max(booked_at),
            checks: 0,
            max_checks: 0,
            standing: Standing::Booked,
            flags: BTreeMap::new(),
            run: None,
            scope: None,
        }
    }

    /// Records which routes the booking chain could use, so its follow-up is held to them.
    #[must_use]
    pub fn booked_within(mut self, scope: ChainScope) -> Self {
        self.scope = Some(scope);
        self
    }

    /// Records the flags the booking chain was composed with, so its follow-up is too.
    #[must_use]
    pub fn with_flags(mut self, flags: BTreeMap<String, bool>) -> Self {
        self.flags = flags;
        self
    }

    /// Records the run that set this down, so what it reported can be handed on.
    #[must_use]
    pub fn booked_in(mut self, run: RunId) -> Self {
        self.run = Some(run);
        self
    }

    /// Returns `true` when this should be picked up at `now`.
    #[must_use]
    pub fn is_due(&self, now: Timestamp) -> bool {
        self.standing.is_pending() && now >= self.due_at
    }

    /// Records that a new itinerary has been opened for this work.
    pub fn resumed(&mut self) {
        self.standing = Standing::Resumed;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handover::{Handover, Steer};

    fn at(rfc3339: &str) -> Timestamp {
        rfc3339.parse().expect("valid timestamp")
    }

    fn booked(due: &str) -> Layover {
        Layover::book(
            "developer".into(),
            ItineraryId::generate(),
            "comments on PR 1543477",
            Handover::steered(
                Steer {
                    previous: crate::flight::RunId::generate(),
                    note: "address review comments".to_owned(),
                    at: at("2026-09-16T10:00:00Z"),
                },
                Vec::new(),
            ),
            at("2026-09-16T10:00:00Z"),
            at(due),
        )
    }

    #[test]
    fn a_layover_starts_booked_and_becomes_due_at_its_time() {
        let layover = booked("2026-09-16T11:00:00Z");

        assert_eq!(layover.standing, Standing::Booked);
        assert!(!layover.is_due(at("2026-09-16T10:30:00Z")));
        assert!(layover.is_due(at("2026-09-16T11:00:00Z")));
    }

    #[test]
    fn a_layover_due_before_it_was_booked_is_held_to_the_booking_time() {
        // Otherwise it is picked up instantly, finds nothing, and spins.
        let layover = booked("2026-09-16T09:00:00Z");

        assert_eq!(layover.due_at, at("2026-09-16T10:00:00Z"));
    }

    #[test]
    fn resuming_takes_it_out_of_the_queue() {
        let mut resumed = booked("2026-09-16T11:00:00Z");
        resumed.resumed();

        assert!(!resumed.standing.is_pending());
        assert!(!resumed.is_due(at("2026-09-16T12:00:00Z")));
    }

    #[test]
    fn the_resumed_run_is_given_what_the_earlier_one_knew() {
        // The whole reason this is not just a scheduled pipeline. A fresh itinerary that arrives
        // knowing nothing cannot follow anything up.
        let layover = booked("2026-09-16T11:00:00Z");

        assert!(layover.handover.brief().contains("address review comments"));
    }

    #[test]
    fn standings_round_trip() {
        for standing in [Standing::Booked, Standing::Resumed] {
            assert_eq!(Standing::from_slug(standing.slug()), Some(standing));
        }
        assert_eq!(Standing::from_slug("parked"), None);
    }

    #[test]
    fn a_layover_serialises_readably() {
        let line = serde_json::to_string(&booked("2026-09-16T11:00:00Z")).expect("serialises");

        assert!(line.contains(r#""standing":"booked""#), "{line}");
        assert!(
            line.contains(r#""due_at":"2026-09-16T11:00:00Z""#),
            "{line}"
        );
        assert!(line.contains("comments on PR 1543477"), "{line}");
    }

    #[test]
    fn a_layover_still_carries_the_fields_older_releases_need() {
        // Releases up to 1.7.0 refuse a layover without `checks` and `max_checks`, so dropping
        // them would strand every layover across a downgrade. Reading one without them is the
        // other direction, for when they are finally gone.
        let layover = booked("2026-09-16T11:00:00Z");
        let mut value = serde_json::to_value(&layover).expect("serialises");

        assert_eq!(value["checks"], 0, "{value}");
        assert_eq!(value["max_checks"], 0, "{value}");

        let object = value.as_object_mut().expect("an object");
        object.remove("checks");
        object.remove("max_checks");
        let read: Layover = serde_json::from_value(value).expect("reads without them");
        assert_eq!(read.id, layover.id);
    }
}
