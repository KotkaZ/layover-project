//! A Tower installing a release into itself: draining for it, swapping the binary, handing over to
//! the new version, and — in the new Tower — proving it, or rolling it back.
//!
//! # Two ways back up
//!
//! Under systemd or launchd — the service `layover autostart` writes sets `LAYOVER_SUPERVISOR` —
//! the old Tower exits 75 once the binary is swapped, and the supervisor starts the new one: both
//! end every process in the service when its main one exits, so a successor the old Tower started
//! would die with it. Everywhere else, Windows always, the old Tower hands over in place: it closes
//! its dashboard, starts the new binary with the same arguments, detached, and waits up to two
//! minutes for the same address to answer as the new version. If it does not, the old Tower stops
//! the new one, puts its own binary back, and carries on serving — so even a release that cannot
//! start at all is rolled back by the Tower it was meant to replace.

use std::ffi::OsString;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use jiff::{SignedDuration, Timestamp};
use layover_store::stop::{self, StopRequest};
use layover_update::handover::{self, HEALTHY_WITHIN, Handover, Request, Serving};
use layover_update::swap::{Swapped, install_lock, restore, swap};
use layover_update::{Settings, Version, log, method, state};

use crate::tower::TowerControl;

/// How often a draining Tower looks again.
const LOOK: Duration = Duration::from_millis(500);

/// How long an old Tower waits for the new one to answer.
const ANSWER_WITHIN: Duration = Duration::from_mins(2);

/// How a drain for an update ended.
pub enum Drain {
    /// Quiet, or cut off at the deadline: install it.
    Ready(Request),
    /// The Tower was asked to stop instead.
    Stop(StopRequest),
    /// Postponed or cancelled: the Tower carries on serving.
    Abandoned,
}

/// What the old Tower does once it has tried to hand over.
pub enum HandOver {
    /// Exit with this code: 0 handed over in place, 75 for the supervisor to start the new one.
    Exit(i32),
    /// Carry on serving: the install failed and was undone.
    Resume,
}

fn say(state: &Path, line: &str, outcome: Option<&str>) {
    println!("  {line}");
    handover::progress(state, line, outcome);
}

/// Drains for `request`: until quiet, until the deadline, or until told otherwise.
pub async fn drain_for(
    control: &TowerControl,
    state: &Path,
    request: Request,
    max_drain: Duration,
) -> Drain {
    control.start_draining();
    say(
        state,
        &format!(
            "Installing {}: nothing new starts until it is in.",
            request.version
        ),
        None,
    );
    let deadline = Instant::now()
        + if request.now {
            Duration::ZERO
        } else {
            max_drain
        };
    let mut said = String::new();
    loop {
        if let Some(asked) = stop::pending(state) {
            say(
                state,
                "Cancelled: the Tower was asked to stop.",
                Some("cancelled"),
            );
            return Drain::Stop(asked);
        }
        if handover::request(state).is_none_or(|now| now.outcome.is_some()) {
            control.stop_draining();
            println!("  The install was cancelled; work starts again.");
            return Drain::Abandoned;
        }
        if control.is_quiescent() {
            say(state, "Nothing is running or parked.", None);
            return Drain::Ready(request);
        }
        if request.until.is_some_and(|until| Timestamp::now() >= until) {
            return postpone(
                control,
                state,
                &request,
                "Postponed: the window closed while runs were still going. Work starts again; the \
                 next window tries again.",
            );
        }
        if Instant::now() >= deadline {
            return at_deadline(control, state, request).await;
        }
        let waiting = crate::stopping::waiting_for(control);
        if waiting != said {
            say(state, &format!("Waiting for {waiting}."), None);
            said = waiting;
        }
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                let asked = StopRequest::drain("Ctrl+C");
                let _ = stop::request(state, &asked);
                say(state, "Cancelled: the Tower was asked to stop.", Some("cancelled"));
                return Drain::Stop(asked);
            }
            () = tokio::time::sleep(LOOK) => {}
        }
    }
}

