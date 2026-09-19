//! The loop that makes a factory lights-out: fire what is due, run what is waiting, sleep.
//!
//! # Why this lives with `serve` rather than with `run`
//!
//! `layover run` drains what is queued and stops, which is right for a command a person types. A
//! factory that only works while somebody is typing is not lights-out, and `layover autostart`
//! registers `serve` precisely so that it is not.
//!
//! So `serve` is the Tower: the dashboard, the MCP endpoint agents call back into, and this — the
//! thing that decides when work starts. They share one process because they share one factory
//! definition, one queue and one set of live tokens, and splitting them would mean keeping three
//! copies of that agreeing.
//!
//! # Why it polls rather than waits on an event
//!
//! Work arrives two ways: a schedule comes due, which this can predict, and a human triggers
//! something over the API, which it cannot. A short poll covers both with one mechanism and no
//! cross-thread signalling. The cost is a queue read every couple of seconds, which is a file
//! stat; the alternative is a channel that has to be right in the presence of a Ground Stop, a
//! restart and a dashboard in another task.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use jiff::Timestamp;
use layover_core::config::Config;
use layover_core::flight::{Flight, ItineraryId, Origin};
use layover_core::handover::{Handover, Resumption};
use layover_core::pipeline::PipelineName;
use layover_core::queue::Queued;
use layover_store::Journal;
use layover_tower::{Clock, Factory};

/// How often the queue is checked when nothing is scheduled sooner.
///
/// Short enough that work triggered from the dashboard starts while the person who triggered it is
/// still looking at the page, and long enough to be a rounding error against an agent run.
const POLL: Duration = Duration::from_secs(2);

/// A running factory loop.
pub struct Tower {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Tower {
    /// Starts the loop on its own thread.
    ///
    /// # Errors
    ///
    /// Returns an error when the thread cannot be spawned.
    pub fn start(
        factory: Factory,
        journal: Arc<Journal>,
        announce: impl Fn(String) + Send + 'static,
    ) -> std::io::Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = Arc::clone(&stop);

        let thread = std::thread::Builder::new()
            .name("layover-tower".to_owned())
            .spawn(move || run_loop(&factory, &journal, &stopping, &announce))?;

        Ok(Self {
            stop,
            thread: Some(thread),
        })
    }
}

impl Drop for Tower {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);

        // Joined rather than detached. The loop may be mid-drain with a child process running, and
        // returning before it finishes would leave that child orphaned — which is the one thing a
        // supervisor must not do.
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn run_loop(factory: &Factory, journal: &Journal, stop: &AtomicBool, announce: &impl Fn(String)) {
    let config = factory.config().clone();
    let clock = Clock::new(&config, Timestamp::now());

    while !stop.load(Ordering::Relaxed) {
        // Read from disk every pass rather than cached: a kill switch whose state was sampled at
        // startup is a kill switch that does not work while the thing is running.
        if factory.ground_stop_engaged() {
            std::thread::sleep(POLL);
            continue;
        }

        let now = Timestamp::now();
        let pending = journal.pending().unwrap_or_default();

        let due = clock.tick(&config, now, |pipeline| working(&pending, pipeline));

        for name in due.fire {
            // A resuming pipeline does not open fresh work on its tick; it goes looking for work
            // that was set down and is now due. Its schedule is how often the operator wants that
            // looked at — collecting on every poll instead would ignore what they declared.
            let fired = if config.pipelines.get(&name).is_some_and(|p| p.resumes) {
                resume_due(&config, journal, &name, now, announce)
            } else {
                trigger(&config, journal, &name).map(|flight| {
                    announce(format!("{name} is due; queued {flight}"));
                    1
                })
            };

            match fired {
                Ok(0) => {}
                Ok(count) if count > 1 => announce(format!("{name} resumed {count} layover(s)")),
                Ok(_) => {}
                Err(why) => announce(format!("{name} is due but could not be queued: {why}")),
            }
        }

        for skipped in due.skipped {
            // Said out loud, because a schedule quietly skipping every tick because its work
            // always overruns looks exactly like one that is running fine.
            announce(format!("skipped: {skipped}"));
        }

        drain(factory, journal, announce);

        // Waking for a schedule that is an hour away costs nothing and misses nothing, but it
        // does mean this thread is the one deciding how responsive a manual trigger feels.
        let nap = clock.until_next(Timestamp::now()).unwrap_or(POLL).min(POLL);
        std::thread::sleep(nap);
    }
}

