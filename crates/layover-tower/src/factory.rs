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

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use jiff::Timestamp;
use layover_core::agent::AgentName;
use layover_core::barrier::Delivery;
use layover_core::config::Config;
use layover_core::cost::CostSource;
use layover_core::cost::TokenUsage;
use layover_core::flight::Flight;
use layover_core::graph::RouteGraph;
use layover_core::itinerary::Itinerary;
use layover_core::payload::{Run, compose};
use layover_core::queue::Queued;
use layover_core::run::{Outcome, RunRecord};

use crate::barriers::{Abandoned, Barriers};
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
    /// The flight is waiting at a rendezvous for its siblings.
    ///
    /// Not a failure and not a run: the work is held, and the agent it was for stays asleep until
    /// the rest of what it needs arrives.
    Parked {
        /// Upstreams still outstanding.
        waiting_for: Vec<AgentName>,
    },
    /// The flight arrived after an `any` join had already woken its agent.
    ///
    /// Recorded rather than dropped. Releasing on the first arrival is the point of `any` — waking
    /// a publisher once per straggler means one pull request per straggler — but work that
    /// disappears without a record is indistinguishable from work nobody asked for.
    Superseded,
    /// Something went wrong that is the factory's fault rather than the flight's.
    Failed(String),
}

impl std::fmt::Display for Dispatched {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ran { outcome, usd, .. } => write!(f, "{outcome} ${usd:.2}"),
            Self::Refused(refusal) => write!(f, "refused: {refusal}"),
            Self::Parked { waiting_for } => {
                let names = waiting_for
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ");
                write!(f, "waiting for {names}")
            }
            Self::Superseded => f.write_str("superseded: the join had already released"),
            Self::Failed(why) => write!(f, "could not start: {why}"),
        }
    }
}

/// What a drain did.
#[derive(Debug, Default)]
pub struct Drained {
    /// How many runs started and finished.
    pub ran: usize,
    /// Barriers given up because nothing could still satisfy them.
    ///
    /// Reported rather than counted, because each one is work somebody asked for that will not
    /// happen, and a number does not say which.
    pub abandoned: Vec<Abandoned>,
}

