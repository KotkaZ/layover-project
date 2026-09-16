//! Help requests and learnings against a real filesystem.
//!
//! The parts worth testing here are the ones where the journal deliberately differs from run
//! history: help requests can be *changed* after the fact when somebody deals with them, and
//! learnings are state rather than events, so they are neither segmented nor pruned.

use std::fs;
use std::path::{Path, PathBuf};

use jiff::Timestamp;
use jiff::tz::TimeZone;
use layover_core::cost::{Span, Window};
use layover_core::flight::{ItineraryId, RunId};
use layover_core::help::{Blocker, HelpRequest};
use layover_core::learning::{Impact, Learnings, Proposal, State};
use layover_store::{HelpFilter, Journal};

/// A temporary directory that removes itself.
struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "layover-journal-{label}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock is after 1970")
                .as_nanos()
        ));
        fs::create_dir_all(&path).expect("temp dir");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
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

fn all_time() -> Span {
    Window::AllTime.resolve(&at("2026-09-16T12:00:00Z").to_zoned(TimeZone::UTC))
}

fn asked(agent: &str, blocker: Blocker, summary: &str, when: &str) -> HelpRequest {
    HelpRequest::new(
        agent.into(),
        RunId::generate(),
        ItineraryId::generate(),
        blocker,
        summary,
        "tried the obvious thing, it was refused",
        at(when),
    )
}

fn journal(dir: &TempDir) -> Journal {
    Journal::open(dir.path()).expect("opens")
}

#[test]
fn a_help_request_survives_a_round_trip_to_disk() {
    let dir = TempDir::new("help-roundtrip");
    let journal = journal(&dir);
    let request = asked(
        "publisher",
        Blocker::Access,
        "the ADO token expired",
        "2026-09-16T10:00:00Z",
    );

    journal.ask(&request).expect("files it");

    let found = journal
        .help(&all_time(), &HelpFilter::default())
        .expect("reads");
    assert_eq!(found, vec![request]);
}

#[test]
fn help_requests_come_back_most_recent_first() {
    let dir = TempDir::new("help-order");
    let journal = journal(&dir);
    for hour in ["08", "11", "09"] {
        journal
            .ask(&asked(
                "publisher",
                Blocker::Access,
                &format!("blocker raised at {hour}"),
                &format!("2026-09-16T{hour}:00:00Z"),
            ))
            .expect("files it");
    }

    let found = journal
        .help(&all_time(), &HelpFilter::default())
        .expect("reads");
    let hours: Vec<i8> = found
        .iter()
        .map(|request| request.at.to_zoned(TimeZone::UTC).hour())
        .collect();

    assert_eq!(hours, [11, 9, 8]);
}

#[test]
fn filters_narrow_by_agent_category_and_whether_anybody_has_dealt_with_it() {
    let dir = TempDir::new("help-filter");
    let journal = journal(&dir);
    journal
        .ask(&asked(
            "publisher",
            Blocker::Access,
            "no token",
            "2026-09-16T08:00:00Z",
        ))
        .expect("files it");
    journal
        .ask(&asked(
            "reviewer",
            Blocker::Tooling,
            "the linter would not start",
            "2026-09-16T09:00:00Z",
        ))
        .expect("files it");
    journal
        .ask(
            &asked(
                "publisher",
                Blocker::Access,
                "the branch push was refused",
                "2026-09-16T10:00:00Z",
            )
            .fatal(),
        )
        .expect("files it");

    let by_agent = HelpFilter {
        agent: Some("publisher".into()),
        ..HelpFilter::default()
    };
    let by_kind = HelpFilter {
        blocker: Some(Blocker::Tooling),
        ..HelpFilter::default()
    };
    let only_fatal = HelpFilter {
        fatal_only: true,
        ..HelpFilter::default()
    };

    assert_eq!(
        journal.help(&all_time(), &by_agent).expect("reads").len(),
        2
    );
    assert_eq!(journal.help(&all_time(), &by_kind).expect("reads").len(), 1);
    assert_eq!(
        journal.help(&all_time(), &only_fatal).expect("reads").len(),
        1
    );
}

