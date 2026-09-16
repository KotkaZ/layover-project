//! History behaviour that only shows up against a real filesystem.
//!
//! The unit tests in `layover-core` cover the shape of a record and the arithmetic of a window.
//! What is left is what the two do to each other on disk: segmentation, retention, and the
//! local/UTC boundary that is the whole reason the files are named the way they are.

use std::fs;
use std::path::{Path, PathBuf};

use jiff::tz::TimeZone;
use jiff::{Timestamp, ToSpan, Zoned};
use layover_core::cost::{CostSource, Span, TokenUsage, Window};
use layover_core::flight::{ItineraryId, RunId};
use layover_core::run::{Outcome, RunRecord};
use layover_store::{History, RunFilter};

/// A temporary directory that removes itself.
///
/// Hand-rolled rather than pulled from a crate: this is the only place in the workspace that
/// needs one, and a dependency that exists to save nine lines is a dependency to audit forever.
struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "layover-history-{label}-{}",
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

fn tallinn() -> TimeZone {
    TimeZone::get("Europe/Tallinn").expect("bundled zone")
}

/// A fixed moment to resolve windows against.
///
/// Windows end at the instant they are resolved, so a test that used the real clock would start
/// failing the moment its fixture timestamps drifted into the future — which is to say, on a
/// machine whose date is a few hours behind the one the test was written on.
fn now() -> Zoned {
    at("2026-09-16T12:00:00Z").to_zoned(tallinn())
}

fn all_time() -> Span {
    Window::AllTime.resolve(&now())
}

fn finished(agent: &str, when: &str, usd: f64) -> RunRecord {
    let end = at(when);
    RunRecord::started(
        RunId::generate(),
        ItineraryId::generate(),
        agent.into(),
        end - 60.seconds(),
    )
    .finished(Outcome::Succeeded, end)
    .costing(usd, CostSource::Reported, TokenUsage::default())
}

/// A history whose calendar windows are reckoned somewhere fixed, so the tests do not depend on
/// where the machine running them happens to be.
fn history(dir: &TempDir) -> History {
    History::open(dir.path()).expect("opens").in_zone(tallinn())
}

#[test]
fn a_record_survives_a_round_trip_to_disk() {
    let dir = TempDir::new("roundtrip");
    let store = history(&dir);
    let record = finished("developer", "2026-09-16T10:00:00Z", 1.25);

    store.append(&record).expect("appends");

    let back = store
        .runs(&all_time(), &RunFilter::default())
        .expect("reads");
    assert_eq!(back, vec![record]);
}

#[test]
fn records_are_filed_into_one_segment_per_utc_day() {
    let dir = TempDir::new("segments");
    let store = history(&dir);

    store
        .append(&finished("a", "2026-09-14T23:00:00Z", 1.0))
        .expect("appends");
    store
        .append(&finished("b", "2026-09-15T01:00:00Z", 1.0))
        .expect("appends");
    store
        .append(&finished("c", "2026-09-15T22:00:00Z", 1.0))
        .expect("appends");

    let mut names: Vec<String> = fs::read_dir(dir.path())
        .expect("lists")
        .map(|entry| entry.expect("entry").file_name().to_string_lossy().into())
        .collect();
    names.sort();

    assert_eq!(names, ["runs-2026-09-14.jsonl", "runs-2026-09-15.jsonl"]);
}

#[test]
fn a_local_window_picks_up_runs_from_the_neighbouring_utc_day() {
    // This is the case the segmentation exists to survive. In UTC+3, local midnight on the 16th
    // is 21:00 UTC on the 15th, so a run at 22:00 UTC on the 15th is "today" locally while
    // living in the previous day's file. Reading only the matching UTC day would lose it.
    let dir = TempDir::new("boundary");
    let store = history(&dir);
    let evening_before = finished("analyst", "2026-09-15T22:00:00Z", 2.0);
    store.append(&evening_before).expect("appends");

    let span = Window::Today.resolve(&at("2026-09-16T09:00:00Z").to_zoned(tallinn()));
    let found = store.runs(&span, &RunFilter::default()).expect("reads");

    assert_eq!(found, vec![evening_before]);
}

#[test]
fn a_window_excludes_what_falls_outside_it() {
    let dir = TempDir::new("window");
    let store = history(&dir);
    store
        .append(&finished("old", "2026-09-01T10:00:00Z", 5.0))
        .expect("appends");
    let recent = finished("new", "2026-09-16T10:00:00Z", 1.0);
    store.append(&recent).expect("appends");

    let found = store
        .runs(&Window::Last7Days.resolve(&now()), &RunFilter::default())
        .expect("reads");

    assert_eq!(found, vec![recent]);
}

#[test]
fn runs_come_back_most_recent_first() {
    let dir = TempDir::new("order");
    let store = history(&dir);
    for hour in ["08", "11", "09"] {
        store
            .append(&finished("dev", &format!("2026-09-16T{hour}:00:00Z"), 1.0))
            .expect("appends");
    }

    let found = store
        .runs(&all_time(), &RunFilter::default())
        .expect("reads");
    let hours: Vec<i8> = found
        .iter()
        .map(|record| record.filed_at().to_zoned(TimeZone::UTC).hour())
        .collect();

    assert_eq!(hours, [11, 9, 8], "newest first is what a dashboard shows");
}

