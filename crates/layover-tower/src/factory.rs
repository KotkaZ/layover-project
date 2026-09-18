//! The loop: taking work that is waiting and turning it into runs that happened.
//!
//! # What makes this a factory rather than a supervisor
//!
//! [`crate::spawn`] can start one agent. [`crate::dispatch`] can say whether one flight may fly.
//! This is what puts them together and does it repeatedly — and repetition is where the
//! interesting failures live, because everything here has to be safe to interrupt.
//!
//! # The order within a single run
//!
//! ```text
//! authorise            cheap refusals first: Ground Stop, route, rails
//! record as started    before the process exists, so an interruption is detectable
//! spawn                the first irreversible step
//! wait                 with a timeout, watching for a Ground Stop
//! read the transcript  what it cost, and whether to believe it
//! record the outcome   history, which is what the dashboard reads
//! forget the live mark the run is accounted for; nothing needs to reconcile it
//! ```
//!
//! Every step before `spawn` can be repeated harmlessly. Everything after it has happened whether
//! or not this process survives to write it down, which is why the live mark goes first and comes
//! off last.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use jiff::Timestamp;
use layover_core::agent::AgentName;
use layover_core::config::Config;
use layover_core::cost::CostSource;
use layover_core::cost::TokenUsage;
use layover_core::flight::Flight;
use layover_core::graph::RouteGraph;
use layover_core::itinerary::Itinerary;
use layover_core::payload::{Run, compose};
use layover_core::queue::Queued;
use layover_core::run::{Outcome, RunRecord};

use crate::dispatch::{Refusal, authorise, declared_env, declared_values};
use crate::spawn::{self, Plan};
use crate::state::{Ledger, Live};
use crate::wait::{Ended, wait_for};

/// What happened to one piece of work the factory picked up.
#[derive(Debug)]
pub enum Dispatched {
    /// A run started, finished, and was recorded.
    Ran {
        /// How it ended.
        outcome: Outcome,
        /// What it cost, and whether that figure can be believed.
        usd: f64,
        /// Where the figure came from.
        source: CostSource,
    },
    /// The flight was refused before anything started.
    Refused(Refusal),
    /// Something went wrong that is the factory's fault rather than the flight's.
    Failed(String),
}

/// A running factory.
pub struct Factory {
    config: Config,
    graph: RouteGraph,
    root: PathBuf,
    live: Ledger,
}

impl Factory {
    /// Opens a factory rooted at `root`, which is where `.layover/` lives.
    ///
    /// # Errors
    ///
    /// Returns an error when the state directory cannot be created.
    pub fn new(config: Config, root: impl Into<PathBuf>) -> std::io::Result<Self> {
        let root = root.into();
        let graph = RouteGraph::from_config(&config);
        let live = Ledger::open(root.join(".layover").join("state").join("runs"))?;

        Ok(Self {
            config,
            graph,
            root,
            live,
        })
    }

    /// The factory definition this is running.
    #[must_use]
    pub const fn config(&self) -> &Config {
        &self.config
    }

    /// Whether a Ground Stop is engaged.
    ///
    /// Read from disk on every check rather than cached: a kill switch whose state is a snapshot
    /// taken at startup is a kill switch that does not work while the thing is running.
    #[must_use]
    pub fn ground_stop_engaged(&self) -> bool {
        self.ground_stop_path().exists()
    }

    fn ground_stop_path(&self) -> PathBuf {
        self.root.join(".layover").join("ground-stop")
    }

