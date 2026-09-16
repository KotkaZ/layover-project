//! On-disk history: what the factory did, for as long as it is worth keeping.
//!
//! # Why a directory of text files and not a database
//!
//! History is append-only, written once and read in whole windows. That is the one shape a
//! database is *least* needed for, and a database would cost more than it looks: `rusqlite`
//! bundles a C library, which means a C cross-compiler for each of the five targets Layover
//! releases to. Trading the musl and aarch64 builds for query planning that a few megabytes of
//! JSON does not need is a bad deal.
//!
//! So: one JSON object per line, in files segmented by day. That gives three things a single
//! growing file would not.
//!
//! - **Retention is deleting files.** No rewriting, no compaction, no window where history is
//!   half-pruned because the process died in the middle of it.
//! - **Reading a window touches only the days in it.** "Last 7 days" opens seven files, not all
//!   ninety.
//! - **A corrupt line costs one line.** A partial write at the end of a file — which is what a
//!   power cut leaves — is skipped, and everything before it still reads.
//!
//! # Why the segments are UTC days
//!
//! The *reporting* windows are local ([`layover_core::cost::Window`]), but the *files* are UTC,
//! and the mismatch is deliberate. A local-day filename shifts when the machine changes zone or
//! the clocks go back: two different days would want the same name, or one day would be written
//! into two files. UTC has no such day. Reading a local window therefore opens one extra file at
//! each end and filters by instant, which is cheap and cannot be wrong.

mod history;
mod journal;
mod segment;

pub use history::{History, RunFilter, StoreError};
pub use journal::{HelpFilter, Journal};
