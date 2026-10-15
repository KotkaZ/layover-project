//! A Tower starting after an update: it counts its start, takes over the dashboard's token so a
//! page already open keeps working, proves the new version healthy — or, under a supervisor, rolls
//! it back when it cannot.
//!
//! Healthy means the factory loaded, the Tower went once round its loop, and its own dashboard
//! answered as the new version, all within ten minutes of the handover. Then the update is logged,
//! and the binary it replaced is kept for `[update] keep_previous` and removed at a start after.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use jiff::{SignedDuration, Timestamp};
use layover_update::handover::{self, Handover, Serving};
use layover_update::swap::{Swapped, forget_previous, restore};
use layover_update::{Settings, Version, log, state};

use crate::tower::TowerControl;

/// What a starting Tower takes from the one before it.
#[derive(Debug, Default)]
pub struct Arrival {
    /// The dashboard's token, to keep rather than mint.
    pub token: Option<String>,
    /// Whether this Tower is a new version still to prove itself.
    pub proving: bool,
}

/// The lock that says this process serves the factory, shared with what may give it up.
pub type Held = Arc<Mutex<Option<Serving>>>;

/// Takes the factory for this Tower — one Tower serves a factory — and reads what the Tower before
/// it left. A `--watch-only` dashboard takes nothing and reads nothing.
///
/// # Errors
///
/// Says another Tower already serves the factory.
pub fn claim(
    state: &Path,
    path: &Path,
    watch_only: bool,
    keep_previous: Duration,
) -> Result<(Held, Arrival), String> {
    if watch_only {
        return Ok((Arc::new(Mutex::new(None)), Arrival::default()));
    }
    let serving = handover::serve(state).map_err(|why| format!("{why}: {}", path.display()))?;
    let arrival = arrive(state, &Settings::from_env(), keep_previous);
    Ok((Arc::new(Mutex::new(Some(serving))), arrival))
}

/// Reads what the last Tower left, before anything else starts.
///
/// Under a supervisor, a new version that has started twice without proving itself, or has run out
/// of time, is rolled back here: the previous binary put back, the version skipped, and the
/// process ended with 75 for the supervisor to start the previous one.
pub fn arrive(state: &Path, settings: &Settings, keep_previous: Duration) -> Arrival {
    let running = Version::running();
    let now = Timestamp::now();

    if let Some(request) = handover::request(state)
        && request.outcome.is_none()
    {
        if request.version == running {
            let what = if request.rollback {
                "rolled_back"
            } else {
                "applied"
            };
            handover::progress(state, &format!("Layover {running} started."), Some(what));
        } else {
            handover::progress(
                state,
                "Cancelled: the Tower that took it went away.",
                Some("cancelled"),
            );
        }
    }

    let Some(mut record) = handover::handover(state) else {
        return Arrival::default();
    };
    let settled = record.healthy_at.is_some() || record.rolled_back_at.is_some();

    if record.to == running && !settled {
        record.boots += 1;
        let token = record.token.take();
        let overdue = record.boots > 2 || now > record.healthy_by;
        if overdue && crate::updating::supervised().is_some() {
            roll_back_self(state, settings, &mut record);
        }
        let _ = handover::put_handover(state, &record);
        return Arrival {
            token,
            proving: true,
        };
    }

    if record.from == running && record.to != running && !settled {
        // The previous version is running again: the new one never proved itself.
        record.rolled_back_at = Some(now);
        record.token = None;
        let _ = handover::put_handover(state, &record);
        let _ = state::skip(&settings.dir, &record.to);
        log::note(
            state,
            "rolled_back",
            &record.to.to_string(),
            &running.to_string(),
            "it never proved itself healthy",
        );
        println!(
            "Layover {} never proved itself healthy here, so {running} is running again and {} \
             will not be offered again.",
            record.to, record.to
        );
    }

    if let Some(healthy) = record.healthy_at
        && healthy
            .checked_add(SignedDuration::try_from(keep_previous).unwrap_or(SignedDuration::MAX))
            .is_ok_and(|until| until < now)
        && record.previous.exists()
    {
        forget_previous(&record.previous);
    }
    Arrival::default()
}

/// Puts the previous binary back and exits 75, for the supervisor to start it.
fn roll_back_self(state: &Path, settings: &Settings, record: &mut Handover) -> ! {
    let running = Version::running();
    let swapped = Swapped {
        binary: record.binary.clone(),
        previous: record.previous.clone(),
    };
    let restored = restore(&swapped);
    record.rolled_back_at = Some(Timestamp::now());
    record.token = None;
    let _ = handover::put_handover(state, record);
    let _ = state::skip(&settings.dir, &record.to);
    let why = format!(
        "started {} time(s) without proving itself healthy by {}",
        record.boots, record.healthy_by
    );
    log::note(
        state,
        "rolled_back",
        &running.to_string(),
        &record.from.to_string(),
        &why,
    );
    match restored {
        Ok(()) => eprintln!(
            "Layover {running} {why}: {} is back in place. Exiting 75 to be started again.",
            record.from
        ),
        Err(error) => eprintln!(
            "Layover {running} {why}, and {} could not be put back: {error}",
            record.from
        ),
    }
    std::process::exit(75);
}

/// Proves a new version healthy once it has been round its loop and its dashboard answers, on a
/// thread of its own, and records it.
pub fn prove(state: PathBuf, address: SocketAddr, token: Option<String>, control: TowerControl) {
    let spawned = std::thread::Builder::new()
        .name("layover-prove".to_owned())
        .spawn(move || {
            let running = Version::running();
            let Some(record) = handover::handover(&state) else {
                return;
            };
            // Round the loop once: a second pass has begun.
            while control.passes_begun() < 2 {
                if Timestamp::now() > record.healthy_by {
                    return;
                }
                std::thread::sleep(Duration::from_millis(200));
            }
            if !handover::answers_as(
                &address.to_string(),
                token.as_deref(),
                &running,
                Duration::from_secs(30),
            ) {
                return;
            }
            let Some(mut record) = handover::handover(&state) else {
                return;
            };
            if record.to != running || record.rolled_back_at.is_some() {
                return;
            }
            let now = Timestamp::now();
            record.healthy_at = Some(now);
            let _ = handover::put_handover(&state, &record);
            log::note(
                &state,
                "healthy",
                &record.from.to_string(),
                &running.to_string(),
                &format!("{} is kept at {}", record.from, record.previous.display()),
            );
            println!(
                "  Layover {running} is healthy. {} is kept at {} for a rollback.",
                record.from,
                record.previous.display()
            );
        });
    if let Err(error) = spawned {
        eprintln!("could not start proving the new version: {error}");
    }
}
