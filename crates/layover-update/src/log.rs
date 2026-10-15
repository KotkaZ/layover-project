//! What happened to a factory's Layover: checked, staged, refused, applied, rolled back — kept in
//! `.layover/updates.jsonl` beside its runs, for `layover update log` and for whoever asks later.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

/// One thing that happened.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    /// When.
    pub at: Timestamp,
    /// What: `applied`, `rolled_back`, `postponed`, `refused`, `failed`, `cancelled`.
    pub what: String,
    /// The version running before.
    pub from: String,
    /// The version it concerns.
    pub to: String,
    /// What else there is to say.
    #[serde(default)]
    pub detail: String,
}

fn path(state: &Path) -> PathBuf {
    state.join("updates.jsonl")
}

/// Appends `event`. Best effort: a log that cannot be written never stops an update.
pub fn append(state: &Path, event: &Event) {
    let Ok(line) = serde_json::to_string(event) else {
        return;
    };
    let _ = std::fs::create_dir_all(state);
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path(state))
    {
        let _ = writeln!(file, "{line}");
    }
}

/// Every event recorded, oldest first. Lines that do not read are skipped.
#[must_use]
pub fn read(state: &Path) -> Vec<Event> {
    std::fs::read_to_string(path(state))
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

/// Records `what` happened between `from` and `to`, now.
pub fn note(state: &Path, what: &str, from: &str, to: &str, detail: &str) {
    append(
        state,
        &Event {
            at: Timestamp::now(),
            what: what.to_owned(),
            from: from.to_owned(),
            to: to.to_owned(),
            detail: detail.to_owned(),
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_are_kept_in_order_and_a_bad_line_is_skipped() {
        let state = std::env::temp_dir().join(format!("layover-update-log-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&state);
        note(&state, "applied", "1.14.0", "1.15.0", "");
        std::fs::OpenOptions::new()
            .append(true)
            .open(path(&state))
            .and_then(|mut file| writeln!(file, "not json"))
            .expect("appends");
        note(&state, "rolled_back", "1.15.0", "1.14.0", "never healthy");

        let events = read(&state);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].what, "applied");
        assert_eq!(events[1].detail, "never healthy");
        let _ = std::fs::remove_dir_all(&state);
    }
}
