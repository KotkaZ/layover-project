//! `layover update apply`, `rollback`, `status` and `log`: installing a release by hand, going back,
//! and seeing where things stand.
//!
//! With a Tower serving the factory, `apply` asks it — through `.layover/update/request.json`, as
//! `layover stop` asks it to stop — and watches it drain, swap and hand over. With none, it
//! replaces the binary itself under the install lock: nothing is running for it to wait for.
//!
//! Exit codes: 0 installed (or already current), 2 nothing to install, 3 refused, 4 postponed, 5
//! rolled back.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use jiff::Timestamp;
use layover_core::Config;
use layover_update::handover::{self, Handover, Request};
use layover_update::swap::{install_lock, swap};
use layover_update::{Settings, Version, log, method, state};

/// What `apply` and `rollback` print, and the exit code that goes with it.
pub type Said = (String, u8);

const INSTALLED: u8 = 0;
const NOTHING: u8 = 2;
const REFUSED: u8 = 3;
const POSTPONED: u8 = 4;
const ROLLED_BACK: u8 = 5;

fn factory(path: &Path) -> Result<(PathBuf, Config, PathBuf), Said> {
    let path = std::path::absolute(path).map_err(|error| (error.to_string(), REFUSED))?;
    let config = Config::load(&path).map_err(|error| (error.to_string(), REFUSED))?;
    let state = path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(".layover");
    Ok((path, config, state))
}

/// Who is asking, for the record.
fn who() -> String {
    std::env::var("USERNAME")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_else(|_| "layover update".to_owned())
}

/// `layover update apply`.
pub fn apply(path: &Path, now: bool, including_manual: bool, trust_checksum_only: bool) -> Said {
    let (path, config, state) = match factory(path) {
        Ok(found) => found,
        Err(said) => return said,
    };
    let settings = Settings::from_env();
    let running = Version::running();
    if let Err(error) = layover_update::check(&settings, config.update.channel, Timestamp::now()) {
        return (format!("Could not look for a release: {error}"), REFUSED);
    }
    let Some(available) = layover_update::status(&settings, &config.update, &running).available
    else {
        return (
            format!("Layover {running} is the newest release.\n"),
            NOTHING,
        );
    };
    let version = available.release.version.clone();

    let binary = match std::env::current_exe() {
        Ok(binary) => binary,
        Err(error) => {
            return (
                format!("Could not tell which binary is running: {error}"),
                REFUSED,
            );
        }
    };
    let how = method::detect(&binary, method::cargo_home().as_deref());
    if let Some(instead) = how.instead(&version.to_string()) {
        return (
            format!(
                "This Layover is installed by {how}, which Layover does not replace. Run: {instead}\n"
            ),
            REFUSED,
        );
    }

    let (staged, gate) = match crate::staging::ready(&settings, &available.release, &path) {
        Some(pair) => pair,
        None => match (
            layover_update::staged(&settings, &version),
            layover_update::gate::recorded(&path),
        ) {
            (Some(staged), Some(gate)) => (staged, gate),
            _ => crate::staging::stage_and_gate(&settings, &available.release, &path, false),
        },
    };
    let (staged, gate) = if !staged.is_ready() && trust_checksum_only {
        crate::staging::stage_and_gate(&settings, &available.release, &path, true)
    } else {
        (staged, gate)
    };
    let mut out = crate::staging::describe(&staged, Some(&gate));
    if !staged.is_ready() {
        out.push_str("Not installed: it is not staged and verified.\n");
        return (out, REFUSED);
    }
    if staged.crosses_layout {
        out.push_str(
            "Not installed: it reads a different state-directory layout, and installing such a \
             release is not something `apply` does yet. Install it by hand.\n",
        );
        return (out, REFUSED);
    }
    if !gate.passes() {
        out.push_str("Not installed: it refuses this factory.\n");
        return (out, REFUSED);
    }
    print!("{out}");

    let request = Request {
        version,
        rollback: false,
        now,
        including_manual,
        by: who(),
        at: Timestamp::now(),
        progress: Vec::new(),
        outcome: None,
    };
    let source = staged.binary.clone().unwrap_or_default();
    if handover::is_served(&state) {
        ask_the_tower(&state, &request)
    } else {
        replace(&settings, &state, &binary, &source, &request)
    }
}

