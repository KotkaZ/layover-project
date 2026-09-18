//! The channel agents talk to Layover through.
//!
//! # Why MCP and not a protocol of our own
//!
//! All three target CLIs speak MCP natively, so agents gain the ability to message each other with
//! no adapter code. That also answered the interop question: the control channel and the
//! integration standard turned out to be the same problem.
//!
//! # What a tool call is allowed to assume about its caller
//!
//! Nothing. A request arrives with a token, and the token *is* the identity — the Tower looks up
//! which run, which agent and which itinerary it belongs to, and the agent never asserts any of
//! them. An agent that could name itself could name a different agent, and every rail in the
//! system is indexed by that name.
//!
//! That is why [`Session`] is built by the Tower from a token rather than parsed from a request.
//! There is no code path in which a field an agent sent becomes an identity.

pub mod protocol;
pub mod runtime;
pub mod serve;

pub use protocol::{PROTOCOL_VERSION, Request, Response, handle, malformed};
pub use runtime::{Peer, Runtime, Session, ToolError};
pub use serve::{AUTH_HEADER, ENDPOINT_PATH, Served, Sessions, router, token_from};
