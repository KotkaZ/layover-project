//! Knowing a run existed, even if nothing was watching when it ended.
//!
//! # Why this is written before the process starts
//!
//! A run that was alive when the supervisor died leaves no exit code, no final record, and no
//! trace in anything the supervisor holds in memory. The only way to know it existed is to have
//! written that down first — and the only way to know whether it is *still* alive is to have
//! recorded enough to check.
//!
//! Writing the record after spawning would leave a window in which a real process is running, may
//! be spending money, and nothing knows about it. That window is precisely where a crash falls.
//!
//! # Why the start time is recorded alongside the process identifier
//!
//! Operating systems reuse process identifiers. A supervisor that restarts an hour later, finds a
//! recorded identifier, asks the system whether it is alive and is told yes, may be looking at an
//! entirely unrelated program that happened to inherit the number.
//!
//! Recovering into that mistake is worse than not recovering: the supervisor would decide a run is
//! still going, wait for it, and wait forever. So the record carries when the process began, and a
//! match requires both.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use jiff::Timestamp;
use layover_core::agent::AgentName;
use layover_core::flight::{ItineraryId, RunId};
use serde::{Deserialize, Serialize};

/// A run the supervisor started and has not yet seen finish.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Live {
    /// Which run this is.
    pub run: RunId,
    /// The chain it belongs to.
    pub itinerary: ItineraryId,
    /// Which agent is running.
    pub agent: AgentName,
    /// The operating system's identifier for the child.
    pub pid: u32,
    /// When the supervisor started it.
    pub started_at: Timestamp,
    /// Where the run's files are.
    pub hangar: PathBuf,
}

/// Where live-run records are kept.
///
/// One file per run rather than one file with many lines: a run ending is a deletion, and deleting
/// a file is atomic in a way that rewriting a shared file after a crash is not.
#[derive(Debug, Clone)]
pub struct Ledger {
    root: PathBuf,
}

impl Ledger {
    /// Opens — and creates — the directory live records are kept in.
    ///
    /// # Errors
    ///
    /// Returns an error when the directory cannot be created.
    pub fn open(root: impl Into<PathBuf>) -> io::Result<Self> {
        let root = root.into();
        fs::create_dir_all(&root)?;
        Ok(Self { root })
    }

    /// Records that a run is about to start.
    ///
    /// Call this **before** spawning. The record is the only evidence the run existed if the
    /// supervisor does not survive to write another.
    ///
    /// # Errors
    ///
    /// Returns an error when the record cannot be written.
    pub fn starting(&self, live: &Live) -> io::Result<()> {
        let text = serde_json::to_string(live).map_err(io::Error::other)?;
        fs::write(self.path_for(&live.run), text)
    }

    /// Forgets a run that has been seen to finish.
    ///
    /// # Errors
    ///
    /// Returns an error when the record exists and cannot be removed.
    pub fn finished(&self, run: &RunId) -> io::Result<()> {
        match fs::remove_file(self.path_for(run)) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }

    /// Every run recorded as started and not recorded as finished.
    ///
    /// # Errors
    ///
    /// Returns an error when the directory cannot be read.
    pub fn live(&self) -> io::Result<Vec<Live>> {
        let mut found = Vec::new();

        for entry in fs::read_dir(&self.root)? {
            let path = entry?.path();
            if path.extension().is_none_or(|ext| ext != "json") {
                continue;
            }

            // A record that cannot be parsed is skipped rather than fatal. It means a crash
            // mid-write, and refusing to start because of one unreadable file would turn a lost
            // run into a supervisor that will not run at all.
            if let Ok(text) = fs::read_to_string(&path)
                && let Ok(live) = serde_json::from_str::<Live>(&text)
            {
                found.push(live);
            }
        }

        found.sort_by_key(|live| live.started_at);
        Ok(found)
    }

    fn path_for(&self, run: &RunId) -> PathBuf {
        self.root.join(format!("{run}.json"))
    }

    /// The directory records are kept in.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }
}