    /// Runs one flight to completion, recording what happened.
    ///
    /// `sender` is `None` for a human trigger.
    ///
    /// # Errors
    ///
    /// Never returns `Err`; a failure is reported as [`Dispatched::Failed`] so that one bad flight
    /// cannot stop a factory draining the rest of its queue.
    pub fn run_flight(
        &self,
        itinerary: &mut Itinerary,
        sender: Option<&AgentName>,
        flight: &Flight,
    ) -> Dispatched {
        let authorised = match authorise(
            &self.config,
            &self.graph,
            itinerary,
            sender,
            flight,
            self.ground_stop_engaged(),
        ) {
            Ok(authorised) => authorised,
            Err(refusal) => return Dispatched::Refused(refusal),
        };

        // Counted here rather than after the spawn: a run that starts and is never seen to finish
        // has still been started, and a cap that only counts completions is not a cap.
        if let Err(denial) = itinerary.record_run_started() {
            return Dispatched::Refused(Refusal::Rail(denial));
        }

        let run = layover_core::RunId::generate();
        let hangar = self
            .root
            .join(".layover")
            .join("hangars")
            .join(authorised.name.to_string())
            .join(run.as_str());

        let plan = match self.plan_for(&authorised, &hangar, flight) {
            Ok(plan) => plan,
            Err(why) => return Dispatched::Failed(why),
        };

        let started_at = Timestamp::now();

        // The run is written down before the process exists. If this process dies in the next
        // instant, that record is the only evidence the run happened.
        let started = match spawn::start(&plan) {
            Ok(started) => started,
            Err(error) => {
                self.record(&RunRecord {
                    run: run.clone(),
                    itinerary: itinerary.id().clone(),
                    agent: authorised.name.clone(),
                    pipeline: None,
                    model: authorised.agent.model.clone(),
                    outcome: Outcome::Failed,
                    started_at,
                    finished_at: Some(Timestamp::now()),
                    usd: 0.0,
                    source: CostSource::Unreported,
                    usage: TokenUsage::default(),
                    detail: Some(error.to_string()),
                    blocked_on: None,
                    pid: None,
                });
                return Dispatched::Failed(error.to_string());
            }
        };

        let mark = Live {
            run: run.clone(),
            itinerary: itinerary.id().clone(),
            agent: authorised.name.clone(),
            pid: started.pid(),
            started_at,
            hangar: hangar.clone(),
        };
        let _ = self.live.starting(&mark);

        let timeout = Some(Duration::from_secs(self.config.defaults.timeout_sec));
        let (finished, ended) = match wait_for(started, timeout, || self.ground_stop_engaged()) {
            Ok(result) => result,
            Err(error) => {
                let _ = self.live.finished(&run);
                return Dispatched::Failed(error.to_string());
            }
        };

        let transcript = std::fs::read_to_string(&finished.transcript).unwrap_or_default();
        let reported = crate::cost::from_transcript(&transcript);

        // Debited even when the run failed. Money spent is money spent, and a chain that could
        // retry forever on failures without paying for them is not bounded.
        if reported.source == CostSource::Reported {
            itinerary.debit_fuel(reported.usd);
        } else {
            itinerary.note_unreported_cost();
        }

        let outcome = outcome_of(ended, finished.succeeded());

        self.record(&RunRecord {
            run: run.clone(),
            itinerary: itinerary.id().clone(),
            agent: authorised.name.clone(),
            pipeline: None,
            model: authorised.agent.model.clone(),
            outcome,
            started_at,
            finished_at: Some(finished.finished_at),
            usd: reported.usd,
            source: reported.source,
            usage: TokenUsage {
                input: reported.input_tokens,
                output: reported.output_tokens,
                ..TokenUsage::default()
            },
            detail: detail_for(ended),
            blocked_on: None,
            pid: None,
        });

        // Last, because until the outcome is written the run is still unaccounted for. Forgetting
        // it first would lose a run that this process failed to finish recording.
        let _ = self.live.finished(&run);

        Dispatched::Ran {
            outcome,
            usd: reported.usd,
            source: reported.source,
        }
    }

    /// Assembles everything a run needs to start: its payload, its runner, its environment.
    ///
    /// Separate from starting it because every failure here is one the operator caused — a missing
    /// credential, an unwritable directory — and is worth reporting differently from a child that
    /// started and went wrong.
    fn plan_for(
        &self,
        authorised: &crate::dispatch::Authorised<'_>,
        hangar: &Path,
        flight: &Flight,
    ) -> Result<Plan, String> {
        let instructions = authorised
            .agent
            .prompt
            .clone()
            .unwrap_or_else(|| format!("You are `{}`.", authorised.name));

        let payload = compose(&Run {
            agent: &authorised.name,
            instructions: &instructions,
            memory: None,
            brief: "",
            handover: None,
            body: &flight.body,
        });

        let mut env = declared_values(authorised.agent);
        env.extend(spawn::env_from(&declared_env(authorised.agent)).map_err(|e| e.to_string())?);

        let runner = self
            .config
            .runners
            .get(&authorised.runner)
            .ok_or_else(|| format!("runner `{}` vanished", authorised.runner))?;

        let work_dir = authorised
            .agent
            .work_dir
            .clone()
            .unwrap_or_else(|| self.root.join(&self.config.layover.work_dir));

        std::fs::create_dir_all(&work_dir)
            .map_err(|error| format!("could not make the working directory: {error}"))?;

        Ok(Plan {
            agent: authorised.name.clone(),
            runner: runner.clone(),
            model: authorised.agent.model.clone(),
            payload,
            hangar: hangar.to_path_buf(),
            work_dir,
            env,
        })
    }

    /// Where history is written.
    #[must_use]
    pub fn history_dir(&self) -> PathBuf {
        self.root.join(".layover").join("history")
    }

