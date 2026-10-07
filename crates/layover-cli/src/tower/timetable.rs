//! The Tower's clock, as the dashboard in the same process reads it.
//!
//! An `every` schedule is counted from when this Tower started, and a tick held by a Ground Stop
//! fires when it is released, so when a schedule next fires is something only the Tower keeping it
//! knows. The dashboard asks it here rather than working out a second answer that looks exact and
//! is not.

use std::collections::BTreeSet;
use std::sync::Arc;

use jiff::Timestamp;
use layover_core::pipeline::PipelineName;
use layover_store::Journal;
use layover_tower::{Clock, Factory};

/// What `GET /upcoming` asks of the Tower.
pub(crate) struct TowerTimetable {
    clock: Arc<Clock>,
    factory: Arc<Factory>,
    journal: Arc<Journal>,
}

impl TowerTimetable {
    pub(crate) fn new(clock: Arc<Clock>, factory: Arc<Factory>, journal: Arc<Journal>) -> Self {
        Self {
            clock,
            factory,
            journal,
        }
    }
}

impl std::fmt::Debug for TowerTimetable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TowerTimetable")
            .field("clock", &self.clock)
            .finish_non_exhaustive()
    }
}

impl layover_dashboard::Timetable for TowerTimetable {
    fn next_due(&self, pipeline: &PipelineName) -> Option<Timestamp> {
        self.clock.next_due(pipeline)
    }

    fn working(&self) -> BTreeSet<PipelineName> {
        // The question the clock asks before it fires a tick, asked of every pipeline at once, so
        // the dashboard's "may be skipped" and the Tower's skip are the same decision.
        let pending = self.journal.pending().unwrap_or_default();
        let busy = self.factory.busy_pipelines();

        self.factory
            .config()
            .pipelines
            .keys()
            .filter(|pipeline| super::working(&pending, &busy, pipeline))
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::time::Duration;

    use jiff::SignedDuration;
    use layover_core::agent::AgentName;
    use layover_core::cost::Window;
    use layover_core::flight::{Flight, ItineraryId, Origin};
    use layover_core::queue::Queued;

    use super::super::fixture::{factory_config, temp};
    use super::super::{Tower, fire_due};
    use super::*;

    const SWEEP: &str = r#"
[pipelines.sweep]
entry = "worker"
trigger = { every = "1h" }

[pipelines.by_hand]
entry = "worker"
"#;

    fn sweep_flight() -> Queued {
        Queued::new(
            Flight::new(
                ItineraryId::generate(),
                Origin::Human,
                AgentName::new("worker"),
                "the last wave",
                4,
            ),
            Some(PipelineName::new("sweep")),
            BTreeMap::new(),
        )
    }

    #[test]
    fn a_skipped_tick_is_written_down_as_well_as_said() {
        // It used to be printed to the console and nowhere else, so a schedule that skipped every
        // tick looked, a day later, like one that had simply run less.
        let root = temp("skip-recorded");
        let journal = Journal::open(root.join("journal")).expect("opens");
        let config = factory_config(SWEEP);
        let factory = Factory::new(config.clone(), &root).expect("opens");
        journal.queue(sweep_flight()).expect("queues");

        let start = Timestamp::now();
        let clock = Clock::new(&config, start);
        let later = start + SignedDuration::from_mins(61);
        let said = std::cell::RefCell::new(Vec::new());
        fire_due(&factory, &journal, &clock, &config, later, &|line| {
            said.borrow_mut().push(line);
        });

        let span = Window::AllTime
            .resolve(&(later + SignedDuration::from_mins(1)).to_zoned(jiff::tz::TimeZone::UTC));
        let skips = journal.skips(&span).expect("reads");
        assert_eq!(skips.len(), 1, "{:?}", said.borrow());
        assert_eq!(skips[0].pipeline, PipelineName::new("sweep"));
        assert_eq!(skips[0].at, later);
        assert!(
            said.borrow()
                .iter()
                .any(|line| line.starts_with("skipped: ")),
            "still said: {:?}",
            said.borrow()
        );
        assert_eq!(
            journal.pending().expect("reads").len(),
            1,
            "and nothing second was queued"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_dashboard_is_told_the_towers_own_clock_and_what_is_still_working() {
        let root = temp("timetable");
        let journal = Arc::new(Journal::open(root.join("journal")).expect("opens"));
        // Engaged, so the loop leaves the queued flight where it is for the whole test.
        std::fs::create_dir_all(root.join(".layover")).expect("dirs");
        std::fs::write(root.join(".layover").join("ground-stop"), "").expect("engages");
        journal.queue(sweep_flight()).expect("queues");

        let started = Timestamp::now();
        let factory = Factory::new(factory_config(SWEEP), &root).expect("opens");
        let tower = Tower::start(factory, Arc::clone(&journal), |_| {}).expect("starts");
        std::thread::sleep(Duration::from_millis(50));
        let timetable = tower.timetable();

        let next = timetable
            .next_due(&PipelineName::new("sweep"))
            .expect("scheduled");
        assert!(
            next >= started + SignedDuration::from_hours(1)
                && next <= Timestamp::now() + SignedDuration::from_hours(1),
            "an hour after the Tower started: {next}"
        );
        assert_eq!(timetable.next_due(&PipelineName::new("by_hand")), None);
        assert_eq!(
            timetable.working(),
            BTreeSet::from([PipelineName::new("sweep")]),
            "its last wave is still queued"
        );

        drop(tower);
        let _ = std::fs::remove_dir_all(&root);
    }
}
