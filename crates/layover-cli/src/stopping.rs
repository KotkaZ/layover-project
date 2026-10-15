//! How `serve` stops: drained by default, cut off only when asked twice.
//!
//! A Tower that ends its process at the first Ctrl+C cuts every live run off — repeated work and
//! spend at the next start — and loses any arrival parked at a rendezvous. So the first Ctrl+C, or
//! `layover stop`, drains instead: nothing new starts, the dashboard and the MCP endpoint stay up
//! for the runs still going, and the process exits when the factory is quiet. A second Ctrl+C, or
//! `layover stop --now`, cuts what is left off: each live run is killed and left for the next Tower
//! to settle, which is what the first Ctrl+C used to do to any run that shared its console.
//!
//! Both ways in write the same request file, so a dashboard watching the factory from another
//! process can say it is stopping, and `layover stop` from another terminal is no different from a
//! Ctrl+C in this one. See [`layover_store::stop`].

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use layover_core::agent::AgentName;
use layover_store::stop::{self, StopRequest};
use layover_update::handover::{self, Request};

use crate::tower::TowerControl;

/// How often a stopping Tower looks for a request and for whether it has gone quiet.
const LOOK: Duration = Duration::from_millis(500);

/// How long open connections get once the Tower has stopped.
const CLOSING: Duration = Duration::from_secs(3);

/// How long a Tower cutting off waits for its runs to be killed before it exits regardless.
const CUTTING: Duration = Duration::from_secs(30);

/// How a `serve` came to stop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stopped {
    /// Everything it was running finished, and nothing was parked.
    Drained,
    /// It was told to stop now, and these runs were still alive.
    CutOff(Vec<AgentName>),
    /// A dashboard with no Tower behind it, which has nothing to drain.
    Watching,
    /// Quiet, for a release to be installed and the Tower handed over.
    ForUpdate(Request),
    /// Handed over to a new release; the process ends with this code.
    HandedOver(i32),
}

/// Serves `router` on `listener` until the Tower has stopped, and says how it stopped.
///
/// The server stops taking requests once the Tower has stopped, and then gives open connections a
/// few seconds: a session's live transcript is a stream that never ends by itself, and waiting for
/// it would keep a stopped Tower from exiting.
///
/// # Errors
///
/// Returns the server's own I/O error.
pub async fn serve_until_stopped(
    listener: tokio::net::TcpListener,
    router: axum::Router,
    control: Option<TowerControl>,
    state: PathBuf,
    max_drain: Duration,
) -> std::io::Result<Option<Stopped>> {
    let stopped = Arc::new(Mutex::new(None));
    let noted = Arc::clone(&stopped);
    let finished = Arc::new(tokio::sync::Notify::new());
    let finishing = Arc::clone(&finished);
    let signal = async move {
        let how = until_stopped(control, state, max_drain).await;
        *noted.lock().unwrap_or_else(PoisonError::into_inner) = Some(how);
        finishing.notify_one();
    };
    let server = axum::serve(listener, router).with_graceful_shutdown(signal);
    tokio::select! {
        result = server => result?,
        () = async {
            finished.notified().await;
            tokio::time::sleep(CLOSING).await;
        } => {}
    }
    Ok(stopped
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take())
}

/// Waits until the Tower should stop, draining it once asked, and says how it stopped.
///
/// `control` is `None` for `--watch-only`, which honours only its own Ctrl+C: the request file
/// belongs to the Tower, and a watching dashboard must neither act on it nor write one, or
/// stopping the dashboard would stop the factory it is watching.
///
/// A request to install a release drains the Tower the same way, and ends it ready for the install
/// — unless the install is postponed or cancelled, when the Tower carries on serving and waits for
/// the next request.
pub async fn until_stopped(
    control: Option<TowerControl>,
    state: PathBuf,
    max_drain: Duration,
) -> Stopped {
    let Some(control) = control else {
        let _ = tokio::signal::ctrl_c().await;
        return Stopped::Watching;
    };

    loop {
        match first_request(&state).await {
            Asked::Stop(asked) => return stopping(&control, &state, &asked).await,
            Asked::Update(request) => {
                match crate::updating::drain_for(&control, &state, request, max_drain).await {
                    crate::updating::Drain::Ready(request) => return Stopped::ForUpdate(request),
                    crate::updating::Drain::Stop(asked) => {
                        return stopping(&control, &state, &asked).await;
                    }
                    crate::updating::Drain::Abandoned => {}
                }
            }
        }
    }
}

/// Drains to stop, or cuts off when asked to stop now.
async fn stopping(control: &TowerControl, state: &Path, asked: &StopRequest) -> Stopped {
    if asked.now {
        return Stopped::CutOff(cut_off(control).await);
    }

    control.start_draining();
    let mut said = String::new();
    loop {
        if control.is_quiescent() {
            return Stopped::Drained;
        }
        if stop::pending(state).is_some_and(|request| request.now) {
            return Stopped::CutOff(cut_off(control).await);
        }

        let waiting = waiting_for(control);
        if waiting != said {
            println!("  Stopping: nothing new starts. Waiting for {waiting}.");
            println!("  Press Ctrl+C again, or run `layover stop --now`, to cut it off.");
            said = waiting;
        }

        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                let _ = stop::request(state, &StopRequest::now("a second Ctrl+C"));
                return Stopped::CutOff(cut_off(control).await);
            }
            () = tokio::time::sleep(LOOK) => {}
        }
    }
}

