//! The Flight envelope, and the identifiers that track work through the Tower.

use jiff::Timestamp;

use serde::{Deserialize, Serialize};
use ulid::Ulid;

use crate::agent::AgentName;
use crate::layover::LayoverId;
use crate::pipeline::PipelineName;

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
        Self(format!("run_{}", Ulid::generate()))
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
        Self(format!("flt_{}", Ulid::generate()))
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
        Self(format!("itn_{}", Ulid::generate()))
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
/// several flights at once and must be able to tell them apart, and every run is told who woke it.
///
/// Only [`Origin::Agent`] has an upstream edge for the route map to check. The others — a person,
/// a clock, a layover coming due — are ways work *enters* the mesh, and used to be recorded as
/// `Human` alike, so a run woken by a schedule was told nothing it could use.
///
/// # On disk
///
/// A 1.0.0 binary knows only `human` and `agent`, and its queue reader silently drops a line it
/// cannot parse — and then rewrites the queue without it. So a schedule and a layover are written
/// as `"from": "human"` with the detail in a `via` field that 1.0.0 ignores: a downgrade loses the
/// label, never the work. See [`Flight`]'s serialisation.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    /// A person, through the dashboard or the HTTP API.
    Human,
    /// Sent by an agent via the MCP control channel.
    Agent(AgentName),
    /// Fired by the named pipeline's schedule.
    Schedule(PipelineName),
    /// A layover that came due and was picked up by a resuming pipeline.
    Resumed(LayoverId),
}

impl Origin {
    /// Returns the sending agent, or `None` for work that entered the mesh from outside it.
    #[must_use]
    pub fn agent(&self) -> Option<&AgentName> {
        match self {
            Self::Agent(name) => Some(name),
            Self::Human | Self::Schedule(_) | Self::Resumed(_) => None,
        }
    }
}

/// A message in transit between agents.
///
/// `hops_remaining` is mirrored here for the transcript. The authoritative value lives in the
/// Tower, keyed by itinerary, because a wrapped CLI is a black box that could otherwise claim
/// any budget it liked.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(from = "wire::Flight", into = "wire::Flight")]
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

/// The shape a flight is stored in, which a 1.0.0 binary can still read.
mod wire {
    use jiff::Timestamp;
    use serde::{Deserialize, Serialize};

    use super::{FlightId, ItineraryId, Origin};
    use crate::agent::AgentName;
    use crate::layover::LayoverId;
    use crate::pipeline::PipelineName;

    /// The two senders 1.0.0 understands.
    #[derive(Deserialize, Serialize)]
    #[serde(rename_all = "snake_case")]
    pub(super) enum Sender {
        Human,
        Agent(AgentName),
    }

    /// How work that no agent sent entered the mesh, when it was not a person.
    #[derive(Deserialize, Serialize)]
    #[serde(rename_all = "snake_case")]
    pub(super) enum Via {
        Schedule(PipelineName),
        Resumed(LayoverId),
    }

    #[derive(Deserialize, Serialize)]
    pub(super) struct Flight {
        id: FlightId,
        itinerary: ItineraryId,
        from: Sender,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        via: Option<Via>,
        to: AgentName,
        body: String,
        hops_remaining: u32,
        sent_at: Timestamp,
    }

    impl From<Flight> for super::Flight {
        fn from(wire: Flight) -> Self {
            let from = match (wire.from, wire.via) {
                (Sender::Agent(name), _) => Origin::Agent(name),
                (Sender::Human, Some(Via::Schedule(pipeline))) => Origin::Schedule(pipeline),
                (Sender::Human, Some(Via::Resumed(layover))) => Origin::Resumed(layover),
                (Sender::Human, None) => Origin::Human,
            };

            Self {
                id: wire.id,
                itinerary: wire.itinerary,
                from,
                to: wire.to,
                body: wire.body,
                hops_remaining: wire.hops_remaining,
                sent_at: wire.sent_at,
            }
        }
    }

