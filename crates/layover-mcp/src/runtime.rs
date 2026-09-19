//! What a tool call needs from the world, and what it may know about its caller.
//!
//! # Identity comes from the token, never from the request
//!
//! A [`Session`] is built by the Tower from the token a run was given, not parsed from anything
//! the agent sent. That is the whole of `architecture.md` §4.2 in one type: an agent that could
//! name itself could name a *different* agent, and every rail in the system — Hops, Fuel, the run
//! cap, who may send to whom — is indexed by that name.
//!
//! So there is deliberately no constructor taking an agent name from a request. The only way to
//! get a `Session` is to resolve a token that the Tower minted.

use layover_core::agent::AgentName;
use layover_core::flight::{ItineraryId, RunId};

/// Who is calling, as the Tower knows them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    /// The run that holds the token.
    pub run: RunId,
    /// The agent that run is.
    pub agent: AgentName,
    /// The chain it belongs to.
    pub itinerary: ItineraryId,
    /// Messages this chain may still send.
    ///
    /// Carried here so a tool call can be refused without going back to storage, and so the agent
    /// can be told what it has left rather than discovering it by being refused.
    pub hops_remaining: u32,
}

/// An agent this one may send to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Peer {
    /// Its name, which is what `layover_send` takes.
    pub name: AgentName,
    /// What it is for, written for another agent to read.
    pub description: Option<String>,
    /// Whether reaching it opens a new chain rather than continuing this one.
    pub spawns: bool,
}

/// Why a tool call did not do what was asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolError {
    /// The route map does not permit it.
    NotPermitted {
        /// Who tried.
        from: AgentName,
        /// Who they tried to reach.
        to: AgentName,
    },
    /// No agent of that name is declared.
    NoSuchAgent {
        /// The name that is not declared.
        agent: AgentName,
    },
    /// A safety rail refused it.
    Refused {
        /// What the agent should be told.
        because: String,
    },
    /// An argument was missing or the wrong shape.
    BadArguments {
        /// What was wrong.
        detail: String,
    },
    /// Something failed that is not the agent's fault.
    Unavailable {
        /// What failed.
        detail: String,
    },
}

impl std::fmt::Display for ToolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotPermitted { from, to } => write!(
                f,
                "`{from}` may not send to `{to}`. Call layover_peers to see who you can reach."
            ),
            Self::NoSuchAgent { agent } => {
                write!(f, "there is no agent called `{agent}` in this factory")
            }
            // One arm for three variants: each already holds a sentence written for the agent to
            // read, so wrapping them in further words would only get between the agent and the
            // answer. They stay separate variants because the *caller* distinguishes them.
            Self::Refused { because: text }
            | Self::BadArguments { detail: text }
            | Self::Unavailable { detail: text } => f.write_str(text),
        }
    }
}

/// Everything a tool call needs from the world outside itself.
///
/// A trait so the protocol layer can be tested exhaustively without a factory, a filesystem or a
/// process — which matters because the protocol layer is where a malformed request from an
/// untrusted child is first handled.
pub trait Runtime {
    /// Who `session`'s agent may send to.
    fn peers(&self, session: &Session) -> Vec<Peer>;

    /// Sends work to another agent, returning the flight's identifier.
    ///
    /// # Errors
    ///
    /// Returns [`ToolError`] when the route is not permitted, the agent is unknown, or a rail
    /// refuses it.
    fn send(&self, session: &Session, to: &AgentName, body: &str) -> Result<String, ToolError>;

    /// Records what this run concluded.
    ///
    /// # Errors
    ///
    /// Returns [`ToolError`] when the report cannot be stored.
    fn report(&self, session: &Session, headline: &str, body: &str) -> Result<(), ToolError>;

    /// Records that this run could not get past something.
    ///
    /// # Errors
    ///
    /// Returns [`ToolError`] when the request cannot be stored.
    fn help(
        &self,
        session: &Session,
        summary: &str,
        detail: &str,
        fatal: bool,
    ) -> Result<(), ToolError>;

    /// Reads this agent's own notes.
    ///
    /// # Errors
    ///
    /// Returns [`ToolError`] when they cannot be read.
    fn memory_read(&self, session: &Session) -> Result<String, ToolError>;

    /// Adds to this agent's own notes.
    ///
    /// # Errors
    ///
    /// Returns [`ToolError`] when they cannot be written.
    fn memory_write(&self, session: &Session, text: &str) -> Result<(), ToolError>;

    /// Sets this work down to be picked up later, returning when it will be.
    ///
    /// `until` is how long to wait — `30m`, `2h`, `3d` — using the same vocabulary as a pipeline's
    /// `every`. An agent asked to wait "until the review lands" has no way to know when that is,
    /// so it names an interval and is brought back to look.
    ///
    /// # Errors
    ///
    /// Returns [`ToolError`] when `until` cannot be read, or the layover cannot be stored.
    fn wait(&self, session: &Session, until: &str, because: &str) -> Result<String, ToolError>;

    /// Proposes something future runs of this agent should know.
    ///
    /// The answer says what became of it, because the states are not interchangeable: a proposal
    /// taken up applies to the next run, one that echoes advice the agent was already given is
    /// evidence of nothing, and one a human rejected is not reopened by repetition.
    ///
    /// # Errors
    ///
    /// Returns [`ToolError`] when the proposal cannot be stored.
    fn learn(&self, session: &Session, text: &str) -> Result<String, ToolError>;

    /// Adds a line to the factory's shared memory, which every agent can read.
    ///
    /// # Errors
    ///
    /// Returns [`ToolError`] when it cannot be written.
    fn logbook_append(&self, session: &Session, text: &str) -> Result<(), ToolError>;
}