/// Gives up on `request` for now: work starts again, and it is said and logged why.
fn postpone(control: &TowerControl, state: &Path, request: &Request, why: &str) -> Drain {
    control.stop_draining();
    say(state, why, Some("postponed"));
    let running = Version::running().to_string();
    log::note(
        state,
        "postponed",
        &running,
        &request.version.to_string(),
        why,
    );
    Drain::Abandoned
}

/// At `max_drain`: cuts off what is still running, for recovery to restart on the new version —
/// unless an agent whose work must not be done twice is among it, which postpones the install.
async fn at_deadline(control: &TowerControl, state: &Path, request: Request) -> Drain {
    let manual = control.alive_manual_agents();
    if !manual.is_empty() && !request.including_manual {
        let names: Vec<String> = manual.iter().map(ToString::to_string).collect();
        let line = format!(
            "Postponed: {} still running, and its work must not be done twice \
             (`recovery = \"manual\"`). Work starts again; ask again later.",
            names.join(", ")
        );
        return postpone(control, state, &request, &line);
    }
    let cut = crate::stopping::cut_off(control).await;
    if !cut.is_empty() {
        let names: Vec<String> = cut.iter().map(ToString::to_string).collect();
        say(
            state,
            &format!(
                "Cut off {} run(s) at the deadline ({}); recovery restarts them on {}.",
                names.len(),
                names.join(", "),
                request.version
            ),
            None,
        );
    }
    Drain::Ready(request)
}

/// The supervisor a service runs the Tower under, if any. Never on Windows, where the Task
/// Scheduler does not restart a task that exits non-zero.
///
/// `LAYOVER_SUPERVISOR` is what the service `layover autostart` writes says. A systemd unit written
/// by hand, or by an older release, is recognised by the `INVOCATION_ID` systemd gives every
/// service: under it a successor started in place would be killed with the service's control
/// group the moment this process exited.
pub(crate) fn supervised() -> Option<String> {
    if cfg!(windows) {
        return None;
    }
    let set = |name: &str| std::env::var(name).ok().filter(|value| !value.is_empty());
    set("LAYOVER_SUPERVISOR").or_else(|| set("INVOCATION_ID").map(|_| "systemd".to_owned()))
}

/// What a quiet Tower needs to install a release and hand over.
pub struct Context<'a> {
    /// The factory's `.layover`.
    pub state: &'a Path,
    /// Where its dashboard was bound.
    pub address: SocketAddr,
    /// Its dashboard's token.
    pub token: Option<&'a str>,
    /// The Tower.
    pub control: &'a TowerControl,
    /// The lock that says this Tower serves the factory, given up while the new one starts.
    pub serving: &'a Mutex<Option<Serving>>,
    /// The machine's update settings.
    pub settings: &'a Settings,
}

