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
use layover_core::queue::Queued;
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
    /// The work the run was given, as it was queued — kept so that a Tower restarting after this
    /// run was interrupted can start it again rather than only noting that it existed.
    ///
    /// Absent in a record written by a release that did not keep it; such a run can be recorded
    /// as interrupted, and not restarted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queued: Option<Queued>,
    /// The Tower watching the run, so that another Tower — or `layover run` beside it — leaves a
    /// run alone while the one that started it is still alive. The Tower's `recovery` module settles a run whose owner has gone.
    ///
    /// Absent in a record written by a release that did not keep it, which is treated as a run
    /// whose Tower has gone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    /// The agents whose flights started the run, as its history record will say: empty for work
    /// from outside the mesh, `None` when not known. See `RunRecord::sent_by`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sent_by: Option<Vec<AgentName>>,
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

    /// The records in `root`, for reading only: nothing is created, and a directory that does not
    /// exist yet reads as no run alive.
    ///
    /// For the dashboard, which watches a factory without being the one running it, and must not
    /// leave a state directory behind in a folder it was only pointed at.
    #[must_use]
    pub fn at(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// The record of `run`, if it is alive.
    ///
    /// `None` both when it is not and when its record cannot be read: either way there is nothing
    /// to report about it as running.
    #[must_use]
    pub fn find(&self, run: &RunId) -> Option<Live> {
        let text = fs::read_to_string(self.path_for(run)).ok()?;
        serde_json::from_str(&text).ok()
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

        let entries = match fs::read_dir(&self.root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(found),
            Err(error) => return Err(error),
        };
        for entry in entries {
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

#[cfg(test)]
mod tests {
    use super::*;

    struct Temp(PathBuf);

    impl Temp {
        fn new(name: &str) -> Self {
            let path =
                std::env::temp_dir().join(format!("layover-live-{name}-{}", std::process::id()));
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
            queued: None,
            owner: None,
            sent_by: None,
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
    fn a_reader_creates_nothing_and_finds_a_run_by_its_identifier() {
        let temp = Temp::new("reader");
        let reader = Ledger::at(temp.0.join("runs"));
        assert!(reader.live().expect("reads").is_empty());
        assert!(!temp.0.exists(), "watching a factory leaves nothing behind");

        let record = live();
        Ledger::open(temp.0.join("runs"))
            .expect("opens")
            .starting(&record)
            .expect("records");

        assert_eq!(reader.find(&record.run), Some(record));
        assert_eq!(reader.find(&RunId::generate()), None);
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
}