/// A running factory.
pub struct Factory {
    config: Config,
    graph: RouteGraph,
    root: PathBuf,
    live: Ledger,
    tokens: Arc<Tokens>,
    chains: Chains,
    barriers: Barriers,
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
            barriers: Barriers::new(),
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
                self.record(&self.never_started(
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
            pipeline: self.chains.pipeline_of(itinerary.id()),
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
        &self,
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
            pipeline: self.chains.pipeline_of(itinerary.id()),
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
    ) -> Drained {
        self.drain_with(pending, &mut unqueue, &mut report, |_| Vec::new())
    }

    /// Drains, asking `refill` for newly queued work after each pass.
    pub fn drain_with(
        &self,
        pending: Vec<Queued>,
        unqueue: &mut impl FnMut(&Flight),
        report: &mut impl FnMut(&Flight, &Dispatched),
        mut refill: impl FnMut(&[Flight]) -> Vec<Queued>,
    ) -> Drained {
        let mut ran = 0;
        let mut batch = pending;
        let mut done: Vec<Flight> = Vec::new();

        while !batch.is_empty() {
            let before = ran;

            for queued in std::mem::take(&mut batch) {
                if self.ground_stop_engaged() {
                    return Drained {
                        ran,
                        abandoned: Vec::new(),
                    };
                }

                unqueue(&queued.flight);

                // Recorded before anything runs, because only this first flight knows which
                // pipeline opened the chain: a flight an agent sends carries none, and every run
                // after this one reads the answer from here.
                self.chains
                    .opened_by(&queued.flight.itinerary, queued.pipeline.as_ref());

                let Some(flight) = self.past_the_barrier(&queued.flight, report) else {
                    done.push(queued.flight);
                    continue;
                };

                // The chain is looked up, not created. Every flight in one causal chain is
                // accounted against the same Hops, Fuel and run cap; minting a fresh itinerary
                // per flight would reset all three and a loop between two agents would never end.
                let sender = flight.from.agent().cloned();
                let result = self
                    .chains
                    .with(&flight.itinerary, &self.config.defaults, |chain| {
                        self.run_flight(chain, sender.as_ref(), &flight)
                    })
                    .unwrap_or_else(|| {
                        Dispatched::Failed("the itinerary ledger was poisoned".to_owned())
                    });

                report(&flight, &result);

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

        // Nothing is running and nothing is queued, so any barrier still holding work is waiting
        // for something that will never arrive. Giving up loudly beats a silent permanent stall,
        // which is the worst outcome in this system: a failure at least says something happened.
        let abandoned = self
            .barriers
            .abandon_unreachable(&self.graph, &BTreeSet::new());

        Drained { ran, abandoned }
    }

    /// Resolves a flight against any rendezvous guarding its destination.
    ///
    /// Returns the flight that should actually run — which for a released join is **one** flight
    /// carrying everything the agent was waiting for, not one run per upstream. Two edges into one
    /// agent without a join fire it twice; for a publisher that means two pull requests.
    ///
    /// Returns `None` when nothing should run: the flight was parked, or it arrived after an `any`
    /// join had already fired.
    fn past_the_barrier(
        &self,
        flight: &Flight,
        report: &mut impl FnMut(&Flight, &Dispatched),
    ) -> Option<Flight> {
        match self.barriers.deliver(&self.graph, flight.clone()) {
            // The common case: the destination declares no join at all.
            None => Some(flight.clone()),
            Some(Delivery::Direct(direct)) => Some(*direct),
            Some(Delivery::Ready(arrived)) => Some(combine(arrived)),
            Some(Delivery::Parked { waiting_for }) => {
                report(flight, &Dispatched::Parked { waiting_for });
                None
            }
            Some(Delivery::Late(late)) => {
                report(&late, &Dispatched::Superseded);
                None
            }
        }
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

/// Folds everything a join was waiting for into the one flight that wakes its agent.
///
/// Each body is labelled with who sent it. A joined agent is looking at several verdicts about the
/// same work — a test result and a review, say — and "approved" means nothing without knowing
/// which of them said it.
///
/// The surviving flight keeps the first sender for the route check. Every parked sender has a
/// permitted edge to this agent, so any of them establishes the same thing; taking one keeps the
/// record honest about the fact that a single run happened.
fn combine(mut arrived: Vec<Flight>) -> Flight {
    let Some(mut first) = arrived.first().cloned() else {
        unreachable!("a barrier does not release with nothing parked");
    };

    if arrived.len() == 1 {
        return first;
    }

    arrived.sort_by(|a, b| a.from.agent().cmp(&b.from.agent()));

    let mut body = String::new();
    for flight in &arrived {
        let who = flight
            .from
            .agent()
            .map_or_else(|| "a human".to_owned(), ToString::to_string);

        let _ = writeln!(body, "## From `{who}`\n\n{}\n", flight.body.trim());
    }

    first.body.clear();
    first.body.push_str(body.trim_end());
    first
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

        assert_eq!(ran.ran, 1);
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

        assert_eq!(ran.ran, 0);
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

        assert_eq!(ran.ran, 2, "both hops ran: {seen:?}");
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

        assert_eq!(
            ran.ran, 10,
            "ran up to the configured run cap and no further"
        );
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

    /// The shape the reference factory is built around: two verdicts, one publisher.
    fn joined(temp: &Temp) -> Factory {
        let text = format!(
            r#"
[layover]
work_dir = "work"

[defaults]
runner = "shell"
max_hops = 6
fuel_usd = 5.0
max_runs = 20
timeout_sec = 30

[runners.shell]
command = {}

[agents.developer]
prompt = "develop"
entry = true

[agents.tester]
prompt = "test"

[agents.reviewer]
prompt = "review"

[agents.publisher]
prompt = "publish"

[pipelines.build]
entry = "developer"

[[routes]]
from = "developer"
to = ["tester", "reviewer"]

[[routes]]
from = ["tester", "reviewer"]
to = "publisher"
join = "all"
"#,
            shell("echo done")
        );

        let config: Config = toml::from_str(&text).expect("parses");
        Factory::new(config, &temp.0).expect("opens")
    }

    fn verdict(chain: &ItineraryId, from: &str, to: &str, body: &str) -> Queued {
        Queued::new(
            Flight::new(
                chain.clone(),
                Origin::Agent(AgentName::new(from)),
                AgentName::new(to),
                body,
                4,
            ),
            None,
            BTreeMap::new(),
        )
    }

    #[test]
    fn every_run_in_a_chain_is_labelled_with_the_pipeline_that_opened_it() {
        // Only the first flight carries a pipeline; one an agent sends has none. Labelling only
        // the first hop would leave the rest of a chain looking like it belonged to nothing.
        let temp = Temp::new("pipeline-label");
        let factory = pair(&temp);
        let chain = ItineraryId::generate();

        let mut opener = queued_to(&chain, Origin::Human, "analyst", 4);
        opener.pipeline = Some(layover_core::pipeline::PipelineName::new("build"));

        let mut handed_over = false;
        factory.drain_with(vec![opener], &mut |_| {}, &mut |_, _| {}, |_| {
            if handed_over {
                Vec::new()
            } else {
                handed_over = true;
                // As an agent's `layover_send` would: no pipeline of its own.
                vec![queued_to(
                    &chain,
                    Origin::Agent(AgentName::new("analyst")),
                    "developer",
                    3,
                )]
            }
        });

        let history = std::fs::read_dir(factory.history_dir())
            .expect("a history directory")
            .filter_map(Result::ok)
            .filter_map(|entry| std::fs::read_to_string(entry.path()).ok())
            .collect::<String>();

        assert_eq!(
            history.matches("\"pipeline\":\"build\"").count(),
            2,
            "both runs should carry the pipeline: {history}"
        );
    }

    #[test]
    fn a_publisher_behind_a_join_runs_once_not_once_per_verdict() {
        // Two edges into one agent without a join fire it twice. For a publisher that is two pull
        // requests for one piece of work.
        let temp = Temp::new("join-once");
        let factory = joined(&temp);
        let chain = ItineraryId::generate();

        let mut woke = Vec::new();
        let drained = factory.drain(
            vec![
                verdict(&chain, "tester", "publisher", "tests pass"),
                verdict(&chain, "reviewer", "publisher", "looks good"),
            ],
            |_: &Flight| {},
            |flight, result| {
                if matches!(result, Dispatched::Ran { .. }) {
                    woke.push(flight.to.to_string());
                }
            },
        );

        assert_eq!(drained.ran, 1, "the publisher ran once: {woke:?}");
        assert_eq!(woke, ["publisher"]);
        assert!(drained.abandoned.is_empty(), "{:?}", drained.abandoned);
    }

    #[test]
    fn the_first_verdict_parks_and_says_who_it_is_waiting_for() {
        let temp = Temp::new("join-park");
        let factory = joined(&temp);
        let chain = ItineraryId::generate();

        let mut parked = Vec::new();
        factory.drain(
            vec![verdict(&chain, "tester", "publisher", "tests pass")],
            |_: &Flight| {},
            |_, result| {
                if let Dispatched::Parked { waiting_for } = result {
                    parked.clone_from(waiting_for);
                }
            },
        );

        assert_eq!(parked, [AgentName::new("reviewer")]);
    }

    #[test]
    fn a_released_join_hands_the_agent_every_verdict_labelled_by_sender() {
        // "Approved" means nothing without knowing which of them said it.
        let temp = Temp::new("join-body");
        let factory = joined(&temp);
        let chain = ItineraryId::generate();

        factory.drain(
            vec![
                verdict(&chain, "tester", "publisher", "17 tests pass"),
                verdict(&chain, "reviewer", "publisher", "no blocking comments"),
            ],
            |_: &Flight| {},
            |_, _| {},
        );

        let payload = std::fs::read_dir(temp.0.join(".layover").join("hangars").join("publisher"))
            .expect("a Hangar")
            .filter_map(Result::ok)
            .map(|entry| entry.path().join("prompt.md"))
            .find(|path| path.exists())
            .and_then(|path| std::fs::read_to_string(path).ok())
            .expect("a composed payload");

        assert!(payload.contains("From `tester`"), "{payload}");
        assert!(payload.contains("17 tests pass"), "{payload}");
        assert!(payload.contains("From `reviewer`"), "{payload}");
        assert!(payload.contains("no blocking comments"), "{payload}");
    }

    #[test]
    fn a_join_that_can_never_complete_is_given_up_and_named() {
        // Silent permanent stalling is the worst outcome in the system. When the drain goes quiet
        // with a barrier still holding work, nothing can ever deliver the rest.
        let temp = Temp::new("join-dead");
        let factory = joined(&temp);
        let chain = ItineraryId::generate();

        let drained = factory.drain(
            vec![verdict(&chain, "tester", "publisher", "tests pass")],
            |_: &Flight| {},
            |_, _| {},
        );

        assert_eq!(drained.ran, 0, "the publisher never woke");
        assert_eq!(drained.abandoned.len(), 1);
        assert_eq!(drained.abandoned[0].missing, [AgentName::new("reviewer")]);
        assert_eq!(
            drained.abandoned[0].stranded, 1,
            "the flight that was held is accounted for"
        );
    }

    #[test]
    fn a_human_reaching_a_joined_agent_is_not_held_up_by_the_join() {
        // A join declares which inputs an agent needs together, not when it may run.
        let temp = Temp::new("join-human");
        let factory = joined(&temp);

        let direct = Queued::new(
            Flight::new(
                ItineraryId::generate(),
                Origin::Human,
                AgentName::new("publisher"),
                "publish it anyway",
                4,
            ),
            None,
            BTreeMap::new(),
        );

        let drained = factory.drain(vec![direct], |_| {}, |_, _| {});

        assert_eq!(drained.ran, 1, "a human trigger bypasses the barrier");
    }
}