/// Installs what `request` asks for and hands over to it. Blocking.
pub fn hand_over(context: &Context, request: &Request) -> HandOver {
    let state = context.state;
    let running = Version::running();
    let fail = |why: String| {
        say(state, &format!("Not installed: {why}"), Some("failed"));
        log::note(
            state,
            "failed",
            &running.to_string(),
            &request.version.to_string(),
            &why,
        );
        resume(context);
        HandOver::Resume
    };

    let binary = match std::env::current_exe() {
        Ok(binary) => binary,
        Err(error) => return fail(format!("could not tell which binary is running: {error}")),
    };
    let how = method::detect(&binary, method::cargo_home().as_deref());
    if let Some(instead) = how.instead(&request.version.to_string()) {
        return fail(format!(
            "this Layover is not Layover's to replace ({how}); {instead}"
        ));
    }
    let source = match source_of(context.settings, state, request) {
        Ok(source) => source,
        Err(why) => return fail(why),
    };

    let lock = match install_lock(context.settings) {
        Ok(lock) => lock,
        Err(why) => return fail(why),
    };
    let swapped = match swap(&binary, &source, &running) {
        Ok(swapped) => swapped,
        Err(why) => return fail(why),
    };
    drop(lock);
    method::update_receipt(&binary, &request.version.to_string());
    say(
        state,
        &format!(
            "Installed {}; {running} is kept at {}.",
            request.version,
            swapped.previous.display()
        ),
        None,
    );

    let held = context
        .control
        .queue_held_ticks(&|line| println!("  {line}"));
    let started = Timestamp::now();
    let record = Handover {
        from: running.clone(),
        to: request.version.clone(),
        binary: swapped.binary.clone(),
        previous: swapped.previous.clone(),
        started,
        healthy_by: started
            .checked_add(SignedDuration::try_from(HEALTHY_WITHIN).unwrap_or(SignedDuration::MAX))
            .unwrap_or(started),
        boots: 0,
        healthy_at: None,
        rolled_back_at: None,
        token: context.token.map(str::to_owned),
        held: held.iter().map(ToString::to_string).collect(),
    };
    if let Err(error) = handover::put_handover(state, &record) {
        let _ = restore(&swapped);
        return fail(format!("could not write the handover: {error}"));
    }

    if let Some(supervisor) = supervised() {
        say(
            state,
            &format!("Exiting 75 for {supervisor} to start {}.", request.version),
            None,
        );
        return HandOver::Exit(75);
    }
    in_place(context, request, &swapped, &running)
}

/// The binary a request installs: the staged release, or for a rollback the kept previous one.
fn source_of(settings: &Settings, state: &Path, request: &Request) -> Result<PathBuf, String> {
    if request.rollback {
        let record = handover::handover(state)
            .filter(|record| record.from == request.version && record.previous.exists())
            .ok_or_else(|| format!("{} is no longer kept to go back to", request.version))?;
        return Ok(record.previous);
    }
    let staged = layover_update::staged(settings, &request.version)
        .filter(layover_update::Staged::is_ready)
        .ok_or_else(|| format!("{} is not staged and verified", request.version))?;
    staged
        .binary
        .ok_or_else(|| format!("{} was staged without its binary", request.version))
}

/// Starts the new binary beside this one and waits for it to answer, or undoes the install.
fn in_place(
    context: &Context,
    request: &Request,
    swapped: &Swapped,
    running: &Version,
) -> HandOver {
    let state = context.state;
    // Given up so the new Tower can take it; taken back if it does not.
    drop(
        context
            .serving
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take(),
    );
    let mut child = match successor(&swapped.binary, context.address, state) {
        Ok(child) => child,
        Err(error) => return undo(context, request, swapped, &error.to_string()),
    };
    say(
        state,
        &format!(
            "Started {} (pid {}); waiting for it to answer on {}.",
            request.version,
            child.id(),
            context.address
        ),
        None,
    );
    if let Err(why) = answers(&mut child, context, &request.version) {
        let _ = child.kill();
        let _ = child.wait();
        return undo(context, request, swapped, &why);
    }
    let what = if request.rollback {
        "rolled_back"
    } else {
        "applied"
    };
    let line = format!(
        "{} {running} → {}. The new Tower writes to {}.",
        if request.rollback {
            "Rolled back"
        } else {
            "Updated"
        },
        request.version,
        state.join("serve.log").display()
    );
    say(state, &line, Some(what));
    log::note(
        state,
        what,
        &running.to_string(),
        &request.version.to_string(),
        "",
    );
    HandOver::Exit(0)
}

/// Waits for the new Tower to answer as `version`, giving up early if it exits.
fn answers(child: &mut Child, context: &Context, version: &Version) -> Result<(), String> {
    let began = Instant::now();
    while began.elapsed() < ANSWER_WITHIN {
        if let Ok(Some(status)) = child.try_wait() {
            return Err(format!("{version} exited ({status}) before it answered"));
        }
        if handover::answers_as(
            &context.address.to_string(),
            context.token,
            version,
            Duration::from_secs(1),
        ) {
            return Ok(());
        }
    }
    Err(format!("{version} did not answer within two minutes"))
}