/// Opens a fresh chain for each layover that has come due.
///
/// # Why a new itinerary rather than reviving the old one
///
/// The chain that booked the layover is over. Its Hops are spent, its Fuel is spent, and reviving
/// it would mean a follow-up costing the budget of the work it follows up — so the second comment
/// on a pull request would be cheaper than the first and the tenth would be free or refused,
/// depending on which rail ran out. A layover is new work about an old subject, and it is priced
/// that way.
///
/// What carries over is context, not budget: the resumed run is told which chain set this down,
/// what it was waiting for, and how many times it has looked.
fn resume_due(
    config: &Config,
    journal: &Journal,
    name: &PipelineName,
    now: Timestamp,
    announce: &impl Fn(String),
) -> Result<usize, String> {
    let pipeline = config
        .pipelines
        .get(name)
        .ok_or_else(|| format!("`{name}` is not declared"))?;

    let due = journal.due(now).map_err(|error| error.to_string())?;
    let mut resumed = 0;

    for layover in due {
        let resumption = Resumption {
            booked_by: layover.booked_by.clone(),
            waiting_for: layover.waiting_for.clone(),
            booked_at: layover.booked_at,
            checks: layover.checks,
        };

        let body = Handover::resumed(resumption, Vec::new()).brief();

        // The layover names the agent to come back to; the pipeline only says that this factory
        // collects them. Sending to the pipeline's entry agent instead would hand a follow-up to
        // whatever happens to be first in the route map.
        let flight = Flight::new(
            ItineraryId::generate(),
            Origin::Human,
            layover.agent.clone(),
            body,
            config.defaults.max_hops,
        );

        let flags = pipeline
            .flags_for_run(&std::collections::BTreeMap::new())
            .map(|flags| {
                flags
                    .iter()
                    .map(|(flag, value)| (flag.to_owned(), value))
                    .collect()
            })
            .unwrap_or_default();

        if let Err(error) = journal.queue(Queued::new(flight, Some(name.clone()), flags)) {
            announce(format!("could not resume {}: {error}", layover.id));
            continue;
        }

        // Marked resumed only after the work is queued. The other order loses the layover if the
        // queue write fails — work somebody is owed, gone with nothing to show it existed.
        let _ = journal.amend(&layover.id, layover_core::layover::Layover::resumed);
        resumed += 1;

        announce(format!(
            "resumed {} for `{}`: {}",
            layover.id, layover.agent, layover.waiting_for
        ));
    }

    Ok(resumed)
}

/// Runs whatever is waiting, if anything is.
fn drain(factory: &Factory, journal: &Journal, announce: &impl Fn(String)) {
    let waiting = journal.pending().unwrap_or_default();
    if waiting.is_empty() {
        return;
    }

    let drained = factory.drain_with(
        waiting,
        &mut |flight| {
            let _ = journal.unqueue(&flight.id);
        },
        &mut |flight, result| announce(format!("{} {result}", flight.to)),
        |_| journal.pending().unwrap_or_default(),
    );

    for given_up in &drained.abandoned {
        // Written down, not just printed. Every run in a stalled chain says `succeeded`, so read
        // back from history it is indistinguishable from one that finished. This is the only
        // record that the work never happened.
        let stall = layover_core::stall::Stall::new(
            given_up.key.itinerary.clone(),
            given_up.key.to.clone(),
            given_up.missing.clone(),
            given_up.stranded,
            jiff::Timestamp::now(),
        );

        if let Err(error) = journal.record_stall(&stall) {
            announce(format!("could not record a stall: {error}"));
        }

        announce(format!(
            "{given_up} ({} flight(s) stranded)",
            given_up.stranded
        ));
    }
}

