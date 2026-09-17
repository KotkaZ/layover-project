//! Agents asking for help.
//!
//! A lights-out factory's worst failure is not a crash — a crash is loud. It is an agent that
//! quietly cannot do the thing it was asked to do, produces something plausible anyway, and
//! passes it downstream. The way out is for the agent to say so, in a form a human can act on
//! without reconstructing the run.
//!
//! # Why this is not just a log line
//!
//! Logs are written for the run; a help request is written for a person who is not watching. It
//! needs to survive the run, be findable without knowing which run produced it, and carry enough
//! to act on. That makes it a record, not a message.
//!
//! # The text is not trusted
//!
//! `summary` and `detail` are written by an agent explaining why something did not work, and the
//! commonest reason is a credential. They are therefore redacted and capped on the way in — see
//! [`redact`] — because from here they go to disk, to an unauthenticated HTTP API and to a
//! dashboard, and none of those can take it back.
//!
//! # What the shape is for
//!
//! Every field here exists to answer a question somebody asks when they open the dashboard and
//! find work stopped: *what is stuck, why, was it fatal, and has it been happening?*
//!
//! The categories come from measurement rather than imagination. In a sibling project's help file,
//! five of six entries were permission or access failures — a denied tool guard, a denied git
//! read, an HTTP 422, a TLS handshake. [`Blocker::Access`] is the common case by a wide margin,
//! and it is worth being able to filter for it.

pub mod redact;

use std::fmt;

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use crate::agent::AgentName;
use crate::flight::{ItineraryId, RunId};

/// What kind of thing an agent is stuck on.
///
/// Coarse on purpose. The point is to make "half my runs are stuck on credentials" visible at a
/// glance; the detail lives in the prose.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Blocker {
    /// A credential is missing, expired, or refused. The single most common blocker in practice.
    Access,
    /// A tool, command or service the agent needed was unavailable or failed.
    Tooling,
    /// The instructions or the work item did not say enough to proceed.
    Ambiguity,
    /// The workspace was not in the state the agent needed: a missing checkout, a dirty tree.
    Environment,
    /// A decision that is not the agent's to make.
    Decision,
    /// None of the above.
    Other,
}

impl Blocker {
    /// Every category, in the order a dashboard should offer them.
    pub const ALL: [Self; 6] = [
        Self::Access,
        Self::Tooling,
        Self::Ambiguity,
        Self::Environment,
        Self::Decision,
        Self::Other,
    ];

    /// The identifier used in query strings and JSON.
    #[must_use]
    pub fn slug(self) -> &'static str {
        match self {
            Self::Access => "access",
            Self::Tooling => "tooling",
            Self::Ambiguity => "ambiguity",
            Self::Environment => "environment",
            Self::Decision => "decision",
            Self::Other => "other",
        }
    }

    /// Parses a slug.
    #[must_use]
    pub fn from_slug(slug: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.slug() == slug)
    }

    /// A one-line description, used when prompting an agent to choose one.
    #[must_use]
    pub fn describe(self) -> &'static str {
        match self {
            Self::Access => "a credential or permission was missing, expired or refused",
            Self::Tooling => "a command, tool or service was unavailable or failed",
            Self::Ambiguity => "the instructions or the work item did not say enough",
            Self::Environment => "the workspace was not in a usable state",
            Self::Decision => "a choice that is not yours to make",
            Self::Other => "something else",
        }
    }
}

impl fmt::Display for Blocker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.slug())
    }
}

/// One agent asking a human for something.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct HelpRequest {
    /// Who is asking.
    pub agent: AgentName,
    /// The run that raised it.
    pub run: RunId,
    /// The chain it belonged to.
    pub itinerary: ItineraryId,
    /// What kind of thing is in the way.
    pub blocker: Blocker,
    /// One line, for a list.
    pub summary: String,
    /// The whole explanation: what was tried, what happened, what is needed.
    pub detail: String,
    /// Whether this stopped the work or merely limited it.
    ///
    /// The distinction matters and is easy to lose. An agent can finish its task and still have
    /// been unable to check something — that is worth reporting, and it is not an outage.
    pub fatal: bool,
    /// When it was raised.
    pub at: Timestamp,
    /// When a human marked it dealt with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_at: Option<Timestamp>,
}