#[test]
fn a_request_can_be_marked_dealt_with_and_then_stops_being_open() {
    // The one append-only thing here that genuinely changes after the fact. A list of blockers
    // that cannot be cleared is a list nobody reads twice.
    let dir = TempDir::new("help-resolve");
    let journal = journal(&dir);
    journal
        .ask(&asked(
            "publisher",
            Blocker::Access,
            "the ADO token expired",
            "2026-09-16T10:00:00Z",
        ))
        .expect("files it");

    let open = HelpFilter {
        open_only: true,
        ..HelpFilter::default()
    };
    assert_eq!(journal.help(&all_time(), &open).expect("reads").len(), 1);

    let resolved = journal
        .resolve(&all_time(), &open, at("2026-09-16T11:00:00Z"))
        .expect("resolves");

    assert_eq!(resolved, 1);
    assert!(journal.help(&all_time(), &open).expect("reads").is_empty());
    assert_eq!(
        journal
            .help(&all_time(), &HelpFilter::default())
            .expect("reads")
            .len(),
        1,
        "it is closed, not deleted"
    );
}

#[test]
fn resolving_can_be_narrowed_to_one_agent() {
    let dir = TempDir::new("help-resolve-one");
    let journal = journal(&dir);
    journal
        .ask(&asked(
            "publisher",
            Blocker::Access,
            "no token",
            "2026-09-16T08:00:00Z",
        ))
        .expect("files it");
    journal
        .ask(&asked(
            "reviewer",
            Blocker::Tooling,
            "the linter would not start",
            "2026-09-16T09:00:00Z",
        ))
        .expect("files it");

    let just_the_publisher = HelpFilter {
        agent: Some("publisher".into()),
        ..HelpFilter::default()
    };
    let resolved = journal
        .resolve(&all_time(), &just_the_publisher, at("2026-09-16T11:00:00Z"))
        .expect("resolves");

    let still_open = journal
        .help(
            &all_time(),
            &HelpFilter {
                open_only: true,
                ..HelpFilter::default()
            },
        )
        .expect("reads");

    assert_eq!(resolved, 1);
    assert_eq!(still_open.len(), 1);
    assert_eq!(still_open[0].agent.as_str(), "reviewer");
}

#[test]
fn resolving_twice_does_not_double_count() {
    let dir = TempDir::new("help-resolve-twice");
    let journal = journal(&dir);
    journal
        .ask(&asked(
            "publisher",
            Blocker::Access,
            "no token",
            "2026-09-16T08:00:00Z",
        ))
        .expect("files it");

    let open = HelpFilter {
        open_only: true,
        ..HelpFilter::default()
    };
    journal
        .resolve(&all_time(), &open, at("2026-09-16T11:00:00Z"))
        .expect("resolves");

    assert_eq!(
        journal
            .resolve(&all_time(), &open, at("2026-09-16T12:00:00Z"))
            .expect("resolves"),
        0
    );
}

#[test]
fn help_is_pruned_on_the_retention_horizon() {
    let dir = TempDir::new("help-prune");
    let journal = journal(&dir);
    journal
        .ask(&asked(
            "publisher",
            Blocker::Access,
            "ancient blocker",
            "2026-01-01T10:00:00Z",
        ))
        .expect("files it");
    journal
        .ask(&asked(
            "publisher",
            Blocker::Access,
            "recent blocker",
            "2026-09-16T10:00:00Z",
        ))
        .expect("files it");

    assert_eq!(
        journal.prune(at("2026-06-18T10:00:00Z")).expect("prunes"),
        1
    );
    assert_eq!(
        journal
            .help(&all_time(), &HelpFilter::default())
            .expect("reads")
            .len(),
        1
    );
}

#[test]
fn learnings_survive_a_round_trip() {
    let dir = TempDir::new("learn-roundtrip");
    let journal = journal(&dir);
    let mut learnings = Learnings::new();
    learnings.propose(&Proposal::new(
        "publisher".into(),
        "the ADO token expires every thirty days",
        Impact::High,
        at("2026-09-16T10:00:00Z"),
    ));

    journal.save_learnings(&learnings).expect("saves");
    let back = journal.learnings().expect("reads");

    assert_eq!(back.len(), 1);
    assert_eq!(back.all().next().expect("one").impact, Impact::High);
    assert_eq!(back.all().next().expect("one").state, State::Provisional);
}