/// `layover update rollback`.
pub fn rollback(path: &Path) -> Said {
    let (_, _, state) = match factory(path) {
        Ok(found) => found,
        Err(said) => return said,
    };
    let settings = Settings::from_env();
    let running = Version::running();
    let Some(record) = handover::handover(&state)
        .filter(|record| record.to == running && record.previous.exists())
    else {
        return (
            format!("There is no earlier version kept to go back to from {running}.\n"),
            NOTHING,
        );
    };
    let _ = state::skip(&settings.dir, &running);
    let request = Request {
        version: record.from.clone(),
        rollback: true,
        now: false,
        including_manual: false,
        by: who(),
        at: Timestamp::now(),
        progress: Vec::new(),
        outcome: None,
    };
    println!(
        "Going back to {}; {running} will not be offered again on this machine.",
        record.from
    );
    if handover::is_served(&state) {
        return ask_the_tower(&state, &request);
    }
    let binary = match std::env::current_exe() {
        Ok(binary) => binary,
        Err(error) => {
            return (
                format!("Could not tell which binary is running: {error}"),
                REFUSED,
            );
        }
    };
    replace(&settings, &state, &binary, &record.previous, &request)
}

/// Asks the serving Tower to install `request`, and follows it until it says how it went.
fn ask_the_tower(state: &Path, request: &Request) -> Said {
    if let Err(error) = handover::put_request(state, request) {
        return (format!("Could not ask the Tower: {error}"), REFUSED);
    }
    println!(
        "The Tower serving this factory will drain and install {}. Ctrl+C here stops watching, \
         not the install.",
        request.version
    );
    let mut shown = 0;
    loop {
        std::thread::sleep(Duration::from_millis(500));
        let Some(now) = handover::request(state) else {
            return ("The request was withdrawn.\n".to_owned(), REFUSED);
        };
        for line in now.progress.iter().skip(shown) {
            println!("  {line}");
        }
        shown = now.progress.len();
        let code = match now.outcome.as_deref() {
            None => continue,
            Some("applied") => INSTALLED,
            Some("rolled_back") => ROLLED_BACK,
            Some("postponed") => POSTPONED,
            Some(_) => REFUSED,
        };
        return (String::new(), code);
    }
}

/// Replaces the binary here and now: no Tower is running it.
fn replace(
    settings: &Settings,
    state: &Path,
    binary: &Path,
    source: &Path,
    request: &Request,
) -> Said {
    let running = Version::running();
    let lock = match install_lock(settings) {
        Ok(lock) => lock,
        Err(why) => return (format!("Not installed: {why}\n"), REFUSED),
    };
    let swapped = match swap(binary, source, &running) {
        Ok(swapped) => swapped,
        Err(why) => return (format!("Not installed: {why}\n"), REFUSED),
    };
    drop(lock);
    method::update_receipt(binary, &request.version.to_string());
    let now = Timestamp::now();
    let _ = handover::put_handover(
        state,
        &Handover {
            from: running.clone(),
            to: request.version.clone(),
            binary: swapped.binary.clone(),
            previous: swapped.previous.clone(),
            started: now,
            healthy_by: now,
            boots: 0,
            // Nothing is running it to prove; the next Tower to start is the proof.
            healthy_at: Some(now),
            rolled_back_at: None,
            token: None,
            held: Vec::new(),
        },
    );
    let what = if request.rollback {
        "rolled_back"
    } else {
        "applied"
    };
    log::note(
        state,
        what,
        &running.to_string(),
        &request.version.to_string(),
        "by hand",
    );
    (
        format!(
            "{} {running} → {}. {running} is kept at {}. No Tower was serving this factory; the \
             next one to start runs {}.\n",
            if request.rollback {
                "Rolled back"
            } else {
                "Updated"
            },
            request.version,
            swapped.previous.display(),
            request.version
        ),
        if request.rollback {
            ROLLED_BACK
        } else {
            INSTALLED
        },
    )
}

