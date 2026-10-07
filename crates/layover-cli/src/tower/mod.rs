//! The loop that makes a factory lights-out: fire what is due, run what is waiting, keep going.
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
//! # One loop, several runs
//!
//! Runs are waited for on threads of their own, up to `max_concurrent_runs` at once, and this loop
//! is what starts them — see [`Factory::dispatch`]. The clock is ticked here as well, between one
//! step of dispatching and the next, rather than on a thread of its own.
//!
//! That is deliberate. A schedule skips a tick while its previous wave is queued *or* running, and
//! a flight moves from the one to the other as it starts. A clock on another thread could look at
//! both in between and see neither, and start a second copy of work already under way. Ticked
//! here, nothing moves while it looks. The dispatcher looks for new work four times a second while
//! runs are alive, so a schedule still fires on time during an hour-long run.
//!
//! # Why it polls rather than waits on an event
//!
//! Work arrives two ways: a schedule comes due, which this can predict, and a human triggers
//! something over the API, which it cannot. A short poll covers both with one mechanism and no
//! cross-thread signalling. The cost is a queue read every fraction of a second while runs are
//! alive, which is a file read; the alternative is a channel that has to be right in the presence
//! of a Ground Stop, a restart and a dashboard in another task.

mod resume;
mod timetable;

use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use jiff::Timestamp;
use layover_core::config::Config;
use layover_core::flight::{Flight, ItineraryId, Origin};
use layover_core::pipeline::PipelineName;
use layover_core::queue::Queued;
use layover_store::Journal;
use layover_tower::{Clock, Factory};

/// How long the loop rests when nothing is running and nothing is scheduled sooner.
///
/// Short enough that work triggered from the dashboard starts while the person who triggered it is
/// still looking at the page, and long enough to be a rounding error against an agent run.
const POLL: Duration = Duration::from_secs(2);

/// Where the Tower reads the time: the real clock, except in a test that needs a schedule to come
/// due faster than its sixty-second floor allows.
type Now = Arc<dyn Fn() -> Timestamp + Send + Sync>;

/// A running factory loop.
pub struct Tower {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
    timetable: Arc<timetable::TowerTimetable>,
}

impl Tower {
    /// Settles whatever the last Tower left running, then starts the loop on its own thread.
    ///
    /// # Errors
    ///
    /// Returns an error when the thread cannot be spawned.
    pub fn start(
        factory: Factory,
        journal: Arc<Journal>,
        announce: impl Fn(String) + Send + 'static,
    ) -> std::io::Result<Self> {
        Self::start_at(factory, journal, announce, Arc::new(Timestamp::now))
    }

    /// The Tower's clock and what it counts as still working, for the dashboard in this process.
    pub fn timetable(&self) -> Arc<dyn layover_dashboard::Timetable> {
        Arc::clone(&self.timetable) as Arc<dyn layover_dashboard::Timetable>
    }

    fn start_at(
        factory: Factory,
        journal: Arc<Journal>,
        announce: impl Fn(String) + Send + 'static,
        now: Now,
    ) -> std::io::Result<Self> {
        // Before anything starts. A run the last Tower was watching when it went away is either
        // still going — cut off from Layover, spending on work nobody will receive — or gone
        // without a word in history. Either way the queue is not the whole truth until it is
        // settled, and a restart it earns has to be queued before the first dispatch reads it.
        for settled in factory
            .reconcile(&mut |restart| journal.queue(restart).map_err(|error| error.to_string()))
        {
            announce(settled.to_string());
        }

        // Shared rather than owned by the loop, so the dashboard can ask when each schedule next
        // fires — the one thing about a schedule only the Tower keeping it knows.
        let factory = Arc::new(factory);
        let clock = Arc::new(Clock::new(factory.config(), now()));
        let timetable = Arc::new(timetable::TowerTimetable::new(
            Arc::clone(&clock),
            Arc::clone(&factory),
            Arc::clone(&journal),
        ));

        let stop = Arc::new(AtomicBool::new(false));
        let stopping = Arc::clone(&stop);

        let thread = std::thread::Builder::new()
            .name("layover-tower".to_owned())
            .spawn(move || run_loop(&factory, &clock, &journal, &stopping, &announce, &*now))?;

        Ok(Self {
            stop,
            thread: Some(thread),
            timetable,
        })
    }
}