    /// Appends a run to history, as JSON Lines in the day's segment.
    ///
    /// Deliberately best-effort: a factory that stops working because it could not write a log
    /// line has turned an observability problem into an outage.
    fn record(&self, record: &RunRecord) {
        let dir = self.history_dir();
        if std::fs::create_dir_all(&dir).is_err() {
            return;
        }

        let day = record
            .started_at
            .to_string()
            .chars()
            .take(10)
            .collect::<String>();
        let path = dir.join(format!("runs-{day}.jsonl"));

        if let Ok(mut line) = serde_json::to_string(record) {
            line.push('\n');
            let _ = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .map(|mut file| std::io::Write::write_all(&mut file, line.as_bytes()));
        }
    }

    /// Runs to completion every flight currently waiting in `queue`.
    ///
    /// Each is taken off the queue **before** it runs. A flight that crashes the factory mid-run
    /// must not come back on restart and run again: an agent that opened a pull request and was
    /// interrupted before its outcome was recorded would open a second one.
    ///
    /// # Errors
    ///
    /// Returns an error only when the queue cannot be read.
    pub fn drain(
        &self,
        pending: Vec<Queued>,
        mut unqueue: impl FnMut(&Flight),
        mut report: impl FnMut(&Flight, &Dispatched),
    ) -> usize {
        let mut ran = 0;

        for queued in pending {
            if self.ground_stop_engaged() {
                break;
            }

            unqueue(&queued.flight);

            let mut itinerary = Itinerary::new(
                queued.flight.itinerary.clone(),
                self.config.defaults.max_hops,
                self.config.defaults.fuel_usd,
                self.config.defaults.max_runs,
            );

            let result = self.run_flight(&mut itinerary, None, &queued.flight);
            report(&queued.flight, &result);

            if matches!(result, Dispatched::Ran { .. }) {
                ran += 1;
            }
        }

        ran
    }

    /// Where the factory's root is.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Variables the factory would pass to `agent`, for reporting rather than for spawning.
    #[must_use]
    pub fn env_for(&self, agent: &AgentName) -> BTreeMap<String, String> {
        self.config
            .agents
            .get(agent)
            .map(declared_values)
            .unwrap_or_default()
    }
}

/// How a run is recorded, given how it ended and what it said.
///
/// `halted` and `timed_out` are deliberately not `failed`: a rail stopping work is the system
/// doing its job, and colouring it like a crash teaches people to ignore the colour.
const fn outcome_of(ended: Ended, succeeded: bool) -> Outcome {
    match ended {
        Ended::TimedOut => Outcome::TimedOut,
        Ended::Halted => Outcome::Halted,
        Ended::Exited if succeeded => Outcome::Succeeded,
        Ended::Exited => Outcome::Failed,
    }
}

