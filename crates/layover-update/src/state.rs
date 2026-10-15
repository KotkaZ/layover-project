//! What this machine knows about releases: the last check on each channel, and the versions a
//! person said to skip.
//!
//! Kept per machine rather than per factory, because the binary being updated is the machine's:
//! two factories served by one installation should not each ask GitHub, or disagree about what is
//! skipped. Windows keeps it in `%LOCALAPPDATA%\layover\updates`, macOS in
//! `~/Library/Application Support/layover/updates`, and everything else in
//! `$XDG_STATE_HOME/layover/updates`, by default `~/.local/state/layover/updates`.

use std::path::{Path, PathBuf};

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use crate::Channel;
use crate::source::Etag;
use crate::version::Version;

/// A release, as discovery found it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Release {
    /// Its version.
    pub version: Version,
    /// The tag it was cut from.
    pub tag: String,
    /// Whether it is a pre-release.
    pub prerelease: bool,
    /// Its title: `1.15.0 — 2026-10-17`.
    #[serde(default)]
    pub title: Option<String>,
    /// Its notes: its CHANGELOG section, Compatibility first.
    #[serde(default)]
    pub notes: String,
    /// When it was published, where that is known.
    #[serde(default)]
    pub published_at: Option<Timestamp>,
    /// Where a person reads about it.
    pub page: String,
}

/// The last time this machine looked for a release on one channel, and what it found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LastCheck {
    /// The channel looked on.
    pub channel: Channel,
    /// When it last looked.
    pub attempted_at: Timestamp,
    /// When looking last worked.
    #[serde(default)]
    pub succeeded_at: Option<Timestamp>,
    /// Why the last look failed, if it did.
    #[serde(default)]
    pub error: Option<String>,
    /// How many looks in a row have failed, for backing off.
    #[serde(default)]
    pub failures: u32,
    /// The newest release the last look that worked found.
    #[serde(default)]
    pub newest: Option<Release>,
    /// What to send next time to be told nothing changed.
    #[serde(default)]
    pub etag: Option<Etag>,
}

/// Where this machine keeps what it knows about releases, when the platform says.
#[must_use]
pub fn machine_dir() -> Option<PathBuf> {
    let var = |name: &str| std::env::var_os(name).filter(|value| !value.is_empty());
    let base = if cfg!(windows) {
        PathBuf::from(var("LOCALAPPDATA")?)
    } else if cfg!(target_os = "macos") {
        PathBuf::from(var("HOME")?)
            .join("Library")
            .join("Application Support")
    } else {
        var("XDG_STATE_HOME").map_or_else(
            || var("HOME").map(|home| PathBuf::from(home).join(".local").join("state")),
            |state| Some(PathBuf::from(state)),
        )?
    };
    Some(base.join("layover").join("updates"))
}

fn last_check_path(dir: &Path, channel: Channel) -> PathBuf {
    dir.join(format!("last-check-{channel}.json"))
}

fn skipped_path(dir: &Path) -> PathBuf {
    dir.join("skipped.json")
}

/// The last check on `channel`, if there was one this machine can read.
#[must_use]
pub fn last_check(dir: &Path, channel: Channel) -> Option<LastCheck> {
    let text = std::fs::read_to_string(last_check_path(dir, channel)).ok()?;
    serde_json::from_str(&text).ok()
}

/// Records a check.
///
/// # Errors
///
/// Returns the I/O error when the directory cannot be written.
pub fn record(dir: &Path, check: &LastCheck) -> std::io::Result<()> {
    write(&last_check_path(dir, check.channel), check)
}

/// The versions a person said to skip.
#[must_use]
pub fn skipped(dir: &Path) -> Vec<Version> {
    std::fs::read_to_string(skipped_path(dir))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// Adds `version` to the versions to skip.
///
/// # Errors
///
/// Returns the I/O error when the directory cannot be written.
pub fn skip(dir: &Path, version: &Version) -> std::io::Result<()> {
    let mut versions = skipped(dir);
    if !versions.contains(version) {
        versions.push(version.clone());
        versions.sort();
    }
    write(&skipped_path(dir), &versions)
}

/// Forgets every skipped version.
///
/// # Errors
///
/// Returns the I/O error when the file exists and cannot be removed.
pub fn clear_skipped(dir: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(skipped_path(dir)) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

/// Writes `value` whole or not at all: to a file beside `path`, then renamed over it. Two Towers
/// on one machine may write the same file; each sees one or the other, never half of both.
fn write(path: &Path, value: &impl Serialize) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(value).map_err(std::io::Error::other)?;
    let partial = path.with_extension(format!("{}.partial", std::process::id()));
    std::fs::write(&partial, text)?;
    std::fs::rename(&partial, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&partial);
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("layover-update-state-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn skipped_versions_are_kept_once_and_in_order_until_cleared() {
        let dir = temp("skip");
        assert!(skipped(&dir).is_empty());

        for version in ["1.16.0", "1.15.0", "1.16.0"] {
            skip(&dir, &version.parse().expect("valid")).expect("writes");
        }
        let versions: Vec<String> = skipped(&dir).iter().map(ToString::to_string).collect();
        assert_eq!(versions, ["1.15.0", "1.16.0"]);

        clear_skipped(&dir).expect("clears");
        clear_skipped(&dir).expect("clearing nothing is fine");
        assert!(skipped(&dir).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_check_is_kept_per_channel() {
        let dir = temp("checks");
        let check = LastCheck {
            channel: Channel::Prerelease,
            attempted_at: "2026-10-15T09:00:00Z".parse().expect("valid"),
            succeeded_at: None,
            error: Some("offline".to_owned()),
            failures: 1,
            newest: None,
            etag: None,
        };
        record(&dir, &check).expect("writes");

        assert_eq!(last_check(&dir, Channel::Prerelease), Some(check));
        assert_eq!(last_check(&dir, Channel::Stable), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