/// Kills what is running and waits for each run to be left for the next Tower, so nothing the
/// factory started outlives the Tower that could hear from it. Says which agents were cut off.
pub(crate) async fn cut_off(control: &TowerControl) -> Vec<AgentName> {
    let agents = control.alive_agents();
    control.cut_off();
    let began = std::time::Instant::now();
    while !control.is_idle() && began.elapsed() < CUTTING {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    agents
}

/// What the Tower was first asked.
enum Asked {
    Stop(StopRequest),
    Update(Request),
}

/// The first request: to stop — a Ctrl+C here, or a file another process wrote — or to install a
/// release.
async fn first_request(state: &Path) -> Asked {
    loop {
        if let Some(request) = stop::pending(state) {
            return Asked::Stop(request);
        }
        if let Some(request) = handover::request(state)
            && request.outcome.is_none()
        {
            return Asked::Update(request);
        }
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                let request = StopRequest::drain("Ctrl+C");
                // Written so a dashboard watching from another process can say the Tower is
                // stopping. Best-effort: the drain does not depend on it.
                let _ = stop::request(state, &request);
                return Asked::Stop(request);
            }
            () = tokio::time::sleep(LOOK) => {}
        }
    }
}

/// What a drain is waiting for, as a person reads it: "2 run(s) (wolf, bob) and 1 rendezvous".
pub(crate) fn waiting_for(control: &TowerControl) -> String {
    let mut agents: Vec<String> = control
        .alive_agents()
        .iter()
        .map(ToString::to_string)
        .collect();
    agents.sort();
    let parked = control.parked_rendezvous();

    let runs = match agents.len() {
        0 => None,
        n => Some(format!("{n} run(s) to finish ({})", agents.join(", "))),
    };
    let joins = (parked > 0).then(|| format!("{parked} rendezvous to release"));
    match (runs, joins) {
        (Some(runs), Some(joins)) => format!("{runs} and {joins}"),
        (Some(only), None) | (None, Some(only)) => only,
        (None, None) => "the last run to be recorded".to_owned(),
    }
}

/// What to print once stopped.
#[must_use]
pub fn report(stopped: &Stopped) -> String {
    match stopped {
        Stopped::Drained => {
            "Stopped: everything that was running has finished. Queued work waits for the next start."
                .to_owned()
        }
        Stopped::Watching => "Stopped watching.".to_owned(),
        Stopped::ForUpdate(request) => format!("Stopped to install {}.", request.version),
        Stopped::HandedOver(0) => "Handed over to the new Tower.".to_owned(),
        Stopped::HandedOver(_) => {
            "Stopped for the service to start the new release.".to_owned()
        }
        Stopped::CutOff(agents) if agents.is_empty() => {
            "Stopped now. Nothing was running.".to_owned()
        }
        Stopped::CutOff(agents) => {
            let mut names: Vec<String> = agents.iter().map(ToString::to_string).collect();
            names.sort();
            format!(
                "Stopped now, cutting off {} run(s) ({}). The next Tower to open this factory \
                 records them as interrupted, and restarts them where their agents' `recovery` \
                 allows.",
                names.len(),
                names.join(", ")
            )
        }
    }
}

/// Says how `serve` stopped and, when it ran the factory, takes the request to stop back.
///
/// Every way of stopping exits 0, cut off or not: the Tower stopped because it was asked to, and a
/// service manager that restarts a Tower which fails — systemd's `Restart=on-failure`, launchd's
/// `KeepAlive` — must not bring back one somebody stopped. A Tower that was cut off exits here
/// rather than through its own shutdown, which would wait for a run whose kill had not landed.
pub fn finish(stopped: Option<Stopped>, state: Option<&Path>) {
    let Some(stopped) = stopped else {
        return;
    };
    println!("{}", report(&stopped));
    if let Some(state) = state {
        stop::clear(state);
    }
    match stopped {
        Stopped::CutOff(_) => std::process::exit(0),
        Stopped::HandedOver(code) => std::process::exit(code),
        _ => {}
    }
}

/// Removes a request left behind by a Tower that went away before it honoured it, and says so.
///
/// Any request present when a Tower starts was written before it: honouring it would stop a Tower
/// nobody asked to stop.
#[must_use]
pub fn clear_stale(state: &Path) -> Option<String> {
    let left = stop::pending(state)?;
    stop::clear(state);
    Some(format!(
        "Ignored a request to stop from {} by {}, left by a Tower that has gone.",
        left.at, left.by
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_drain_says_it_finished_and_a_cut_off_says_what_it_cut() {
        let said = report(&Stopped::Drained);
        assert!(said.starts_with("Stopped: everything"), "{said}");

        let said = report(&Stopped::CutOff(vec![
            AgentName::new("wolf"),
            AgentName::new("bob"),
        ]));
        assert!(said.contains("cutting off 2 run(s) (bob, wolf)"), "{said}");
        assert!(said.contains("restarts them"), "{said}");
    }

    #[test]
    fn a_request_left_by_a_tower_that_went_away_is_ignored_and_removed() {
        let state = std::env::temp_dir().join(format!("layover-stale-stop-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&state);
        assert_eq!(clear_stale(&state), None);

        stop::request(&state, &StopRequest::drain("karl")).expect("writes");
        let said = clear_stale(&state).expect("said");

        assert!(said.contains("by karl"), "{said}");
        assert!(stop::pending(&state).is_none());
        let _ = std::fs::remove_dir_all(&state);
    }
}