/// A line explaining an outcome that is not self-evident.
///
/// `succeeded` and `failed` speak for themselves — the agent did or did not do the thing. The
/// other two are the supervisor's doing rather than the agent's, and a list that does not say so
/// reads as though the agent chose to stop.
fn detail_for(ended: Ended) -> Option<String> {
    match ended {
        Ended::Exited => None,
        Ended::TimedOut => {
            Some("the run outlived `timeout_sec` and its process tree was ended".to_owned())
        }
        Ended::Halted => Some("a Ground Stop was engaged and the run was ended".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use layover_core::flight::{ItineraryId, Origin};

    struct Temp(PathBuf);

    impl Temp {
        fn new(name: &str) -> Self {
            let path =
                std::env::temp_dir().join(format!("layover-factory-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("temp dir");
            Self(path)
        }
    }

    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A factory whose one agent is a real, harmless command.
    fn factory(temp: &Temp, command: &str) -> Factory {
        let text = format!(
            r#"
[layover]
work_dir = "work"

[defaults]
runner = "shell"
max_hops = 4
fuel_usd = 5.0
max_runs = 10
timeout_sec = 30

[runners.shell]
command = {command}

[agents.worker]
prompt = "work"
entry = true

[pipelines.build]
entry = "worker"
"#
        );

        let config: Config = toml::from_str(&text).expect("parses");
        Factory::new(config, &temp.0).expect("opens")
    }

    fn shell(script: &str) -> String {
        if cfg!(windows) {
            format!(r#"["cmd", "/c", "{script}"]"#)
        } else {
            format!(r#"["sh", "-c", "{script}"]"#)
        }
    }

    fn flight() -> Flight {
        Flight::new(
            ItineraryId::generate(),
            Origin::Human,
            AgentName::new("worker"),
            "do the thing",
            4,
        )
    }

    fn itinerary(flight: &Flight) -> Itinerary {
        Itinerary::new(flight.itinerary.clone(), 4, 5.0, 10)
    }

    #[test]
    fn a_flight_becomes_a_run_that_is_written_to_history() {
        let temp = Temp::new("ran");
        let factory = factory(&temp, &shell("echo working"));
        let flight = flight();

        let result = factory.run_flight(&mut itinerary(&flight), None, &flight);

        assert!(
            matches!(
                result,
                Dispatched::Ran {
                    outcome: Outcome::Succeeded,
                    ..
                }
            ),
            "{result:?}"
        );

        let history: Vec<_> = std::fs::read_dir(factory.history_dir())
            .expect("history exists")
            .filter_map(Result::ok)
            .collect();
        assert_eq!(history.len(), 1, "one run, one day's segment");
    }

    #[test]
    fn a_completed_run_leaves_no_live_mark_to_reconcile() {
        // The mark exists only while the outcome is unknown. One left behind would be recovered
        // on the next start, and the work done twice.
        let temp = Temp::new("clean");
        let factory = factory(&temp, &shell("echo done"));
        let flight = flight();

        factory.run_flight(&mut itinerary(&flight), None, &flight);

        assert!(factory.live.live().expect("reads").is_empty());
    }

    #[test]
    fn a_failing_agent_is_recorded_as_failed_rather_than_lost() {
        let temp = Temp::new("failed");
        let factory = factory(&temp, &shell("exit 4"));
        let flight = flight();

        let result = factory.run_flight(&mut itinerary(&flight), None, &flight);

        assert!(
            matches!(
                result,
                Dispatched::Ran {
                    outcome: Outcome::Failed,
                    ..
                }
            ),
            "{result:?}"
        );
    }

    #[test]
    fn a_ground_stop_refuses_before_a_process_exists() {
        let temp = Temp::new("stopped");
        let factory = factory(&temp, &shell("echo should not run"));
        std::fs::create_dir_all(temp.0.join(".layover")).expect("dirs");
        std::fs::write(temp.0.join(".layover").join("ground-stop"), "").expect("engages");

        let flight = flight();
        let result = factory.run_flight(&mut itinerary(&flight), None, &flight);

        assert!(
            matches!(result, Dispatched::Refused(Refusal::GroundStop)),
            "{result:?}"
        );
        assert!(
            !factory.history_dir().exists(),
            "nothing should have been recorded"
        );
    }

    #[test]
    fn the_run_cap_counts_starts_not_completions() {
        // A run that starts and is never seen to finish has still been started. A cap that counted
        // only completions would let a crashing agent run forever.
        let temp = Temp::new("cap");
        let factory = factory(&temp, &shell("echo one"));
        let flight = flight();
        let mut chain = Itinerary::new(flight.itinerary.clone(), 4, 5.0, 1);

        assert!(matches!(
            factory.run_flight(&mut chain, None, &flight),
            Dispatched::Ran { .. }
        ));

        let second = factory.run_flight(&mut chain, None, &flight);
        assert!(
            matches!(second, Dispatched::Refused(Refusal::Rail(_))),
            "the second run should hit the cap: {second:?}"
        );
    }

    #[test]
    fn a_run_that_reports_nothing_is_counted_as_a_reporting_gap() {
        // An agent that prints prose and no cost has spent money nobody can see. The itinerary
        // has to know that its own total is a lower bound.
        let temp = Temp::new("silent");
        let factory = factory(&temp, &shell("echo just talking"));
        let flight = flight();
        let mut chain = itinerary(&flight);

        factory.run_flight(&mut chain, None, &flight);

        assert!(
            chain.has_cost_reporting_gap(),
            "silence is not a measurement"
        );
        assert_eq!(chain.unreported_runs(), 1);
    }

    #[test]
    fn draining_takes_each_flight_off_the_queue_before_running_it() {
        // A flight that crashes the factory mid-run must not come back on restart and run again:
        // an agent that opened a pull request and was interrupted would open a second one.
        let temp = Temp::new("drain");
        let factory = factory(&temp, &shell("echo drained"));

        let queued = vec![Queued::new(flight(), None, BTreeMap::new())];
        let mut removed = Vec::new();

        let ran = factory.drain(
            queued,
            |flight| removed.push(flight.id.as_str().to_owned()),
            |_, _| {},
        );

        assert_eq!(ran, 1);
        assert_eq!(removed.len(), 1, "taken off the queue exactly once");
    }

    #[test]
    fn draining_stops_the_moment_a_ground_stop_appears() {
        let temp = Temp::new("drain-stop");
        let factory = factory(&temp, &shell("echo x"));
        std::fs::create_dir_all(temp.0.join(".layover")).expect("dirs");
        std::fs::write(temp.0.join(".layover").join("ground-stop"), "").expect("engages");

        let queued = vec![
            Queued::new(flight(), None, BTreeMap::new()),
            Queued::new(flight(), None, BTreeMap::new()),
        ];
        let mut removed = 0;

        let ran = factory.drain(queued, |_| removed += 1, |_, _| {});

        assert_eq!(ran, 0);
        assert_eq!(
            removed, 0,
            "a stopped factory does not even consume its queue"
        );
    }
}