#[test]
fn filters_narrow_by_agent_outcome_and_count() {
    let dir = TempDir::new("filter");
    let store = history(&dir);
    store
        .append(&finished("developer", "2026-09-16T08:00:00Z", 1.0))
        .expect("appends");
    store
        .append(&finished("tester", "2026-09-16T09:00:00Z", 1.0))
        .expect("appends");
    store
        .append(
            &finished("developer", "2026-09-16T10:00:00Z", 1.0)
                .finished(Outcome::Failed, at("2026-09-16T10:00:00Z")),
        )
        .expect("appends");

    let all = all_time();
    let by_agent = RunFilter {
        agent: Some("developer".into()),
        ..RunFilter::default()
    };
    let by_outcome = RunFilter {
        outcome: Some(Outcome::Failed),
        ..RunFilter::default()
    };
    let capped = RunFilter {
        limit: Some(2),
        ..RunFilter::default()
    };

    assert_eq!(store.runs(&all, &by_agent).expect("reads").len(), 2);
    assert_eq!(store.runs(&all, &by_outcome).expect("reads").len(), 1);
    assert_eq!(store.runs(&all, &capped).expect("reads").len(), 2);
}

#[test]
fn a_truncated_final_line_costs_one_record_and_not_the_file() {
    // What a power cut leaves behind. Losing the rest of the day to it would turn a crash into
    // data loss, which is a poor trade for a file format chosen partly for its durability.
    let dir = TempDir::new("torn");
    let store = history(&dir);
    let intact = finished("developer", "2026-09-16T10:00:00Z", 1.0);
    store.append(&intact).expect("appends");

    let segment = dir.path().join("runs-2026-09-16.jsonl");
    let mut raw = fs::read_to_string(&segment).expect("reads");
    raw.push_str(r#"{"run":"run_01","itinerary":"itn_01","agent":"dev"#);
    fs::write(&segment, raw).expect("writes");

    let found = store
        .runs(&all_time(), &RunFilter::default())
        .expect("reads despite the torn line");

    assert_eq!(found, vec![intact]);
}

#[test]
fn files_that_are_not_segments_are_left_alone() {
    // The directory is the operator's to look in. An editor's swap file or a note they left
    // themselves is not a corruption, and must not become an error on every page load.
    let dir = TempDir::new("strays");
    let store = history(&dir);
    store
        .append(&finished("developer", "2026-09-16T10:00:00Z", 1.0))
        .expect("appends");
    fs::write(dir.path().join("notes.txt"), "remember to check the tester").expect("writes");
    fs::write(dir.path().join(".runs-2026-09-16.jsonl.swp"), "garbage").expect("writes");

    let found = store
        .runs(&all_time(), &RunFilter::default())
        .expect("reads");

    assert_eq!(found.len(), 1);
}

#[test]
fn retention_deletes_whole_segments_past_the_horizon() {
    let dir = TempDir::new("prune");
    let store = history(&dir);
    store
        .append(&finished("ancient", "2026-01-01T10:00:00Z", 1.0))
        .expect("appends");
    let kept = finished("recent", "2026-09-16T10:00:00Z", 1.0);
    store.append(&kept).expect("appends");

    let horizon = at("2026-06-18T10:00:00Z");
    assert_eq!(store.prune(horizon).expect("prunes"), 1);

    let found = store
        .runs(&all_time(), &RunFilter::default())
        .expect("reads");
    assert_eq!(found, vec![kept]);
    assert!(!dir.path().join("runs-2026-01-01.jsonl").exists());
}

#[test]
fn retention_keeps_a_segment_that_still_holds_live_records() {
    // Segments are deleted whole, so the day containing the horizon must survive — deleting it
    // would take records that are still inside the window along with the expired ones.
    let dir = TempDir::new("prune-edge");
    let store = history(&dir);
    let straddling = finished("edge", "2026-06-18T23:00:00Z", 1.0);
    store.append(&straddling).expect("appends");

    assert_eq!(store.prune(at("2026-06-18T10:00:00Z")).expect("prunes"), 0);
    assert_eq!(
        store
            .runs(&all_time(), &RunFilter::default())
            .expect("reads"),
        vec![straddling]
    );
}

#[test]
fn a_ledger_built_from_history_totals_the_window() {
    let dir = TempDir::new("ledger");
    let store = history(&dir);
    store
        .append(&finished("developer", "2026-09-16T08:00:00Z", 4.00))
        .expect("appends");
    store
        .append(&finished("tester", "2026-09-16T09:00:00Z", 0.50))
        .expect("appends");

    let ledger = store.ledger(&all_time()).expect("totals");

    assert_eq!(ledger.total().runs, 2);
    assert!((ledger.total().usd - 4.50).abs() < 1e-9);
    assert!(ledger.total().is_fully_measured());
}

#[test]
fn a_run_still_going_is_not_billed_yet() {
    // Its cost is not known, and counting a zero would make the total jump when the real figure
    // arrives — a dashboard whose numbers move backwards is one nobody trusts.
    let dir = TempDir::new("live");
    let store = history(&dir);
    store
        .append(&RunRecord::started(
            RunId::generate(),
            ItineraryId::generate(),
            "developer".into(),
            at("2026-09-16T10:00:00Z"),
        ))
        .expect("appends");

    let all = all_time();
    assert_eq!(
        store
            .runs(&all, &RunFilter::default())
            .expect("reads")
            .len(),
        1
    );
    assert_eq!(store.ledger(&all).expect("totals").total().runs, 0);
}

#[test]
fn an_empty_history_reads_as_empty_rather_than_failing() {
    let dir = TempDir::new("empty");
    let store = history(&dir);

    let all = all_time();
    assert!(
        store
            .runs(&all, &RunFilter::default())
            .expect("reads")
            .is_empty()
    );
    assert!(store.ledger(&all).expect("totals").is_empty());
    assert_eq!(store.prune(Timestamp::now()).expect("prunes"), 0);
}

#[test]
fn opening_a_history_creates_the_directory() {
    let dir = TempDir::new("create");
    let nested = dir.path().join("state").join("history");

    let store = History::open(&nested).expect("creates");

    assert!(nested.is_dir());
    assert_eq!(store.root(), nested);
}
