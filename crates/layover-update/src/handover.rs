//! What one Tower leaves the next across an update: the request that started it, the record of the
//! handover, and the lock that says a Tower is serving the factory at all.
//!
//! All three live in the factory's `.layover/` — the request and the handover in `update/` — because
//! the Tower that drains, swaps and restarts is the factory's, and it already watches that folder
//! for its Ground Stop.

use std::fs::{File, TryLockError};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use crate::version::Version;

/// A person's request to install a release into the factory's running Tower.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    /// What to install: a release version, or the version a rollback goes back to.
    pub version: Version,
    /// Whether this goes back to the kept previous binary rather than forward to a staged one.
    #[serde(default)]
    pub rollback: bool,
    /// Cut off what is running rather than waiting for it.
    #[serde(default)]
    pub now: bool,
    /// Cut off even an agent whose `recovery` is `manual`, whose work must not happen twice.
    #[serde(default)]
    pub including_manual: bool,
    /// Who asked.
    pub by: String,
    /// When.
    pub at: Timestamp,
    /// What has happened since, newest last, for whoever asked to watch.
    #[serde(default)]
    pub progress: Vec<String>,
    /// How it ended, once it has: `applied`, `postponed`, `failed`, `cancelled`.
    #[serde(default)]
    pub outcome: Option<String>,
    /// When an unattended install's window closes: still waiting for runs then, it is postponed.
    #[serde(default)]
    pub until: Option<Timestamp>,
}

/// The record a Tower leaves for the Tower that replaces it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Handover {
    /// The version handed over from.
    pub from: Version,
    /// The version handed over to.
    pub to: Version,
    /// The binary, now holding `to`.
    pub binary: PathBuf,
    /// Where `from` is kept, for a rollback.
    pub previous: PathBuf,
    /// When the handover began.
    pub started: Timestamp,
    /// The latest the new Tower may prove itself by.
    pub healthy_by: Timestamp,
    /// Starts of `to` so far.
    #[serde(default)]
    pub boots: u32,
    /// When `to` proved itself.
    #[serde(default)]
    pub healthy_at: Option<Timestamp>,
    /// When it was rolled back, if it was.
    #[serde(default)]
    pub rolled_back_at: Option<Timestamp>,
    /// The dashboard's token, so a page already open keeps working. Removed once read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    /// Workflows whose scheduled tick came due while the Tower drained, for the next to fire.
    #[serde(default)]
    pub held: Vec<String>,
}

/// How long a new Tower has to prove itself.
pub const HEALTHY_WITHIN: Duration = Duration::from_mins(10);

fn dir(state: &Path) -> PathBuf {
    state.join("update")
}

/// Where the request is kept.
#[must_use]
pub fn request_path(state: &Path) -> PathBuf {
    dir(state).join("request.json")
}

/// Where the handover is kept.
#[must_use]
pub fn handover_path(state: &Path) -> PathBuf {
    dir(state).join("handover.json")
}

fn read<T: for<'de> Deserialize<'de>>(path: &Path) -> Option<T> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

fn write<T: Serialize>(path: &Path, value: &T) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(value).map_err(std::io::Error::other)?;
    let partial = path.with_extension(format!("json.partial-{}", std::process::id()));
    std::fs::write(&partial, text)?;
    #[cfg(unix)]
    {
        // It may carry the dashboard's token.
        use std::os::unix::fs::PermissionsExt as _;
        let _ = std::fs::set_permissions(&partial, std::fs::Permissions::from_mode(0o600));
    }
    std::fs::rename(partial, path)
}

/// The request waiting or under way, if any.
#[must_use]
pub fn request(state: &Path) -> Option<Request> {
    read(&request_path(state))
}

/// Writes `request`.
///
/// # Errors
///
/// Returns the I/O error.
pub fn put_request(state: &Path, request: &Request) -> std::io::Result<()> {
    write(&request_path(state), request)
}