/// What reconciliation concluded about one recorded run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// The process is still running: same identifier, same start time.
    StillRunning,
    /// Nothing is running under that identifier. The run was interrupted and is recoverable.
    ConfirmedGone,
    /// Something is running under that identifier, but it did not start when this run did.
    ///
    /// The identifier was reused. The original run is gone, and the supervisor must not wait for
    /// whatever inherited its number.
    Reused,
}

impl Verdict {
    /// Whether recovery may start a replacement for this run.
    ///
    /// Recovery requires the previous process to be *confirmed* gone. A reused identifier is also
    /// gone — that is why it is a separate verdict rather than an error: the conclusion is the
    /// same, the evidence is different, and saying so makes the log readable.
    #[must_use]
    pub const fn is_recoverable(self) -> bool {
        matches!(self, Self::ConfirmedGone | Self::Reused)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Temp(PathBuf);

    impl Temp {
        fn new(name: &str) -> Self {
            let path =
                std::env::temp_dir().join(format!("layover-state-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&path);
            Self(path)
        }
    }

    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn live() -> Live {
        Live {
            run: RunId::generate(),
            itinerary: ItineraryId::generate(),
            agent: AgentName::new("tester"),
            pid: 4242,
            started_at: Timestamp::now(),
            hangar: PathBuf::from("hangar"),
        }
    }

    #[test]
    fn a_started_run_is_readable_before_anything_else_happens() {
        // This is the whole point: if the supervisor dies here, the record is what survives.
        let temp = Temp::new("started");
        let ledger = Ledger::open(&temp.0).expect("opens");
        let record = live();

        ledger.starting(&record).expect("records");

        let found = ledger.live().expect("reads");
        assert_eq!(found, vec![record]);
    }

    #[test]
    fn a_finished_run_is_forgotten() {
        let temp = Temp::new("finished");
        let ledger = Ledger::open(&temp.0).expect("opens");
        let record = live();

        ledger.starting(&record).expect("records");
        ledger.finished(&record.run).expect("forgets");

        assert!(ledger.live().expect("reads").is_empty());
    }

    #[test]
    fn forgetting_a_run_twice_is_not_an_error() {
        // A supervisor that crashed between removing the record and writing the outcome will try
        // again on restart, and refusing would leave it unable to start.
        let temp = Temp::new("twice");
        let ledger = Ledger::open(&temp.0).expect("opens");
        let record = live();

        ledger.starting(&record).expect("records");
        ledger.finished(&record.run).expect("first");
        ledger.finished(&record.run).expect("second");
    }

    #[test]
    fn a_half_written_record_does_not_stop_the_supervisor_starting() {
        // A crash mid-write leaves a truncated file. Refusing to start because of it would turn
        // one lost run into a factory that will not run at all.
        let temp = Temp::new("corrupt");
        let ledger = Ledger::open(&temp.0).expect("opens");
        ledger.starting(&live()).expect("records");
        fs::write(temp.0.join("run_bad.json"), "{\"run\":").expect("writes rubbish");

        let found = ledger.live().expect("reads");
        assert_eq!(found.len(), 1, "the readable record still arrives");
    }

    #[test]
    fn live_runs_come_back_oldest_first() {
        let temp = Temp::new("order");
        let ledger = Ledger::open(&temp.0).expect("opens");

        let mut first = live();
        first.started_at = Timestamp::now()
            .checked_sub(jiff::SignedDuration::from_hours(2))
            .expect("in range");
        let second = live();

        ledger.starting(&second).expect("records");
        ledger.starting(&first).expect("records");

        let found = ledger.live().expect("reads");
        assert_eq!(found[0].run, first.run, "the longest-running is first");
    }

    #[test]
    fn a_reused_identifier_is_still_recoverable_but_says_why() {
        assert!(Verdict::ConfirmedGone.is_recoverable());
        assert!(Verdict::Reused.is_recoverable());
        assert!(!Verdict::StillRunning.is_recoverable());
    }
}