/// Puts this Tower's binary back, skips the version that failed, and carries on.
fn undo(context: &Context, request: &Request, swapped: &Swapped, why: &str) -> HandOver {
    let state = context.state;
    let running = Version::running();
    let restored = restore(swapped);
    if !request.rollback {
        let _ = state::skip(&context.settings.dir, &request.version);
    }
    if let Some(mut record) = handover::handover(state) {
        record.rolled_back_at = Some(Timestamp::now());
        record.token = None;
        let _ = handover::put_handover(state, &record);
    }
    let line = match restored {
        Ok(()) => format!(
            "Rolled back: {why}. {running} is back in place, {} will not be offered again here, \
             and work starts again.",
            request.version
        ),
        Err(error) => format!(
            "{why}, and {running} could not be put back ({error}): the binary at {} is {}. Fix \
             it by hand before this Tower restarts.",
            swapped.binary.display(),
            request.version
        ),
    };
    say(state, &line, Some("failed"));
    log::note(
        state,
        "rolled_back",
        &request.version.to_string(),
        &running.to_string(),
        why,
    );
    resume(context);
    HandOver::Resume
}

/// Takes the factory back after an install that did not happen.
fn resume(context: &Context) {
    let mut serving = context
        .serving
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if serving.is_none() {
        *serving = handover::serve(context.state).ok();
    }
    drop(serving);
    for settled in context.control.settle_cut_off() {
        println!("  {settled}");
    }
    context.control.stop_draining();
}

/// Starts the new Tower with this one's arguments, at this one's address, writing to `serve.log`.
fn successor(binary: &Path, address: SocketAddr, state: &Path) -> std::io::Result<Child> {
    let args = successor_args(std::env::args_os().skip(1).collect(), address);
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(state.join("serve.log"))?;
    let mut command = Command::new(binary);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        // DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP: it outlives this process and the console
        // or task that started it. Never CREATE_BREAKAWAY_FROM_JOB, which a scheduled task refuses.
        command.creation_flags(0x0000_0008 | 0x0000_0200);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        command.process_group(0);
    }
    layover_update::probe::spawn_retrying(&mut command)
}

/// This process's arguments, with `--addr` set to where it was actually bound — so a Tower
/// started on port 0 hands over to one on the same port.
fn successor_args(args: Vec<OsString>, address: SocketAddr) -> Vec<OsString> {
    let mut kept = Vec::new();
    let mut skip_next = false;
    for arg in args {
        if skip_next {
            skip_next = false;
            continue;
        }
        let text = arg.to_string_lossy();
        if text == "--addr" {
            skip_next = true;
            continue;
        }
        if text.starts_with("--addr=") {
            continue;
        }
        kept.push(arg);
    }
    kept.push("--addr".into());
    kept.push(address.to_string().into());
    kept
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_successor_is_told_the_address_this_tower_was_bound_to() {
        let address: SocketAddr = "127.0.0.1:51234".parse().expect("valid");
        let args = successor_args(
            [
                "serve",
                "--config",
                "f.toml",
                "--addr",
                "127.0.0.1:0",
                "--no-auth",
            ]
            .into_iter()
            .map(OsString::from)
            .collect(),
            address,
        );
        assert_eq!(
            args,
            [
                "serve",
                "--config",
                "f.toml",
                "--no-auth",
                "--addr",
                "127.0.0.1:51234"
            ]
            .map(OsString::from)
        );
        let args = successor_args(vec!["serve".into(), "--addr=1.2.3.4:5".into()], address);
        assert_eq!(
            args,
            ["serve", "--addr", "127.0.0.1:51234"].map(OsString::from)
        );
    }
}
