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

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use layover_core::agent::AgentName;
use layover_core::config::Config;
use layover_core::flight::{Flight, ItineraryId, Origin};
use layover_core::graph::RouteGraph;
use layover_core::itinerary::Itinerary;
use layover_core::queue::Queued;
use layover_mcp::{Peer, Runtime, Session, ToolError};

/// The itineraries a running factory is accounting against.
///
/// One per causal chain, created when the chain begins and reused by every flight within it.
#[derive(Debug, Default)]
pub struct Chains {
    live: Mutex<HashMap<String, Itinerary>>,
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
}

/// Everything a tool call needs, wired to a real factory.
///
/// Owns rather than borrows, because this has to live in an HTTP handler that outlives any
/// particular call and is shared across threads.
pub struct FactoryRuntime {
    config: Arc<Config>,
    graph: Arc<RouteGraph>,
    queue: Arc<dyn Fn(Queued) -> Result<(), String> + Send + Sync>,
    hangars: PathBuf,
}

impl FactoryRuntime {
    /// Wires a runtime to a factory definition and somewhere to put sent flights.
    #[must_use]
    pub fn new(
        config: Arc<Config>,
        graph: Arc<RouteGraph>,
        hangars: PathBuf,
        queue: Arc<dyn Fn(Queued) -> Result<(), String> + Send + Sync>,
    ) -> Self {
        Self {
            config,
            graph,
            queue,
            hangars,
        }
    }
}

impl Runtime for FactoryRuntime {
    fn peers(&self, session: &Session) -> Vec<Peer> {
        self.graph
            .successors(&session.agent)
            .map(|name| Peer {
                name: name.clone(),
                description: self
                    .config
                    .agents
                    .get(name)
                    .and_then(|agent| agent.description.clone()),
                spawns: self.graph.is_spawn(&session.agent, name),
            })
            .collect()
    }

    fn send(&self, session: &Session, to: &AgentName, body: &str) -> Result<String, ToolError> {
        if !self.config.agents.contains_key(to) {
            return Err(ToolError::NoSuchAgent { agent: to.clone() });
        }

        if !self.graph.permits(&session.agent, to) {
            return Err(ToolError::NotPermitted {
                from: session.agent.clone(),
                to: to.clone(),
            });
        }

        // A spawn edge is the one case where Hops do not apply: it is not continuing this chain,
        // it is starting another. Checking the caller's remaining Hops would refuse a fan-out for
        // a budget the new chain does not draw on.
        let spawns = self.graph.is_spawn(&session.agent, to);

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

        (self.queue)(Queued::new(flight, None, std::collections::BTreeMap::new()))
            .map_err(|detail| ToolError::Unavailable { detail })?;

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

        let path = self.agent_dir(&session.agent).join("reports.jsonl");
        append_json(&path, &report).map_err(|detail| ToolError::Unavailable { detail })
    }

    fn help(
        &self,
        session: &Session,
        summary: &str,
        detail: &str,
        fatal: bool,
    ) -> Result<(), ToolError> {
        let mut request = layover_core::help::HelpRequest::new(
            session.agent.clone(),
            session.run.clone(),
            session.itinerary.clone(),
            layover_core::help::Blocker::Other,
            summary,
            detail,
            jiff::Timestamp::now(),
        );
        request.fatal = fatal;

        let path = self.agent_dir(&session.agent).join("help.jsonl");
        append_json(&path, &request).map_err(|detail| ToolError::Unavailable { detail })
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
}

impl FactoryRuntime {
    /// Where one agent's own files live.
    fn agent_dir(&self, agent: &AgentName) -> PathBuf {
        self.hangars.join(agent.to_string())
    }
}

/// Appends one JSON record to a file, creating it if needed.
fn append_json<T: serde::Serialize>(path: &std::path::Path, value: &T) -> Result<(), String> {
    use std::io::Write as _;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }

    let mut line = serde_json::to_string(value).map_err(|error| error.to_string())?;
    line.push('\n');

    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .and_then(|mut file| file.write_all(line.as_bytes()))
        .map_err(|error| error.to_string())
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

    /// A runtime over a temporary directory, with everything it queued kept for inspection.
    struct Fixture {
        runtime: FactoryRuntime,
        sent: Arc<Mutex<Vec<Queued>>>,
        defaults: layover_core::config::Defaults,
        dir: PathBuf,
    }

    impl Fixture {
        fn new(name: &str) -> Self {
            let config: Config = toml::from_str(FACTORY).expect("the fixture factory parses");
            let graph = RouteGraph::from_config(&config);
            let defaults = config.defaults.clone();
            let dir =
                std::env::temp_dir().join(format!("layover-rt-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("a temporary directory");

            let sent: Arc<Mutex<Vec<Queued>>> = Arc::new(Mutex::new(Vec::new()));
            let sink = Arc::clone(&sent);

            Self {
                runtime: FactoryRuntime::new(
                    Arc::new(config),
                    Arc::new(graph),
                    dir.clone(),
                    Arc::new(move |queued| {
                        sink.lock().map_err(|_| "poisoned".to_owned())?.push(queued);
                        Ok(())
                    }),
                ),
                sent,
                defaults,
                dir,
            }
        }

        fn sent(&self) -> Vec<Queued> {
            self.sent.lock().expect("not poisoned").clone()
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
        }
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
        let fixture = Fixture::new("report");
        let who = session("analyst", 3);

        fixture
            .runtime
            .report(&who, "Found the cause", "It was the cache all along.")
            .expect("writes");

        let written = std::fs::read_to_string(fixture.dir.join("analyst").join("reports.jsonl"))
            .expect("a report file");
        assert!(written.contains("Found the cause"), "{written}");
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
