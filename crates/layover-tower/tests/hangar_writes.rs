//! Runs of one agent alive at once write to the same Hangar, the same journal and the same logbook,
//! and none of them may lose what another wrote.
//!
//! Driven through the runtime the MCP endpoint calls, wired to a real journal, from many threads at
//! once — the shape two Eagle reviews in parallel produce.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;

use jiff::{SignedDuration, Timestamp};
use layover_core::agent::AgentName;
use layover_core::cost::Window;
use layover_core::flight::{ItineraryId, RunId};
use layover_core::help::Blocker;
use layover_core::scope::RouteMap;
use layover_mcp::{Runtime, Session};
use layover_store::{HelpFilter, Journal};
use layover_tower::{FactoryRuntime, Wiring};

mod common;

const RUNS: usize = 12;
const EACH: usize = 5;

struct Fixture {
    runtime: Arc<FactoryRuntime>,
    journal: Arc<Journal>,
    dir: PathBuf,
}

impl Fixture {
    fn new(tag: &str) -> Self {
        let config = common::config();
        let dir = std::env::temp_dir().join(format!("layover-writes-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let journal = Arc::new(Journal::open(dir.join("journal")).expect("opens"));
        let (asking, filing, learning) = (
            Arc::clone(&journal),
            Arc::clone(&journal),
            Arc::clone(&journal),
        );

        let runtime = FactoryRuntime::new(Wiring {
            routes: Arc::new(RouteMap::from_config(&config)),
            config: Arc::new(config),
            hangars: dir.join("hangars"),
            logbook: dir.join("logbook.md"),
            queue: Arc::new(|_| Ok(())),
            book: Arc::new(|_| Ok(())),
            ask: Arc::new(move |request| asking.ask(&request).map_err(|e| e.to_string())),
            file: Arc::new(move |report| filing.file(&report).map_err(|e| e.to_string())),
            update_learnings: Arc::new(move |change| {
                learning
                    .update_learnings(|learnings| change(learnings))
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            }),
        });

        Self {
            runtime: Arc::new(runtime),
            journal,
            dir,
        }
    }

    /// Runs `write` from `RUNS` runs of `eagle` at once, each writing `EACH` times.
    fn at_once(&self, write: impl Fn(&FactoryRuntime, &Session, usize, usize) + Sync) {
        std::thread::scope(|scope| {
            for run in 0..RUNS {
                let write = &write;
                let runtime = &self.runtime;
                scope.spawn(move || {
                    let session = session();
                    for each in 0..EACH {
                        write(runtime, &session, run, each);
                    }
                });
            }
        });
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn session() -> Session {
    Session {
        run: RunId::generate(),
        agent: AgentName::new("eagle"),
        itinerary: ItineraryId::generate(),
        hops_remaining: 4,
        pipeline: None,
        flags: BTreeMap::new(),
        flight: None,
        within: BTreeSet::new(),
    }
}

fn span() -> layover_core::cost::Span {
    Window::AllTime.resolve(
        &(Timestamp::now() + SignedDuration::from_mins(1)).to_zoned(jiff::tz::TimeZone::UTC),
    )
}

fn lines_with(text: &str, marker: &str) -> BTreeSet<String> {
    text.lines()
        .filter(|line| line.contains(marker))
        .map(str::to_owned)
        .collect()
}

#[test]
fn every_note_written_to_one_memory_at_once_survives() {
    let fixture = Fixture::new("memory");
    fixture.at_once(|runtime, session, run, each| {
        runtime
            .memory_write(session, &format!("note {run}-{each}"))
            .expect("writes");
    });

    let memory =
        std::fs::read_to_string(fixture.dir.join("hangars").join("eagle").join("memory.md"))
            .expect("written");
    assert_eq!(lines_with(&memory, "note ").len(), RUNS * EACH, "{memory}");
}

#[test]
fn every_entry_appended_to_the_logbook_at_once_survives() {
    let fixture = Fixture::new("logbook");
    fixture.at_once(|runtime, session, run, each| {
        runtime
            .logbook_append(session, &format!("entry {run}-{each}"))
            .expect("appends");
    });

    let logbook = std::fs::read_to_string(fixture.dir.join("logbook.md")).expect("written");
    assert_eq!(lines_with(&logbook, "entry ").len(), RUNS * EACH);
}

#[test]
fn every_learning_proposed_at_once_is_kept() {
    // Each is its own claim, so none is folded into another as a rediscovery.
    let fixture = Fixture::new("learnings");
    fixture.at_once(|runtime, session, run, each| {
        runtime
            .learn(
                session,
                &format!("module m{run}x{each} needs its cache warmed before the suite starts"),
            )
            .expect("proposes");
    });

    let learnings = fixture.journal.learnings().expect("reads");
    assert_eq!(learnings.len(), RUNS * EACH);
}

#[test]
fn every_help_request_and_report_filed_at_once_is_readable() {
    let fixture = Fixture::new("journal");
    fixture.at_once(|runtime, session, run, each| {
        runtime
            .help(
                session,
                Blocker::Access,
                &format!("token {run}-{each} expired"),
                "the pull request API said 401",
                false,
            )
            .expect("asks");
        runtime
            .report(session, &format!("report {run}-{each}"), "done")
            .expect("files");
    });

    let asked = fixture
        .journal
        .help(&span(), &HelpFilter::default())
        .expect("every line parses");
    let filed = fixture.journal.reports(&span()).expect("every line parses");
    assert_eq!(asked.len(), RUNS * EACH);
    assert_eq!(filed.len(), RUNS * EACH);
}
