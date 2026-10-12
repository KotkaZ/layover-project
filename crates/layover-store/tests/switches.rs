//! Switched-off workflows against a real filesystem: they survive reopening the journal, switching
//! twice changes nothing, and a line written by hand counts.

use std::fs;
use std::path::PathBuf;

use jiff::Timestamp;
use layover_core::pipeline::PipelineName;
use layover_store::Journal;

struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "layover-switches-{label}-{}",
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

fn names(journal: &Journal) -> Vec<String> {
    journal
        .switched_off()
        .expect("reads")
        .into_iter()
        .map(|off| off.pipeline.to_string())
        .collect()
}

#[test]
fn nothing_is_off_until_somebody_switches_it_off() {
    let dir = TempDir::new("empty");
    let journal = Journal::open(&dir.0).expect("opens");

    assert!(names(&journal).is_empty());
    assert!(!dir.0.join("switched-off.jsonl").exists());
}

#[test]
fn a_workflow_switched_off_stays_off_across_a_restart_until_switched_on() {
    let dir = TempDir::new("survives");
    let sweep = PipelineName::new("review-sweep");
    Journal::open(&dir.0)
        .expect("opens")
        .switch_off(&sweep, at("2026-10-12T07:00:00Z"))
        .expect("switches off");

    // A Tower restarted — reopening is all a restart does to the journal.
    let journal = Journal::open(&dir.0).expect("reopens");
    let off = journal.switched_off().expect("reads");
    assert_eq!(off.len(), 1);
    assert_eq!(off[0].pipeline, sweep);
    assert_eq!(off[0].since, Some(at("2026-10-12T07:00:00Z")));

    assert!(journal.switch_on(&sweep).expect("switches on"));
    assert!(names(&journal).is_empty());
    assert!(
        !journal.switch_on(&sweep).expect("switches on again"),
        "it was not off"
    );
}

#[test]
fn switching_off_twice_keeps_when_it_was_first_switched_off() {
    let dir = TempDir::new("twice");
    let journal = Journal::open(&dir.0).expect("opens");
    let sweep = PipelineName::new("review-sweep");

    journal
        .switch_off(&sweep, at("2026-10-12T07:00:00Z"))
        .expect("switches off");
    let again = journal
        .switch_off(&sweep, at("2026-10-12T09:00:00Z"))
        .expect("switches off again");

    assert_eq!(again.since, Some(at("2026-10-12T07:00:00Z")));
    assert_eq!(names(&journal), ["review-sweep"]);
}

#[test]
fn only_the_workflow_switched_on_comes_back() {
    let dir = TempDir::new("one");
    let journal = Journal::open(&dir.0).expect("opens");
    for name in ["development", "review-sweep", "upstream-watch"] {
        journal
            .switch_off(&PipelineName::new(name), at("2026-10-12T07:00:00Z"))
            .expect("switches off");
    }

    journal
        .switch_on(&PipelineName::new("review-sweep"))
        .expect("switches on");

    assert_eq!(names(&journal), ["development", "upstream-watch"]);
}

#[test]
fn a_line_written_by_hand_switches_a_workflow_off() {
    // For when nothing is answering: the file is the switch, as the Ground Stop's file is.
    let dir = TempDir::new("by-hand");
    fs::write(
        dir.0.join("switched-off.jsonl"),
        "{\"pipeline\":\"review-sweep\"}\n{\"pipeline\":\"review-sweep\"}\nnot json\n",
    )
    .expect("writes");
    let journal = Journal::open(&dir.0).expect("opens");

    let off = journal.switched_off().expect("reads");
    assert_eq!(off.len(), 1, "a repeated line is the same switch: {off:?}");
    assert_eq!(off[0].pipeline.as_str(), "review-sweep");
    assert_eq!(off[0].since, None);
}