/// Whether a pipeline's previous wave is still going.
///
/// Read off the queue rather than tracked separately, because the queue is the thing that is true
/// after a restart: a Tower that came back up with work still booked should not decide it is idle
/// simply because it has forgotten what it was doing.
fn working(pending: &[Queued], pipeline: &PipelineName) -> bool {
    pending
        .iter()
        .any(|queued| queued.pipeline.as_ref() == Some(pipeline))
}

/// Opens a chain for a scheduled pipeline.
fn trigger(config: &Config, journal: &Journal, name: &PipelineName) -> Result<String, String> {
    let pipeline = config
        .pipelines
        .get(name)
        .ok_or_else(|| format!("`{name}` is not declared"))?;

    let flags = pipeline
        .flags_for_run(&std::collections::BTreeMap::new())
        .map_err(|error| error.to_string())?;

    let flight = Flight::new(
        ItineraryId::generate(),
        // A schedule is not a person, but it is not an agent either, and `Origin` exists to answer
        // "is there an upstream edge to check?". For a clock there is not.
        Origin::Human,
        pipeline.entry.clone(),
        String::new(),
        config.defaults.max_hops,
    );
    let id = flight.id.as_str().to_owned();

    let values = flags
        .iter()
        .map(|(name, value)| (name.to_owned(), value))
        .collect();

    journal
        .queue(Queued::new(flight, Some(name.clone()), values))
        .map_err(|error| error.to_string())?;

    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use layover_core::agent::AgentName;

    fn factory_config(pipelines: &str) -> Config {
        let text = format!(
            r#"
[layover]
work_dir = "work"

[defaults]
runner = "shell"
max_hops = 4

[runners.shell]
command = ["echo"]

[agents.worker]
prompt = "work"
entry = true

{pipelines}
"#
        );

        toml::from_str(&text).expect("the fixture factory parses")
    }

    fn temp(name: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("layover-loop-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("a temporary directory");
        path
    }

    #[test]
    fn a_due_pipeline_is_queued_against_its_own_name() {
        // The pipeline has to travel with the flight, or nothing downstream can tell which
        // schedule's work this is — including the check that stops a schedule overlapping itself.
        let root = temp("trigger");
        let journal = Journal::open(root.join("journal")).expect("opens");
        let config = factory_config(
            r#"
[pipelines.sweep]
entry = "worker"
trigger = { every = "1h" }
"#,
        );

        trigger(&config, &journal, &PipelineName::new("sweep")).expect("queues");

        let pending = journal.pending().expect("readable");
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].pipeline, Some(PipelineName::new("sweep")));
        assert_eq!(pending[0].flight.to, AgentName::new("worker"));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_scheduled_flight_starts_a_chain_with_the_configured_budget() {
        let root = temp("budget");
        let journal = Journal::open(root.join("journal")).expect("opens");
        let config = factory_config(
            r#"
[pipelines.sweep]
entry = "worker"
trigger = { every = "1h" }
"#,
        );

        trigger(&config, &journal, &PipelineName::new("sweep")).expect("queues");

        let pending = journal.pending().expect("readable");
        assert_eq!(pending[0].flight.hops_remaining, 4);
        assert_eq!(
            pending[0].flight.from,
            Origin::Human,
            "a clock has no upstream edge to check"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_pipeline_with_work_still_queued_counts_as_working() {
        let queued = vec![Queued::new(
            Flight::new(
                ItineraryId::generate(),
                Origin::Human,
                AgentName::new("worker"),
                "go",
                4,
            ),
            Some(PipelineName::new("sweep")),
            std::collections::BTreeMap::new(),
        )];

        assert!(working(&queued, &PipelineName::new("sweep")));
        assert!(
            !working(&queued, &PipelineName::new("other")),
            "one pipeline's backlog must not hold up another"
        );
    }

    #[test]
    fn scheduled_flags_come_from_the_declared_defaults() {
        // A schedule cannot pass flags, so whatever the pipeline declares as default is what an
        // unattended run gets — and a prompt that reads a flag has to see the same value a person
        // would get by triggering it with nothing set.
        let root = temp("flags");
        let journal = Journal::open(root.join("journal")).expect("opens");
        let config = factory_config(
            r#"
[pipelines.sweep]
entry = "worker"
trigger = { every = "1h" }

[pipelines.sweep.flags.deep]
default = true
"#,
        );

        trigger(&config, &journal, &PipelineName::new("sweep")).expect("queues");

        let pending = journal.pending().expect("readable");
        assert_eq!(pending[0].flags.get("deep"), Some(&true));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_due_layover_is_resumed_as_a_new_chain_with_a_fresh_budget() {
        // The chain that booked it is over: its Hops and Fuel are spent. Reviving it would make
        // the second follow-up cheaper than the first and the tenth refused.
        let root = temp("resume");
        let journal = Journal::open(root.join("journal")).expect("opens");
        let config = factory_config(
            r#"
[pipelines.follow_up]
entry = "worker"
trigger = { every = "20m" }
resumes = true
"#,
        );

        let booked_by = ItineraryId::generate();
        let now = Timestamp::now();
        journal
            .book(layover_core::layover::Layover::book(
                AgentName::new("worker"),
                booked_by.clone(),
                "the review to land",
                layover_core::handover::Handover::dispatch(Vec::new()),
                now.checked_sub(jiff::SignedDuration::from_hours(2))
                    .expect("in range"),
                now.checked_sub(jiff::SignedDuration::from_hours(1))
                    .expect("in range"),
                12,
            ))
            .expect("books");

        resume_due(
            &config,
            &journal,
            &PipelineName::new("follow_up"),
            now,
            &|_| {},
        )
        .expect("collects");

        let pending = journal.pending().expect("readable");
        assert_eq!(pending.len(), 1, "the due layover became work");
        assert_ne!(
            pending[0].flight.itinerary, booked_by,
            "a resumed layover opens a new chain"
        );
        assert_eq!(pending[0].flight.hops_remaining, 4);
        assert_eq!(pending[0].pipeline, Some(PipelineName::new("follow_up")));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_resumed_run_is_told_what_it_is_coming_back_for() {
        // Without this it is a fresh run with no idea which work item it is following up, which
        // is the failure booking a layover exists to avoid.
        let root = temp("resume-body");
        let journal = Journal::open(root.join("journal")).expect("opens");
        let config = factory_config(
            r#"
[pipelines.follow_up]
entry = "worker"
resumes = true
"#,
        );

        let now = Timestamp::now();
        journal
            .book(layover_core::layover::Layover::book(
                AgentName::new("worker"),
                ItineraryId::generate(),
                "comments on pull request 41",
                layover_core::handover::Handover::dispatch(Vec::new()),
                now,
                now,
                12,
            ))
            .expect("books");

        resume_due(
            &config,
            &journal,
            &PipelineName::new("follow_up"),
            now,
            &|_| {},
        )
        .expect("collects");

        let body = journal.pending().expect("readable")[0].flight.body.clone();
        assert!(body.contains("comments on pull request 41"), "{body}");
        assert!(body.contains("set down"), "{body}");
        assert!(
            !body.contains("anywhere from nowhere to almost finished"),
            "a resumed layover left nothing half-done: {body}"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_layover_that_is_not_due_yet_is_left_alone() {
        let root = temp("resume-early");
        let journal = Journal::open(root.join("journal")).expect("opens");
        let config = factory_config(
            r#"
[pipelines.follow_up]
entry = "worker"
resumes = true
"#,
        );

        let now = Timestamp::now();
        journal
            .book(layover_core::layover::Layover::book(
                AgentName::new("worker"),
                ItineraryId::generate(),
                "later",
                layover_core::handover::Handover::dispatch(Vec::new()),
                now,
                now.checked_add(jiff::SignedDuration::from_hours(2))
                    .expect("in range"),
                12,
            ))
            .expect("books");

        resume_due(
            &config,
            &journal,
            &PipelineName::new("follow_up"),
            now,
            &|_| {},
        )
        .expect("collects");

        assert!(journal.pending().expect("readable").is_empty());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_resumed_layover_is_not_resumed_twice() {
        let root = temp("resume-once");
        let journal = Journal::open(root.join("journal")).expect("opens");
        let config = factory_config(
            r#"
[pipelines.follow_up]
entry = "worker"
resumes = true
"#,
        );

        let now = Timestamp::now();
        journal
            .book(layover_core::layover::Layover::book(
                AgentName::new("worker"),
                ItineraryId::generate(),
                "once",
                layover_core::handover::Handover::dispatch(Vec::new()),
                now,
                now,
                12,
            ))
            .expect("books");

        resume_due(
            &config,
            &journal,
            &PipelineName::new("follow_up"),
            now,
            &|_| {},
        )
        .expect("collects");
        resume_due(
            &config,
            &journal,
            &PipelineName::new("follow_up"),
            now,
            &|_| {},
        )
        .expect("collects");

        assert_eq!(
            journal.pending().expect("readable").len(),
            1,
            "a layover picked up twice is one follow-up done twice"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_ordinary_schedule_never_collects_booked_work() {
        // Only a pipeline that declares `resumes` goes looking. Otherwise a factory's hourly
        // sweep would quietly start following up other people's work.
        let root = temp("resume-none");
        let journal = Journal::open(root.join("journal")).expect("opens");
        let config = factory_config(
            r#"
[pipelines.build]
entry = "worker"
trigger = { every = "1h" }
"#,
        );

        let now = Timestamp::now();
        journal
            .book(layover_core::layover::Layover::book(
                AgentName::new("worker"),
                ItineraryId::generate(),
                "nobody collects this",
                layover_core::handover::Handover::dispatch(Vec::new()),
                now,
                now,
                12,
            ))
            .expect("books");

        // What the loop does for a pipeline that does not resume.
        trigger(&config, &journal, &PipelineName::new("build")).expect("queues fresh work");

        let pending = journal.pending().expect("readable");
        assert_eq!(pending.len(), 1);
        assert!(
            !pending[0].flight.body.contains("set down"),
            "an ordinary tick opens fresh work, not a follow-up: {}",
            pending[0].flight.body
        );
        assert_eq!(
            journal.due(now).expect("readable").len(),
            1,
            "the layover is still owed, and still visible"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_loop_stops_when_it_is_told_to() {
        // A supervisor that cannot be shut down is a supervisor that has to be killed, which
        // orphans whatever it was running.
        let root = temp("stop");
        let journal = Arc::new(Journal::open(root.join("journal")).expect("opens"));
        let config = factory_config(
            r#"
[pipelines.sweep]
entry = "worker"
"#,
        );
        let factory = Factory::new(config, &root).expect("opens");

        let tower = Tower::start(factory, journal, |_| {}).expect("starts");
        std::thread::sleep(Duration::from_millis(50));
        drop(tower);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_ground_stop_keeps_the_loop_from_starting_anything() {
        let root = temp("groundstop");
        let journal = Arc::new(Journal::open(root.join("journal")).expect("opens"));
        std::fs::create_dir_all(root.join(".layover")).expect("dirs");
        std::fs::write(root.join(".layover").join("ground-stop"), "").expect("engages");

        let config = factory_config(
            r#"
[pipelines.sweep]
entry = "worker"
"#,
        );

        // Work is waiting, and must stay waiting.
        journal
            .queue(Queued::new(
                Flight::new(
                    ItineraryId::generate(),
                    Origin::Human,
                    AgentName::new("worker"),
                    "go",
                    4,
                ),
                None,
                std::collections::BTreeMap::new(),
            ))
            .expect("queues");

        let factory = Factory::new(config, &root).expect("opens");
        let tower = Tower::start(factory, Arc::clone(&journal), |_| {}).expect("starts");
        std::thread::sleep(Duration::from_millis(200));
        drop(tower);

        assert_eq!(
            journal.pending().expect("readable").len(),
            1,
            "a stopped factory does not even consume its queue"
        );

        let _ = std::fs::remove_dir_all(&root);
    }
}
