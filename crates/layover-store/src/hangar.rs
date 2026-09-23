//! Retention for Hangars: the per-run directories, not the agent state beside them.
//!
//! # Why this is not `prune_segments`
//!
//! History and the help journal are day-segmented files, and retention there deletes whole
//! segments. A Hangar is a *directory tree* — one per run, holding the composed prompt and the
//! transcript — and it is laid out so that an agent's own durable state lives in the same place:
//!
//! ```text
//! hangars/
//!   analyst/
//!     memory.md              <- the agent's notes to itself. Survives everything.
//!     run_01M31.../          <- one run. Prompt, transcript, MCP config.
//!     run_01M32.../
//! ```
//!
//! So this cannot simply empty a directory. `memory.md` is the thing an agent writes for its own
//! future runs, it is deliberately not segmented by time, and deleting it would quietly reset an
//! agent's accumulated knowledge — a far worse outcome than the disk it saves.
//!
//! Only entries that are **directories** named like a run are considered. Anything else in a
//! Hangar is left alone, on the same principle that a stray file among the history segments is
//! ignored rather than reported: the directory is a person's to look in.
//!
//! # Why it is needed
//!
//! Found by a 48-hour soak. Run records are pruned at the ninety-day horizon and Hangars were
//! not, so two things went wrong slowly: the tree grew without bound, and after ninety days a
//! factory held evidence for runs it could no longer look up — transcripts attached to nothing.

use std::fs;
use std::path::Path;

use jiff::Timestamp;
use layover_core::RunId;

use crate::history::StoreError;

/// Deletes Hangars for runs that began before `horizon`, returning how many went.
///
/// `root` is the `hangars` directory itself. A missing directory is not an error: a factory that
/// has never run has nothing to prune, and treating that as a failure would stop it starting.
///
/// # Errors
///
/// Returns [`StoreError::Io`] if a directory cannot be listed or removed.
pub fn prune(root: &Path, horizon: Timestamp) -> Result<usize, StoreError> {
    if !root.exists() {
        return Ok(0);
    }

    let mut removed = 0;

    for agent in directories(root)? {
        for hangar in directories(&agent)? {
            let Some(name) = hangar.file_name().and_then(|name| name.to_str()) else {
                continue;
            };

            // A name that does not decode is left alone rather than swept up. It was not minted
            // here, so its age is unknown, and deleting on a guess is how a person's own notes
            // disappear.
            let Some(minted) = RunId::from(name).minted_at() else {
                continue;
            };

            if minted < horizon {
                fs::remove_dir_all(&hangar).map_err(StoreError::at(&hangar))?;
                removed += 1;
            }
        }
    }

    Ok(removed)
}

/// The subdirectories of `path`, ignoring files.
fn directories(path: &Path) -> Result<Vec<std::path::PathBuf>, StoreError> {
    let mut found = Vec::new();

    for entry in fs::read_dir(path).map_err(StoreError::at(path))? {
        let entry = entry.map_err(StoreError::at(path))?;
        if entry.file_type().map_err(StoreError::at(path))?.is_dir() {
            found.push(entry.path());
        }
    }

    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempDir(std::path::PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!("layover-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).expect("temp dir");
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// A Hangar whose run id encodes `at`.
    fn hangar(root: &Path, agent: &str, at: Timestamp) -> std::path::PathBuf {
        let ms = u64::try_from(at.as_millisecond()).expect("positive");
        let ulid = ulid::Ulid::from_parts(ms, 0);
        let path = root.join(agent).join(format!("run_{ulid}"));

        fs::create_dir_all(&path).expect("hangar");
        fs::write(path.join("transcript.log"), "output").expect("transcript");
        path
    }

    fn at(text: &str) -> Timestamp {
        text.parse().expect("timestamp")
    }

    #[test]
    fn a_hangar_older_than_the_horizon_is_removed() {
        let temp = TempDir::new("hangar-old");
        let old = hangar(&temp.0, "analyst", at("2026-01-01T00:00:00Z"));

        let removed = prune(&temp.0, at("2026-06-01T00:00:00Z")).expect("prunes");

        assert_eq!(removed, 1);
        assert!(!old.exists());
    }

    #[test]
    fn a_hangar_inside_the_horizon_is_kept() {
        let temp = TempDir::new("hangar-new");
        let recent = hangar(&temp.0, "analyst", at("2026-06-15T00:00:00Z"));

        let removed = prune(&temp.0, at("2026-06-01T00:00:00Z")).expect("prunes");

        assert_eq!(removed, 0);
        assert!(recent.exists());
    }

    #[test]
    fn an_agents_memory_survives_however_old_the_factory_is() {
        // The reason this module exists rather than a `remove_dir_all` on the tree. `memory.md`
        // sits beside the run directories and is what an agent wrote for its own future runs; it
        // is deliberately not segmented by time. Sweeping it up would silently reset everything
        // an agent had worked out, which costs far more than the disk it frees.
        let temp = TempDir::new("hangar-memory");
        hangar(&temp.0, "analyst", at("2026-01-01T00:00:00Z"));

        let memory = temp.0.join("analyst").join("memory.md");
        fs::write(&memory, "what I learned").expect("memory");

        let removed = prune(&temp.0, at("2026-06-01T00:00:00Z")).expect("prunes");

        assert_eq!(removed, 1, "the run directory still goes");
        assert!(memory.exists(), "the agent's own notes do not");
        assert_eq!(
            fs::read_to_string(&memory).expect("reads"),
            "what I learned"
        );
    }

    #[test]
    fn a_directory_that_was_not_minted_here_is_left_alone() {
        // Its age is unknown, so deleting it would be a guess. A Hangar is a directory people
        // look in, and something they put there is not rubbish.
        let temp = TempDir::new("hangar-foreign");
        let theirs = temp.0.join("analyst").join("my-notes");
        fs::create_dir_all(&theirs).expect("dirs");

        let removed = prune(&temp.0, at("2026-06-01T00:00:00Z")).expect("prunes");

        assert_eq!(removed, 0);
        assert!(theirs.exists());
    }

    #[test]
    fn a_factory_that_has_never_run_prunes_nothing_rather_than_failing() {
        let temp = TempDir::new("hangar-absent");

        let removed = prune(&temp.0.join("hangars"), at("2026-06-01T00:00:00Z")).expect("prunes");

        assert_eq!(removed, 0);
    }

    #[test]
    fn pruning_spans_every_agent() {
        let temp = TempDir::new("hangar-agents");
        hangar(&temp.0, "analyst", at("2026-01-01T00:00:00Z"));
        hangar(&temp.0, "reporter", at("2026-01-02T00:00:00Z"));
        let kept = hangar(&temp.0, "reporter", at("2026-06-15T00:00:00Z"));

        let removed = prune(&temp.0, at("2026-06-01T00:00:00Z")).expect("prunes");

        assert_eq!(removed, 2);
        assert!(kept.exists());
    }
}