impl HelpRequest {
    /// Records a request.
    #[must_use]
    pub fn new(
        agent: AgentName,
        run: RunId,
        itinerary: ItineraryId,
        blocker: Blocker,
        summary: impl Into<String>,
        detail: impl Into<String>,
        at: Timestamp,
    ) -> Self {
        Self {
            agent,
            run,
            itinerary,
            blocker,
            summary: redact::detail(&summary.into()),
            detail: redact::detail(&detail.into()),
            fatal: false,
            at,
            resolved_at: None,
        }
    }

    /// Marks the request as having stopped the work.
    #[must_use]
    pub fn fatal(mut self) -> Self {
        self.fatal = true;
        self
    }

    /// Marks it dealt with.
    #[must_use]
    pub fn resolved(mut self, at: Timestamp) -> Self {
        self.resolved_at = Some(at);
        self
    }

    /// Returns `true` when nobody has dealt with this yet.
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.resolved_at.is_none()
    }

    /// Returns `true` when this asks for the same thing as `other`.
    ///
    /// Used to keep a recurring blocker from being filed on every run. A factory whose
    /// credentials expired does not need forty identical requests; it needs one, and a count.
    #[must_use]
    pub fn is_same_ask_as(&self, other: &Self) -> bool {
        self.agent == other.agent
            && self.blocker == other.blocker
            && crate::learning::says_the_same_thing(&self.summary, &other.summary)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(rfc3339: &str) -> Timestamp {
        rfc3339.parse().expect("valid timestamp")
    }

    fn request(agent: &str, blocker: Blocker, summary: &str) -> HelpRequest {
        HelpRequest::new(
            agent.into(),
            RunId::generate(),
            ItineraryId::generate(),
            blocker,
            summary,
            "tried X, got Y, need Z",
            at("2026-09-16T10:00:00Z"),
        )
    }

    #[test]
    fn a_request_starts_open_and_not_fatal() {
        let asked = request("publisher", Blocker::Access, "the ADO token is expired");

        assert!(asked.is_open());
        assert!(
            !asked.fatal,
            "an agent can be limited without being stopped, and the default should not overstate"
        );
    }

    #[test]
    fn a_blocked_run_and_a_limited_one_are_distinguishable() {
        // An agent can finish its task and still have been unable to check something. Reporting
        // that is useful; treating it as an outage is not.
        let stopped = request("publisher", Blocker::Access, "no token at all").fatal();
        let limited = request(
            "reviewer",
            Blocker::Tooling,
            "could not read one linked file",
        );

        assert!(stopped.fatal);
        assert!(!limited.fatal);
    }

    #[test]
    fn resolving_closes_it() {
        let asked = request("publisher", Blocker::Access, "token expired")
            .resolved(at("2026-09-16T11:00:00Z"));

        assert!(!asked.is_open());
        assert_eq!(asked.resolved_at, Some(at("2026-09-16T11:00:00Z")));
    }

    #[test]
    fn the_same_blocker_asked_twice_is_recognised() {
        // A factory whose credentials expired does not need forty identical requests. It needs
        // one, and a count.
        let first = request("publisher", Blocker::Access, "The ADO token has expired");
        let again = request("publisher", Blocker::Access, "the ADO token  has expired.");

        assert!(first.is_same_ask_as(&again));
    }

    #[test]
    fn a_different_agent_or_category_is_a_different_ask() {
        let publisher = request("publisher", Blocker::Access, "the ADO token is expired");

        assert!(!publisher.is_same_ask_as(&request(
            "developer",
            Blocker::Access,
            "the ADO token is expired"
        )));
        assert!(!publisher.is_same_ask_as(&request(
            "publisher",
            Blocker::Tooling,
            "the ADO token is expired"
        )));
    }

    #[test]
    fn categories_round_trip_and_describe_themselves() {
        for blocker in Blocker::ALL {
            assert_eq!(Blocker::from_slug(blocker.slug()), Some(blocker));
            assert!(!blocker.describe().is_empty());
        }

        assert_eq!(Blocker::from_slug("vibes"), None);
    }

    #[test]
    fn a_request_serialises_to_one_readable_line() {
        let line = serde_json::to_string(&request("publisher", Blocker::Access, "token expired"))
            .expect("serialises");

        assert!(!line.contains('\n'));
        assert!(line.contains(r#""blocker":"access""#), "{line}");
        assert!(!line.contains("resolved_at"), "absent, not null: {line}");
    }
}
