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
//! - [cost] — the ledger, rate cards and the factory-wide Fuel Reserve
//! - [itinerary] — Hops, Fuel and the deterministic run cap
//! - [`barrier`] — rendezvous joins, including reset and reachability-based abandonment
//!
//! See `docs/architecture.md` and `docs/routing.md` in the repository for the design these
//! types implement.

pub mod agent;
pub mod autostart;
pub mod barrier;
pub mod brief;
pub mod config;
pub mod cost;
pub mod diagram;
pub mod flight;
pub mod graph;
pub mod handover;
pub mod help;
pub mod itinerary;
pub mod layover;
pub mod learning;
pub mod mcp;
pub mod payload;
pub mod pipeline;
pub mod prompt;
pub mod queue;
pub mod report;
pub mod route;
pub mod run;
pub mod slots;
pub mod tools;
pub mod validate;

pub use agent::{Access, Agent, AgentName, PromptSpec, PromptSpecError};
pub use autostart::{Autostart, Platform};
pub use barrier::{Barrier, BarrierKey, Delivery};
pub use brief::brief;
pub use config::{Config, ConfigError, Defaults, McpWiring, Paths, ReserveConfig, Runner};
pub use cost::{
    CostSource, Ledger, ModelRates, RateCard, Reserve, ReserveState, RunCost, Summary, TokenUsage,
};
pub use diagram::{Activity, Layout, Live, render_svg, route_map};
pub use flight::{Flight, FlightId, ItineraryId, Origin, RunId};
pub use graph::RouteGraph;
pub use handover::{
    Cause, ChildState, Handover, Interruption, Recovery, RecoveryDenied, RecoveryPolicy, Steer,
};
pub use help::{Blocker, HelpRequest};
pub use itinerary::{Denial, Itinerary};
pub use layover::{Layover, LayoverId};
pub use learning::{Impact, Learning, LearningId, Learnings, Proposal, Uptake};
pub use mcp::{McpServer, McpTransport};
pub use pipeline::{
    FlagError, FlagSpec, Flags, Pipeline, PipelineName, Schedule, Trigger, TriggerError, Workspace,
};
pub use prompt::{PromptDir, PromptError, PromptMap, PromptSource};
pub use report::Report;
pub use route::{Join, Mode, Route};
pub use run::{Outcome, RunRecord};
pub use slots::{Admission, Slots};
pub use validate::{Diagnostic, Severity, validate, validate_prompts};