#[test]
fn a_factory_that_has_learned_nothing_reads_as_empty_rather_than_failing() {
    // Every factory starts here, so a missing file must not be an error.
    let dir = TempDir::new("learn-empty");

    assert!(journal(&dir).learnings().expect("reads").is_empty());
}

#[test]
fn learnings_are_not_segmented_by_day() {
    // They are state, not events. Segmenting would scatter one learning's life across ninety
    // files, and its life is exactly the thing worth reading.
    let dir = TempDir::new("learn-single");
    let journal = journal(&dir);
    let mut learnings = Learnings::new();
    learnings.propose(&Proposal::new(
        "publisher".into(),
        "the ADO token expires every thirty days",
        Impact::High,
        at("2026-01-01T10:00:00Z"),
    ));
    learnings.propose(&Proposal::new(
        "publisher".into(),
        "prefer ripgrep when searching the tree",
        Impact::Low,
        at("2026-09-16T10:00:00Z"),
    ));
    journal.save_learnings(&learnings).expect("saves");

    let files: Vec<String> = fs::read_dir(dir.path())
        .expect("lists")
        .map(|entry| entry.expect("entry").file_name().to_string_lossy().into())
        .collect();

    assert_eq!(files, ["learnings.jsonl"]);
}

#[test]
fn learnings_are_not_pruned_by_the_retention_horizon() {
    // A confirmed learning that expired for being ninety days old would be the one thing here
    // that got worse the longer it was right.
    let dir = TempDir::new("learn-keep");
    let journal = journal(&dir);
    let mut learnings = Learnings::new();
    learnings.propose(&Proposal::new(
        "publisher".into(),
        "the ADO token expires every thirty days",
        Impact::High,
        at("2026-01-01T10:00:00Z"),
    ));
    journal.save_learnings(&learnings).expect("saves");

    journal.prune(at("2026-09-16T10:00:00Z")).expect("prunes");

    assert_eq!(journal.learnings().expect("reads").len(), 1);
}

#[test]
fn a_save_leaves_no_partial_file_behind() {
    // Written to a neighbour and renamed. Appending can tolerate a torn line because a line is
    // one record; here the file is the record.
    let dir = TempDir::new("learn-atomic");
    let journal = journal(&dir);
    let mut learnings = Learnings::new();
    learnings.propose(&Proposal::new(
        "publisher".into(),
        "the ADO token expires every thirty days",
        Impact::High,
        at("2026-09-16T10:00:00Z"),
    ));

    journal.save_learnings(&learnings).expect("saves");
    journal.save_learnings(&learnings).expect("saves again");

    let leftovers: Vec<String> = fs::read_dir(dir.path())
        .expect("lists")
        .map(|entry| entry.expect("entry").file_name().to_string_lossy().into())
        .filter(|name: &String| name.contains("writing"))
        .collect();

    assert!(
        leftovers.is_empty(),
        "staging file left behind: {leftovers:?}"
    );
}

#[test]
fn the_whole_life_of_a_learning_persists() {
    // Rediscovery counts, lapse state and remaining runs all have to survive a restart, or the
    // confirmation signal resets every time the Tower blinks.
    let dir = TempDir::new("learn-life");
    let journal = journal(&dir);
    let agent = "publisher".into();
    let mut learnings = Learnings::new();
    let proposal = Proposal::new(
        "publisher".into(),
        "the ADO token expires every thirty days",
        Impact::High,
        at("2026-09-16T10:00:00Z"),
    );

    learnings.propose(&proposal);
    for _ in 0..layover_core::learning::PROVISIONAL_RUNS {
        learnings.charge_run(&agent);
    }
    learnings.propose(&proposal);

    journal.save_learnings(&learnings).expect("saves");
    let back = journal.learnings().expect("reads");
    let learning = back.all().next().expect("one");

    assert_eq!(learning.proposals, 2, "the rediscovery count survived");
    assert_eq!(learning.state, State::Provisional);
    assert_eq!(
        learning.runs_left,
        layover_core::learning::PROVISIONAL_RUNS,
        "and so did its second life"
    );
}
