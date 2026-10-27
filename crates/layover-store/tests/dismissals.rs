//! Dismissed chains against a real filesystem: a dismissal survives reopening the journal, is kept
//! once per chain, can be undone, and goes when retention removes the runs it was about.

use std::fs;
use std::path::PathBuf;

use jiff::Timestamp;
use layover_core::flight::{ItineraryId, RunId};
use layover_store::{Dismissal, Journal};

struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "layover-dismissals-{label}-{}",
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

fn dismissal(chain: &ItineraryId, run: &str, by: &str, when: &str) -> Dismissal {
    Dismissal {
        itinerary: chain.clone(),
        run: RunId::from(run),
        by: by.to_owned(),
        at: at(when),
        note: Some(format!("{by} fixed it by hand")),
    }
}

#[test]
fn nothing_is_dismissed_until_somebody_dismisses_it() {
    let dir = TempDir::new("empty");
    let journal = Journal::open(&dir.0).expect("opens");

    assert!(journal.dismissals().expect("reads").is_empty());
    assert!(
        !journal.undismiss(&ItineraryId::generate()).expect("undoes"),
        "nothing to undo"
    );
    assert!(
        !dir.0.join("dismissed.jsonl").exists(),
        "and nothing written"
    );
}

#[test]
fn a_dismissal_survives_a_restart_until_it_is_undone() {
    let dir = TempDir::new("survives");
    let chain = ItineraryId::generate();
    Journal::open(&dir.0)
        .expect("opens")
        .dismiss(dismissal(
            &chain,
            "run_wolf",
            "karl",
            "2026-10-27T09:00:00Z",
        ))
        .expect("dismisses");

    // A Tower restarted — reopening is all a restart does to the journal.
    let journal = Journal::open(&dir.0).expect("reopens");
    let kept = journal.dismissals().expect("reads");
    assert_eq!(
        kept,
        vec![dismissal(
            &chain,
            "run_wolf",
            "karl",
            "2026-10-27T09:00:00Z"
        )]
    );

    assert!(journal.undismiss(&chain).expect("undoes"));
    assert!(journal.dismissals().expect("reads").is_empty());
}

#[test]
fn dismissing_the_same_failure_twice_keeps_who_did_it_first() {
    let dir = TempDir::new("twice");
    let journal = Journal::open(&dir.0).expect("opens");
    let chain = ItineraryId::generate();

    let first = dismissal(&chain, "run_wolf", "karl", "2026-10-27T09:00:00Z");
    assert_eq!(journal.dismiss(first.clone()).expect("dismisses"), first);
    let again = journal
        .dismiss(dismissal(&chain, "run_wolf", "ada", "2026-10-27T10:00:00Z"))
        .expect("dismisses");

    assert_eq!(again, first, "a second press does not move who or when");
    assert_eq!(journal.dismissals().expect("reads"), vec![first]);
}

#[test]
fn dismissing_a_later_failure_of_a_chain_replaces_the_earlier_dismissal() {
    let dir = TempDir::new("later");
    let journal = Journal::open(&dir.0).expect("opens");
    let chain = ItineraryId::generate();
    let other = ItineraryId::generate();
    journal
        .dismiss(dismissal(&chain, "run_one", "karl", "2026-10-27T09:00:00Z"))
        .expect("dismisses");
    journal
        .dismiss(dismissal(&other, "run_x", "karl", "2026-10-27T09:30:00Z"))
        .expect("dismisses");

    let later = dismissal(&chain, "run_two", "ada", "2026-10-27T11:00:00Z");
    journal.dismiss(later.clone()).expect("dismisses");

    let kept = journal.dismissals().expect("reads");
    assert_eq!(kept.len(), 2, "one per chain: {kept:?}");
    assert!(kept.contains(&later));
    assert!(kept.iter().any(|kept| kept.itinerary == other));
}

#[test]
fn a_dismissal_goes_when_retention_removes_the_runs_it_was_about() {
    let dir = TempDir::new("retention");
    let journal = Journal::open(&dir.0).expect("opens");
    let old = ItineraryId::generate();
    let recent = ItineraryId::generate();
    journal
        .dismiss(dismissal(&old, "run_old", "karl", "2026-06-10T09:00:00Z"))
        .expect("dismisses");
    journal
        .dismiss(dismissal(
            &recent,
            "run_new",
            "karl",
            "2026-06-17T09:00:00Z",
        ))
        .expect("dismisses");

    // Run segments go a whole day at a time, the day before the horizon's being the last kept; a
    // dismissal comes after the run it dismisses, so it goes on the same rule and never before
    // that run does.
    journal.prune(at("2026-06-18T10:00:00Z")).expect("prunes");

    let kept: Vec<ItineraryId> = journal
        .dismissals()
        .expect("reads")
        .into_iter()
        .map(|kept| kept.itinerary)
        .collect();
    assert_eq!(kept, vec![recent]);
}

#[test]
fn a_line_written_by_hand_without_a_note_counts() {
    let dir = TempDir::new("by-hand");
    let chain = ItineraryId::generate();
    fs::write(
        dir.0.join("dismissed.jsonl"),
        format!(
            "{{\"itinerary\":\"{}\",\"run\":\"run_wolf\",\"by\":\"karl\",\"at\":\"2026-10-27T09:00:00Z\"}}\n",
            chain.as_str()
        ),
    )
    .expect("writes");

    let kept = Journal::open(&dir.0)
        .expect("opens")
        .dismissals()
        .expect("reads");

    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].itinerary, chain);
    assert_eq!(kept[0].note, None);
}
