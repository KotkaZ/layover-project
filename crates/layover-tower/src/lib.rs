//! The supervisor: starting an agent CLI, watching it, and writing down what happened.
//!
//! # What lives here and what does not
//!
//! Everything in `layover-core` is a decision that can be checked without running anything —
//! whether a route is permitted, whether an itinerary has Fuel left, what a run should be told.
//! This crate is where those decisions meet an operating system, and it is the first place in the
//! project that can do something irreversible.
//!
//! It deliberately decides nothing. Routing, accounting and prompt composition belong to
//! `layover-core` and are already tested without a process in sight. What this adds is the part
//! that cannot be tested that way: a child that writes to disk, spends money, and may not come
//! back.
//!
//! # The order of operations is a safety property
//!
//! A run is recorded as started **before** the process is spawned, and the record carries the
//! process identifier and the moment it began. That order is not an implementation detail:
//!
//! - A run that was alive when the supervisor died leaves no exit code. The only way to know it
//!   existed is to have written it down first.
//! - A record written *after* spawning leaves a window in which a real process is running and
//!   nothing knows about it — exactly the window a crash falls into.
//!
//! Recovery needs to say "this was running, and it is confirmed gone". Both halves come from the
//! record written before the spawn.

pub mod spawn;

pub use spawn::{Finished, Plan, SpawnError, Started, start};
