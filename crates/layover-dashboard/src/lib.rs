//! The monitoring dashboard.
//!
//! Three questions, which is what a factory running unattended actually raises: *what is it
//! wired up to do*, *what has it been doing*, and *what is that costing*. The route map answers
//! the first, the run history the second, and the cost page the third.
//!
//! # Why this is server-rendered
//!
//! Everything here is HTML, CSS and a little vanilla JavaScript, embedded in the binary. There is
//! no npm, no build step and no framework, for three reasons that all point the same way.
//!
//! - Layover ships as **one binary** to five targets, installed by people who are not expected to
//!   have a Rust toolchain, let alone a Node one. A second toolchain in the release pipeline
//!   would be a real cost for a UI this size.
//! - The graph is drawn as **SVG generated in Rust**, so the layout is a pure function with unit
//!   tests rather than a 2.5 MB JavaScript dependency whose output could only be eyeballed.
//! - It has to work **offline**, on a machine left running overnight, which rules out a CDN.
//!
//! The choice is reversible: the page only consumes the HTTP API, so replacing it with a
//! single-page application later changes nothing behind it.
//!
//! # What it deliberately does not do
//!
//! One control writes: a manual trigger queues a flight. What makes that honest is that the thing
//! it produces is real and durable — the Tower will drain the queue when it exists — and the window
//! says the work is queued rather than started. Everything else is read-only, and the operations
//! that need a running supervisor answer [`layover_http::Problem`] with `501` rather than
//! pretending. A page that silently does nothing when you press the button is worse than one with
//! no button.

mod api;
mod assets;
mod itinerary;
mod view;

pub use api::{Dashboard, DashboardState};
pub use assets::router;
