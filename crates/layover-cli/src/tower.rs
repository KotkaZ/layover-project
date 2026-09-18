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
            match trigger(&config, journal, &name) {
                Ok(flight) => announce(format!("{name} is due; queued {flight}")),
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
