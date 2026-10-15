//! Asking a running Tower to stop, from outside it.
//!
//! A Tower stopped by ending its process cuts every live run off. One asked to stop drains: nothing
//! new starts, what is running finishes, and only then does it exit. The request is a file in the
//! state directory, which the Tower looks for on every pass the way it looks for its Ground Stop,
//! so asking needs neither the dashboard's token nor a Tower that is answering HTTP — a command in
//! another terminal, a script, or the Tower's own Ctrl+C handler all write the same file.
//!
//! The file also tells a dashboard watching the factory that the Tower is stopping, and it is
//! removed by the Tower that honoured it. One left behind by a Tower that went away anyway is
//! older than the next Tower's start, which ignores and removes it.

use std::path::{Path, PathBuf};

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

/// The request's file name, in the state directory beside the Ground Stop.
pub const FILE: &str = "stop-request.json";

/// A request for the Tower to stop.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct StopRequest {
    /// When it was asked for.
    pub at: Timestamp,
    /// Who asked: a person's name, `Ctrl+C`, or the command that wrote it.
    pub by: String,
    /// Cut runs off rather than waiting for them to finish. The next Tower settles them: stopped,
    /// recorded as interrupted, and restarted where their agents allow.
    #[serde(default)]
    pub now: bool,
}

impl StopRequest {
    /// A request to drain and stop, made now.
    #[must_use]
    pub fn drain(by: impl Into<String>) -> Self {
        Self {
            at: Timestamp::now(),
            by: by.into(),
            now: false,
        }
    }

    /// A request to stop now, cutting runs off.
    #[must_use]
    pub fn now(by: impl Into<String>) -> Self {
        Self {
            now: true,
            ..Self::drain(by)
        }
    }
}

/// Where the request lives for the state directory `state`.
#[must_use]
pub fn path(state: &Path) -> PathBuf {
    state.join(FILE)
}

/// Writes a request, replacing any already there. A request to stop now is never weakened to a
/// drain by a later one: once somebody asked for runs to be cut off, a second, gentler request
/// does not undo it.
///
/// # Errors
///
/// Returns an error when the file cannot be written.
pub fn request(state: &Path, request: &StopRequest) -> std::io::Result<()> {
    let mut request = request.clone();
    if pending(state).is_some_and(|earlier| earlier.now) {
        request.now = true;
    }
    std::fs::create_dir_all(state)?;
    let text = serde_json::to_string_pretty(&request).map_err(std::io::Error::other)?;
    let staging = path(state).with_extension("json.writing");
    std::fs::write(&staging, text)?;
    std::fs::rename(staging, path(state))
}

/// The request waiting to be honoured, if any. One that cannot be read is treated as a request to
/// drain: whoever wrote it wanted the Tower to stop.
#[must_use]
pub fn pending(state: &Path) -> Option<StopRequest> {
    let text = std::fs::read_to_string(path(state)).ok()?;
    Some(
        serde_json::from_str(&text).unwrap_or_else(|_| StopRequest::drain("an unreadable request")),
    )
}

/// Removes the request, once it has been honoured or found stale.
pub fn clear(state: &Path) {
    let _ = std::fs::remove_file(path(state));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("layover-stop-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        path
    }

    #[test]
    fn a_request_is_read_back_and_cleared() {
        let state = temp("roundtrip");
        assert_eq!(pending(&state), None);

        request(&state, &StopRequest::drain("karl")).expect("writes");
        let read = pending(&state).expect("pending");
        assert_eq!(read.by, "karl");
        assert!(!read.now);

        clear(&state);
        assert_eq!(pending(&state), None);
        let _ = std::fs::remove_dir_all(&state);
    }

    #[test]
    fn asking_to_stop_now_is_not_undone_by_a_gentler_request() {
        let state = temp("now");
        request(&state, &StopRequest::now("karl")).expect("writes");
        request(&state, &StopRequest::drain("Ctrl+C")).expect("writes");

        assert!(pending(&state).expect("pending").now);
        let _ = std::fs::remove_dir_all(&state);
    }

    #[test]
    fn an_unreadable_request_still_asks_for_a_stop() {
        let state = temp("garbage");
        std::fs::create_dir_all(&state).expect("dir");
        std::fs::write(path(&state), "{not json").expect("writes");

        let read = pending(&state).expect("still a request");
        assert!(!read.now, "but never a request to cut runs off");
        let _ = std::fs::remove_dir_all(&state);
    }
}
