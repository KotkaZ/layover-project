//! Domain types for Layover.
//!
//! This crate is deliberately free of I/O and process handling. It contains the parts of the
//! design that can be reasoned about and tested in isolation:
//!
//! - [`config`] — parsing a factory definition from `layover.toml`
//! - [`graph`] — the route map as a directed graph
//! - [`mod@validate`] — load-time checks that fail before a human stops watching
//! - [`flight`] — the message envelope
//! - [`itinerary`] — Hops, Fuel and the deterministic run cap
//! - [`barrier`] — rendezvous joins, including reset and reachability-based abandonment
//!
//! See `docs/architecture.md` and `docs/routing.md` in the repository for the design these
//! types implement.

pub mod barrier;
pub mod config;
pub mod flight;
pub mod graph;
pub mod itinerary;
pub mod validate;

pub use barrier::{Barrier, BarrierKey, Delivery};
pub use config::{Access, AgentName, Config, ConfigError, Join, Mode};
pub use flight::{Flight, FlightId, ItineraryId, Origin};
pub use graph::RouteGraph;
pub use itinerary::{Denial, Itinerary};
pub use validate::{Diagnostic, Severity, validate};