/// Adds a line of progress to the request under way, and ends it with `outcome` when given.
pub fn progress(state: &Path, line: &str, outcome: Option<&str>) {
    if let Some(mut request) = request(state) {
        request.progress.push(line.to_owned());
        if let Some(outcome) = outcome {
            request.outcome = Some(outcome.to_owned());
        }
        let _ = put_request(state, &request);
    }
}

/// Removes the request.
pub fn clear_request(state: &Path) {
    let _ = std::fs::remove_file(request_path(state));
}

/// The handover, if one was left.
#[must_use]
pub fn handover(state: &Path) -> Option<Handover> {
    read(&handover_path(state))
}

/// Writes `handover`.
///
/// # Errors
///
/// Returns the I/O error.
pub fn put_handover(state: &Path, handover: &Handover) -> std::io::Result<()> {
    write(&handover_path(state), handover)
}

/// A lock a serving Tower holds for as long as it lives, so another process can tell one is.
#[derive(Debug)]
pub struct Serving(File);

impl Drop for Serving {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

/// Claims the factory's serving lock.
///
/// # Errors
///
/// Says another Tower holds it, or that it could not be taken.
pub fn serve(state: &Path) -> Result<Serving, String> {
    std::fs::create_dir_all(state).map_err(|error| error.to_string())?;
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(state.join("serving.lock"))
        .map_err(|error| error.to_string())?;
    match file.try_lock() {
        Ok(()) => Ok(Serving(file)),
        Err(TryLockError::WouldBlock) => {
            Err("another Tower is already serving this factory".to_owned())
        }
        Err(TryLockError::Error(error)) => Err(error.to_string()),
    }
}

/// Whether a Tower is serving the factory whose state is in `state`.
#[must_use]
pub fn is_served(state: &Path) -> bool {
    let Ok(file) = File::open(state.join("serving.lock")) else {
        return false;
    };
    match file.try_lock() {
        Ok(()) => {
            let _ = file.unlock();
            false
        }
        Err(_) => true,
    }
}

/// Waits up to `limit` for the dashboard at `address` to say it runs `version`.
#[must_use]
pub fn answers_as(address: &str, token: Option<&str>, version: &Version, limit: Duration) -> bool {
    let url = format!("http://{address}/health");
    let began = Instant::now();
    while began.elapsed() < limit {
        let mut request = crate::source::local_agent(Duration::from_secs(5)).get(&url);
        if let Some(token) = token {
            request = request.header("Authorization", format!("Bearer {token}"));
        }
        if let Ok(mut response) = request.call()
            && response.status().as_u16() == 200
            && let Ok(text) = response.body_mut().read_to_string()
            && serde_json::from_str::<serde_json::Value>(&text)
                .ok()
                .and_then(|health| health["version"].as_str().map(str::to_owned))
                .is_some_and(|said| said == version.to_string())
        {
            return true;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("layover-handover-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn a_serving_tower_is_seen_by_everyone_else_until_it_lets_go() {
        let state = temp("serving");
        assert!(!is_served(&state));

        let held = serve(&state).expect("claims");
        assert!(is_served(&state));
        assert!(serve(&state).is_err(), "one Tower serves a factory");

        drop(held);
        assert!(!is_served(&state));
        let _ = std::fs::remove_dir_all(&state);
    }

    #[test]
    fn progress_is_added_to_the_request_under_way() {
        let state = temp("progress");
        put_request(
            &state,
            &Request {
                version: "1.15.0".parse().expect("valid"),
                rollback: false,
                now: false,
                including_manual: false,
                by: "karl".to_owned(),
                at: Timestamp::now(),
                progress: Vec::new(),
                outcome: None,
                until: None,
            },
        )
        .expect("writes");

        progress(&state, "draining", None);
        progress(&state, "applied", Some("applied"));

        let request = request(&state).expect("reads");
        assert_eq!(request.progress, ["draining", "applied"]);
        assert_eq!(request.outcome.as_deref(), Some("applied"));
        clear_request(&state);
        assert!(super::request(&state).is_none());
        let _ = std::fs::remove_dir_all(&state);
    }
}
