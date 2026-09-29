//! What a tool call actually does, once it reaches the factory.
//!
//! # Why a chain shares one itinerary
//!
//! The rails are per-chain, not per-run: Hops bound how far a *causal chain* travels, Fuel bounds
//! what that chain may spend in total, the run cap bounds how many runs it may start. A flight
//! sent by an agent continues the chain that woke it, so it must be accounted against the same
//! itinerary — mint a fresh one and every rail resets, and a loop between two agents runs forever
//! on a budget that is renewed each time round.
//!
//! That is why itineraries are held here and looked up by identifier rather than constructed per
//! flight.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use layover_core::agent::AgentName;
use layover_core::config::Config;
use layover_core::flight::{Flight, ItineraryId, Origin};
use layover_core::handover::Handover;
use layover_core::itinerary::Itinerary;
use layover_core::layover::Layover;
use layover_core::learning::{Impact, Learnings, Proposal, Uptake};
use layover_core::pipeline::PipelineName;
use layover_core::queue::Queued;
use layover_core::scope::{ChainScope, RouteMap};
use layover_mcp::{Peer, Runtime, Session, ToolError};

/// The itineraries a running factory is accounting against.
///
/// One per causal chain, created when the chain begins and reused by every flight within it.
#[derive(Debug, Default)]
pub struct Chains {
    live: Mutex<HashMap<String, Itinerary>>,
    /// Which pipeline each chain was triggered through.
    ///
    /// Held here as well as on every queued flight, because this is what a run looks up; the
    /// queued copy is what lets a chain keep its attribution across a restart, when this map
    /// starts empty.
    pipelines: Mutex<HashMap<String, PipelineName>>,
    /// The flags each chain was triggered with.
    ///
    /// Recorded from the first queued flight that carries any, exactly like the pipeline. Without
    /// it every run composed its prompt from the pipeline's defaults, so a flag an operator set at
    /// trigger time was accepted, stored, and then ignored.
    flags: Mutex<HashMap<String, BTreeMap<String, bool>>>,
    /// The pipelines each chain must also stay within, for a chain resuming another's work.
    ///
    /// Recorded from queued work like the pipeline, so a restart cannot widen a resumed chain's
    /// reach by forgetting where its work came from.
    within: Mutex<HashMap<String, BTreeSet<Option<PipelineName>>>>,
}

impl Chains {
    /// An empty set of chains.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Runs `act` against the itinerary for `id`, creating it on first sight.
    ///
    /// Creating on first sight rather than requiring registration means a queued flight from a
    /// previous process still lands in a chain with the configured rails, instead of being
    /// refused for belonging to an itinerary this process has never heard of.
    pub fn with<T>(
        &self,
        id: &ItineraryId,
        defaults: &layover_core::config::Defaults,
        act: impl FnOnce(&mut Itinerary) -> T,
    ) -> Option<T> {
        let mut live = self.live.lock().ok()?;
        let chain = live.entry(id.as_str().to_owned()).or_insert_with(|| {
            Itinerary::new(
                id.clone(),
                defaults.max_hops,
                defaults.fuel_usd,
                defaults.max_runs,
            )
        });

        Some(act(chain))
    }

    /// How many chains are being accounted, for reporting.
    #[must_use]
    pub fn count(&self) -> usize {
        self.live.lock().map_or(0, |live| live.len())
    }

    /// Remembers how a chain was opened: the pipeline it came in through and its flags.
    ///
    /// Called for every queued flight. The first to carry a pipeline, and the first to carry any
    /// flags, decides; later flights of the same chain carry the same values anyway, and a chain
    /// whose flags changed halfway through would compose two different prompts for one piece of
    /// work.
    pub fn opened_by(
        &self,
        id: &ItineraryId,
        pipeline: Option<&PipelineName>,
        flags: &BTreeMap<String, bool>,
        within: &BTreeSet<Option<PipelineName>>,
    ) {
        if !within.is_empty()
            && let Ok(mut known) = self.within.lock()
        {
            known
                .entry(id.as_str().to_owned())
                .or_insert_with(|| within.clone());
        }

        if let Some(pipeline) = pipeline
            && let Ok(mut known) = self.pipelines.lock()
        {
            known
                .entry(id.as_str().to_owned())
                .or_insert_with(|| pipeline.clone());
        }

        if !flags.is_empty()
            && let Ok(mut known) = self.flags.lock()
        {
            known
                .entry(id.as_str().to_owned())
                .or_insert_with(|| flags.clone());
        }
    }

    /// Which pipeline a chain was triggered through, if it is known.
    #[must_use]
    pub fn pipeline_of(&self, id: &ItineraryId) -> Option<PipelineName> {
        self.pipelines.lock().ok()?.get(id.as_str()).cloned()
    }

    /// The flags a chain was triggered with, if it recorded any.
    #[must_use]
    pub fn flags_of(&self, id: &ItineraryId) -> Option<BTreeMap<String, bool>> {
        self.flags.lock().ok()?.get(id.as_str()).cloned()
    }

    /// Which routes a chain may use: its pipeline's, narrowed by any work it resumes.
    ///
    /// A chain this process has not recorded belongs to no pipeline, and so gets the global
    /// routes only — the narrowest answer, which is the safe one to give by default.
    #[must_use]
    pub fn scope_of(&self, id: &ItineraryId) -> ChainScope {
        let within = self
            .within
            .lock()
            .ok()
            .and_then(|known| known.get(id.as_str()).cloned())
            .unwrap_or_default();
        ChainScope::new(self.pipeline_of(id), within)
    }
}

