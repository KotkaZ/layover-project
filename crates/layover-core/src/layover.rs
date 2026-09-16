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

use std::fmt;

use jiff::{Timestamp, ToSpan};
use serde::{Deserialize, Serialize};
use ulid::Ulid;

use crate::agent::AgentName;
use crate::flight::ItineraryId;
use crate::handover::Handover;

/// Identifier of a booked layover.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize, Serialize)]
#[serde(transparent)]
pub struct LayoverId(String);

impl LayoverId {
    /// Mints a new identifier.
    #[must_use]
    pub fn generate() -> Self {
        Self(format!("lay_{}", Ulid::new()))
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

/// Where a booked layover has got to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Standing {
    /// Waiting for its time to come round.
    Booked,
    /// A new itinerary has been opened for it.
    Resumed,
    /// Given up on: it was checked too many times without the thing it waits for happening.
    Expired,
    /// Cancelled, because the work it was following stopped mattering.
    Cancelled,
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
            Self::Expired => "expired",
            Self::Cancelled => "cancelled",
        }
    }

    /// Parses a slug.
    #[must_use]
    pub fn from_slug(slug: &str) -> Option<Self> {
        [Self::Booked, Self::Resumed, Self::Expired, Self::Cancelled]
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
    /// How many times it has been picked up and set down again.
    ///
    /// A layover waiting on a human is checked repeatedly and usually finds nothing. Counting
    /// gives the difference between "waiting patiently" and "waiting forever", which is the
    /// difference between a follow-up and a leak.
    pub checks: u32,
    /// How many checks it may have before it is given up on.
    pub max_checks: u32,
    /// Where it has got to.
    pub standing: Standing,
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
        max_checks: u32,
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
            max_checks,
            standing: Standing::Booked,
        }
    }

    /// Returns `true` when this should be picked up at `now`.
    #[must_use]
    pub fn is_due(&self, now: Timestamp) -> bool {
        self.standing.is_pending() && now >= self.due_at
    }

    /// Records that it was picked up and the thing it waits for had not happened.
    ///
    /// Each check pushes the next one further out, so a layover waiting on a human does not poll
    /// at the same rate on day six as on minute one. Backing off is what makes a long wait
    /// affordable — the alternative is paying for a run every few minutes to be told nothing has
    /// changed.
    pub fn set_down_again(&mut self, now: Timestamp) {
        self.checks = self.checks.saturating_add(1);

        if self.checks >= self.max_checks {
            self.standing = Standing::Expired;
            return;
        }

        let minutes = backoff_minutes(self.checks);
        self.due_at = now.checked_add(minutes.minutes()).unwrap_or(Timestamp::MAX);
    }

    /// Records that a new itinerary has been opened for this work.
    pub fn resumed(&mut self) {
        self.standing = Standing::Resumed;
    }

    /// Gives up on it, because the work it was following stopped mattering.
    pub fn cancel(&mut self) {
        self.standing = Standing::Cancelled;
    }

    /// How long until it is next due, in whole minutes, or `None` when it is due now.
    #[must_use]
    pub fn minutes_until_due(&self, now: Timestamp) -> Option<i64> {
        let seconds = self.due_at.as_second() - now.as_second();
        (seconds > 0).then_some(seconds / 60)
    }
}

/// How long to wait before the next check, after `checks` fruitless ones.
///
/// Doubling from fifteen minutes, capped at six hours. The cap matters: without it the tenth
/// check would be days out, so a comment arriving on a quiet pull request would sit unanswered
/// for longer than the work took.
fn backoff_minutes(checks: u32) -> i64 {
    const FIRST: i64 = 15;
    const CAP: i64 = 6 * 60;

    let doublings = checks.saturating_sub(1).min(8);
    (FIRST << doublings).min(CAP)
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
            8,
        )
    }

    #[test]
    fn a_layover_starts_booked_and_becomes_due_at_its_time() {
        let layover = booked("2026-09-16T11:00:00Z");

        assert_eq!(layover.standing, Standing::Booked);
        assert!(!layover.is_due(at("2026-09-16T10:30:00Z")));
        assert!(layover.is_due(at("2026-09-16T11:00:00Z")));
        assert_eq!(
            layover.minutes_until_due(at("2026-09-16T10:30:00Z")),
            Some(30)
        );
    }

    #[test]
    fn a_layover_due_before_it_was_booked_is_held_to_the_booking_time() {
        // Otherwise it is picked up instantly, finds nothing, and spins.
        let layover = booked("2026-09-16T09:00:00Z");

        assert_eq!(layover.due_at, at("2026-09-16T10:00:00Z"));
    }

    #[test]
    fn checking_and_finding_nothing_pushes_the_next_check_further_out() {
        // A layover waiting on a human should not poll at the same rate on day six as on minute
        // one. Paying for a run every few minutes to be told nothing changed is how a follow-up
        // becomes more expensive than the work.
        let mut layover = booked("2026-09-16T11:00:00Z");

        layover.set_down_again(at("2026-09-16T11:00:00Z"));
        let first = layover.minutes_until_due(at("2026-09-16T11:00:00Z"));

        layover.set_down_again(at("2026-09-16T11:15:00Z"));
        let second = layover.minutes_until_due(at("2026-09-16T11:15:00Z"));

        assert_eq!(first, Some(15));
        assert_eq!(second, Some(30));
        assert!(second > first);
    }

    #[test]
    fn the_backoff_is_capped_so_a_late_comment_is_not_ignored_for_days() {
        // Without a cap the tenth check lands days out, and a comment on a quiet pull request
        // waits longer than the work took.
        let mut layover = booked("2026-09-16T11:00:00Z");
        layover.max_checks = 40;

        for _ in 0..20 {
            layover.set_down_again(at("2026-09-16T11:00:00Z"));
        }

        assert_eq!(
            layover.minutes_until_due(at("2026-09-16T11:00:00Z")),
            Some(6 * 60)
        );
    }

    #[test]
    fn a_layover_that_waits_forever_is_given_up_on() {
        // The difference between waiting patiently and leaking. Something waiting on a human who
        // has moved on must eventually stop costing money.
        let mut layover = booked("2026-09-16T11:00:00Z");

        for _ in 0..8 {
            layover.set_down_again(at("2026-09-16T11:00:00Z"));
        }

        assert_eq!(layover.standing, Standing::Expired);
        assert!(!layover.is_due(at("2027-01-01T00:00:00Z")));
    }

    #[test]
    fn resuming_and_cancelling_both_take_it_out_of_the_queue() {
        let mut resumed = booked("2026-09-16T11:00:00Z");
        resumed.resumed();

        let mut cancelled = booked("2026-09-16T11:00:00Z");
        cancelled.cancel();

        for layover in [&resumed, &cancelled] {
            assert!(!layover.standing.is_pending());
            assert!(!layover.is_due(at("2026-09-16T12:00:00Z")));
        }
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
        for standing in [
            Standing::Booked,
            Standing::Resumed,
            Standing::Expired,
            Standing::Cancelled,
        ] {
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
}
