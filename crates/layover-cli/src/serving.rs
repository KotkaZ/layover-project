//! The dashboard's side of `serve`: binding, saying where, serving until the Tower stops — and,
//! when it stopped to install a release that then failed, binding again and carrying on.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use layover_dashboard::{Dashboard, Guard};
use layover_update::handover::Serving;

use crate::stopping::{Stopped, serve_until_stopped};
use crate::tower::TowerControl;
use crate::updating::{Context, HandOver, hand_over};

/// Everything `serve` hands the dashboard's loop.
pub struct Served {
    /// Where to listen.
    pub addr: String,
    /// The factory's definition.
    pub path: PathBuf,
    /// Where its history is.
    pub history_dir: PathBuf,
    /// Who may connect.
    pub guard: Guard,
    /// Where agents reach Layover, when this process runs the factory.
    pub endpoint: Option<String>,
    /// The Tower, when this process runs one.
    pub control: Option<TowerControl>,
    /// The factory's `.layover`.
    pub state: PathBuf,
    /// The dashboard.
    pub dashboard: Dashboard,
    /// How long an install waits for runs to finish.
    pub max_drain: Duration,
    /// The lock that says this process serves the factory.
    pub serving: Arc<Mutex<Option<Serving>>>,
    /// Whether this is a new release still to prove itself.
    pub proving: bool,
}

/// Serves until the Tower stops, and says how it stopped.
///
/// # Errors
///
/// Says why the address could not be bound, or the server failed.
pub async fn run(served: Served) -> Result<Option<Stopped>, String> {
    let mut again: Option<SocketAddr> = None;
    loop {
        // Bind before announcing, so the address printed is a fact rather than a hope. Bound again
        // at the same address after an install that failed, which the new Tower may have held.
        let listener = match again {
            None => tokio::net::TcpListener::bind(&served.addr)
                .await
                .map_err(|error| format!("could not listen on {}: {error}", served.addr))?,
            Some(bound) => rebind(bound).await?,
        };
        let bound = listener.local_addr().map_err(|error| error.to_string())?;
        if again.is_some() {
            println!("  Serving again on {bound}.");
        } else {
            announce(
                &served.guard,
                bound,
                &served.path,
                &served.history_dir,
                served.endpoint.as_deref(),
                served.control.is_some(),
            );
            if served.proving
                && let Some(control) = &served.control
            {
                crate::arriving::prove(
                    served.state.clone(),
                    bound,
                    served.guard.token().map(str::to_owned),
                    control.clone(),
                );
            }
        }

        let router = layover_dashboard::router(served.dashboard.clone(), served.guard.clone());
        let stopped = serve_until_stopped(
            listener,
            router,
            served.control.clone(),
            served.state.clone(),
            served.max_drain,
        )
        .await
        .map_err(|error| error.to_string())?;

        let (Some(Stopped::ForUpdate(request)), Some(control)) = (&stopped, &served.control) else {
            return Ok(stopped);
        };
        let settings = layover_update::Settings::from_env();
        let context = Context {
            state: &served.state,
            address: bound,
            token: served.guard.token(),
            control,
            serving: &served.serving,
            settings: &settings,
        };
        match hand_over(&context, request) {
            HandOver::Exit(code) => return Ok(Some(Stopped::HandedOver(code))),
            HandOver::Resume => again = Some(bound),
        }
    }
}

/// Binds `address` again, waiting up to thirty seconds for a Tower that was stopped to let it go.
async fn rebind(address: SocketAddr) -> Result<tokio::net::TcpListener, String> {
    let began = Instant::now();
    loop {
        match tokio::net::TcpListener::bind(address).await {
            Ok(listener) => return Ok(listener),
            Err(error) if began.elapsed() > Duration::from_secs(30) => {
                return Err(format!("could not listen on {address} again: {error}"));
            }
            Err(_) => tokio::time::sleep(Duration::from_millis(500)).await,
        }
    }
}

/// What `serve` says once it is listening: where, what it reads, and how to stop it.
fn announce(
    guard: &Guard,
    bound: std::net::SocketAddr,
    path: &Path,
    history_dir: &Path,
    endpoint: Option<&str>,
    drains: bool,
) {
    println!("Layover dashboard on {}", guard.address(&bound));
    println!("Reading {}", path.display());
    println!("History in {}", history_dir.display());

    if guard.token().is_some() {
        println!("The token is in that address; the page keeps it in a cookie afterwards.");
    } else {
        println!("Open to anything that can reach the port (--no-auth).");
    }

    match endpoint {
        Some(url) => {
            println!("Agents reach Layover at {url}");
            println!("Running the factory: scheduled pipelines fire, queued work starts.");
        }
        // Said plainly, because a dashboard showing a queue that nothing will drain is the
        // failure this whole surface is meant to avoid.
        None => println!("Watching only: nothing here will start work (--watch-only)."),
    }

    // A token makes an off-loopback bind defensible; without one it is an open control plane
    // on a network. The warning is about the combination, not the address.
    if !bound.ip().is_loopback() && guard.token().is_none() {
        eprintln!();
        eprintln!("warning: {bound} is not loopback, and --no-auth is set.");
        eprintln!("         Anyone who can reach it can read run history, reports and help");
        eprintln!("         requests, and queue work this process will run. Drop --no-auth,");
        eprintln!("         or put something in front of it.");
    }

    if drains {
        println!("Press Ctrl+C, or run `layover stop`, to stop once what is running finishes.");
    } else {
        println!("Press Ctrl+C to stop.");
    }
}