/// Everything a tool call needs, wired to a real factory.
///
/// Owns rather than borrows, because this has to live in an HTTP handler that outlives any
/// particular call and is shared across threads.
/// How a runtime reads the factory's accumulated learnings.
pub type ReadLearnings = Arc<dyn Fn() -> Result<Learnings, String> + Send + Sync>;

/// How it writes them back.
pub type WriteLearnings = Arc<dyn Fn(&Learnings) -> Result<(), String> + Send + Sync>;

/// Where a sent flight goes.
pub type QueueFlight = Arc<dyn Fn(Queued) -> Result<(), String> + Send + Sync>;

/// Where work set down goes.
pub type BookLayover = Arc<dyn Fn(Layover) -> Result<(), String> + Send + Sync>;

/// Where a request for help goes.
pub type AskForHelp =
    Arc<dyn Fn(layover_core::help::HelpRequest) -> Result<(), String> + Send + Sync>;

/// Where a run's report on itself goes.
pub type FileReport = Arc<dyn Fn(layover_core::report::Report) -> Result<(), String> + Send + Sync>;

/// Everything a tool call needs, wired to a real factory.
///
/// Owns rather than borrows, because this has to live in an HTTP handler that outlives any
/// particular call and is shared across threads.
pub struct FactoryRuntime {
    config: Arc<Config>,
    routes: Arc<RouteMap>,
    queue: QueueFlight,
    book: BookLayover,
    ask: AskForHelp,
    file: FileReport,
    read_learnings: ReadLearnings,
    write_learnings: WriteLearnings,
    hangars: PathBuf,
    logbook: PathBuf,
}

/// Where a runtime reads and writes everything outside itself.
///
/// A struct rather than six positional arguments: they are all closures or paths, so the compiler
/// would not catch two of them being swapped, and swapping the queue for the layover shelf is the
/// kind of mistake that only shows up in production.
pub struct Wiring {
    /// The factory definition.
    pub config: Arc<Config>,
    /// Its route map, resolved per pipeline. A call is answered from the calling chain's own
    /// graph, never from the whole factory's.
    pub routes: Arc<RouteMap>,
    /// Where agents' own notes live.
    pub hangars: PathBuf,
    /// The factory's shared memory.
    pub logbook: PathBuf,
    /// Where a sent flight goes.
    pub queue: QueueFlight,
    /// Where work set down goes.
    pub book: BookLayover,
    /// Where a request for help goes: the journal, which is what the dashboard and `doctor` read.
    pub ask: AskForHelp,
    /// Where a run's report goes: the journal, which the dashboard and a resumed layover read.
    pub file: FileReport,
    /// How to read what the factory has learned.
    pub read_learnings: ReadLearnings,
    /// How to write it back.
    pub write_learnings: WriteLearnings,
}

impl FactoryRuntime {
    /// Wires a runtime to a factory definition and the places it keeps things.
    #[must_use]
    pub fn new(wiring: Wiring) -> Self {
        Self {
            config: wiring.config,
            routes: wiring.routes,
            queue: wiring.queue,
            book: wiring.book,
            ask: wiring.ask,
            file: wiring.file,
            read_learnings: wiring.read_learnings,
            write_learnings: wiring.write_learnings,
            hangars: wiring.hangars,
            logbook: wiring.logbook,
        }
    }
}

impl Runtime for FactoryRuntime {
    fn peers(&self, session: &Session) -> Vec<Peer> {
        let graph = self.routes_for(session);
        graph
            .successors(&session.agent)
            .map(|name| Peer {
                name: name.clone(),
                description: self
                    .config
                    .agents
                    .get(name)
                    .and_then(|agent| agent.description.clone()),
                spawns: graph.is_spawn(&session.agent, name),
            })
            .collect()
    }

    fn send(&self, session: &Session, to: &AgentName, body: &str) -> Result<String, ToolError> {
        if !self.config.agents.contains_key(to) {
            return Err(ToolError::NoSuchAgent { agent: to.clone() });
        }

        // The calling chain's own graph: its pipeline's routes, narrowed by any work it resumes. An
        // edge only another workflow has is refused exactly like one the map does not draw — the
        // agent is told to ask `layover_peers`, which lists only what this chain may reach.
        let graph = self.routes_for(session);

        if !graph.permits(&session.agent, to) {
            return Err(ToolError::NotPermitted {
                from: session.agent.clone(),
                to: to.clone(),
            });
        }

        // A spawn edge is the one case where Hops do not apply: it is not continuing this chain,
        // it is starting another. Checking the caller's remaining Hops would refuse a fan-out for
        // a budget the new chain does not draw on.
        let spawns = graph.is_spawn(&session.agent, to);

        // Refused here as well as at dispatch, because being told now is worth more than being
        // told later: the agent can report what it could not pass on, rather than finishing
        // believing it handed the work over.
        if !spawns && session.hops_remaining == 0 {
            return Err(ToolError::Refused {
                because: "this chain has no messages left; finish and report instead of sending"
                    .to_owned(),
            });
        }

        // A spawn edge opens a fresh itinerary, with its own Hops, Fuel and run cap; every other
        // edge continues the caller's. Minting a fresh itinerary for an ordinary edge would reset
        // every rail, and a loop between two agents would run forever on a renewed budget.
        //
        // The reverse mistake is subtler and is why `mode` is declared rather than inferred: a
        // fan-out of twenty pull-request reviews sharing one chain would have the twenty-first
        // review refused for a budget the first twenty spent.
        let (itinerary, hops) = if spawns {
            (ItineraryId::generate(), self.config.defaults.max_hops)
        } else {
            (session.itinerary.clone(), session.hops_remaining)
        };

        let flight = Flight::new(
            itinerary,
            Origin::Agent(session.agent.clone()),
            to.clone(),
            body,
            hops,
        );
        let id = flight.id.as_str().to_owned();

        // The chain's pipeline and flags travel with the work, including into a spawned chain. A
        // flight carrying neither was composed from defaults, which silently undid whatever the
        // operator chose at trigger time; a spawned chain carrying neither merged every
        // pipeline's defaults, so the last declaration won.
        //
        // So does its scope, taken from the session and never from the call: a spawn gives a chain
        // a fresh budget, not a fresh set of permissions.
        let queued = Queued::new(flight, session.pipeline.clone(), session.flags.clone())
            .narrowed_by(session.within.clone());

        (self.queue)(queued).map_err(|detail| ToolError::Unavailable { detail })?;

        Ok(id)
    }

