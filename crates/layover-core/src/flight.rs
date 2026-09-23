//! The Flight envelope, and the identifiers that track work through the Tower.

use jiff::Timestamp;

use serde::{Deserialize, Serialize};
use ulid::Ulid;

use crate::agent::AgentName;

/// Identifier of one supervised CLI execution.
///
/// Lives here with the other identifiers rather than with the cost ledger that first needed it: a
/// run is a core domain object, and several parts of the Tower will key off it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize, Serialize)]
#[serde(transparent)]
pub struct RunId(String);

impl RunId {
    /// Mints a new identifier.
    #[must_use]
    pub fn generate() -> Self {
        Self(format!("run_{}", Ulid::new()))
    }

    /// Returns the identifier as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// When this identifier was minted, which is when its run began.
    ///
    /// A ULID carries a millisecond timestamp in its leading bits, so a run's own name says how
    /// old it is. That matters because a run's Hangar outlives the process: retention needs a
    /// directory's age, and asking the *name* is exact where asking the filesystem is a guess — a
    /// copy, a restore or a backup tool rewrites `mtime`, which would make old work look new or
    /// new work look expired.
    ///
    /// `None` when the identifier was not minted by [`Self::generate`] — anything hand-made, or a
    /// directory somebody else left in a Hangar. Callers should read that as "do not touch"
    /// rather than "too old".
    #[must_use]
    pub fn minted_at(&self) -> Option<Timestamp> {
        let ulid: Ulid = self.0.strip_prefix("run_")?.parse().ok()?;

        Timestamp::from_millisecond(i64::try_from(ulid.timestamp_ms()).ok()?).ok()
    }
}

impl From<&str> for RunId {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl std::fmt::Display for RunId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Identifier of a single flight.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize, Serialize)]
#[serde(transparent)]
pub struct FlightId(String);

impl FlightId {
    /// Mints a new identifier.
    #[must_use]
    pub fn generate() -> Self {
        Self(format!("flt_{}", Ulid::new()))
    }

    /// Returns the identifier as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Reads an identifier that came from outside — a URL path, say.
///
/// Deliberately not validated. This type identifies a flight; it does not certify that one
/// exists, and the only thing done with an identifier that names nothing is to say so. Rejecting
/// a malformed string here would turn "no such flight" into "bad request", which tells the caller
/// less about what actually happened.
impl From<&str> for FlightId {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl std::fmt::Display for FlightId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Identifier of one causal chain of flights.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize, Serialize)]
#[serde(transparent)]
pub struct ItineraryId(String);

impl ItineraryId {
    /// Mints a new identifier.
    #[must_use]
    pub fn generate() -> Self {
        Self(format!("itn_{}", Ulid::new()))
    }

    /// Returns the identifier as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Who sent a flight.
///
/// Sender identity is mandatory rather than optional: an agent behind a rendezvous receives
/// several flights at once and must be able to tell them apart.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    /// Injected over the HTTP API by a human.
    Human,
    /// Sent by an agent via the MCP control channel.
    Agent(AgentName),
}

impl Origin {
    /// Returns the sending agent, or `None` for a human entry point.
    #[must_use]
    pub fn agent(&self) -> Option<&AgentName> {
        match self {
            Self::Human => None,
            Self::Agent(name) => Some(name),
        }
    }
}

/// A message in transit between agents.
///
/// `hops_remaining` is mirrored here for the transcript. The authoritative value lives in the
/// Tower, keyed by itinerary, because a wrapped CLI is a black box that could otherwise claim
/// any budget it liked.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct Flight {
    /// Unique identifier.
    pub id: FlightId,
    /// The chain this flight belongs to.
    pub itinerary: ItineraryId,
    /// Who sent it.
    pub from: Origin,
    /// Which agent it is addressed to.
    pub to: AgentName,
    /// The payload handed to the receiving agent.
    pub body: String,
    /// Legs remaining before the chain is cut.
    pub hops_remaining: u32,
    /// When the Tower accepted it.
    pub sent_at: Timestamp,
}

impl Flight {
    /// Creates a flight, minting a fresh identifier and timestamp.
    #[must_use]
    pub fn new(
        itinerary: ItineraryId,
        from: Origin,
        to: AgentName,
        body: impl Into<String>,
        hops_remaining: u32,
    ) -> Self {
        Self {
            id: FlightId::generate(),
            itinerary,
            from,
            to,
            body: body.into(),
            hops_remaining,
            sent_at: Timestamp::now(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_are_unique_and_prefixed() {
        let first = FlightId::generate();
        let second = FlightId::generate();

        assert_ne!(first, second);
        assert!(first.as_str().starts_with("flt_"));
        assert!(ItineraryId::generate().as_str().starts_with("itn_"));
    }

    #[test]
    fn a_run_id_says_when_it_was_minted() {
        // Retention for Hangars reads the age off the directory name rather than off the
        // filesystem, so this is the thing that decides whether a transcript is kept.
        let before = Timestamp::now();
        let run = RunId::generate();
        let after = Timestamp::now();

        let minted = run.minted_at().expect("a generated id decodes");

        // ULIDs carry whole milliseconds, so the lower bound is rounded down rather than equal.
        assert!(
            minted.as_millisecond() >= before.as_millisecond() - 1
                && minted <= after + jiff::Span::new().milliseconds(1),
            "minted {minted} is outside {before}..{after}"
        );
    }

    #[test]
    fn a_real_run_id_decodes_to_when_that_run_happened() {
        // Taken from the 48-hour soak, whose first `analyst` run started on 2026-09-21. A
        // hand-written expectation rather than a round trip: if the decoding were subtly wrong --
        // wrong epoch, wrong bit width -- a round trip would agree with itself and still delete
        // the wrong Hangars.
        let minted = RunId::from("run_01M31S7S94MCCD56RC8S6TFQ4Y")
            .minted_at()
            .expect("decodes");

        let day = minted.to_string();
        assert!(day.starts_with("2026-09-21"), "decoded to {day}");
    }

    #[test]
    fn an_id_not_minted_here_has_no_time_rather_than_a_wrong_one() {
        // Retention reads this as "do not touch". Returning some default would make a directory
        // somebody else put in a Hangar look infinitely old and delete it.
        assert!(RunId::from("my-notes").minted_at().is_none());
        assert!(RunId::from("run_not-a-ulid").minted_at().is_none());
        assert!(
            RunId::from("itn_01M31S7S94MCCD56RC8S6TFQ4Y")
                .minted_at()
                .is_none()
        );
    }

    #[test]
    fn human_origin_has_no_agent() {
        assert!(Origin::Human.agent().is_none());
        assert_eq!(
            Origin::Agent("planner".into()).agent(),
            Some(&AgentName::from("planner"))
        );
    }
}