/// `layover update status`: where things stand, from what is on disk. Asks nothing.
///
/// # Errors
///
/// Returns why the factory could not be read.
pub fn status(path: &Path) -> Result<String, String> {
    let (path, config, state) = factory(path).map_err(|(said, _)| said)?;
    let settings = Settings::from_env();
    let running = Version::running();
    let mut out = String::new();
    let binary = std::env::current_exe().unwrap_or_default();
    let how = method::detect(&binary, method::cargo_home().as_deref());
    let _ = writeln!(out, "Layover {running} at {} ({how}).", binary.display());
    let _ = writeln!(
        out,
        "[update] mode = \"{}\", channel = \"{}\"{}.",
        config.update.mode,
        config.update.channel,
        if settings.switched_off {
            "; LAYOVER_UPDATE=off on this machine"
        } else {
            ""
        }
    );
    let _ = writeln!(
        out,
        "{}",
        crate::update::doctor_line(&config.update, &state, Timestamp::now())
    );
    let found = layover_update::status(&settings, &config.update, &running);
    if let Some(available) = &found.available
        && let Some(staged) = layover_update::staged(&settings, &available.release.version)
    {
        let gate =
            layover_update::gate::recorded(&path).filter(|gate| gate.version == staged.version);
        out.push_str(&crate::staging::describe(&staged, gate.as_ref()));
    }
    if let Some(request) = handover::request(&state) {
        let _ = writeln!(
            out,
            "{} {} asked by {} at {}: {}.",
            if request.rollback {
                "Rollback to"
            } else {
                "Install of"
            },
            request.version,
            request.by,
            request.at.strftime("%Y-%m-%d %H:%M"),
            request.outcome.as_deref().unwrap_or("under way")
        );
        for line in &request.progress {
            let _ = writeln!(out, "  {line}");
        }
    }
    if let Some(record) = handover::handover(&state) {
        let _ = writeln!(out, "{}", describe_handover(&record));
    }
    Ok(out)
}

fn describe_handover(record: &Handover) -> String {
    let when = record.started.strftime("%Y-%m-%d %H:%M");
    let how = match (record.healthy_at, record.rolled_back_at) {
        (_, Some(at)) => format!("rolled back {}", at.strftime("%Y-%m-%d %H:%M")),
        (Some(_), None) => "healthy".to_owned(),
        (None, None) => format!("not yet proved healthy ({} start(s))", record.boots),
    };
    let kept = if record.previous.exists() {
        format!("; {} is kept at {}", record.from, record.previous.display())
    } else {
        String::new()
    };
    format!(
        "Last update: {} → {} on {when}, {how}{kept}.",
        record.from, record.to
    )
}

/// `layover update log`: what happened to this factory's Layover.
///
/// # Errors
///
/// Returns why the factory could not be read.
pub fn history(path: &Path) -> Result<String, String> {
    let (_, _, state) = factory(path).map_err(|(said, _)| said)?;
    let events = log::read(&state);
    if events.is_empty() {
        return Ok("Nothing has been installed or rolled back here.\n".to_owned());
    }
    let mut out = String::new();
    for event in events {
        let _ = writeln!(
            out,
            "{}  {:<11} {} → {}{}",
            event.at.strftime("%Y-%m-%d %H:%M"),
            event.what,
            event.from,
            event.to,
            if event.detail.is_empty() {
                String::new()
            } else {
                format!("  ({})", event.detail)
            }
        );
    }
    Ok(out)
}