    fn report(&self, session: &Session, headline: &str, body: &str) -> Result<(), ToolError> {
        let report = layover_core::report::Report::new(
            session.run.clone(),
            session.agent.clone(),
            session.itinerary.clone(),
            headline,
            body,
            jiff::Timestamp::now(),
        );

        // To the journal, which the dashboard's report view and a resumed layover read. Written to
        // the agent's Hangar instead, a report was the account of a run that nobody could find.
        (self.file)(report).map_err(|detail| ToolError::Unavailable { detail })
    }

    fn help(
        &self,
        session: &Session,
        blocker: layover_core::help::Blocker,
        summary: &str,
        detail: &str,
        fatal: bool,
    ) -> Result<(), ToolError> {
        let mut request = layover_core::help::HelpRequest::new(
            session.agent.clone(),
            session.run.clone(),
            session.itinerary.clone(),
            blocker,
            summary,
            detail,
            jiff::Timestamp::now(),
        );
        request.fatal = fatal;

        // To the journal, which is what the dashboard's help tab, `doctor` and the run record's
        // `blocked_on` read. Written to the agent's Hangar instead, a request reached nobody: the
        // one channel meant to reach a person was the one nothing looked at.
        (self.ask)(request).map_err(|detail| ToolError::Unavailable { detail })
    }

    fn memory_read(&self, session: &Session) -> Result<String, ToolError> {
        let path = self.agent_dir(&session.agent).join("memory.md");

        match std::fs::read_to_string(&path) {
            Ok(text) => Ok(text),
            // Nothing written yet is not a failure; it is the first run of this agent.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok("You have written nothing down yet.".to_owned())
            }
            Err(error) => Err(ToolError::Unavailable {
                detail: error.to_string(),
            }),
        }
    }

    fn memory_write(&self, session: &Session, text: &str) -> Result<(), ToolError> {
        let dir = self.agent_dir(&session.agent);
        std::fs::create_dir_all(&dir).map_err(|error| ToolError::Unavailable {
            detail: error.to_string(),
        })?;

        let path = dir.join("memory.md");
        let mut existing = std::fs::read_to_string(&path).unwrap_or_default();
        if !existing.is_empty() && !existing.ends_with('\n') {
            existing.push('\n');
        }
        existing.push_str(text.trim());
        existing.push('\n');

        std::fs::write(&path, existing).map_err(|error| ToolError::Unavailable {
            detail: error.to_string(),
        })
    }

    fn wait(&self, session: &Session, until: &str, because: &str) -> Result<String, ToolError> {
        let wait = parse_wait(until).ok_or_else(|| ToolError::BadArguments {
            detail: format!(
                "`{until}` is not a length of time. Use a number and a unit — `30m`, `2h`, `3d` — \
                 which is how long to wait before this is looked at again."
            ),
        })?;

        let now = jiff::Timestamp::now();
        let due_at = now
            .checked_add(jiff::SignedDuration::from_secs(wait))
            .map_err(|_| ToolError::BadArguments {
                detail: format!("`{until}` is further away than this factory can plan for"),
            })?;

        // What the later run needs is which work this is: the message that woke the run setting
        // it down, cut to size. Its report is looked up when the layover is resumed, because a
        // run usually reports after it books and its conclusion does not exist yet.
        let woke = session
            .flight
            .iter()
            .map(|flight| {
                let mut carried = flight.clone();
                carried.body = layover_core::handover::excerpt(&flight.body);
                carried
            })
            .collect();
        let handover = Handover::dispatch(woke);

        let layover = Layover::book(
            session.agent.clone(),
            session.itinerary.clone(),
            because,
            handover,
            now,
            due_at,
            DEFAULT_MAX_CHECKS,
        )
        .with_flags(session.flags.clone())
        .booked_in(session.run.clone())
        .booked_within(ChainScope::new(
            session.pipeline.clone(),
            session.within.clone(),
        ));

        let when = layover.due_at.to_string();

        (self.book)(layover).map_err(|detail| ToolError::Unavailable { detail })?;

        Ok(format!(
            "Set down. This will be picked up no sooner than {when}, by a pipeline that resumes \
             layovers. Finish and report now — nothing is kept running in the meantime."
        ))
    }

    fn learn(&self, session: &Session, text: &str) -> Result<String, ToolError> {
        let proposal = Proposal::new(
            session.agent.clone(),
            text,
            // The agent's own rating of its own work, and not load-bearing: a learning becomes
            // permanent through independent rediscovery, which is evidence, rather than through
            // how important its author said it was. Medium because there is nothing to read it
            // from and inventing a scale for the agent to game would be worse.
            Impact::Medium,
            jiff::Timestamp::now(),
        );

        let mut learnings =
            (self.read_learnings)().map_err(|detail| ToolError::Unavailable { detail })?;

        let uptake = learnings.propose(&proposal);

        // Malformed and Refused change nothing, so writing would be a needless rewrite of the
        // whole file — and `Refused` writing anything at all would let repetition look like it
        // had an effect.
        if !matches!(
            uptake,
            Uptake::Malformed | Uptake::Refused | Uptake::Echo | Uptake::Unacceptable(_)
        ) {
            (self.write_learnings)(&learnings)
                .map_err(|detail| ToolError::Unavailable { detail })?;
        }

        // Said differently for each outcome, because they are not interchangeable and an agent
        // that hears "noted" every time learns nothing about what its proposals are worth.
        Ok(match uptake {
            Uptake::Taken => "Noted. Future runs of you will be given this until it lapses, and \
                              it becomes permanent if later runs arrive at it independently."
                .to_owned(),
            Uptake::Echo => "You were already told this, so repeating it is not evidence of \
                             anything. It stands as it was."
                .to_owned(),
            Uptake::Rediscovered { proposals } => format!(
                "Rediscovered — proposed independently {proposals} time(s) now, so it applies \
                 again and is closer to becoming permanent."
            ),
            Uptake::Confirmed => "Rediscovered often enough to be treated as real. It will be \
                                  given to future runs indefinitely."
                .to_owned(),
            Uptake::Refused => {
                return Err(ToolError::Refused {
                    because: "a human rejected this, and proposing it again does not reopen it. \
                              If it is genuinely true now, say so in a report."
                        .to_owned(),
                });
            }
            Uptake::Malformed => {
                return Err(ToolError::BadArguments {
                    detail: "a learning is one or two sentences. Empty text, or more than will \
                             fit in a prompt alongside everything else, is not one."
                        .to_owned(),
                });
            }
            Uptake::Unacceptable(reason) => {
                return Err(ToolError::Refused {
                    because: reason.to_string(),
                });
            }
        })
    }

    fn logbook_append(&self, session: &Session, text: &str) -> Result<(), ToolError> {
        use std::io::Write as _;

        let line = text.trim();
        if line.is_empty() {
            return Err(ToolError::BadArguments {
                detail: "the logbook is read by every agent; an empty entry is noise".to_owned(),
            });
        }

        if let Some(parent) = self.logbook.parent() {
            std::fs::create_dir_all(parent).map_err(|error| ToolError::Unavailable {
                detail: error.to_string(),
            })?;
        }

        // Stamped with who wrote it and when. The logbook is shared, so an entry nobody can
        // attribute is one nobody can follow up or correct.
        let entry = format!(
            "\n## {} — `{}`\n\n{line}\n",
            jiff::Timestamp::now(),
            session.agent
        );

        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.logbook)
            .and_then(|mut file| file.write_all(entry.as_bytes()))
            .map_err(|error| ToolError::Unavailable {
                detail: error.to_string(),
            })
    }
}