    impl From<super::Flight> for Flight {
        fn from(flight: super::Flight) -> Self {
            let (from, via) = match flight.from {
                Origin::Agent(name) => (Sender::Agent(name), None),
                Origin::Human => (Sender::Human, None),
                Origin::Schedule(pipeline) => (Sender::Human, Some(Via::Schedule(pipeline))),
                Origin::Resumed(layover) => (Sender::Human, Some(Via::Resumed(layover))),
            };

            Self {
                id: flight.id,
                itinerary: flight.itinerary,
                from,
                via,
                to: flight.to,
                body: flight.body,
                hops_remaining: flight.hops_remaining,
                sent_at: flight.sent_at,
            }
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

    #[test]
    fn a_clock_and_a_layover_have_no_upstream_edge_either() {
        assert!(
            Origin::Schedule(PipelineName::new("nightly"))
                .agent()
                .is_none()
        );
        assert!(Origin::Resumed(LayoverId::from("lay_1")).agent().is_none());
    }

    fn sent(from: Origin) -> Flight {
        Flight::new(
            ItineraryId::generate(),
            from,
            AgentName::new("worker"),
            "go",
            4,
        )
    }

    #[test]
    fn every_origin_survives_a_round_trip_through_storage() {
        for origin in [
            Origin::Human,
            Origin::Agent(AgentName::new("analyst")),
            Origin::Schedule(PipelineName::new("nightly")),
            Origin::Resumed(LayoverId::from("lay_01TEST")),
        ] {
            let flight = sent(origin);
            let json = serde_json::to_string(&flight).expect("serialises");
            let read: Flight = serde_json::from_str(&json).expect("deserialises");
            assert_eq!(read, flight, "{json}");
        }
    }

    /// The flight as a 1.0.0 binary declares it, which is what has to keep parsing.
    #[derive(Deserialize)]
    #[allow(dead_code)]
    struct OneZeroFlight {
        id: FlightId,
        itinerary: ItineraryId,
        from: OneZeroOrigin,
        to: AgentName,
        body: String,
        hops_remaining: u32,
        sent_at: Timestamp,
    }

    #[derive(Deserialize, Debug, PartialEq)]
    #[serde(rename_all = "snake_case")]
    enum OneZeroOrigin {
        Human,
        Agent(AgentName),
    }

    #[test]
    fn a_scheduled_flight_is_stored_in_a_shape_1_0_can_still_read() {
        // 1.0.0 drops a queue line it cannot parse and then rewrites the queue without it. A new
        // variant on disk would turn a downgrade into lost work; a field it ignores loses a label.
        for origin in [
            Origin::Schedule(PipelineName::new("nightly")),
            Origin::Resumed(LayoverId::from("lay_01TEST")),
        ] {
            let json = serde_json::to_string(&sent(origin)).expect("serialises");
            let old: OneZeroFlight = serde_json::from_str(&json)
                .unwrap_or_else(|error| panic!("1.0.0 cannot read {json}: {error}"));
            assert_eq!(old.from, OneZeroOrigin::Human, "{json}");
        }
    }

    #[test]
    fn a_flight_written_by_1_0_still_reads() {
        let json = r#"{"id":"flt_1","itinerary":"itn_1","from":{"agent":"analyst"},"to":"developer","body":"go","hops_remaining":3,"sent_at":"2026-09-20T10:00:00Z"}"#;
        let read: Flight = serde_json::from_str(json).expect("reads");
        assert_eq!(read.from, Origin::Agent(AgentName::new("analyst")));

        let json = r#"{"id":"flt_2","itinerary":"itn_1","from":"human","to":"developer","body":"go","hops_remaining":3,"sent_at":"2026-09-20T10:00:00Z"}"#;
        let read: Flight = serde_json::from_str(json).expect("reads");
        assert_eq!(read.from, Origin::Human);
    }
}
