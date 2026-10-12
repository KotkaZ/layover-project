//! A resuming pipeline needs no `entry`: its tick wakes whoever set the work down.

use jiff::{SignedDuration, Timestamp};
use layover_core::agent::AgentName;
use layover_core::flight::ItineraryId;
use layover_core::pipeline::PipelineName;
use layover_store::Journal;
use layover_tower::{Clock, Factory};

use super::fire_due;
use super::fixture::{factory_config, temp};

#[test]
fn a_resuming_pipeline_with_no_entry_collects_what_is_due_for_whoever_set_it_down() {
    let root = temp("entryless-follow-up");
    let config = factory_config(
        r#"
[pipelines.follow_up]
trigger = { every = "20m" }
resumes = true
"#,
    );
    assert_eq!(
        config.pipelines[&PipelineName::new("follow_up")].entry,
        None
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
            now - SignedDuration::from_hours(2),
            now - SignedDuration::from_hours(1),
        ))
        .expect("books");
    let clock = Clock::new(&config, now);

    fire_due(
        &factory,
        &journal,
        &clock,
        &config,
        now + SignedDuration::from_mins(21),
        &|_| {},
    );

    let pending = journal.pending().expect("reads");
    assert_eq!(pending.len(), 1, "the due layover became work");
    assert_eq!(pending[0].flight.to, AgentName::new("worker"));
    assert_eq!(pending[0].pipeline, Some(PipelineName::new("follow_up")));

    let _ = std::fs::remove_dir_all(&root);
}
