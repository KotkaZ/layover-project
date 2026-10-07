//! Skipped ticks against a real filesystem: kept by day, read back newest first, and pruned with
//! everything else on the retention horizon.

use std::fs;
use std::path::PathBuf;

use jiff::Timestamp;
use jiff::tz::TimeZone;
use layover_core::cost::Window;
use layover_core::pipeline::PipelineName;
use layover_core::skip::Skip;
use layover_store::Journal;

struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "layover-skips-{label}-{}",
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

fn skipped(pipeline: &str, when: &str) -> Skip {
    Skip::still_working(PipelineName::new(pipeline), at(when))
}

#[test]
fn skipped_ticks_are_read_back_newest_first_within_the_window() {
    let dir = TempDir::new("read");
    let journal = Journal::open(&dir.0).expect("opens");
    for (pipeline, when) in [
        ("sweep", "2026-10-06T09:00:00Z"),
        ("sweep", "2026-10-07T10:00:00Z"),
        ("digest", "2026-10-07T08:00:00Z"),
        ("sweep", "2026-09-20T10:00:00Z"),
    ] {
        journal
            .record_skip(&skipped(pipeline, when))
            .expect("records");
    }

    let week = Window::Last7Days.resolve(&at("2026-10-07T12:00:00Z").to_zoned(TimeZone::UTC));
    let found = journal.skips(&week).expect("reads");

    let seen: Vec<_> = found
        .iter()
        .map(|skip| (skip.pipeline.as_str(), skip.at.to_string()))
        .collect();
    assert_eq!(
        seen,
        [
            ("sweep", "2026-10-07T10:00:00Z".to_owned()),
            ("digest", "2026-10-07T08:00:00Z".to_owned()),
            ("sweep", "2026-10-06T09:00:00Z".to_owned()),
        ],
        "the one from September is outside the week"
    );
}

#[test]
fn skipped_ticks_are_pruned_on_the_retention_horizon() {
    // A schedule that skips every minute writes 1,440 lines a day. Kept for ever, that is the
    // one record here that grows without bound.
    let dir = TempDir::new("prune");
    let journal = Journal::open(&dir.0).expect("opens");
    journal
        .record_skip(&skipped("sweep", "2026-01-01T10:00:00Z"))
        .expect("records");
    journal
        .record_skip(&skipped("sweep", "2026-10-07T10:00:00Z"))
        .expect("records");

    assert_eq!(
        journal.prune(at("2026-07-09T10:00:00Z")).expect("prunes"),
        1
    );

    let all = Window::AllTime.resolve(&at("2026-10-07T12:00:00Z").to_zoned(TimeZone::UTC));
    assert_eq!(journal.skips(&all).expect("reads").len(), 1);
}
