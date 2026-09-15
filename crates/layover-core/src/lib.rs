//! Domain types for Layover.
//!
//! This crate holds the parts of the design that can be reasoned about and tested without
//! spawning anything. It reads configuration and prompt files from disk; it does not run
//! processes, serve HTTP or call an LLM.
//!
//! - [`agent`] — who an agent is: name, description, purpose, access
//! - [`route`] — edges in the route map
//! - [`pipeline`] — named entry points, their triggers and their flags
//! - [`prompt`] — composing an agent's instructions from files, conditional on flags
//! - [`config`] — parsing a factory definition from `layover.toml`
//! - [`graph`] — the route map as a directed graph
//! - [`mod@validate`] — load-time checks that fail before a human stops watching
//! - [`flight`] — the message envelope
//! - [`itinerary`] — Hops, Fuel and the deterministic run cap
//! - [`barrier`] — rendezvous joins, including reset and reachability-based abandonment
//!
//! See `docs/architecture.md` and `docs/routing.md` in the repository for the design these
//! types implement.

pub mod agent;
pub mod barrier;
pub mod config;
pub mod flight;
pub mod graph;
pub mod itinerary;
pub mod pipeline;
pub mod prompt;
pub mod route;
pub mod validate;

pub use agent::{Access, Agent, AgentName, PromptSpec, PromptSpecError};
pub use barrier::{Barrier, BarrierKey, Delivery};
pub use config::{Config, ConfigError, Defaults, McpWiring, Paths, Runner};
pub use flight::{Flight, FlightId, ItineraryId, Origin};
pub use graph::RouteGraph;
pub use itinerary::{Denial, Itinerary};
pub use pipeline::{
    FlagError, FlagSpec, Flags, Pipeline, PipelineName, Schedule, Trigger, TriggerError,
};
pub use prompt::{PromptDir, PromptError, PromptMap, PromptSource};
pub use route::{Join, Mode, Route};
pub use validate::{Diagnostic, Severity, validate, validate_prompts};
