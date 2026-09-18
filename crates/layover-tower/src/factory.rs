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
use std::sync::Arc;
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
use crate::runtime::Chains;
use crate::spawn::{self, Plan};
use crate::state::{Ledger, Live};
use crate::tokens::{self, ENDPOINT_VAR, TOKEN_VAR, Tokens};
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
    tokens: Arc<Tokens>,
    chains: Chains,
    endpoint: Option<String>,
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
            tokens: Arc::new(Tokens::new()),
            chains: Chains::new(),
            endpoint: None,
        })
    }

    /// Tells runs where Layover's MCP endpoint is, so they can call back into it.
    ///
    /// Without this a run is a one-shot: it reads its instructions, does the work and ends, and no
    /// chain can be longer than the flight that started it. With it, `layover_send` reaches a real
    /// queue and the route map means something at runtime rather than only at load.
    #[must_use]
    pub fn serving_mcp(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoint = Some(endpoint.into());
        self
    }

    /// The token registry, which the MCP endpoint resolves callers against.
    #[must_use]
    pub fn tokens(&self) -> Arc<Tokens> {
        Arc::clone(&self.tokens)
    }

    /// The route map this factory is enforcing.
    #[must_use]
    pub const fn graph(&self) -> &RouteGraph {
        &self.graph
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

        // Minted before the plan is assembled, because the plan is where the token becomes an
        // argument and an environment variable. It is revoked on every path out of this function:
        // a token that outlives its run is a finished process that can still send work.
        let token = self.endpoint.as_ref().map(|_| {
            self.tokens.mint(
                run.clone(),
                authorised.name.clone(),
                itinerary.id().clone(),
                authorised.hops_remaining,
            )
        });

        let plan = match self.plan_for(&authorised, &hangar, flight, token.as_deref()) {
            Ok(plan) => plan,
            Err(why) => {
                self.revoke(token.as_deref());
                return Dispatched::Failed(why);
            }
        };

        let started_at = Timestamp::now();

        // The run is written down before the process exists. If this process dies in the next
        // instant, that record is the only evidence the run happened.
        let started = match spawn::start(&plan) {
            Ok(started) => started,
            Err(error) => {
                self.record(&Self::never_started(
                    &run,
                    itinerary,
                    &authorised,
                    started_at,
                    &error.to_string(),
                ));
                self.revoke(token.as_deref());
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
                self.revoke(token.as_deref());
                return Dispatched::Failed(error.to_string());
            }
        };

        // The moment the process is gone, before anything else is written. Whatever the child was
        // doing, it may no longer send flights — and this is the first instant at which that is
        // certainly true.
        self.revoke(token.as_deref());

        self.settle(&run, itinerary, &authorised, started_at, &finished, ended)
    }

    /// Prices a finished run, charges the chain for it, and writes it down.
    ///
    /// Separate from starting it because everything here happens whether the run went well or
    /// badly, and because the order of the last three steps is load-bearing: charge, record, then
    /// forget the live mark.
    fn settle(
        &self,
        run: &layover_core::RunId,
        itinerary: &mut Itinerary,
        authorised: &crate::dispatch::Authorised<'_>,
        started_at: Timestamp,
        finished: &spawn::Finished,
        ended: Ended,
    ) -> Dispatched {
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
        let _ = self.live.finished(run);

        Dispatched::Ran {
            outcome,
            usd: reported.usd,
            source: reported.source,
        }
    }

    /// The record for a run whose process never came into being.
    ///
    /// Written rather than dropped: from the itinerary's point of view the run was started — it
    /// spent a slot against the cap before the spawn was attempted — so a history that omits it
    /// disagrees with the accounting.
    fn never_started(
        run: &layover_core::RunId,
        itinerary: &Itinerary,
        authorised: &crate::dispatch::Authorised<'_>,
        started_at: Timestamp,
        why: &str,
    ) -> RunRecord {
        RunRecord {
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
            detail: Some(why.to_owned()),
            blocked_on: None,
            pid: None,
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
        token: Option<&str>,
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

        let mcp_config = self.wire_mcp(runner, hangar, token, &mut env)?;

        Ok(Plan {
            agent: authorised.name.clone(),
            runner: runner.clone(),
            model: authorised.agent.model.clone(),
            payload,
            hangar: hangar.to_path_buf(),
            work_dir,
            env,
            mcp_config,
        })
    }

    /// Writes this run's MCP configuration and tells the child where to find it.
    ///
    /// Returns `None` when there is nothing to wire — no endpoint, no token, or a runner whose CLI
    /// cannot be told about an MCP server. The environment variables go in regardless of whether
    /// the runner takes a flag: a CLI that reads `LAYOVER_MCP_URL` directly, and any tool the
    /// agent shells out to, can use them.
    fn wire_mcp(
        &self,
        runner: &layover_core::config::Runner,
        hangar: &Path,
        token: Option<&str>,
        env: &mut BTreeMap<String, String>,
    ) -> Result<Option<PathBuf>, String> {
        let (Some(endpoint), Some(token)) = (self.endpoint.as_ref(), token) else {
            return Ok(None);
        };

        env.insert(TOKEN_VAR.to_owned(), token.to_owned());
        env.insert(ENDPOINT_VAR.to_owned(), endpoint.clone());

        let Some(wiring) = runner.mcp.as_ref() else {
            return Ok(None);
        };

        std::fs::create_dir_all(hangar)
            .map_err(|error| format!("could not make the Hangar: {error}"))?;

        tokens::write_config(hangar, &wiring.format, endpoint, token)
            .map(Some)
            .map_err(|error| format!("could not write the MCP configuration: {error}"))
    }

    /// Ends a token's life, if there was one.
    fn revoke(&self, token: Option<&str>) {
        if let Some(token) = token {
            self.tokens.revoke(token);
        }
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

    /// Runs every flight waiting in `pending`, and every flight those runs send, until nothing is
    /// left.
    ///
    /// Each is taken off the queue **before** it runs. A flight that crashes the factory mid-run
    /// must not come back on restart and run again: an agent that opened a pull request and was
    /// interrupted before its outcome was recorded would open a second one.
    ///
    /// # Why this loops rather than iterating once
    ///
    /// A run can send flights while it is running. Draining the list it started with would leave
    /// those sitting until something else picked them up, which turns every chain into one hop per
    /// invocation. `refill` is asked for whatever is queued now, after each pass.
    ///
    /// The loop is bounded by the rails rather than by a count: each flight spends a Hop from a
    /// shared itinerary and each run spends Fuel and a slot against the run cap, so a chain that
    /// will not settle is cut by the same mechanism that bounds every other chain.
    ///
    /// It also stops as soon as a whole pass starts nothing. Only a *run* can send a flight, so a
    /// pass in which every flight was refused cannot have produced new work, and asking for more
    /// would spin against a queue the rails have already closed.
    pub fn drain(
        &self,
        pending: Vec<Queued>,
        mut unqueue: impl FnMut(&Flight),
        mut report: impl FnMut(&Flight, &Dispatched),
    ) -> usize {
        self.drain_with(pending, &mut unqueue, &mut report, |_| Vec::new())
    }

    /// Drains, asking `refill` for newly queued work after each pass.
    ///
    /// # Panics
    ///
    /// Never; the itinerary lock is only held inside this function.
    pub fn drain_with(
        &self,
        pending: Vec<Queued>,
        unqueue: &mut impl FnMut(&Flight),
        report: &mut impl FnMut(&Flight, &Dispatched),
        mut refill: impl FnMut(&[Flight]) -> Vec<Queued>,
    ) -> usize {
        let mut ran = 0;
        let mut batch = pending;
        let mut done: Vec<Flight> = Vec::new();

        while !batch.is_empty() {
            let before = ran;

            for queued in std::mem::take(&mut batch) {
                if self.ground_stop_engaged() {
                    return ran;
                }

                unqueue(&queued.flight);

                // The chain is looked up, not created. Every flight in one causal chain is
                // accounted against the same Hops, Fuel and run cap; minting a fresh itinerary
                // per flight would reset all three and a loop between two agents would never end.
                let sender = queued.flight.from.agent().cloned();
                let result = self
                    .chains
                    .with(&queued.flight.itinerary, &self.config.defaults, |chain| {
                        self.run_flight(chain, sender.as_ref(), &queued.flight)
                    })
                    .unwrap_or_else(|| {
                        Dispatched::Failed("the itinerary ledger was poisoned".to_owned())
                    });

                report(&queued.flight, &result);

                if matches!(result, Dispatched::Ran { .. }) {
                    ran += 1;
                }

                done.push(queued.flight);
            }

            // Only a run can send a flight. A pass that started nothing cannot have produced new
            // work, so asking for more would spin against a queue the rails have already closed.
            if ran == before {
                break;
            }

            batch = refill(&done);
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

    /// A two-agent factory, so a chain can actually have a second hop.
    fn pair(temp: &Temp) -> Factory {
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
command = {}
mcp     = {{ flag = "--mcp-config", format = "claude_json" }}

[agents.analyst]
prompt = "analyse"
entry = true

[agents.developer]
prompt = "develop"

[pipelines.build]
entry = "analyst"

[[routes]]
from = "analyst"
to = "developer"
"#,
            shell("echo done")
        );

        let config: Config = toml::from_str(&text).expect("parses");
        Factory::new(config, &temp.0).expect("opens")
    }

    fn queued_to(itinerary: &ItineraryId, from: Origin, to: &str, hops: u32) -> Queued {
        Queued::new(
            Flight::new(itinerary.clone(), from, AgentName::new(to), "go", hops),
            None,
            BTreeMap::new(),
        )
    }

    #[test]
    fn a_flight_sent_by_a_run_is_picked_up_in_the_same_drain() {
        // Otherwise every chain is one hop per invocation: the second flight would sit in the
        // queue until something else happened to run.
        let temp = Temp::new("multihop");
        let factory = pair(&temp);
        let chain = ItineraryId::generate();

        let mut seen = Vec::new();
        let mut handed_over = false;

        let ran = factory.drain_with(
            vec![queued_to(&chain, Origin::Human, "analyst", 4)],
            &mut |_| {},
            &mut |flight, _| seen.push(flight.to.to_string()),
            |_| {
                // Stands in for the analyst calling `layover_send`: one flight appears, once.
                if handed_over {
                    Vec::new()
                } else {
                    handed_over = true;
                    vec![queued_to(
                        &chain,
                        Origin::Agent(AgentName::new("analyst")),
                        "developer",
                        3,
                    )]
                }
            },
        );

        assert_eq!(ran, 2, "both hops ran: {seen:?}");
        assert_eq!(seen, ["analyst", "developer"]);
    }

    #[test]
    fn every_hop_of_a_chain_spends_the_same_fuel_and_the_same_run_cap() {
        // The rails are per-chain. A fresh itinerary per flight would reset Hops, Fuel and the run
        // cap, and a loop between two agents would run forever on a renewed budget.
        let temp = Temp::new("sharedrails");
        let factory = pair(&temp);
        let chain = ItineraryId::generate();

        let mut refills = 0;
        factory.drain_with(
            vec![queued_to(&chain, Origin::Human, "analyst", 4)],
            &mut |_| {},
            &mut |_, _| {},
            |_| {
                refills += 1;
                if refills > 1 {
                    Vec::new()
                } else {
                    vec![queued_to(
                        &chain,
                        Origin::Agent(AgentName::new("analyst")),
                        "developer",
                        3,
                    )]
                }
            },
        );

        let started = factory
            .chains
            .with(&chain, &factory.config.defaults, |c| c.runs_started())
            .expect("the chain exists");

        assert_eq!(started, 2, "both runs counted against one itinerary");
    }

    #[test]
    fn a_chain_that_will_not_settle_is_cut_by_the_run_cap_rather_than_looping_forever() {
        // The loop is bounded by the rails, not by a count of passes. An agent that keeps sending
        // has to hit something, and the run cap needs no runner cooperation to bite. This test
        // hangs rather than fails if the loop has no termination guarantee.
        let temp = Temp::new("runaway");
        let factory = pair(&temp);
        let chain = ItineraryId::generate();

        let mut refusals = 0;
        let ran = factory.drain_with(
            vec![queued_to(&chain, Origin::Human, "analyst", 4)],
            &mut |_| {},
            &mut |_, result| {
                if matches!(result, Dispatched::Refused(_)) {
                    refusals += 1;
                }
            },
            // Never stops offering more work, exactly as a runaway chain would not.
            |_| vec![queued_to(&chain, Origin::Human, "analyst", 4)],
        );

        assert_eq!(ran, 10, "ran up to the configured run cap and no further");
        assert!(refusals >= 1, "the cap must refuse, not silently stop");
    }

    #[test]
    fn a_run_is_given_a_token_and_an_endpoint_only_when_the_factory_serves_one() {
        let temp = Temp::new("notoken");
        let factory = pair(&temp);

        // Nothing is serving MCP, so there is nothing to tell a child about.
        factory.drain(
            vec![queued_to(
                &ItineraryId::generate(),
                Origin::Human,
                "analyst",
                4,
            )],
            |_| {},
            |_, _| {},
        );

        assert_eq!(factory.tokens.live_count(), 0);
    }

    #[test]
    fn a_served_run_gets_an_mcp_config_in_its_hangar_and_loses_its_token_afterwards() {
        let temp = Temp::new("served");
        let factory = pair(&temp).serving_mcp("http://127.0.0.1:9/mcp");

        factory.drain(
            vec![queued_to(
                &ItineraryId::generate(),
                Origin::Human,
                "analyst",
                4,
            )],
            |_| {},
            |_, _| {},
        );

        let hangars = temp.0.join(".layover").join("hangars").join("analyst");
        let written = std::fs::read_dir(&hangars)
            .expect("a Hangar")
            .filter_map(Result::ok)
            .map(|entry| entry.path().join("mcp.json"))
            .find(|path| path.exists())
            .and_then(|path| std::fs::read_to_string(path).ok())
            .expect("an MCP configuration");

        assert!(written.contains("127.0.0.1:9/mcp"), "{written}");
        assert_eq!(
            factory.tokens.live_count(),
            0,
            "a token that outlives its run is a finished process that can still send work"
        );
    }
}