impl Drop for Tower {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);

        // Joined rather than detached. Runs may be alive, and returning before they finish would
        // leave them orphaned — which is the one thing a supervisor must not do. Nothing new
        // starts once stopping; what is running is waited for.
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn run_loop(
    factory: &Factory,
    clock: &Clock,
    journal: &Journal,
    stop: &AtomicBool,
    announce: &dyn Fn(String),
    now: &dyn Fn() -> Timestamp,
) {
    let config = factory.config().clone();

    while !stop.load(Ordering::Relaxed) {
        // Read from disk every pass rather than cached: a kill switch whose state was sampled at
        // startup is a kill switch that does not work while the thing is running.
        if factory.ground_stop_engaged() {
            nap(stop, POLL);
            continue;
        }

        let tick = || fire_due(factory, journal, clock, &config, now(), announce);
        tick();
        dispatch(factory, journal, stop, announce, &tick);

        // Waking for a schedule that is an hour away costs nothing and misses nothing, but it
        // does mean this thread is the one deciding how responsive a manual trigger feels.
        let rest = clock.until_next(now()).unwrap_or(POLL).min(POLL);
        nap(stop, rest);
    }
}

/// Queues whatever the clock says is due.
fn fire_due(
    factory: &Factory,
    journal: &Journal,
    clock: &Clock,
    config: &Config,
    now: Timestamp,
    announce: &dyn Fn(String),
) {
    // What is running, then what is queued. A run that finishes between the two reads queued
    // whatever it sent before it finished, and nothing moves a flight from the queue to a slot
    // while this looks, because the same thread does both.
    let busy = factory.busy_pipelines();
    let pending = journal.pending().unwrap_or_default();

    let due = clock.tick(config, now, |pipeline| working(&pending, &busy, pipeline));

    for name in due.fire {
        // A resuming pipeline does not open fresh work on its tick; it goes looking for work
        // that was set down and is now due. Its schedule is how often the operator wants that
        // looked at — collecting on every poll instead would ignore what they declared.
        let fired = if config.pipelines.get(&name).is_some_and(|p| p.resumes) {
            resume::resume_due(config, journal, &name, now, announce)
        } else {
            trigger(config, journal, &name).map(|flight| {
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
        // always overruns looks exactly like one that is running fine. Written down too, because
        // the console is the one place nobody looks a day later.
        if let Err(error) = journal.record_skip(&skipped.record(now)) {
            announce(format!("could not record a skipped tick: {error}"));
        }
        announce(format!("skipped: {skipped}"));
    }
}

/// Runs whatever is waiting, up to the factory's limit at once, until nothing is running and
/// nothing is left to start — ticking the clock whenever it looks for more work.
fn dispatch(
    factory: &Factory,
    journal: &Journal,
    stop: &AtomicBool,
    announce: &dyn Fn(String),
    tick: &dyn Fn(),
) {
    let drained = factory.dispatch(
        journal.pending().unwrap_or_default(),
        &mut |flight| match journal.unqueue(&flight.id) {
            // Not there any more: somebody cancelled it a moment ago.
            Ok(was_queued) => was_queued,
            // Not run. A flight that could not be taken off the queue would still be there after
            // a restart and run a second time; left queued, it is tried again on the next pass.
            Err(error) => {
                announce(format!(
                    "could not take {} off the queue, so it waits: {error}",
                    flight.id
                ));
                false
            }
        },
        &mut |flight, result| announce(format!("{} {result}", flight.to)),
        |_| {
            tick();
            journal.pending().unwrap_or_default()
        },
        &|| stop.load(Ordering::Relaxed),
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
            Timestamp::now(),
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

/// Sleeps for `duration`, or until told to stop.
fn nap(stop: &AtomicBool, duration: Duration) {
    let until = Instant::now() + duration;
    while !stop.load(Ordering::Relaxed) {
        let left = until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return;
        }
        std::thread::sleep(left.min(Duration::from_millis(100)));
    }
}

/// Whether a pipeline's previous wave is still going: a flight of it queued, or a run of it alive.
///
/// Both, because a flight leaves the queue the moment it starts: read off the queue alone, an
/// hour-long run looks idle for the whole hour. The queue is still half the answer because it is
/// what is true after a restart — a Tower that came back up with work still booked should not
/// decide it is idle simply because it has forgotten what it was doing.
fn working(pending: &[Queued], busy: &BTreeSet<PipelineName>, pipeline: &PipelineName) -> bool {
    busy.contains(pipeline)
        || pending
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
        // A clock is not a person, and the run it wakes is told so: nobody is watching.
        Origin::Schedule(name.clone()),
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
pub(super) mod fixture {
    use layover_core::config::Config;

    pub fn factory_config(pipelines: &str) -> Config {
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

    pub fn temp(name: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("layover-loop-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("a temporary directory");
        path
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fixture::{factory_config, temp};
    use layover_core::agent::AgentName;

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
            Origin::Schedule(PipelineName::new("sweep")),
            "a run woken by a clock is told so"
        );
        assert!(
            pending[0].flight.from.agent().is_none(),
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

        assert!(working(
            &queued,
            &BTreeSet::new(),
            &PipelineName::new("sweep")
        ));
        assert!(
            !working(&queued, &BTreeSet::new(), &PipelineName::new("other")),
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

    #[test]
    fn a_pipeline_with_a_run_alive_counts_as_working() {
        // An hour-long run has left the queue for the whole hour. Read off the queue alone, the
        // schedule would start a second copy on every tick of it.
        let busy = BTreeSet::from([PipelineName::new("sweep")]);

        assert!(working(&[], &busy, &PipelineName::new("sweep")));
        assert!(!working(&[], &busy, &PipelineName::new("other")));
    }

    /// A runner command that takes about `seconds` and exits cleanly.
    fn sleeps(seconds: u32) -> String {
        if cfg!(windows) {
            format!(r#"["cmd", "/c", "ping -n {} 127.0.0.1 >nul"]"#, seconds + 1)
        } else {
            format!(r#"["sh", "-c", "sleep {seconds}"]"#)
        }
    }

    fn history(root: &std::path::Path) -> Vec<layover_core::run::RunRecord> {
        let Ok(history) = layover_store::History::open(root.join(".layover").join("history"))
        else {
            return Vec::new();
        };
        let span = history.resolve(layover_core::cost::Window::AllTime);
        let mut found = history
            .runs(&span, &layover_store::RunFilter::default())
            .unwrap_or_default();
        found.sort_by_key(|run| run.started_at);
        found
    }

    fn wait_until(what: &str, done: impl Fn() -> bool) {
        let began = Instant::now();
        while !done() {
            assert!(
                began.elapsed() < Duration::from_secs(40),
                "{what} never happened"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    #[test]
    fn a_schedule_fires_on_time_while_a_long_run_is_alive_and_never_overlaps_its_own_wave() {
        // The 14:00 tick that fired at 14:23, when the run before it ended. Time runs sixty times
        // faster here, so the one-minute schedule is due every second of a nine-second build.
        let root = temp("clock-during-run");
        let config: Config = toml::from_str(&format!(
            r#"
[layover]
work_dir = "work"

[defaults]
max_concurrent_runs = 3
timeout_sec = 60

[runners.slow]
command = {}

[runners.quick]
command = {}

[agents.builder]
prompt = "build"
runner = "slow"
entry = true

[agents.poller]
prompt = "poll"
runner = "quick"

[pipelines.build]
entry = "builder"

[pipelines.poll]
entry = "poller"
trigger = {{ every = "1m" }}
"#,
            sleeps(9),
            sleeps(2)
        ))
        .expect("parses");

        let journal =
            Arc::new(Journal::open(root.join(".layover").join("journal")).expect("opens"));
        journal
            .queue(Queued::new(
                Flight::new(
                    ItineraryId::generate(),
                    Origin::Human,
                    AgentName::new("builder"),
                    "build it",
                    4,
                ),
                Some(PipelineName::new("build")),
                std::collections::BTreeMap::new(),
            ))
            .expect("queues");

        let said: Arc<std::sync::Mutex<Vec<String>>> = Arc::default();
        let saying = Arc::clone(&said);
        let began = Timestamp::now();
        let real = Instant::now();
        let fast: Now = Arc::new(move || {
            let elapsed = i64::try_from(real.elapsed().as_millis()).unwrap_or(i64::MAX);
            began + jiff::SignedDuration::from_millis(elapsed.saturating_mul(60))
        });

        let factory = Factory::new(config, &root).expect("opens");
        let tower = Tower::start_at(
            factory,
            journal,
            move |line| saying.lock().expect("lock").push(line),
            fast,
        )
        .expect("starts");
        wait_until("the build", || {
            history(&root)
                .iter()
                .any(|run| run.agent.as_str() == "builder")
        });
        drop(tower);

        let runs = history(&root);
        let build = runs
            .iter()
            .find(|run| run.agent.as_str() == "builder")
            .expect("built");
        let polls: Vec<_> = runs
            .iter()
            .filter(|run| run.agent.as_str() == "poller")
            .collect();
        let finished = build.finished_at.expect("finished");
        assert!(
            polls
                .iter()
                .filter(|poll| poll.started_at < finished)
                .count()
                >= 2,
            "the schedule fired while the build was running: {runs:?}"
        );
        for (earlier, later) in polls.iter().zip(polls.iter().skip(1)) {
            assert!(
                earlier
                    .finished_at
                    .is_some_and(|end| end <= later.started_at),
                "a poll started while the last was still running: {polls:?}"
            );
        }
        let said = said.lock().expect("lock");
        assert!(
            said.iter().any(|line| line.starts_with("skipped: ")),
            "a tick that found its wave running was skipped, and said so: {said:?}"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_tower_restarts_work_the_last_one_left_running() {
        let root = temp("restart");
        let quick = if cfg!(windows) {
            r#"["cmd", "/c", "echo done"]"#
        } else {
            r#"["sh", "-c", "echo done"]"#
        };
        let config: Config = toml::from_str(&format!(
            r#"
[layover]
work_dir = "work"

[defaults]
runner = "shell"
timeout_sec = 30

[runners.shell]
command = {quick}

[agents.worker]
prompt = "work"
entry = true
"#
        ))
        .expect("parses");

        // The record a Tower killed mid-run leaves behind, for a process that has since exited.
        let mut exited = if cfg!(windows) {
            std::process::Command::new("cmd")
                .args(["/c", "exit 0"])
                .spawn()
        } else {
            std::process::Command::new("sh")
                .args(["-c", "true"])
                .spawn()
        }
        .expect("spawns");
        let pid = exited.id();
        let _ = exited.wait();

        let left = layover_tower::Live {
            run: layover_core::RunId::generate(),
            itinerary: ItineraryId::generate(),
            agent: AgentName::new("worker"),
            pid,
            started_at: Timestamp::now(),
            hangar: root
                .join(".layover")
                .join("hangars")
                .join("worker")
                .join("old"),
            queued: None,
            owner: Some("tower-killed-mid-run".to_owned()),
            sent_by: None,
        };
        let left = layover_tower::Live {
            queued: Some(Queued::new(
                Flight::new(
                    left.itinerary.clone(),
                    Origin::Human,
                    AgentName::new("worker"),
                    "finish the report",
                    4,
                ),
                None,
                std::collections::BTreeMap::new(),
            )),
            ..left
        };
        let ledger = layover_tower::Ledger::open(root.join(".layover").join("state").join("runs"))
            .expect("ledger");
        ledger.starting(&left).expect("records");

        let said: Arc<std::sync::Mutex<Vec<String>>> = Arc::default();
        let saying = Arc::clone(&said);
        let journal =
            Arc::new(Journal::open(root.join(".layover").join("journal")).expect("opens"));
        let factory = Factory::new(config, &root).expect("opens");
        let tower = Tower::start(factory, journal, move |line| {
            saying.lock().expect("lock").push(line);
        })
        .expect("starts");
        wait_until("the restart", || history(&root).len() >= 2);
        drop(tower);

        let runs = history(&root);
        let outcomes: Vec<_> = runs.iter().map(|run| run.outcome).collect();
        assert_eq!(
            outcomes,
            vec![
                layover_core::run::Outcome::Interrupted,
                layover_core::run::Outcome::Succeeded
            ],
            "{runs:?}"
        );
        assert_eq!(runs[0].run, left.run, "the old run is in history");
        assert!(
            ledger.live().expect("reads").is_empty(),
            "and nothing is left behind"
        );
        assert!(
            said.lock()
                .expect("lock")
                .iter()
                .any(|line| line.contains("restarted as attempt 2")),
            "{said:?}"
        );

        let _ = std::fs::remove_dir_all(&root);
    }
}
