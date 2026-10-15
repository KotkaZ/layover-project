//! Finding out whether a newer Layover has been released.
//!
//! Layover may update itself — never while a run is alive or a rendezvous holds work — and this
//! crate is the part that needs no Tower: where releases are published, which one is newer, what
//! this machine has already been told, and what a person said to skip. It depends on neither the
//! Tower nor the dashboard, which ask it.
//!
//! What a factory does about a release is its `[update]` table. By default it is told: once a day
//! the Tower asks, and the dashboard, `layover doctor` and `layover update check` say what was
//! found. `LAYOVER_UPDATE=off` stops every request on a machine, whatever a factory says.

pub mod check;
pub mod manifest;
pub mod source;
pub mod state;
pub mod version;

pub use check::{
    Available, CheckError, Settings, Status, available, check, compatibility, due, next_check,
    status,
};
pub use layover_core::config::{UpdateChannel as Channel, UpdateConfig, UpdateMode as Mode};
pub use source::{Source, SourceError};
pub use state::{LastCheck, Release};
pub use version::{Kind, Version};

/// The platform this binary was built for, such as `x86_64-pc-windows-msvc`: what an update must
/// have been built for too.
pub const TARGET: &str = env!("LAYOVER_TARGET");

/// The oldest release this one can be updated from in one step.
///
/// Raised only by a release that cannot be reached directly — one whose migration of the state
/// directory needs a step an older release has to take first. Every release so far can be reached
/// from 1.0.0.
pub const MIN_UPGRADE_FROM: &str = "1.0.0";