/// How many fruitless checks a layover gets before it is given up on.
///
/// Twelve, against the backoff in `layover_core::layover`, is a little over two days of looking.
/// Long enough for a review to come back over a weekend; short enough that something nobody ever
/// answers stops costing money.
const DEFAULT_MAX_CHECKS: u32 = 12;

/// Reads a wait as a number of seconds.
///
/// Same vocabulary as a pipeline's `every`, deliberately: an operator who has written `every =
/// "2h"` should not have to learn a second way to say two hours in order to read a prompt.
fn parse_wait(text: &str) -> Option<i64> {
    let trimmed = text.trim();
    let (digits, unit) = match trimmed.char_indices().next_back() {
        Some((index, unit)) => (&trimmed[..index], unit),
        None => return None,
    };

    let multiplier = match unit {
        's' => 1_i64,
        'm' => 60,
        'h' => 60 * 60,
        'd' => 24 * 60 * 60,
        _ => return None,
    };

    digits.trim().parse::<i64>().ok()?.checked_mul(multiplier)
}

impl FactoryRuntime {
    /// The graph the calling chain may use, from the Tower's record of it.
    fn routes_for(
        &self,
        session: &Session,
    ) -> std::borrow::Cow<'_, layover_core::graph::RouteGraph> {
        self.routes.for_scope(&ChainScope::new(
            session.pipeline.clone(),
            session.within.clone(),
        ))
    }

    /// Where one agent's own files live.
    fn agent_dir(&self, agent: &AgentName) -> PathBuf {
        self.hangars.join(agent.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use layover_core::flight::RunId;

    const FACTORY: &str = r#"
[layover]
work_dir = "work"

[defaults]
runner = "shell"
max_hops = 4
fuel_usd = 5.0
max_runs = 10

[runners.shell]
command = ["echo"]

[agents.analyst]
description = "Works out what a request means"
prompt = "analyse"
entry = true

[agents.developer]
description = "Writes the code"
prompt = "develop"

[agents.stranger]
prompt = "lurk"

[agents.reviewer]
prompt = "review one pull request"

[pipelines.build]
entry = "analyst"

[[routes]]
from = "analyst"
to = "developer"

[[routes]]
from = "analyst"
to = "reviewer"
mode = "spawn"
"#;

    /// A runtime over a temporary directory, with everything it queued or booked kept for
    /// inspection.
    struct Fixture {
        runtime: FactoryRuntime,
        sent: Arc<Mutex<Vec<Queued>>>,
        booked: Arc<Mutex<Vec<Layover>>>,
        asked: Arc<Mutex<Vec<layover_core::help::HelpRequest>>>,
        filed: Arc<Mutex<Vec<layover_core::report::Report>>>,
        learnings: Arc<Mutex<Learnings>>,
        defaults: layover_core::config::Defaults,
        dir: PathBuf,
    }

    impl Fixture {
        fn new(name: &str) -> Self {
            let config: Config = toml::from_str(FACTORY).expect("the fixture factory parses");
            let routes = RouteMap::from_config(&config);
            let defaults = config.defaults.clone();
            let dir =
                std::env::temp_dir().join(format!("layover-rt-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("a temporary directory");

            let sent: Arc<Mutex<Vec<Queued>>> = Arc::new(Mutex::new(Vec::new()));
            let sink = Arc::clone(&sent);
            let booked: Arc<Mutex<Vec<Layover>>> = Arc::new(Mutex::new(Vec::new()));
            let shelf = Arc::clone(&booked);
            let asked: Arc<Mutex<Vec<layover_core::help::HelpRequest>>> =
                Arc::new(Mutex::new(Vec::new()));
            let inbox = Arc::clone(&asked);
            let filed: Arc<Mutex<Vec<layover_core::report::Report>>> =
                Arc::new(Mutex::new(Vec::new()));
            let cabinet = Arc::clone(&filed);
            let learnings: Arc<Mutex<Learnings>> = Arc::new(Mutex::new(Learnings::new()));
            let reading = Arc::clone(&learnings);
            let writing = Arc::clone(&learnings);

            Self {
                runtime: FactoryRuntime::new(Wiring {
                    config: Arc::new(config),
                    routes: Arc::new(routes),
                    hangars: dir.clone(),
                    logbook: dir.join("logbook.md"),
                    queue: Arc::new(move |queued| {
                        sink.lock().map_err(|_| "poisoned".to_owned())?.push(queued);
                        Ok(())
                    }),
                    book: Arc::new(move |layover| {
                        shelf
                            .lock()
                            .map_err(|_| "poisoned".to_owned())?
                            .push(layover);
                        Ok(())
                    }),
                    ask: Arc::new(move |request| {
                        inbox
                            .lock()
                            .map_err(|_| "poisoned".to_owned())?
                            .push(request);
                        Ok(())
                    }),
                    file: Arc::new(move |report| {
                        cabinet
                            .lock()
                            .map_err(|_| "poisoned".to_owned())?
                            .push(report);
                        Ok(())
                    }),
                    read_learnings: Arc::new(move || {
                        Ok(reading.lock().map_err(|_| "poisoned".to_owned())?.clone())
                    }),
                    write_learnings: Arc::new(move |updated| {
                        *writing.lock().map_err(|_| "poisoned".to_owned())? = updated.clone();
                        Ok(())
                    }),
                }),
                sent,
                booked,
                asked,
                filed,
                learnings,
                defaults,
                dir,
            }
        }

        fn sent(&self) -> Vec<Queued> {
            self.sent.lock().expect("not poisoned").clone()
        }

        fn booked(&self) -> Vec<Layover> {
            self.booked.lock().expect("not poisoned").clone()
        }

        fn asked(&self) -> Vec<layover_core::help::HelpRequest> {
            self.asked.lock().expect("not poisoned").clone()
        }

        fn filed(&self) -> Vec<layover_core::report::Report> {
            self.filed.lock().expect("not poisoned").clone()
        }

        fn learnings(&self) -> Learnings {
            self.learnings.lock().expect("not poisoned").clone()
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn session(agent: &str, hops: u32) -> Session {
        Session {
            run: RunId::generate(),
            agent: AgentName::new(agent),
            itinerary: ItineraryId::generate(),
            hops_remaining: hops,
            pipeline: None,
            flags: BTreeMap::new(),
            flight: None,
            within: std::collections::BTreeSet::new(),
        }
    }

    /// A session for a run whose chain came in through `build` with `run_e2e` switched on.
    fn triggered(agent: &str) -> Session {
        Session {
            pipeline: Some(PipelineName::new("build")),
            flags: BTreeMap::from([("run_e2e".to_owned(), true)]),
            ..session(agent, 3)
        }
    }

    #[test]
    fn a_help_request_is_filed_under_the_category_the_agent_chose() {
        // Every request used to be filed as `other`, and into the agent's Hangar, where neither
        // the dashboard nor `doctor` looks. It now goes wherever the runtime is wired to send it,
        // which in a real factory is the journal.
        let fixture = Fixture::new("help");
        let caller = session("analyst", 3);

        fixture
            .runtime
            .help(
                &caller,
                layover_core::help::Blocker::Access,
                "the ADO token expired",
                "401 on every call",
                true,
            )
            .expect("files it");

        let asked = fixture.asked();
        assert_eq!(asked.len(), 1);
        assert_eq!(asked[0].blocker, layover_core::help::Blocker::Access);
        assert_eq!(asked[0].run, caller.run, "attributed to the run that asked");
        assert!(asked[0].fatal);
        assert!(
            !fixture.dir.join("analyst").join("help.jsonl").exists(),
            "not into the Hangar, where nothing reads it"
        );
    }

    #[test]
    fn a_sent_flight_carries_its_chains_pipeline_and_flags() {
        // Without these the next run composed its prompt from the pipeline's defaults, silently
        // undoing whatever the operator chose at trigger time. Carried on the queued flight, not
        // only remembered in memory, so a restart between the two runs does not lose them.
        let fixture = Fixture::new("carries-flags");

        fixture
            .runtime
            .send(&triggered("analyst"), &AgentName::new("developer"), "go")
            .expect("the route is drawn");

        let sent = fixture.sent();
        assert_eq!(sent[0].pipeline, Some(PipelineName::new("build")));
        assert_eq!(sent[0].flags.get("run_e2e"), Some(&true));
    }

    #[test]
    fn a_spawned_chain_inherits_the_flags_and_pipeline_of_the_chain_that_spawned_it() {
        // A spawned chain has fresh budget, not fresh instructions. With neither carried, it
        // merged every pipeline's defaults and the last declaration won.
        let fixture = Fixture::new("spawn-flags");
        let caller = triggered("analyst");

        fixture
            .runtime
            .send(&caller, &AgentName::new("reviewer"), "review #41")
            .expect("the spawn edge is drawn");

        let sent = fixture.sent();
        assert_ne!(
            sent[0].flight.itinerary, caller.itinerary,
            "still a new chain"
        );
        assert_eq!(sent[0].pipeline, Some(PipelineName::new("build")));
        assert_eq!(sent[0].flags.get("run_e2e"), Some(&true));
    }

    #[test]
    fn a_layover_remembers_the_flags_its_chain_was_composed_with() {
        // The follow-up is about the same work, and the operator's choices about that work were
        // made when it was triggered.
        let fixture = Fixture::new("wait-flags");

        fixture
            .runtime
            .wait(&triggered("analyst"), "2h", "the review to land")
            .expect("books");

        assert_eq!(fixture.booked()[0].flags.get("run_e2e"), Some(&true));
    }

    #[test]
    fn peers_are_what_the_route_map_permits_and_nothing_else() {
        let fixture = Fixture::new("peers");

        let peers = fixture.runtime.peers(&session("analyst", 3));
        let names: Vec<String> = peers.iter().map(|peer| peer.name.to_string()).collect();

        assert_eq!(names, ["developer", "reviewer"], "the two drawn edges");
        assert!(
            !names.contains(&"stranger".to_owned()),
            "an agent with no edge from `analyst` is not a peer"
        );
        assert_eq!(peers[0].description.as_deref(), Some("Writes the code"));
    }

    #[test]
    fn a_sent_flight_continues_the_chain_rather_than_starting_one() {
        // The rails are per-chain. Minting a fresh itinerary here would reset Hops, Fuel and the
        // run cap, so a loop between two agents would run forever on a renewed budget.
        let fixture = Fixture::new("continues");
        let caller = session("analyst", 3);

        fixture
            .runtime
            .send(&caller, &AgentName::new("developer"), "fix it")
            .expect("the route is drawn");

        let sent = fixture.sent();
        assert_eq!(sent.len(), 1);
        assert_eq!(
            sent[0].flight.itinerary, caller.itinerary,
            "the flight must belong to the chain that sent it"
        );
        assert_eq!(sent[0].flight.hops_remaining, 3);
    }

    #[test]
    fn a_sent_flight_records_which_agent_sent_it() {
        // A joined agent receives several flights at once and has to tell them apart.
        let fixture = Fixture::new("origin");

        fixture
            .runtime
            .send(&session("analyst", 2), &AgentName::new("developer"), "go")
            .expect("the route is drawn");

        assert_eq!(
            fixture.sent()[0].flight.from,
            Origin::Agent(AgentName::new("analyst"))
        );
    }

    #[test]
    fn an_edge_the_map_does_not_draw_is_refused_with_advice() {
        let fixture = Fixture::new("refused");

        let error = fixture
            .runtime
            .send(&session("analyst", 3), &AgentName::new("stranger"), "go")
            .expect_err("no such edge");

        assert!(matches!(error, ToolError::NotPermitted { .. }));
        assert!(
            error.to_string().contains("layover_peers"),
            "a refusal should say how to find out what is permitted: {error}"
        );
        assert!(fixture.sent().is_empty(), "nothing may be queued");
    }

    #[test]
    fn sending_to_an_agent_that_does_not_exist_says_so() {
        let fixture = Fixture::new("ghost");

        let error = fixture
            .runtime
            .send(&session("analyst", 3), &AgentName::new("ghost"), "go")
            .expect_err("no such agent");

        assert!(matches!(error, ToolError::NoSuchAgent { .. }), "{error}");
    }

    #[test]
    fn a_chain_with_no_hops_left_is_told_to_finish_rather_than_send() {
        // Being told now is worth more than being told at dispatch: the agent can report what it
        // could not pass on, instead of finishing in the belief that it handed the work over.
        let fixture = Fixture::new("nohops");

        let error = fixture
            .runtime
            .send(&session("analyst", 0), &AgentName::new("developer"), "go")
            .expect_err("out of hops");

        assert!(error.to_string().contains("report"), "{error}");
        assert!(fixture.sent().is_empty(), "nothing may be queued");
    }

    #[test]
    fn a_spawn_edge_opens_a_new_chain_with_its_own_budget() {
        // A fan-out of twenty pull-request reviews sharing one chain would have the twenty-first
        // refused for a budget the first twenty spent. That is what `mode = "spawn"` exists for.
        let fixture = Fixture::new("spawn");
        let caller = session("analyst", 2);

        fixture
            .runtime
            .send(&caller, &AgentName::new("reviewer"), "review #41")
            .expect("the spawn edge is drawn");

        let sent = fixture.sent();
        assert_ne!(
            sent[0].flight.itinerary, caller.itinerary,
            "a spawn edge starts a chain rather than continuing one"
        );
        assert_eq!(
            sent[0].flight.hops_remaining, fixture.defaults.max_hops,
            "the new chain gets the configured budget, not the caller's remainder"
        );
    }

    #[test]
    fn a_spawn_may_be_sent_even_when_the_caller_has_no_hops_left() {
        // Hops bound one causal chain. A spawn is not continuing this one, so refusing it would
        // charge the new chain for a budget it does not draw on.
        let fixture = Fixture::new("spawn-nohops");

        let id = fixture
            .runtime
            .send(&session("analyst", 0), &AgentName::new("reviewer"), "go")
            .expect("a spawn does not spend the caller's hops");

        assert!(!id.is_empty());
        assert_eq!(fixture.sent().len(), 1);
    }

    #[test]
    fn a_spawn_edge_is_still_an_edge_the_route_map_has_to_draw() {
        let fixture = Fixture::new("spawn-refused");

        let error = fixture
            .runtime
            .send(&session("developer", 3), &AgentName::new("reviewer"), "go")
            .expect_err("no edge from developer to reviewer");

        assert!(matches!(error, ToolError::NotPermitted { .. }), "{error}");
    }

    #[test]
    fn peers_say_which_of_them_open_a_new_chain() {
        // An agent deciding where work goes should be able to tell a hand-off from a fan-out.
        let fixture = Fixture::new("spawn-peers");

        let peers = fixture.runtime.peers(&session("analyst", 3));
        let reviewer = peers
            .iter()
            .find(|peer| peer.name == AgentName::new("reviewer"))
            .expect("reviewer is reachable");
        let developer = peers
            .iter()
            .find(|peer| peer.name == AgentName::new("developer"))
            .expect("developer is reachable");

        assert!(reviewer.spawns, "the spawn edge is marked");
        assert!(!developer.spawns, "an ordinary edge is not");
    }

    #[test]
    fn booking_a_layover_sets_the_work_down_and_says_when_it_returns() {
        let fixture = Fixture::new("wait");

        let answer = fixture
            .runtime
            .wait(&session("analyst", 3), "2h", "the review to land")
            .expect("2h is a length of time");

        assert!(answer.contains("Set down"), "{answer}");
        assert!(
            answer.contains("Finish and report"),
            "an agent must be told not to wait: {answer}"
        );

        let booked = fixture.booked();
        assert_eq!(booked.len(), 1);
        assert_eq!(booked[0].agent, AgentName::new("analyst"));
        assert_eq!(booked[0].waiting_for, "the review to land");
    }

    #[test]
    fn a_layover_comes_back_to_the_chain_that_booked_it() {
        // The resumed run is told which chain set this down, which is the only thread back to
        // what it was about.
        let fixture = Fixture::new("wait-chain");
        let caller = session("analyst", 3);

        fixture
            .runtime
            .wait(&caller, "1d", "the build to go green")
            .expect("books");

        assert_eq!(fixture.booked()[0].booked_by, caller.itinerary);
    }

    #[test]
    fn a_layover_is_not_due_before_its_time() {
        let fixture = Fixture::new("wait-due");

        fixture
            .runtime
            .wait(&session("analyst", 3), "2h", "something")
            .expect("books");

        let booked = &fixture.booked()[0];
        assert!(!booked.is_due(jiff::Timestamp::now()));
        assert!(
            booked.is_due(
                jiff::Timestamp::now()
                    .checked_add(jiff::SignedDuration::from_hours(3))
                    .expect("in range")
            )
        );
    }

    #[test]
    fn a_wait_that_is_not_a_length_of_time_is_refused_with_an_example() {
        // An agent given "until the review lands" has to be told what shape the answer takes,
        // not merely that it was wrong.
        let fixture = Fixture::new("wait-bad");

        let error = fixture
            .runtime
            .wait(&session("analyst", 3), "when the review lands", "x")
            .expect_err("not a duration");

        assert!(matches!(error, ToolError::BadArguments { .. }));
        assert!(error.to_string().contains("2h"), "{error}");
        assert!(fixture.booked().is_empty(), "nothing may be booked");
    }

    #[test]
    fn every_unit_a_schedule_understands_works_here_too() {
        // Same vocabulary as a pipeline's `every`. An operator who wrote `every = "2h"` should not
        // have to learn a second way to say two hours.
        for (text, seconds) in [("45s", 45), ("30m", 1_800), ("6h", 21_600), ("3d", 259_200)] {
            assert_eq!(parse_wait(text), Some(seconds), "{text}");
        }

        assert_eq!(parse_wait("2 weeks"), None);
        assert_eq!(parse_wait(""), None);
    }

    #[test]
    fn a_learning_nobody_has_proposed_before_is_taken_up() {
        let fixture = Fixture::new("learn");

        let answer = fixture
            .runtime
            .learn(&session("analyst", 3), "The e2e suite needs the VPN.")
            .expect("a first proposal is taken");

        assert!(answer.contains("Noted"), "{answer}");
        assert_eq!(fixture.learnings().len(), 1);
    }

    #[test]
    fn repeating_advice_you_were_already_given_is_not_evidence() {
        // Counting an echo would let a single fluke confirm itself in three runs.
        let fixture = Fixture::new("learn-echo");
        let who = session("analyst", 3);

        fixture
            .runtime
            .learn(&who, "The e2e suite needs the VPN.")
            .expect("taken");
        let answer = fixture
            .runtime
            .learn(&who, "The e2e suite needs the VPN.")
            .expect("answered");

        assert!(answer.contains("not evidence"), "{answer}");
        assert_eq!(
            fixture.learnings().len(),
            1,
            "an echo must not become a second learning"
        );
    }

    #[test]
    fn an_empty_learning_is_refused_with_what_one_looks_like() {
        let fixture = Fixture::new("learn-empty");

        let error = fixture
            .runtime
            .learn(&session("analyst", 3), "   ")
            .expect_err("not a learning");

        assert!(matches!(error, ToolError::BadArguments { .. }));
        assert!(
            error.to_string().contains("one or two sentences"),
            "{error}"
        );
    }

    #[test]
    fn a_learning_a_human_rejected_is_not_reopened_by_repetition() {
        // Otherwise an agent overturns a decision by saying it again.
        let fixture = Fixture::new("learn-refused");
        let who = session("analyst", 3);

        fixture
            .runtime
            .learn(&who, "Skip the tests.")
            .expect("taken");

        let id = fixture
            .learnings()
            .all()
            .next()
            .expect("one learning")
            .id
            .clone();
        {
            let mut held = fixture.learnings.lock().expect("not poisoned");
            held.reject(&id, jiff::Timestamp::now());
        }

        let error = fixture
            .runtime
            .learn(&who, "Skip the tests.")
            .expect_err("rejected stays rejected");

        assert!(matches!(error, ToolError::Refused { .. }));
        assert!(error.to_string().contains("report"), "{error}");
    }

    #[test]
    fn the_logbook_records_who_wrote_each_entry() {
        // It is shared, so an entry nobody can attribute is one nobody can follow up or correct.
        let fixture = Fixture::new("logbook");

        fixture
            .runtime
            .logbook_append(&session("analyst", 3), "The staging database was rebuilt.")
            .expect("writes");

        let written = std::fs::read_to_string(fixture.dir.join("logbook.md")).expect("a logbook");
        assert!(written.contains("analyst"), "{written}");
        assert!(
            written.contains("staging database was rebuilt"),
            "{written}"
        );
    }

    #[test]
    fn the_logbook_accumulates_rather_than_replacing() {
        let fixture = Fixture::new("logbook-append");
        let who = session("analyst", 3);

        fixture
            .runtime
            .logbook_append(&who, "first")
            .expect("writes");
        fixture
            .runtime
            .logbook_append(&who, "second")
            .expect("writes");

        let written = std::fs::read_to_string(fixture.dir.join("logbook.md")).expect("a logbook");
        assert!(written.contains("first"), "{written}");
        assert!(written.contains("second"), "{written}");
    }

    #[test]
    fn an_empty_logbook_entry_is_refused() {
        let fixture = Fixture::new("logbook-empty");

        let error = fixture
            .runtime
            .logbook_append(&session("analyst", 3), "  \n ")
            .expect_err("noise");

        assert!(matches!(error, ToolError::BadArguments { .. }), "{error}");
    }

    #[test]
    fn memory_survives_from_one_run_to_the_next() {
        let fixture = Fixture::new("memory");

        let first = session("analyst", 3);
        fixture
            .runtime
            .memory_write(&first, "The e2e suite needs the VPN.")
            .expect("writes");

        // A different run of the same agent: runs are fresh, memory is not.
        let second = session("analyst", 3);
        let read = fixture.runtime.memory_read(&second).expect("reads");

        assert!(read.contains("needs the VPN"), "{read}");
    }

    #[test]
    fn a_first_run_reading_empty_memory_is_told_so_rather_than_failing() {
        let fixture = Fixture::new("firstrun");

        let read = fixture
            .runtime
            .memory_read(&session("analyst", 3))
            .expect("an empty memory is not a failure");

        assert!(read.contains("nothing"), "{read}");
    }

    #[test]
    fn memory_accumulates_rather_than_replacing() {
        let fixture = Fixture::new("accumulate");
        let who = session("analyst", 3);

        fixture
            .runtime
            .memory_write(&who, "first thing")
            .expect("writes");
        fixture
            .runtime
            .memory_write(&who, "second thing")
            .expect("writes");

        let read = fixture.runtime.memory_read(&who).expect("reads");
        assert!(read.contains("first thing"), "{read}");
        assert!(read.contains("second thing"), "{read}");
    }

    #[test]
    fn a_report_is_written_where_it_can_be_found_afterwards() {
        // "Where it can be found" used to be the agent's Hangar, which neither the dashboard's
        // report view nor a resumed layover reads. It goes wherever the runtime is wired to file
        // it, which in a real factory is the journal.
        let fixture = Fixture::new("report");
        let who = session("analyst", 3);

        fixture
            .runtime
            .report(&who, "Found the cause", "It was the cache all along.")
            .expect("writes");

        let filed = fixture.filed();
        assert_eq!(filed.len(), 1);
        assert_eq!(filed[0].headline, "Found the cause");
        assert_eq!(filed[0].run, who.run, "attributed to the run that wrote it");
        assert!(!fixture.dir.join("analyst").join("reports.jsonl").exists());
    }

    #[test]
    fn a_layover_carries_what_woke_the_run_that_booked_it() {
        // A resumed run should arrive knowing which work this is. The run that booked the layover
        // knows — it is in the flight that woke it — so the layover keeps it, and names the run so
        // its report can be found when the work is picked up.
        let fixture = Fixture::new("wait-context");
        let woke = Flight::new(
            ItineraryId::generate(),
            Origin::Agent(AgentName::new("developer")),
            AgentName::new("analyst"),
            format!("Work item 4821: fix the retry policy.{}", "x".repeat(5_000)),
            3,
        );
        let caller = Session {
            flight: Some(woke),
            ..session("analyst", 3)
        };

        fixture
            .runtime
            .wait(&caller, "2h", "comments on pull request 41")
            .expect("books");

        let booked = &fixture.booked()[0];
        assert_eq!(booked.run.as_ref(), Some(&caller.run));
        assert_eq!(booked.handover.flights.len(), 1);
        let carried = &booked.handover.flights[0].body;
        assert!(carried.starts_with("Work item 4821"), "{carried}");
        assert!(
            carried.len() < 3_000,
            "cut to size before it is stored: {} bytes",
            carried.len()
        );
    }

    #[test]
    fn a_chain_is_created_once_and_reused() {
        let fixture = Fixture::new("chains");
        let chains = Chains::new();
        let id = ItineraryId::generate();

        chains
            .with(&id, &fixture.defaults, |chain| chain.debit_fuel(1.0))
            .expect("locks");
        let remaining = chains
            .with(&id, &fixture.defaults, |chain| chain.fuel_remaining_usd())
            .expect("locks");

        assert!(
            remaining < fixture.defaults.fuel_usd,
            "the debit must have persisted across lookups"
        );
        assert_eq!(chains.count(), 1, "one chain, not two");
    }
}
