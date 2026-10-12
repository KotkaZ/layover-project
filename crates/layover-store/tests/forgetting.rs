//! Lapsed learnings deleted from disk once their days are up, and nothing else with them.

use std::fs;
use std::path::PathBuf;

use jiff::Timestamp;
use layover_core::agent::AgentName;
use layover_core::learning::{Impact, Learnings, PROVISIONAL_RUNS, Proposal, State};
use layover_store::Journal;

struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "layover-forgetting-{label}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock is after 1970")
                .as_nanos()
        ));
        fs::create_dir_all(&path).expect("temp dir");
        Self(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn at(rfc3339: &str) -> Timestamp {
    rfc3339.parse().expect("valid timestamp")
}

fn propose(learnings: &mut Learnings, text: &str) {
    learnings.propose(&Proposal::new(
        "eagle".into(),
        text,
        Impact::Medium,
        at("2026-10-01T09:00:00Z"),
    ));
}

#[test]
fn a_lapsed_learning_past_its_days_is_deleted_from_disk_and_the_rest_are_kept() {
    let dir = TempDir::new("forget");
    let journal = Journal::open(&dir.0).expect("opens");
    let eagle = AgentName::from("eagle");

    let mut learnings = Learnings::new();
    propose(&mut learnings, "the sweep must skip draft pull requests");
    for _ in 0..PROVISIONAL_RUNS {
        learnings.charge_run(&eagle, at("2026-10-02T09:00:00Z"));
    }
    propose(&mut learnings, "pin the toolchain before building");
    journal.save_learnings(&learnings).expect("saves");

    assert_eq!(
        journal
            .forget_lapsed(at("2026-10-02T08:59:59Z"))
            .expect("reads"),
        0,
        "not yet"
    );
    assert_eq!(journal.learnings().expect("reads").len(), 2);

    assert_eq!(
        journal
            .forget_lapsed(at("2026-10-16T09:00:00Z"))
            .expect("forgets"),
        1
    );
    let left = journal.learnings().expect("reads");
    let states: Vec<State> = left.all().map(|learning| learning.state).collect();
    assert_eq!(states, [State::Provisional]);
}

#[test]
fn when_a_learning_lapsed_survives_a_restart() {
    let dir = TempDir::new("lapsed-at");
    let journal = Journal::open(&dir.0).expect("opens");
    let mut learnings = Learnings::new();
    propose(&mut learnings, "the sweep must skip draft pull requests");
    for _ in 0..PROVISIONAL_RUNS {
        learnings.charge_run(&"eagle".into(), at("2026-10-02T09:00:00Z"));
    }
    journal.save_learnings(&learnings).expect("saves");

    let back = Journal::open(&dir.0)
        .expect("reopens")
        .learnings()
        .expect("reads");
    assert_eq!(
        back.all().next().expect("one").lapsed_at,
        Some(at("2026-10-02T09:00:00Z"))
    );
}
