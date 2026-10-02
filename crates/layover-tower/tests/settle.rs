//! A run 1.3.0 left behind is settled with what can be known about it, not with when it was found.
//!
//! The case seen after the 1.4.0 upgrade: a run that died twenty-nine minutes in was recorded as
//! taking five hours and forty-seven minutes — the time until the next Tower noticed — with no
//! workflow, reading in the Runs list like a failed `devforge` run.

use std::collections::BTreeMap;
use std::process::Command;

use jiff::{SignedDuration, Timestamp};
use layover_core::agent::AgentName;
use layover_core::flight::{ItineraryId, RunId};
use layover_core::pipeline::PipelineName;
use layover_core::run::{Outcome, RunRecord};
use layover_store::History;
use layover_tower::{Factory, Ledger, Live};

mod runs;
use runs::{Temp, factory, history};

const FACTORY: &str = r#"
[layover]
work_dir = "work"

[defaults]
runner = "shell"

[runners.shell]
command = ["echo"]

[agents.bob]
prompt = "build"
entry = true

[pipelines.devforge]
entry = "bob"
"#;

/// A process that has come and gone.
fn exited() -> u32 {
    let mut child = if cfg!(windows) {
        Command::new("cmd").args(["/c", "exit 0"]).spawn()
    } else {
        Command::new("sh").args(["-c", "true"]).spawn()
    }
    .expect("spawns");
    let pid = child.id();
    let _ = child.wait();
    pid
}

/// What 1.3.0 left: no work, no owner, a run that started six hours ago and last wrote to its
/// transcript `last_written` ago.
fn left_by_1_3(temp: &Temp, chain: &ItineraryId, last_written: Option<SignedDuration>) -> Live {
    let run = RunId::generate();
    let hangar = temp
        .0
        .join(".layover")
        .join("hangars")
        .join("bob")
        .join(run.as_str());
    std::fs::create_dir_all(&hangar).expect("hangar");
    if let Some(ago) = last_written {
        let transcript = hangar.join("transcript.log");
        std::fs::write(&transcript, "working…\n").expect("writes");
        let when = std::time::SystemTime::now() - ago.unsigned_abs();
        std::fs::File::options()
            .write(true)
            .open(&transcript)
            .and_then(|file| file.set_modified(when))
            .expect("dates it");
    }
    let live = Live {
        run,
        itinerary: chain.clone(),
        agent: AgentName::new("bob"),
        pid: exited(),
        started_at: Timestamp::now() - SignedDuration::from_hours(6),
        hangar,
        queued: None,
        owner: None,
        sent_by: None,
    };
    Ledger::open(temp.0.join(".layover").join("state").join("runs"))
        .expect("ledger")
        .starting(&live)
        .expect("records");
    live
}

fn settled(temp: &Temp, live: &Live) -> RunRecord {
    let tower: Factory = factory(temp, FACTORY);
    let _ = tower.reconcile(&mut |_| Ok(()));
    history(&temp.0)
        .into_iter()
        .find(|record| record.run == live.run)
        .expect("in history")
}

#[test]
fn a_lost_run_ends_when_it_was_last_seen_alive_in_the_workflow_its_chain_records() {
    let temp = Temp::new("legacy-evidence");
    let chain = ItineraryId::generate();

    // An earlier run of the same chain, recorded with its workflow.
    let mut earlier = RunRecord::started(
        RunId::generate(),
        chain.clone(),
        AgentName::new("bob"),
        Timestamp::now() - SignedDuration::from_hours(7),
    );
    earlier.outcome = Outcome::Succeeded;
    earlier.finished_at = Some(Timestamp::now() - SignedDuration::from_hours(6));
    earlier.pipeline = Some(PipelineName::new("devforge"));
    earlier.flags = BTreeMap::new();
    History::open(temp.0.join(".layover").join("history"))
        .expect("history")
        .append(&earlier)
        .expect("records");

    let last_written = SignedDuration::from_mins(5 * 60 + 31);
    let lost = left_by_1_3(&temp, &chain, Some(last_written));
    let record = settled(&temp, &lost);

    assert_eq!(record.outcome, Outcome::Interrupted);
    let finished = record.finished_at.expect("ended");
    let expected = Timestamp::now() - last_written;
    assert!(
        (finished.as_second() - expected.as_second()).abs() <= 5,
        "ended {finished}, when its transcript was last written ({expected}), not when it was found"
    );
    assert_eq!(
        record.duration_secs(),
        Some(29 * 60),
        "twenty-nine minutes, not six hours"
    );
    assert_eq!(record.pipeline, Some(PipelineName::new("devforge")));
    let detail = record.detail.unwrap_or_default();
    assert!(
        detail.starts_with("lost when the Tower stopped"),
        "{detail}"
    );
    assert!(
        detail.contains("Layover 1.3.0 and earlier did not record its work"),
        "{detail}"
    );
    assert!(
        detail.contains("Re-trigger it if it is still needed."),
        "{detail}"
    );
}

#[test]
fn a_lost_run_nothing_else_records_keeps_no_workflow_and_ends_where_it_began() {
    // Better unplaced than filed under a workflow it never belonged to.
    let temp = Temp::new("legacy-nothing");
    let lost = left_by_1_3(&temp, &ItineraryId::generate(), None);
    let record = settled(&temp, &lost);

    assert_eq!(record.pipeline, None);
    assert_eq!(
        record.finished_at,
        Some(lost.started_at),
        "the start is the last sign of life"
    );
}
