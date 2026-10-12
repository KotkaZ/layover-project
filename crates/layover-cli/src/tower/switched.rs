//! The loop's side of a switched-off workflow: its schedule neither fires nor skips, a resuming one
//! collects nothing, and switched back on it waits for its next tick.

use std::sync::Mutex;

use jiff::{SignedDuration, Timestamp};
use layover_core::agent::AgentName;
use layover_core::flight::ItineraryId;
use layover_core::pipeline::PipelineName;
use layover_store::Journal;
use layover_tower::{Clock, Factory};

use super::fire_due;
use super::fixture::{factory_config, temp};

fn after(now: Timestamp, minutes: i64) -> Timestamp {
    now.checked_add(SignedDuration::from_mins(minutes))
        .expect("in range")
}

fn skips(journal: &Journal, now: Timestamp) -> usize {
    let all = layover_core::cost::Window::AllTime.resolve(&now.to_zoned(jiff::tz::TimeZone::UTC));
    journal.skips(&all).expect("reads").len()
}

#[test]
fn a_switched_off_schedule_neither_fires_nor_skips_and_does_not_catch_up() {
    let root = temp("switched-schedule");
    let config = factory_config(
        r#"
[pipelines.sweep]
entry = "worker"
trigger = { every = "1h" }
"#,
    );
    let factory = Factory::new(config.clone(), &root).expect("opens");
    let journal = Journal::open(root.join(".layover").join("journal")).expect("opens");
    let sweep = PipelineName::new("sweep");
    let now = Timestamp::now();
    let clock = Clock::new(&config, now);
    let said = Mutex::new(Vec::new());
    let announce = |line: String| said.lock().expect("unpoisoned").push(line);

    journal.switch_off(&sweep, now).expect("switches off");
    fire_due(
        &factory,
        &journal,
        &clock,
        &config,
        after(now, 61),
        &announce,
    );

    assert!(
        journal.pending().expect("reads").is_empty(),
        "nothing fired"
    );
    assert_eq!(skips(&journal, now), 0, "and nothing was skipped");
    assert!(said.lock().expect("unpoisoned").is_empty(), "{said:?}");

    journal.switch_on(&sweep).expect("switches on");
    fire_due(
        &factory,
        &journal,
        &clock,
        &config,
        after(now, 62),
        &announce,
    );
    assert!(
        journal.pending().expect("reads").is_empty(),
        "the tick that passed while it was off is not made up"
    );

    fire_due(
        &factory,
        &journal,
        &clock,
        &config,
        after(now, 121),
        &announce,
    );
    assert_eq!(
        journal.pending().expect("reads").len(),
        1,
        "its next tick fires"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_switched_off_follow_up_leaves_its_layovers_waiting() {
    let root = temp("switched-follow-up");
    let config = factory_config(
        r#"
[pipelines.follow_up]
entry = "worker"
trigger = { every = "20m" }
resumes = true
"#,
    );
    let factory = Factory::new(config.clone(), &root).expect("opens");
    let journal = Journal::open(root.join(".layover").join("journal")).expect("opens");
    let now = Timestamp::now();
    journal
        .book(layover_core::layover::Layover::book(
            AgentName::new("worker"),
            ItineraryId::generate(),
            "the review to land",
            layover_core::handover::Handover::dispatch(Vec::new()),
            after(now, -120),
            after(now, -60),
        ))
        .expect("books");
    let clock = Clock::new(&config, now);

    journal
        .switch_off(&PipelineName::new("follow_up"), now)
        .expect("switches off");
    fire_due(&factory, &journal, &clock, &config, after(now, 21), &|_| {});

    assert!(journal.pending().expect("reads").is_empty());
    assert_eq!(
        journal.layovers().expect("reads").len(),
        1,
        "the layover still waits, to be collected once the follow-up is on"
    );
    assert!(
        journal
            .layovers()
            .expect("reads")
            .iter()
            .all(|layover| layover.standing == layover_core::layover::Standing::Booked)
    );

    let _ = std::fs::remove_dir_all(&root);
}
